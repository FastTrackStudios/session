//! The mixer drawn from a frame theme instead of the painters here.
//!
//! A theme is a frame document (`.fm`) whose `mcp.strip` page is one
//! channel strip: its tokens are the colours, its bindings read the
//! track ([`schema`]), its `@name` nodes are the controls a click lands
//! on ([`control`]). See `docs/frame-themes.md`.
//!
//! Loaded from `FTS_THEME` (a path to the `.fm`), in `FTS_THEME_MODE` or
//! the document's first mode, and **hot-reloaded**: [`poll`] notices the
//! file changing and recompiles it, and a widget that sees [`version`]
//! move re-records. A theme that fails to compile leaves the last good
//! one on screen and says why once.
//!
//! The Tone rack is still the painters' own — the theme draws the REAPER
//! part of the strip, below it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant, SystemTime};

use daw_proto::Track;
use theme_runtime::{Fields, Rgba, Schema, Theme, Value};

use crate::arrangement::Palette;
use crate::mcp::Control;

/// The role a theme draws a mixer strip as.
pub const STRIP: &str = "mcp.strip";

/// How often [`poll`] looks at the file.
const POLL: Duration = Duration::from_millis(250);

/// What a strip theme can bind, in slot order.
///
/// Live, because the mixer draws them every frame rather than re-record
/// for them: the values the overlay already draws live (mute, solo, arm,
/// the fader, the meter, which control the pointer is on).
#[must_use]
pub fn schema() -> Schema {
    Schema::new()
        .field("track.name")
        .field("track.label")
        .field("track.number")
        .field("track.color")
        .field("track.has_color")
        .field("track.selected")
        .field("track.folder")
        .field("track.collapsed")
        .field("track.depth")
        .field("track.input")
        .live("track.mute")
        .live("track.solo")
        .live("track.armed")
        .live("track.monitor")
        .live("track.phase")
        .live("track.fader")
        .live("track.volume")
        .live("track.pan")
        .live("track.fx")
        .live("track.sends")
        .live("track.receives")
        .live("meter.left")
        .live("meter.right")
        .live("meter.hold_left")
        .live("meter.hold_right")
        .live("meter.clip")
        .live("ui.hover")
        .live("ui.pressed")
}

/// One strip's values, in [`schema`]'s order.
pub struct StripFields(Vec<Value>);

impl Fields for StripFields {
    fn field(&self, slot: u32) -> Value {
        self.0.field(slot)
    }
}

/// What a strip shows beyond the track itself.
#[derive(Clone, Copy, Default)]
pub struct Shown<'a> {
    /// The name as fitted to the plate.
    pub label: Option<&'a str>,
    pub depth: usize,
    pub collapsed: bool,
    pub level: Option<daw_proto::TrackLevels>,
    pub clipped: bool,
    /// The control the pointer is over, and the one it is pressing.
    pub hover: Option<Control>,
    pub pressed: Option<Control>,
}

/// `track`'s values for a strip theme.
#[must_use]
pub fn fields(palette: &Palette, track: &Track, shown: Shown<'_>) -> StripFields {
    let color = crate::tcp::track_color(palette, track);
    let [r, g, b, a] = color.to_rgba8().to_u8_array();
    let fraction = |v: f32| Value::Num(crate::engine::meter_fraction(v));
    let level = shown.level.unwrap_or_default();
    let named = |c: Option<Control>| c.map_or(Value::text(""), |c| Value::text(name(c)));
    StripFields(vec![
        Value::text(&track.name),
        Value::text(shown.label.unwrap_or(&track.name)),
        Value::Num(f64::from(track.index) + 1.0),
        Value::Color(Rgba::from_rgba8(r, g, b, a)),
        Value::Bool(track.color.is_some()),
        Value::Bool(track.selected),
        Value::Bool(track.is_folder),
        Value::Bool(shown.collapsed),
        Value::Num(crate::num::coord(shown.depth)),
        Value::text(&crate::tcp::record_input(track)),
        Value::Bool(track.muted),
        Value::Bool(track.soloed),
        Value::Bool(track.armed),
        Value::Num(match track.input_monitor {
            daw_proto::InputMonitoringMode::Off => 0.0,
            daw_proto::InputMonitoringMode::Normal => 1.0,
            daw_proto::InputMonitoringMode::NotWhenPlaying => 2.0,
        }),
        Value::Bool(track.phase_inverted),
        Value::Num(crate::tcp::volume_fraction(track.volume)),
        Value::Num(track.volume),
        Value::Num(track.pan.clamp(-1.0, 1.0)),
        Value::Num(f64::from(track.fx_count)),
        Value::Num(f64::from(track.send_count)),
        Value::Num(f64::from(track.receive_count)),
        fraction(level.peak_left),
        fraction(level.peak_right),
        fraction(level.hold_left),
        fraction(level.hold_right),
        Value::Bool(shown.clipped),
        named(shown.hover),
        named(shown.pressed),
    ])
}

/// A control's name in a theme: the `x` of a node named `@x`.
#[must_use]
pub const fn name(control: Control) -> &'static str {
    match control {
        Control::Fx => "fx",
        Control::Pan => "pan",
        Control::RecArm => "arm",
        Control::Mute => "mute",
        Control::Solo => "solo",
        Control::Routing => "routing",
        Control::Volume => "fader",
        Control::Name => "name",
        Control::Monitor => "monitor",
        Control::Clip => "clip",
        Control::Folder => "folder",
    }
}

/// The control a theme's `@name` is.
#[must_use]
pub fn control(name: &str) -> Option<Control> {
    Some(match name {
        "fx" => Control::Fx,
        "pan" => Control::Pan,
        "arm" => Control::RecArm,
        "mute" => Control::Mute,
        "solo" => Control::Solo,
        "routing" => Control::Routing,
        "fader" | "volume" => Control::Volume,
        "name" => Control::Name,
        "monitor" => Control::Monitor,
        "clip" => Control::Clip,
        "folder" => Control::Folder,
        _ => return None,
    })
}

// ── the loaded theme ──────────────────────────────────────────────────

struct Loaded {
    path: PathBuf,
    mode: Option<String>,
    modified: Option<SystemTime>,
    checked: Instant,
    /// The last error reported, so a broken file is reported once.
    reported: Option<String>,
}

static THEME: RwLock<Option<Arc<Theme>>> = RwLock::new(None);
static VERSION: AtomicU64 = AtomicU64::new(0);
static SOURCE: Mutex<Option<Loaded>> = Mutex::new(None);

/// The theme the mixer draws with, when there is one with a strip.
#[must_use]
pub fn current() -> Option<Arc<Theme>> {
    let theme = THEME.read().unwrap_or_else(PoisonError::into_inner).clone()?;
    theme.has_role(STRIP).then_some(theme)
}

/// Bumped whenever [`current`] changes: what a recording compares to
/// know it is stale.
#[must_use]
pub fn version() -> u64 {
    VERSION.load(Ordering::Acquire)
}

/// Use `theme` (or none) from now on.
pub fn set(theme: Option<Arc<Theme>>) {
    *THEME.write().unwrap_or_else(PoisonError::into_inner) = theme;
    VERSION.fetch_add(1, Ordering::AcqRel);
}

/// Load the theme at `path` in `mode` and keep watching it.
///
/// # Errors
/// Why it did not compile; the previous theme stays.
pub fn load(path: PathBuf, mode: Option<String>) -> Result<(), String> {
    let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let result = compile(&path, mode.as_deref());
    *SOURCE.lock().unwrap_or_else(PoisonError::into_inner) =
        Some(Loaded { path, mode, modified, checked: Instant::now(), reported: result.as_ref().err().cloned() });
    result.map(|theme| set(Some(Arc::new(theme))))
}

/// Load `FTS_THEME`, once, if it is set.
pub fn from_env() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let Some(path) = std::env::var_os("FTS_THEME").map(PathBuf::from) else { return };
        let mode = std::env::var("FTS_THEME_MODE").ok();
        if let Err(why) = load(path, mode) {
            tracing::warn!(theme.error = %why, "theme did not load");
        }
    });
}

/// Recompile the theme if its file changed since the last look (looked
/// at no more than every [`POLL`]). True when [`current`] changed.
pub fn poll() -> bool {
    let mut source = SOURCE.lock().unwrap_or_else(PoisonError::into_inner);
    let Some(loaded) = source.as_mut() else { return false };
    if loaded.checked.elapsed() < POLL {
        return false;
    }
    loaded.checked = Instant::now();
    let modified = std::fs::metadata(&loaded.path).and_then(|m| m.modified()).ok();
    if modified == loaded.modified {
        return false;
    }
    loaded.modified = modified;
    match compile(&loaded.path, loaded.mode.as_deref()) {
        Ok(theme) => {
            loaded.reported = None;
            drop(source);
            set(Some(Arc::new(theme)));
            true
        }
        Err(why) => {
            if loaded.reported.as_deref() != Some(why.as_str()) {
                tracing::warn!(theme.error = %why, "theme did not reload; keeping the last good one");
                loaded.reported = Some(why);
            }
            false
        }
    }
}

fn compile(path: &std::path::Path, mode: Option<&str>) -> Result<Theme, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Theme::from_markup(&src, schema(), mode).map_err(|errors| {
        errors.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ")
    })
}
