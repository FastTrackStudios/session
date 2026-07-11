//! Session modes — Organize, Write, Produce, Record, Edit, Mix, Master, Live.
//!
//! Each mode owns a set of REAPER toolbars to show and a layout recipe that
//! places them dynamically based on current monitor dimensions. This file
//! is the foundation: enum + types + state + actions. The toolbar
//! discovery, layout engine, and actual REAPER calls land in follow-ups.

use std::fmt;
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    Organize,
    Write,
    Produce,
    Record,
    Edit,
    Mix,
    Master,
    Live,
    Video,
    /// Scoring mode — saves/loads a native screenset slot like the
    /// others, but doesn't claim any of REAPER's floating toolbar
    /// slots. Intended for running/monitoring multi-agent orchestration
    /// workflows rather than a REAPER editing task.
    Scoring,
}

impl Mode {
    pub const ALL: [Mode; 10] = [
        Mode::Organize,
        Mode::Write,
        Mode::Produce,
        Mode::Record,
        Mode::Edit,
        Mode::Mix,
        Mode::Master,
        Mode::Live,
        Mode::Video,
        Mode::Scoring,
    ];

    /// Stable lowercase identifier used in action IDs.
    pub fn slug(self) -> &'static str {
        match self {
            Mode::Organize => "organize",
            Mode::Write => "write",
            Mode::Produce => "produce",
            Mode::Record => "record",
            Mode::Edit => "edit",
            Mode::Mix => "mix",
            Mode::Master => "master",
            Mode::Live => "live",
            Mode::Video => "video",
            Mode::Scoring => "scoring",
        }
    }

    /// Reverse of [`Mode::slug`]. Case-insensitive, trims whitespace.
    /// Returns `None` for any string that doesn't match a known mode.
    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug.trim().to_ascii_lowercase().as_str() {
            "organize" => Some(Mode::Organize),
            "write" => Some(Mode::Write),
            "produce" => Some(Mode::Produce),
            "record" => Some(Mode::Record),
            "edit" => Some(Mode::Edit),
            "mix" => Some(Mode::Mix),
            "master" => Some(Mode::Master),
            "live" => Some(Mode::Live),
            "video" => Some(Mode::Video),
            "scoring" => Some(Mode::Scoring),
            _ => None,
        }
    }

    /// Title-cased display name.
    pub fn display_name(self) -> &'static str {
        match self {
            Mode::Organize => "Organize",
            Mode::Write => "Write",
            Mode::Produce => "Produce",
            Mode::Record => "Record",
            Mode::Edit => "Edit",
            Mode::Mix => "Mix",
            Mode::Master => "Master",
            Mode::Live => "Live",
            Mode::Video => "Video",
            Mode::Scoring => "Scoring",
        }
    }

    /// Whether this mode owns the standard 3 floating toolbars. Most
    /// modes do; `Scoring` is the lone exception (no toolbars reserved
    /// in `reaper-menu.ini`, no mode-toolbar slots auto-renamed).
    pub fn has_toolbars(self) -> bool {
        !matches!(self, Mode::Scoring)
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

// ─── Layout primitives ──────────────────────────────────────────────────────
//
// Layouts are declared as rules, not snapshots. At apply time the layout
// engine evaluates them against the current monitor dimensions so the same
// mode produces the right layout on a 1080p laptop vs an ultrawide desktop.
// Foundation only — the engine that consumes these is a follow-up.

/// Which REAPER docker holds a docked toolbar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Docker {
    Left,
    Right,
    Top,
    Bottom,
}

/// Anchor point for floating-toolbar positioning, relative to monitor bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Anchor {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    Center,
}

/// How a single toolbar is placed when the mode is active.
#[derive(Debug, Clone)]
pub enum Placement {
    /// Docked into a specific docker. `extent` is width for left/right
    /// dockers, height for top/bottom (REAPER docker pixel size).
    Docked { docker: Docker, extent: u32 },
    /// Floating, anchored to a monitor corner/center with a fixed size.
    Floating {
        anchor: Anchor,
        size: (u32, u32),
        /// Pixel offset from the anchor point. Positive moves inward
        /// (down/right from top-anchors, up/left from bottom-anchors).
        offset: (i32, i32),
    },
}

/// One toolbar's contribution to a mode's layout.
#[derive(Debug, Clone)]
pub struct ToolbarLayout {
    /// Name of the toolbar as configured in REAPER's `reaper-menu.ini`.
    /// Resolved to a floating-toolbar index at toolbar-discovery time.
    pub toolbar_name: String,
    pub placement: Placement,
}

/// What happens when a mode is activated.
#[derive(Debug, Clone, Default)]
pub struct ModeConfig {
    /// Toolbars to show when this mode is active. Toolbars not in any
    /// active mode's list get hidden during transitions.
    pub toolbars: Vec<ToolbarLayout>,
    /// Optional REAPER screenset to apply alongside the toolbar layout.
    /// Use `None` when the toolbar layout alone is sufficient.
    pub screenset_id: Option<String>,
    /// REAPER action command IDs to fire after the layout is applied
    /// (e.g. open mixer, switch to MIDI editor view).
    pub actions_on_apply: Vec<String>,
}

// ─── Mode state ─────────────────────────────────────────────────────────────

static CURRENT: OnceLock<Mutex<Mode>> = OnceLock::new();

fn current_cell() -> &'static Mutex<Mode> {
    CURRENT.get_or_init(|| Mutex::new(Mode::Organize))
}

pub fn current_mode() -> Mode {
    *current_cell().lock().expect("mode mutex poisoned")
}

/// Callback fired after `set_mode` updates the global slot, only when
/// the mode actually changes. Use [`add_mode_change_listener`] to register.
///
/// Stored as `Arc<dyn Fn>` so listeners can be invoked outside the
/// mutex (avoiding re-entrant deadlocks when a listener itself wants
/// to register another listener or read state through a lock).
pub type ModeChangeListener = std::sync::Arc<dyn Fn(Mode) + Send + Sync + 'static>;

static MODE_CHANGE_LISTENERS: OnceLock<Mutex<Vec<ModeChangeListener>>> = OnceLock::new();

fn listeners_cell() -> &'static Mutex<Vec<ModeChangeListener>> {
    MODE_CHANGE_LISTENERS.get_or_init(|| Mutex::new(Vec::new()))
}

/// Register a callback to run whenever [`set_mode`] transitions to a new
/// mode. Listeners run synchronously on the caller's thread after the
/// global slot has been updated and before the window layout is applied,
/// so they can observe the new mode via [`current_mode`].
pub fn add_mode_change_listener<F>(listener: F)
where
    F: Fn(Mode) + Send + Sync + 'static,
{
    if let Ok(mut guard) = listeners_cell().lock() {
        guard.push(std::sync::Arc::new(listener));
    }
}

fn fire_mode_change_listeners(mode: Mode) {
    // Snapshot Arc clones so the lock is dropped before any listener
    // runs — listeners are free to call back into mode_actions or
    // re-acquire the same mutex without deadlocking.
    let listeners: Vec<ModeChangeListener> = match listeners_cell().lock() {
        Ok(g) => g.iter().cloned().collect(),
        Err(_) => return,
    };
    for listener in listeners {
        listener(mode);
    }
}

/// Switch the active mode.
///
/// Updates the global current-mode slot and asks the DAW's WindowManager
/// to apply the layout whose name matches the mode's display name (e.g.
/// `Organize`, `Write`, ...). The layout is resolved against REAPER's
/// `reaper-screensets.ini` and applied via REAPER's native `Screenset:
/// Load #N` action — see `daw_reaper::window_manager`.
///
/// Calls the synchronous trait impl on `daw::reaper::Reaper` directly
/// rather than going through the async RPC client — the apply runs on
/// REAPER's main thread (the only place `main_on_command_ex` is safe
/// to call), and `daw::block_on` would deadlock us against ourselves
/// since the RPC dispatcher would queue work back onto the main thread
/// that's blocked waiting for the RPC to return.
pub fn set_mode(mode: Mode) {
    use daw::service::{WindowLayoutOptions, WindowManager as _};

    let prev = {
        let mut slot = current_cell().lock().expect("mode mutex poisoned");
        let prev = *slot;
        *slot = mode;
        prev
    };
    if prev != mode {
        tracing::info!(from = %prev, to = %mode, "[session] Mode changed");
        fire_mode_change_listeners(mode);
        persist_current_mode(mode);
    } else {
        tracing::debug!(mode = %mode, "[session] set_mode called with current mode (no-op)");
    }

    let result = daw::reaper::Reaper.apply_layout(
        mode.display_name().to_string(),
        WindowLayoutOptions { run_actions: true },
    );
    if result.ok {
        tracing::info!(mode = %mode, "[session] Window layout applied");
    } else {
        tracing::warn!(
            mode = %mode,
            error = result.error.as_deref().unwrap_or("(no error returned)"),
            "[session] Window layout apply reported failure"
        );
    }
}

// ─── Action wiring ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeAction {
    Switch(Mode),
    /// Capture REAPER's current window state into the mode's native
    /// screenset slot (mode index N → REAPER window set #N+1).
    SaveLayout(Mode),
    LogCurrent,
}

pub fn init(_ctx: &daw::module::ModuleContext) {
    // Initialise the global to Organize so `current_mode()` is always sane.
    let _ = current_mode();
}

// ── Mode-change broadcast (for Vox subscribers) ─────────────────────────────

static MODE_BROADCAST: std::sync::OnceLock<tokio::sync::broadcast::Sender<String>> =
    std::sync::OnceLock::new();

/// Process-wide broadcast channel of new mode slugs. Pushed each time
/// `set_mode` transitions to a different mode. Subscribe a receiver to
/// forward into a Vox client `Tx<String>`.
///
/// Lazy-installs the listener that bridges `set_mode` → broadcast the
/// first time it's called, so the channel and its forwarding listener
/// are guaranteed to exist before the first `subscribe` RPC fires.
pub fn mode_broadcast() -> &'static tokio::sync::broadcast::Sender<String> {
    MODE_BROADCAST.get_or_init(|| {
        let (tx, _rx) = tokio::sync::broadcast::channel::<String>(32);
        add_mode_change_listener(|mode| {
            // Safe to call mode_broadcast() recursively — the OnceLock
            // is already past its init call when the listener fires.
            let _ = mode_broadcast().send(mode.slug().to_string());
        });
        tx
    })
}

pub fn action_for_id(action_id: &str) -> Option<ModeAction> {
    // Strip the `fts.session.mode.` prefix produced by `define_actions!`
    // for the `mode_defs` block. Action IDs look like
    // `fts.session.mode.organize`, `fts.session.mode.log_current`, etc.
    let trimmed = action_id.trim().to_lowercase();
    let slug = trimmed
        .strip_prefix("fts.session.mode.")
        .unwrap_or(trimmed.as_str());

    if slug == "log_current" {
        return Some(ModeAction::LogCurrent);
    }
    for mode in Mode::ALL {
        if slug == mode.slug() {
            return Some(ModeAction::Switch(mode));
        }
        if slug == format!("save_{}", mode.slug()) {
            return Some(ModeAction::SaveLayout(mode));
        }
    }
    None
}

pub fn dispatch(action: ModeAction) {
    match action {
        ModeAction::Switch(mode) => set_mode(mode),
        ModeAction::SaveLayout(mode) => save_layout(mode),
        ModeAction::LogCurrent => {
            tracing::info!(mode = %current_mode(), "[session] Current mode");
        }
    }
}

/// Capture REAPER's current window state into the mode's native
/// screenset slot. Calls the sync trait method on `daw::reaper::Reaper`
/// (same pattern as `set_mode`) so it runs on REAPER's main thread.
pub fn save_layout(mode: Mode) {
    use daw::service::{WindowLayout, WindowManager as _};

    let layout = WindowLayout {
        name: mode.display_name().to_string(),
        description: format!("Native screenset capture for {mode} mode"),
        toolbars: Vec::new(),
        actions_on_apply: Vec::new(),
    };
    let result = daw::reaper::Reaper.save_layout(layout);
    if result.ok {
        tracing::info!(mode = %mode, "[session] Layout saved to native screenset");
    } else {
        tracing::warn!(
            mode = %mode,
            error = result.error.as_deref().unwrap_or("(no error)"),
            "[session] Layout save reported failure"
        );
    }
}

// ─── Persistence ────────────────────────────────────────────────────────────

/// REAPER ExtState section + key used for the persisted mode slug.
/// Lives in `reaper-extstate.ini` (global, not per-project) so the
/// extension restores the same mode regardless of which project loads
/// at startup.
const EXTSTATE_SECTION: &str = "FTS_SESSION";
const EXTSTATE_KEY_MODE: &str = "current_mode";

fn persist_current_mode(mode: Mode) {
    use daw::service::ExtState as _;

    // Global ext-state (`reaper-extstate.ini`, persist=true) — not
    // project-scoped, so the mode restores regardless of which project loads.
    let _ = daw::reaper::Reaper.set(EXTSTATE_SECTION, EXTSTATE_KEY_MODE, mode.slug(), true);
    tracing::debug!(mode = %mode, "[session] Persisted mode to extstate");
}

/// Read the persisted mode slug from REAPER ExtState. Returns `None`
/// when no value has been stored yet (first launch) or the stored
/// value doesn't match a known mode.
pub fn persisted_mode() -> Option<Mode> {
    use daw::service::ExtState as _;

    let raw = daw::reaper::Reaper.get(EXTSTATE_SECTION, EXTSTATE_KEY_MODE)?;
    if raw.is_empty() {
        return None;
    }
    Mode::from_slug(&raw)
}

/// Convenience: if there's a persisted mode, switch to it. Intended to
/// run once at extension startup, after session module init but before
/// other listeners depend on the active mode. Returns the restored mode
/// if any.
pub fn restore_persisted_mode() -> Option<Mode> {
    let mode = persisted_mode()?;
    tracing::info!(mode = %mode, "[session] Restoring persisted mode from extstate");
    set_mode(mode);
    Some(mode)
}

// ── architect::actions declaration ──────────────────────────────────────
//
// `ModeAction` / `action_for_id` / `dispatch` above stay put — still the
// live path `daw_module.rs`'s dispatch chain calls into (via `mode_defs`,
// a separate `actions_proto::define_actions!` block so it renders as
// `FTS: Mode - <Name>` rather than nested under Session). Additive
// declarative layer only, mirroring `setlist_actions`'s migration.
// Namespace `FTS_SESSION_MODE` matches `mode_defs`'s `fts.session.mode`
// action-id prefix.

/// Bridges the twenty-one mode actions (10 switch + 10 save-layout + log)
/// onto `#[architect::actions]`. Every method forwards to the existing
/// synchronous `dispatch` — no behavior change, just a declarative front
/// door with real metadata.
pub struct ModeActionsImpl;

#[architect::actions(namespace = "FTS_SESSION_MODE")]
pub trait ModeActions {
    #[action(
        description = "Switch to Organize mode (planning, song structure, setlists)",
        category = "Session",
        group = "Switch"
    )]
    fn organize(&self);
    #[action(
        description = "Switch to Write mode (lyric/melody/idea capture)",
        category = "Session",
        group = "Switch"
    )]
    fn write(&self);
    #[action(
        description = "Switch to Produce mode (arrangement, sound design, instrument selection)",
        category = "Session",
        group = "Switch"
    )]
    fn produce(&self);
    #[action(
        description = "Switch to Record mode (tracking, takes, monitoring)",
        category = "Session",
        group = "Switch"
    )]
    fn record(&self);
    #[action(
        description = "Switch to Edit mode (comping, timing, cleanup)",
        category = "Session",
        group = "Switch"
    )]
    fn edit(&self);
    #[action(
        description = "Switch to Mix mode (mixer focus, processing, automation)",
        category = "Session",
        group = "Switch"
    )]
    fn mix(&self);
    #[action(
        description = "Switch to Master mode (master bus processing, metering, export prep)",
        category = "Session",
        group = "Switch"
    )]
    fn master(&self);
    #[action(
        description = "Switch to Live mode (performance/setlist playback view)",
        category = "Session",
        group = "Switch"
    )]
    fn live(&self);
    #[action(
        description = "Switch to Video mode (sync to picture / video editing layout)",
        category = "Session",
        group = "Switch"
    )]
    fn video(&self);
    #[action(
        description = "Switch to Scoring mode (multi-agent orchestration layout, no mode toolbars)",
        category = "Session",
        group = "Switch"
    )]
    fn scoring(&self);
    #[action(
        description = "Capture current REAPER window state to Organize's native screenset slot",
        category = "Session",
        group = "Save"
    )]
    fn save_organize(&self);
    #[action(
        description = "Capture current REAPER window state to Write's native screenset slot",
        category = "Session",
        group = "Save"
    )]
    fn save_write(&self);
    #[action(
        description = "Capture current REAPER window state to Produce's native screenset slot",
        category = "Session",
        group = "Save"
    )]
    fn save_produce(&self);
    #[action(
        description = "Capture current REAPER window state to Record's native screenset slot",
        category = "Session",
        group = "Save"
    )]
    fn save_record(&self);
    #[action(
        description = "Capture current REAPER window state to Edit's native screenset slot",
        category = "Session",
        group = "Save"
    )]
    fn save_edit(&self);
    #[action(
        description = "Capture current REAPER window state to Mix's native screenset slot",
        category = "Session",
        group = "Save"
    )]
    fn save_mix(&self);
    #[action(
        description = "Capture current REAPER window state to Master's native screenset slot",
        category = "Session",
        group = "Save"
    )]
    fn save_master(&self);
    #[action(
        description = "Capture current REAPER window state to Live's native screenset slot",
        category = "Session",
        group = "Save"
    )]
    fn save_live(&self);
    #[action(
        description = "Capture current REAPER window state to Video's native screenset slot",
        category = "Session",
        group = "Save"
    )]
    fn save_video(&self);
    #[action(
        description = "Capture current REAPER window state to Scoring's native screenset slot",
        category = "Session",
        group = "Save"
    )]
    fn save_scoring(&self);
    #[action(
        description = "Log the current session mode to the console (debug helper)",
        category = "Session",
        group = "Debug"
    )]
    fn log_current(&self);
}

impl ModeActions for ModeActionsImpl {
    fn organize(&self) {
        dispatch(ModeAction::Switch(Mode::Organize));
    }
    fn write(&self) {
        dispatch(ModeAction::Switch(Mode::Write));
    }
    fn produce(&self) {
        dispatch(ModeAction::Switch(Mode::Produce));
    }
    fn record(&self) {
        dispatch(ModeAction::Switch(Mode::Record));
    }
    fn edit(&self) {
        dispatch(ModeAction::Switch(Mode::Edit));
    }
    fn mix(&self) {
        dispatch(ModeAction::Switch(Mode::Mix));
    }
    fn master(&self) {
        dispatch(ModeAction::Switch(Mode::Master));
    }
    fn live(&self) {
        dispatch(ModeAction::Switch(Mode::Live));
    }
    fn video(&self) {
        dispatch(ModeAction::Switch(Mode::Video));
    }
    fn scoring(&self) {
        dispatch(ModeAction::Switch(Mode::Scoring));
    }
    fn save_organize(&self) {
        dispatch(ModeAction::SaveLayout(Mode::Organize));
    }
    fn save_write(&self) {
        dispatch(ModeAction::SaveLayout(Mode::Write));
    }
    fn save_produce(&self) {
        dispatch(ModeAction::SaveLayout(Mode::Produce));
    }
    fn save_record(&self) {
        dispatch(ModeAction::SaveLayout(Mode::Record));
    }
    fn save_edit(&self) {
        dispatch(ModeAction::SaveLayout(Mode::Edit));
    }
    fn save_mix(&self) {
        dispatch(ModeAction::SaveLayout(Mode::Mix));
    }
    fn save_master(&self) {
        dispatch(ModeAction::SaveLayout(Mode::Master));
    }
    fn save_live(&self) {
        dispatch(ModeAction::SaveLayout(Mode::Live));
    }
    fn save_video(&self) {
        dispatch(ModeAction::SaveLayout(Mode::Video));
    }
    fn save_scoring(&self) {
        dispatch(ModeAction::SaveLayout(Mode::Scoring));
    }
    fn log_current(&self) {
        dispatch(ModeAction::LogCurrent);
    }
}

/// Registers all twenty-one mode actions with `backend`.
pub fn register_actions<B>(backend: &B)
where
    B: ::architect::action::ActionBackend + ?Sized,
{
    register_mode_actions_actions(backend, std::sync::Arc::new(ModeActionsImpl));
}
