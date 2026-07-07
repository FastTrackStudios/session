//! Recording-workflow actions.
//!
//! These exist alongside `take_ranking` and `mode_actions` and are
//! registered under the `fts.session.*` action namespace via the
//! `session_actions` `define_actions!` block in `crate::lib`.

use reaper_high::Reaper;
use tracing::info;

/// REAPER native command IDs we call. Kept as named constants here so
/// the handlers below read like a story rather than a sequence of
/// magic numbers.
mod cmd {
    /// `Transport: Stop (DELETE all recorded media)` — abandons the
    /// in-flight recording pass and removes the media that was just
    /// captured. Used as the first half of `RestartRecording`.
    pub const STOP_DELETE: u32 = 40668;
    /// `Transport: Record` — starts a new recording pass with the
    /// current arm / monitor / input-quantize settings. Pressed again
    /// while recording, it stops — i.e. it toggles.
    pub const RECORD: u32 = 1013;
    /// `Transport: Stop` — stops playback/recording, keeping captured media.
    pub const STOP: u32 = 1016;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordAction {
    /// "Bad take, do it again" — stop and discard the just-captured
    /// media, then immediately start a new recording pass.
    RestartRecording,
    /// Toggle record monitor on selected tracks between **on (1)** and
    /// **off (0)** only — skips the auto/tape state that REAPER's
    /// native cycle action (40495) walks through. Semantics: if any
    /// selected track is currently in the "on" state, set all to off;
    /// otherwise set all to on.
    ToggleMonitorOnOff,
    /// Toggle record monitor on selected tracks between **auto/tape
    /// (2)** and **off (0)**. Same any/all semantics as
    /// [`Self::ToggleMonitorOnOff`].
    ToggleMonitorTapeOff,
    /// Start recording into the focused project (the current song's tab).
    Record,
    /// Stop the transport (keeps captured media).
    StopRecording,
    /// Toggle recording in the focused project.
    ToggleRecording,
    /// Arm the selected tracks (set `I_RECARM` = 1) in the focused project.
    ArmSelected,
    /// Disarm the selected tracks (set `I_RECARM` = 0) in the focused project.
    DisarmSelected,
}

pub fn action_for_id(action_id: &str) -> Option<RecordAction> {
    let slug = action_id
        .trim()
        .to_lowercase()
        .strip_prefix("fts.session.")
        .map(str::to_string)
        .unwrap_or_else(|| action_id.to_lowercase());
    match slug.as_str() {
        "record_restart" => Some(RecordAction::RestartRecording),
        "monitor_toggle_on_off" => Some(RecordAction::ToggleMonitorOnOff),
        "monitor_toggle_tape_off" => Some(RecordAction::ToggleMonitorTapeOff),
        "record" => Some(RecordAction::Record),
        "record_stop" => Some(RecordAction::StopRecording),
        "record_toggle" => Some(RecordAction::ToggleRecording),
        "arm_selected" => Some(RecordAction::ArmSelected),
        "disarm_selected" => Some(RecordAction::DisarmSelected),
        _ => None,
    }
}

pub fn dispatch(action: RecordAction) {
    match action {
        RecordAction::RestartRecording => restart_recording(),
        RecordAction::ToggleMonitorOnOff => toggle_monitor_between(MONITOR_ON, MONITOR_OFF),
        RecordAction::ToggleMonitorTapeOff => toggle_monitor_between(MONITOR_TAPE, MONITOR_OFF),
        RecordAction::Record => run_transport(cmd::RECORD, "Record"),
        RecordAction::StopRecording => run_transport(cmd::STOP, "Stop"),
        RecordAction::ToggleRecording => run_transport(cmd::RECORD, "Toggle record"),
        RecordAction::ArmSelected => set_arm_selected(true),
        RecordAction::DisarmSelected => set_arm_selected(false),
    }
}

/// Fire a transport command on REAPER's main thread.
fn run_transport(command: u32, label: &str) {
    let reaper = Reaper::get();
    let low = reaper.medium_reaper().low();
    unsafe {
        low.Main_OnCommand(command as i32, 0);
    }
    info!("[record] {label}");
}

/// Set `I_RECARM` on every selected track (1 = armed, 0 = disarmed) in the
/// focused project. Mirrors the monitor toggle's selection-scoped approach.
fn set_arm_selected(armed: bool) {
    let reaper = Reaper::get();
    let low = reaper.medium_reaper().low();
    let recarm = b"I_RECARM\0".as_ptr() as *const _;
    let target = if armed { 1.0 } else { 0.0 };

    unsafe {
        let n = low.CountSelectedTracks(std::ptr::null_mut());
        if n == 0 {
            info!("[record] arm: no tracks selected");
            return;
        }
        let undo = std::ffi::CString::new(if armed {
            "Arm selected tracks"
        } else {
            "Disarm selected tracks"
        })
        .unwrap();
        low.Undo_BeginBlock();
        low.PreventUIRefresh(1);
        for i in 0..n {
            let t = low.GetSelectedTrack(std::ptr::null_mut(), i);
            if !t.is_null() {
                low.SetMediaTrackInfo_Value(t, recarm, target);
            }
        }
        low.PreventUIRefresh(-1);
        low.Undo_EndBlock(undo.as_ptr(), 1); // 1 = track config changes
        info!(
            armed,
            tracks = n,
            "[record] Set record arm on selected tracks"
        );
    }
}

// REAPER `I_RECMON` values: 0 = off, 1 = normal, 2 = auto/tape.
const MONITOR_OFF: f64 = 0.0;
const MONITOR_ON: f64 = 1.0;
const MONITOR_TAPE: f64 = 2.0;

/// Toggle `I_RECMON` on every selected track between `a` and `b`.
///
/// Semantic: if *any* selected track is currently set to `a`, push them
/// all to `b`; otherwise push them all to `a`. This avoids the
/// per-track flip that would leave a multi-track selection in mixed
/// states and gives a predictable "master switch" feel.
fn toggle_monitor_between(a: f64, b: f64) {
    let reaper = Reaper::get();
    let low = reaper.medium_reaper().low();
    let recmon = b"I_RECMON\0".as_ptr() as *const _;

    unsafe {
        let n = low.CountSelectedTracks(std::ptr::null_mut());
        if n == 0 {
            return;
        }

        // Collect first so SetMediaTrackInfo_Value can't perturb iteration.
        let mut tracks = Vec::with_capacity(n as usize);
        for i in 0..n {
            let t = low.GetSelectedTrack(std::ptr::null_mut(), i);
            if !t.is_null() {
                tracks.push(t);
            }
        }

        let any_at_a = tracks
            .iter()
            .any(|t| low.GetMediaTrackInfo_Value(*t, recmon) == a);
        let target = if any_at_a { b } else { a };

        let undo = std::ffi::CString::new(format!(
            "Toggle record monitor ({} ↔ {})",
            monitor_label(a),
            monitor_label(b)
        ))
        .unwrap();
        low.Undo_BeginBlock();
        low.PreventUIRefresh(1);
        for t in &tracks {
            low.SetMediaTrackInfo_Value(*t, recmon, target);
        }
        low.PreventUIRefresh(-1);
        low.Undo_EndBlock(undo.as_ptr(), 1); // 1 = track config changes
        info!(
            tracks = tracks.len(),
            new_state = target,
            "[record] Set record monitor"
        );
    }
}

fn monitor_label(v: f64) -> &'static str {
    match v as i32 {
        0 => "off",
        1 => "on",
        2 => "auto/tape",
        _ => "?",
    }
}

/// Stop the current recording (discarding anything captured this pass)
/// and immediately start a fresh recording pass. The two REAPER calls
/// happen back-to-back on the main thread, wrapped in a single undo
/// block so the project history records one "Restart Recording"
/// entry rather than two separate transport events.
fn restart_recording() {
    let reaper = Reaper::get();
    let low = reaper.medium_reaper().low();
    let undo = std::ffi::CString::new("Restart recording").unwrap();
    unsafe {
        low.Undo_BeginBlock();
        low.Main_OnCommand(cmd::STOP_DELETE as i32, 0);
        low.Main_OnCommand(cmd::RECORD as i32, 0);
        low.Undo_EndBlock(undo.as_ptr(), -1);
    }
    info!("[record] Restarted recording (stop+delete then record)");
}

// ── architect::actions declaration ──────────────────────────────────────
//
// `RecordAction` / `action_for_id` / `dispatch` above stay put — still the
// live path `daw_module.rs`'s dispatch chain calls into. Additive
// declarative layer only, mirroring `setlist_actions`'s migration.

/// Bridges the eight recording-workflow actions onto
/// `#[architect::actions]`. Every method forwards to the existing
/// synchronous `dispatch` — no behavior change, just a declarative front
/// door with real metadata.
pub struct RecordActionsImpl;

#[architect::actions(namespace = "FTS_SESSION")]
pub trait RecordActions {
    #[action(
        description = "Start a recording pass in the focused project — the current song's tab. Uses the existing arm / monitor / input settings.",
        category = "Transport",
        group = "Recording"
    )]
    fn record(&self);
    #[action(
        description = "Stop the transport in the focused project, keeping the media captured this pass.",
        category = "Transport",
        group = "Recording"
    )]
    fn record_stop(&self);
    #[action(
        description = "Toggle recording in the focused project — the current song's tab.",
        category = "Transport",
        group = "Recording"
    )]
    fn record_toggle(&self);
    #[action(
        description = "Arm every selected track (I_RECARM = 1) in the focused project so it captures input on the next recording pass.",
        category = "Tracks",
        group = "Recording"
    )]
    fn arm_selected(&self);
    #[action(
        description = "Disarm every selected track (I_RECARM = 0) in the focused project.",
        category = "Tracks",
        group = "Recording"
    )]
    fn disarm_selected(&self);
    #[action(
        description = "Stop the current recording (DELETE all recorded media this pass) and immediately start a fresh recording pass. For aborting a bad take without leaving stray media behind.",
        category = "Transport",
        group = "Recording"
    )]
    fn record_restart(&self);
    #[action(
        description = "Toggle the record-monitor state of every selected track between 'on' and 'off' only, skipping the auto/tape state that REAPER's native cycle action walks through. If any selected track is currently 'on', all go to off; otherwise all go to on.",
        category = "Tracks",
        group = "Recording"
    )]
    fn monitor_toggle_on_off(&self);
    #[action(
        description = "Toggle the record-monitor state of every selected track between 'auto/tape' (monitor input only while recording) and 'off'. If any selected track is currently 'auto/tape', all go to off; otherwise all go to auto/tape.",
        category = "Tracks",
        group = "Recording"
    )]
    fn monitor_toggle_tape_off(&self);
}

impl RecordActions for RecordActionsImpl {
    fn record(&self) {
        dispatch(RecordAction::Record);
    }
    fn record_stop(&self) {
        dispatch(RecordAction::StopRecording);
    }
    fn record_toggle(&self) {
        dispatch(RecordAction::ToggleRecording);
    }
    fn arm_selected(&self) {
        dispatch(RecordAction::ArmSelected);
    }
    fn disarm_selected(&self) {
        dispatch(RecordAction::DisarmSelected);
    }
    fn record_restart(&self) {
        dispatch(RecordAction::RestartRecording);
    }
    fn monitor_toggle_on_off(&self) {
        dispatch(RecordAction::ToggleMonitorOnOff);
    }
    fn monitor_toggle_tape_off(&self) {
        dispatch(RecordAction::ToggleMonitorTapeOff);
    }
}

/// Registers all eight recording-workflow actions with `backend`.
pub fn register_actions<B>(backend: &B)
where
    B: ::architect::action::ActionBackend + ?Sized,
{
    register_record_actions_actions(backend, std::sync::Arc::new(RecordActionsImpl));
}
