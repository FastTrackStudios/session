//! Changes, on their way to the engine.
//!
//! The window does not own the session — `daw-standalone` does. A fader
//! drag is not "move the fader", it is "tell the engine this track's
//! volume is now this", and the fader moves because the engine says so.
//!
//! That ordering is the whole point. It is what will let a second
//! window, or a real REAPER over `daw-sync`, see the same change: the
//! engine is the thing both are looking at, and a UI that mutated its
//! own copy would be a UI that disagrees with everyone else the moment
//! anything else touches the session.
//!
//! # Why the edits queue rather than being sent
//!
//! A fader drag fires an event per mouse move — a hundred a second on a
//! trackpad. Sent one RPC each, that is a hundred round trips to
//! deliver one number, ninety-nine of which are already stale when they
//! arrive.
//!
//! So continuous edits COALESCE: the queue keeps the latest value per
//! track and control, and the worker sends what is current when it gets
//! to it. Discrete edits — a mute, a rename — never coalesce, because
//! two mute toggles are two different intentions and collapsing them
//! into one would silently drop half of a double-click.

use std::collections::VecDeque;

/// Something the user did that the engine needs to know about.
#[derive(Clone, PartialEq, Debug)]
pub enum Edit {
    ToggleMute(String),
    ToggleSolo(String),
    ToggleArm(String),
    Select(String),
    /// A gain, not a dB value — `Track::volume`'s own unit.
    SetVolume(String, f64),
    SetPan(String, f64),
    Rename(String, String),
}

impl Edit {
    /// The track this is about.
    #[must_use]
    pub fn guid(&self) -> &str {
        match self {
            Self::ToggleMute(g)
            | Self::ToggleSolo(g)
            | Self::ToggleArm(g)
            | Self::Select(g)
            | Self::SetVolume(g, _)
            | Self::SetPan(g, _)
            | Self::Rename(g, _) => g,
        }
    }

    /// Whether a newer edit of this kind replaces this one.
    ///
    /// True for the things a drag produces a stream of, false for the
    /// things a click produces one of. Two volumes are one volume; two
    /// mutes are two mutes, and collapsing them would eat half of a
    /// double-click.
    #[must_use]
    pub const fn coalesces(&self) -> bool {
        matches!(self, Self::SetVolume(..) | Self::SetPan(..))
    }

    /// Whether `other` is the same control on the same track.
    #[must_use]
    fn same_control_as(&self, other: &Self) -> bool {
        self.guid() == other.guid()
            && std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// Edits waiting to go out.
#[derive(Debug, Default)]
pub struct Queue {
    pending: VecDeque<Edit>,
}

impl Queue {
    /// Add an edit, replacing a superseded one if there is one.
    pub fn push(&mut self, edit: Edit) {
        if edit.coalesces() {
            if let Some(slot) = self
                .pending
                .iter_mut()
                .find(|queued| queued.same_control_as(&edit))
            {
                *slot = edit;
                return;
            }
        }
        self.pending.push_back(edit);
    }

    /// The next edit to send.
    pub fn pop(&mut self) -> Option<Edit> {
        self.pending.pop_front()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// What a gesture on a control means for the engine.
///
/// Returns `None` where the control has no engine-side effect yet — the
/// FX pill opens a chain nothing hosts, and routing opens a matrix that
/// does not exist. They are hit-testable and will mean something; they
/// do not mean anything now, and inventing an edit for them would be
/// worse than doing nothing.
#[must_use]
pub fn click(control: crate::mcp::Control, guid: &str) -> Option<Edit> {
    let guid = guid.to_owned();
    match control {
        crate::mcp::Control::Mute => Some(Edit::ToggleMute(guid)),
        crate::mcp::Control::Solo => Some(Edit::ToggleSolo(guid)),
        crate::mcp::Control::RecArm => Some(Edit::ToggleArm(guid)),
        // Clicking the name selects the track; DOUBLE-clicking renames
        // it, which is a different gesture and a different edit.
        crate::mcp::Control::Name => Some(Edit::Select(guid)),
        crate::mcp::Control::Volume
        | crate::mcp::Control::Pan
        | crate::mcp::Control::Fx
        | crate::mcp::Control::Routing => None,
    }
}

/// What a drag on a control means, given how far it moved.
///
/// `fraction` is the drag as a share of the control's travel, already
/// signed so that up is more — see `gesture::drag_fraction`.
#[must_use]
pub fn drag(
    control: crate::mcp::Control,
    guid: &str,
    from: &daw_proto::Track,
    fraction: f64,
) -> Option<Edit> {
    match control {
        crate::mcp::Control::Volume => {
            // Through the fader's own taper, so dragging a third of the
            // way up moves it a third of the way up the SCALE rather
            // than a third of the way up the gain — which at the top of
            // a fader is most of its range and at the bottom is none.
            let at = daw_theme_art::paint::tcp::gain_norm(from.volume);
            let to = (at + fraction).clamp(0.0, 1.0);
            let db = daw_theme_art::paint::tcp::fader_db(to);
            Some(Edit::SetVolume(guid.to_owned(), db_to_gain(db)))
        }
        crate::mcp::Control::Pan => {
            // Pan is linear and two-sided, so the same drag is worth
            // twice as much of its range as it is of a fader's.
            let to = (from.pan + fraction * 2.0).clamp(-1.0, 1.0);
            Some(Edit::SetPan(guid.to_owned(), to))
        }
        _ => None,
    }
}

/// Where the transport is, polled off the event loop.
///
/// The engine is asked once per frame rather than subscribed to,
/// because a position is a LEVEL and not an event: missing one is
/// harmless (the next is along in a few milliseconds) and the
/// extrapolation covers the gap. A subscription would deliver a
/// backlog after a stall, which is the one thing a playhead must not
/// replay.
pub struct Transport {
    state: std::sync::Arc<std::sync::Mutex<(f64, bool)>>,
}

impl Transport {
    /// Start polling. `None` if the facade is not up.
    #[must_use]
    pub fn start() -> Option<Self> {
        let runtime = crate::open::runtime()?;
        let state = std::sync::Arc::new(std::sync::Mutex::new((0.0, false)));
        let writer = std::sync::Arc::clone(&state);
        std::thread::Builder::new()
            .name("session-daw-transport".into())
            .spawn(move || {
                loop {
                    let read = runtime.block_on(async {
                        let daw = daw::rpc::Daw::try_get()?;
                        let project = daw.current_project().await.ok()?;
                        let transport = project.transport();
                        let at = transport.get_position().await.ok()?;
                        let playing = transport.is_playing().await.ok()?;
                        Some((at, playing))
                    });
                    if let Some(read) = read {
                        if let Ok(mut slot) = writer.lock() {
                            *slot = read;
                        }
                    }
                    // Faster than an audio block, slower than a frame:
                    // polling per frame would ask the engine 240 times
                    // a second for a number that changes 40 times, and
                    // the extrapolation exists precisely so it does not
                    // have to be asked more often than it moves.
                    std::thread::sleep(std::time::Duration::from_millis(8));
                }
            })
            .ok()?;
        Some(Self { state })
    }

    /// The last position read, and whether it is moving.
    #[must_use]
    pub fn read(&self) -> (f64, bool) {
        self.state.lock().map_or((0.0, false), |slot| *slot)
    }
}

/// A dB value as a linear gain. `Track::volume`'s unit.
#[must_use]
pub fn db_to_gain(db: f64) -> f64 {
    // Silence rather than an infinitesimal gain: the bottom of the
    // fader means off, and a gain of 1e-14 is off with extra steps.
    if db <= daw_theme_art::paint::tcp::FADER_BOTTOM_DB {
        return 0.0;
    }
    10.0_f64.powf(db / 20.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::Control;

    fn track(volume: f64, pan: f64) -> daw_proto::Track {
        daw_proto::Track {
            volume,
            pan,
            ..daw_proto::Track::default()
        }
    }

    /// The point of the queue: a hundred fader positions are one fader
    /// position by the time anything is sent.
    #[test]
    fn a_drag_collapses_to_its_latest_value() {
        let mut q = Queue::default();
        for i in 1..=100 {
            q.push(Edit::SetVolume("kick".into(), f64::from(i) / 100.0));
        }
        assert_eq!(q.len(), 1);
        assert_eq!(q.pop(), Some(Edit::SetVolume("kick".into(), 1.0)));
    }

    /// But two mutes are two mutes. Collapsing them would eat half of a
    /// double-click, and a mute that toggles once for two clicks is
    /// worse than one that lags.
    #[test]
    fn clicks_never_collapse() {
        let mut q = Queue::default();
        q.push(Edit::ToggleMute("kick".into()));
        q.push(Edit::ToggleMute("kick".into()));
        assert_eq!(q.len(), 2);
    }

    /// Coalescing is per track AND per control: dragging the kick's
    /// fader must not swallow the snare's.
    #[test]
    fn coalescing_does_not_cross_tracks_or_controls() {
        let mut q = Queue::default();
        q.push(Edit::SetVolume("kick".into(), 0.5));
        q.push(Edit::SetVolume("snare".into(), 0.5));
        q.push(Edit::SetPan("kick".into(), 0.2));
        assert_eq!(q.len(), 3);
        q.push(Edit::SetVolume("kick".into(), 0.9));
        assert_eq!(q.len(), 3, "the kick's volume should have been replaced");
    }

    /// Order is kept for everything that did not coalesce — a mute
    /// before a solo must arrive before it.
    #[test]
    fn order_survives() {
        let mut q = Queue::default();
        q.push(Edit::ToggleMute("a".into()));
        q.push(Edit::ToggleSolo("a".into()));
        assert_eq!(q.pop(), Some(Edit::ToggleMute("a".into())));
        assert_eq!(q.pop(), Some(Edit::ToggleSolo("a".into())));
        assert!(q.is_empty());
    }

    /// A fader drag moves along the SCALE, not along the gain. A third
    /// of the way up is a third of the way up the dB scale — up near
    /// the top a third of the gain would be most of the range, and down
    /// near the bottom almost none of it.
    #[test]
    fn a_fader_drag_moves_in_decibels() {
        let unity = track(1.0, 0.0);
        let Some(Edit::SetVolume(_, gain)) = drag(Control::Volume, "k", &unity, -0.25) else {
            panic!("expected a volume edit");
        };
        let db = 20.0 * gain.log10();
        let span = daw_theme_art::paint::tcp::FADER_TOP_DB
            - daw_theme_art::paint::tcp::FADER_BOTTOM_DB;
        assert!(
            (db + span * 0.25).abs() < 0.01,
            "a quarter of the travel should be a quarter of the scale: {db} dB"
        );
    }

    /// The ends hold. A fader cannot be dragged past unity or below
    /// silence however far the pointer goes.
    #[test]
    fn a_fader_stops_at_both_ends() {
        let unity = track(1.0, 0.0);
        let Some(Edit::SetVolume(_, up)) = drag(Control::Volume, "k", &unity, 5.0) else {
            panic!("expected a volume edit");
        };
        assert!((up - 1.0).abs() < 1e-6, "should have stopped at unity: {up}");

        let Some(Edit::SetVolume(_, down)) = drag(Control::Volume, "k", &unity, -5.0) else {
            panic!("expected a volume edit");
        };
        assert!(down.abs() < 1e-9, "should have reached silence: {down}");
    }

    /// The bottom of the fader is OFF, not a gain too small to hear.
    #[test]
    fn the_bottom_of_the_fader_is_silence() {
        assert!(db_to_gain(daw_theme_art::paint::tcp::FADER_BOTTOM_DB).abs() < f64::EPSILON);
        assert!((db_to_gain(0.0) - 1.0).abs() < 1e-9);
    }

    /// Pan is two-sided and clamps at both ends.
    #[test]
    fn pan_runs_from_hard_left_to_hard_right() {
        let centred = track(1.0, 0.0);
        let Some(Edit::SetPan(_, right)) = drag(Control::Pan, "k", &centred, 9.0) else {
            panic!("expected a pan edit");
        };
        assert!((right - 1.0).abs() < f64::EPSILON);
        let Some(Edit::SetPan(_, left)) = drag(Control::Pan, "k", &centred, -9.0) else {
            panic!("expected a pan edit");
        };
        assert!((left + 1.0).abs() < f64::EPSILON);
    }

    /// Controls with nothing behind them yet produce no edit, rather
    /// than an invented one.
    #[test]
    fn unbacked_controls_do_nothing() {
        assert!(click(Control::Fx, "k").is_none());
        assert!(click(Control::Routing, "k").is_none());
        assert!(drag(Control::Mute, "k", &track(1.0, 0.0), 0.5).is_none());
    }

    /// Clicking a name selects; renaming is a different gesture.
    #[test]
    fn clicking_a_name_selects_it() {
        assert_eq!(click(Control::Name, "k"), Some(Edit::Select("k".into())));
    }
}

/// Applies edits to the engine, off the event loop.
///
/// The window's thread must not block: a `set_volume` that waited for
/// the engine would stall the frame, and stalling the frame while
/// dragging a fader is the one moment a user notices a stall most. So
/// edits go down a channel and a worker applies them.
///
/// One worker, not a task per edit, so edits arrive in the order they
/// were made. Two mutes racing would be a mute that ends up in the
/// wrong state and no way to tell which.
pub struct Applier {
    edits: std::sync::mpsc::Sender<Edit>,
}

impl Applier {
    /// Start the worker. `None` if the facade is not up, in which case
    /// the window runs read-only rather than pretending to edit.
    #[must_use]
    pub fn start() -> Option<Self> {
        let runtime = crate::open::runtime()?;
        let (tx, rx) = std::sync::mpsc::channel::<Edit>();
        std::thread::Builder::new()
            .name("session-daw-edits".into())
            .spawn(move || {
                // Drain, coalesce, apply. Draining first is what makes
                // the coalescing work: a hundred queued fader positions
                // become one before any of them is sent.
                let mut queue = Queue::default();
                while let Ok(first) = rx.recv() {
                    queue.push(first);
                    while let Ok(more) = rx.try_recv() {
                        queue.push(more);
                    }
                    while let Some(edit) = queue.pop() {
                        runtime.block_on(apply(&edit));
                    }
                }
            })
            .ok()?;
        Some(Self { edits: tx })
    }

    /// Send an edit. Dropped if the worker has gone, because a window
    /// that could not edit is better than one that panics trying.
    pub fn send(&self, edit: Edit) {
        let _ = self.edits.send(edit);
    }
}

/// One edit, against the engine.
async fn apply(edit: &Edit) {
    let Some(daw) = daw::rpc::Daw::try_get() else {
        return;
    };
    let Ok(project) = daw.current_project().await else {
        return;
    };
    let Ok(Some(track)) = project.tracks().by_guid(edit.guid()).await else {
        // The track went away between the click and the apply — a
        // project reload, or another client removing it. Nothing to
        // report: the session simply no longer has what was clicked.
        return;
    };
    let outcome = match edit {
        Edit::ToggleMute(_) => track.toggle_mute().await,
        Edit::ToggleSolo(_) => track.toggle_solo().await,
        Edit::ToggleArm(_) => track.toggle_arm().await,
        Edit::Select(_) => track.select().await,
        Edit::SetVolume(_, v) => track.set_volume(*v).await,
        Edit::SetPan(_, p) => track.set_pan(*p).await,
        Edit::Rename(_, name) => track.rename(name).await,
    };
    if let Err(error) = outcome {
        // One line, because a failed edit is a thing the user did that
        // did not happen — silence here is how a mixer starts lying.
        tracing::warn!(error = %error, edit = ?edit, "the engine refused an edit");
    }
}
