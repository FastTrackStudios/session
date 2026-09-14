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
    /// Select this track and nothing else — what a plain click means
    /// everywhere, and what makes "focus the selected track" a focus
    /// rather than an accumulation.
    Select(String),
    /// Add this track to the selection, leaving the rest alone. The
    /// modified click: REAPER's Ctrl, and the same modifier that makes
    /// a drag fine — they apply to different gestures, so one key does
    /// both without ambiguity.
    AddToSelection(String),
    /// A gain, not a dB value — `Track::volume`'s own unit.
    SetVolume(String, f64),
    SetPan(String, f64),
    Rename(String, String),
    /// Polarity. The one control on a strip that changes the signal
    /// without changing a level, which is why it sits on its own in
    /// the corner rather than in the button column.
    ///
    /// Carries the value rather than being a toggle, because the engine
    /// has a setter and not a toggle for it — and because the value the
    /// window is already SHOWING is the one the user meant. Re-reading
    /// the state to invert it would let a click disagree with the
    /// control it landed on.
    SetPhase(String, bool),
    /// Whether the track's input is monitored, and when.
    ///
    /// Three states rather than a toggle, because REAPER's is three:
    /// off, on, and on-except-while-playing. A click walks them.
    SetInputMonitor(String, daw_proto::track::InputMonitoringMode),
    /// Whether the track feeds its parent at all.
    ///
    /// A folder's children normally sum into it; a track with this off
    /// is heard only through its own sends, which is how a parallel
    /// path is built. The routing widget's first lane says which.
    SetParentSend(String, bool),
    /// An ITEM's fade-in: its length in seconds and its shape. The guid
    /// is the item's, not a track's.
    SetFadeIn(String, f64, daw_proto::item::FadeShape),
    /// And its fade-out.
    SetFadeOut(String, f64, daw_proto::item::FadeShape),
    /// Select an item — alone, or added to the selection.
    SelectItem(String, bool),
    /// Nothing selected. Carries no guid; the string is empty.
    DeselectAllItems(String),
    /// Everything selected. The same.
    SelectAllItems(String),
    /// Move an item to a position, in seconds.
    MoveItem(String, f64),
    /// Trim an item to a position and a length, in seconds.
    TrimItem(String, f64, f64),
    /// Split an item at a time: the item keeps the left, and a new one
    /// on the same track (its guid is the third field) takes the right.
    SplitItem(String, f64, String),
    DeleteItem(String),
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
            | Self::AddToSelection(g)
            | Self::SetVolume(g, _)
            | Self::SetPan(g, _)
            | Self::Rename(g, _)
            | Self::SetPhase(g, _)
            | Self::SetInputMonitor(g, _)
            | Self::SetParentSend(g, _)
            | Self::SetFadeIn(g, ..)
            | Self::SetFadeOut(g, ..)
            | Self::SelectItem(g, _)
            | Self::DeselectAllItems(g)
            | Self::SelectAllItems(g)
            | Self::MoveItem(g, _)
            | Self::TrimItem(g, ..)
            | Self::SplitItem(g, ..)
            | Self::DeleteItem(g) => g,
        }
    }

    /// Whether this is about an item rather than a track.
    #[must_use]
    pub const fn is_item(&self) -> bool {
        matches!(
            self,
            Self::SetFadeIn(..)
                | Self::SetFadeOut(..)
                | Self::SelectItem(..)
                | Self::DeselectAllItems(_)
                | Self::SelectAllItems(_)
                | Self::MoveItem(..)
                | Self::TrimItem(..)
                | Self::SplitItem(..)
                | Self::DeleteItem(_)
        )
    }

    /// Whether a newer edit of this kind replaces this one.
    ///
    /// True for the things a drag produces a stream of, false for the
    /// things a click produces one of. Two volumes are one volume; two
    /// mutes are two mutes, and collapsing them would eat half of a
    /// double-click.
    #[must_use]
    pub const fn coalesces(&self) -> bool {
        matches!(
            self,
            Self::SetVolume(..)
                | Self::SetPan(..)
                | Self::SetFadeIn(..)
                | Self::SetFadeOut(..)
                | Self::MoveItem(..)
                | Self::TrimItem(..)
        )
    }

    /// Whether `other` is the same control on the same track.
    #[must_use]
    fn same_control_as(&self, other: &Self) -> bool {
        self.guid() == other.guid() && std::mem::discriminant(self) == std::mem::discriminant(other)
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
pub fn click(
    control: crate::mcp::Control,
    guid: &str,
    from: &daw_proto::Track,
    add: bool,
) -> Option<Edit> {
    let guid = guid.to_owned();
    match control {
        crate::mcp::Control::Mute => Some(Edit::ToggleMute(guid)),
        crate::mcp::Control::Solo => Some(Edit::ToggleSolo(guid)),
        crate::mcp::Control::RecArm => Some(Edit::ToggleArm(guid)),
        // Clicking the name selects the track; DOUBLE-clicking renames
        // it, which is a different gesture and a different edit.
        crate::mcp::Control::Name => Some(if add {
            Edit::AddToSelection(guid)
        } else {
            Edit::Select(guid)
        }),
        crate::mcp::Control::Routing => Some(Edit::SetParentSend(guid, !from.parent_send)),
        // The FX button opens a chain window, and there is no chain and
        // no window — see `tone::placeholder` and `bin/chain-probe`.
        // Binding it to something else would be a button that does the
        // wrong thing rather than one that waits.
        crate::mcp::Control::Volume | crate::mcp::Control::Pan | crate::mcp::Control::Fx => None,
        // Clearing a clip changes nothing about the track — it is the
        // window forgetting something, not the engine being told
        // something. Handled where the latch lives.
        crate::mcp::Control::Clip => None,
        // Folding is the window's: it changes which strips exist, not
        // what any track is — see the window's fold.
        crate::mcp::Control::Folder => None,
        // The lamp is only drawn on an armed track, and what is not
        // drawn is not clicked.
        crate::mcp::Control::Monitor => from
            .armed
            .then(|| Edit::SetInputMonitor(guid, next_monitor(from.input_monitor))),
    }
}

/// The monitoring mode a click moves to.
///
/// Off → on → on-except-while-playing → off. REAPER's own order, and
/// the one that puts the mode you reach for most one click away.
#[must_use]
pub const fn next_monitor(
    from: daw_proto::track::InputMonitoringMode,
) -> daw_proto::track::InputMonitoringMode {
    use daw_proto::track::InputMonitoringMode as M;
    match from {
        M::Off => M::Normal,
        M::Normal => M::NotWhenPlaying,
        M::NotWhenPlaying => M::Off,
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

/// What the transport has been asked to do.
///
/// A command, not a state: the engine owns whether it is playing, and
/// this window asks. Sending "play" while it is already playing has to
/// be harmless, which is why these are the engine's own verbs rather
/// than a boolean this side would have to keep in step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Move {
    /// Play if stopped, stop if playing.
    PlayStop,
    /// Back to the start, whether or not it is moving.
    Home,
    /// Move the play position to a time — what a click on the ruler
    /// means once the transport is following it.
    Seek,
}

/// Send a transport command, off the event loop.
///
/// Fire and forget, like [`Applier`]: the transport's own poll is what
/// tells the window what happened, so waiting for the call to return
/// would be waiting for news the window is already subscribed to.
pub fn transport(command: Move, seconds: f64) {
    let Some(runtime) = crate::open::runtime() else {
        return;
    };
    std::thread::Builder::new()
        .name("session-daw-transport-cmd".into())
        .spawn(move || {
            runtime.block_on(async move {
                let Some(daw) = daw::rpc::Daw::try_get() else {
                    return;
                };
                let Ok(project) = daw.current_project().await else {
                    return;
                };
                let transport = project.transport();
                let outcome = match command {
                    Move::PlayStop => transport.play_stop().await,
                    Move::Home => transport.goto_start().await,
                    Move::Seek => transport.set_position(seconds.max(0.0)).await,
                };
                if let Err(error) = outcome {
                    tracing::warn!(error = %error, command = ?command, "the transport refused");
                }
            });
        })
        .ok();
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
        let span =
            daw_theme_art::paint::tcp::FADER_TOP_DB - daw_theme_art::paint::tcp::FADER_BOTTOM_DB;
        assert!(
            (db + span * 0.25).abs() < 0.01,
            "a quarter of the travel should be a quarter of the scale: {db} dB"
        );
    }

    /// The ends hold. A fader cannot be dragged past its ceiling or
    /// below silence however far the pointer goes.
    ///
    /// The ceiling is +12 dB, not unity: a fader that cannot add gain
    /// makes you reach for a plugin to do the most ordinary thing in a
    /// mix. In linear gain that is about 3.98.
    #[test]
    fn a_fader_stops_at_both_ends() {
        let unity = track(1.0, 0.0);
        let Some(Edit::SetVolume(_, up)) = drag(Control::Volume, "k", &unity, 5.0) else {
            panic!("expected a volume edit");
        };
        let ceiling = 10.0_f64.powf(daw_theme_art::paint::tcp::FADER_TOP_DB / 20.0);
        assert!(
            (up - ceiling).abs() < 1e-6,
            "should have stopped at +12 dB: {up}"
        );

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

    /// The FX button opens a chain window, and there is neither a
    /// chain nor a window — see `bin/chain-probe`. It produces no edit
    /// rather than an invented one.
    #[test]
    fn the_fx_button_waits_for_a_chain() {
        assert!(click(Control::Fx, "k", &track(1.0, 0.0), false).is_none());
        assert!(drag(Control::Mute, "k", &track(1.0, 0.0), 0.5).is_none());
    }

    /// Routing toggles the parent send, and carries the value it is
    /// toggling TO rather than asking the engine to invert — so the
    /// click and the control the click landed on cannot disagree.
    #[test]
    fn routing_toggles_the_parent_send() {
        let mut sending = track(1.0, 0.0);
        sending.parent_send = true;
        assert_eq!(
            click(Control::Routing, "k", &sending, false),
            Some(Edit::SetParentSend("k".into(), false))
        );
        sending.parent_send = false;
        assert_eq!(
            click(Control::Routing, "k", &sending, false),
            Some(Edit::SetParentSend("k".into(), true))
        );
    }

    /// Clicking a name selects; renaming is a different gesture.
    #[test]
    fn clicking_a_name_selects_it() {
        assert_eq!(
            click(Control::Name, "k", &track(1.0, 0.0), false),
            Some(Edit::Select("k".into())),
            "a plain click selects one track"
        );
        assert_eq!(
            click(Control::Name, "k", &track(1.0, 0.0), true),
            Some(Edit::AddToSelection("k".into())),
            "the modifier adds to the selection instead"
        );
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
    if edit.is_item() {
        if let Edit::DeselectAllItems(_) = edit {
            if let Err(error) = project.items().deselect_all().await {
                tracing::warn!(error = %error, "the engine refused to deselect");
            }
            return;
        }
        if let Edit::SelectAllItems(_) = edit {
            if let Err(error) = project.items().select_all().await {
                tracing::warn!(error = %error, "the engine refused to select all");
            }
            return;
        }
        let Ok(Some(item)) = project.items().by_guid(edit.guid()).await else {
            return;
        };
        let secs = daw_proto::primitives::Duration::from_seconds;
        let at = daw_proto::primitives::PositionInSeconds::from_seconds;
        let outcome = match edit {
            Edit::SetFadeIn(_, s, shape) => item.set_fade_in(secs(*s), *shape).await,
            Edit::SetFadeOut(_, s, shape) => item.set_fade_out(secs(*s), *shape).await,
            Edit::SelectItem(_, exclusive) => {
                if *exclusive && let Err(error) = project.items().deselect_all().await {
                    tracing::warn!(error = %error, "the engine refused to deselect");
                }
                item.select().await
            }
            Edit::MoveItem(_, position) => item.set_position(at(*position)).await,
            Edit::TrimItem(_, position, length) => match item.set_position(at(*position)).await {
                Ok(()) => item.set_length(secs(*length)).await,
                Err(error) => Err(error),
            },
            // The engine has no split of its own yet: the left half is
            // the item shortened, the right a new item on the same
            // track — the window's copy did the same and named the new
            // half, so a later edit finds it. The new item has no
            // source until the standalone grows a split that carries
            // one; it says so here rather than pretending.
            Edit::SplitItem(_, split_at, _new_guid) => {
                let (Ok(position), Ok(length)) = (item.position().await, item.length().await)
                else {
                    return;
                };
                let (start, end) = (
                    position.as_seconds(),
                    position.as_seconds() + length.as_seconds(),
                );
                if *split_at <= start || *split_at >= end {
                    return;
                }
                match item.set_length(secs(split_at - start)).await {
                    Ok(()) => {
                        let Ok(Some(track)) = project
                            .tracks()
                            .by_guid(&item_track(&project, edit.guid()).await)
                            .await
                        else {
                            return;
                        };
                        track
                            .items()
                            .add(at(*split_at), secs(end - split_at))
                            .await
                            .map(|_| ())
                    }
                    Err(error) => Err(error),
                }
            }
            Edit::DeleteItem(_) => item.delete().await,
            _ => Ok(()),
        };
        if let Err(error) = outcome {
            tracing::warn!(error = %error, edit = ?edit, "the engine refused an item edit");
        }
        return;
    }
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
        Edit::Select(_) => track.select_exclusive().await,
        Edit::AddToSelection(_) => track.select().await,
        Edit::SetVolume(_, v) => track.set_volume(*v).await,
        Edit::SetPan(_, p) => track.set_pan(*p).await,
        Edit::Rename(_, name) => track.rename(name).await,
        Edit::SetPhase(_, inverted) => track.set_phase_inverted(*inverted).await,
        Edit::SetInputMonitor(_, mode) => track.set_input_monitor(*mode).await,
        Edit::SetParentSend(_, enabled) => track.set_parent_send(*enabled).await,
        Edit::SetFadeIn(..)
        | Edit::SetFadeOut(..)
        | Edit::SelectItem(..)
        | Edit::DeselectAllItems(_)
        | Edit::SelectAllItems(_)
        | Edit::MoveItem(..)
        | Edit::TrimItem(..)
        | Edit::SplitItem(..)
        | Edit::DeleteItem(_) => Ok(()),
    };
    if let Err(error) = outcome {
        // One line, because a failed edit is a thing the user did that
        // did not happen — silence here is how a mixer starts lying.
        tracing::warn!(error = %error, edit = ?edit, "the engine refused an edit");
    }
}

/// The track an item is on, by guid — the facade's item list carries
/// it, and a split needs the track to add the new half to.
async fn item_track(project: &daw_control::Project, item_guid: &str) -> String {
    project
        .items()
        .all()
        .await
        .ok()
        .and_then(|items| items.into_iter().find(|i| i.guid == item_guid))
        .map(|i| i.track_guid)
        .unwrap_or_default()
}

/// The engine's own account of the tracks, as it changes.
///
/// The window predicts an edit so the control moves under the hand —
/// see `apply_locally` — and this is what turns that prediction into a
/// correction. It is also the only way a change made ANYWHERE ELSE
/// reaches the window: another client, a control surface, a REAPER on
/// the other end of daw-sync. A UI that only ever saw its own edits
/// would be a UI that silently disagrees with everyone.
///
/// Events rather than polling, because a change IS an event: missing
/// one leaves the window wrong until something else happens to it,
/// where missing a transport position is covered by the next one a few
/// milliseconds later. That asymmetry is why `Transport` polls and this
/// does not.
pub struct Watch {
    changes: std::sync::mpsc::Receiver<daw_proto::track::TrackEvent>,
}

impl Watch {
    /// Subscribe. `None` if the facade is not up.
    #[must_use]
    pub fn start() -> Option<Self> {
        let runtime = crate::open::runtime()?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("session-daw-watch".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let Some(daw) = daw::rpc::Daw::try_get() else {
                        return;
                    };
                    let Ok(project) = daw.current_project().await else {
                        return;
                    };
                    let Ok(mut stream) = project.tracks().subscribe().await else {
                        return;
                    };
                    while let Ok(Some(event)) = stream.recv().await {
                        // A closed channel means the window has gone;
                        // there is nothing left to correct.
                        if tx.send(event.get().event.clone()).is_err() {
                            break;
                        }
                    }
                });
            })
            .ok()?;
        Some(Self { changes: rx })
    }

    /// Everything that has happened since the last frame.
    ///
    /// Drained rather than waited on: the window asks once per frame
    /// and applies whatever arrived. A frame that finds nothing has
    /// nothing to do, which is most of them.
    pub fn drain(&self) -> impl Iterator<Item = daw_proto::track::TrackEvent> + '_ {
        self.changes.try_iter()
    }
}

/// Apply one engine event to the window's copy.
///
/// The engine is the authority, so this overwrites rather than merges —
/// including over a prediction that turned out wrong. A fader the user
/// is still dragging is the one exception a caller may want to make,
/// and it is the caller's to make because only it knows what is being
/// dragged.
/// The track a CONTINUOUS event is about — a volume or a pan.
///
/// These are the only edits a hand can still be making when the
/// engine's answer to an earlier one arrives, so they are the only
/// ones a caller may want to hold back while a drag is in flight.
/// Everything else is a toggle or a name: by the time the engine
/// answers, the gesture that caused it is over, and there is nothing
/// for its answer to fight with.
#[must_use]
pub fn continuous_for(event: &daw_proto::track::TrackEvent) -> Option<&str> {
    use daw_proto::track::TrackEvent as E;
    match event {
        E::VolumeChanged { guid, .. } | E::PanChanged { guid, .. } => Some(guid),
        _ => None,
    }
}

pub fn apply_event(tracks: &mut [daw_proto::Track], event: &daw_proto::track::TrackEvent) {
    use daw_proto::track::TrackEvent as E;
    let find = |tracks: &mut [daw_proto::Track], guid: &str| -> Option<usize> {
        tracks.iter().position(|t| t.guid == guid)
    };
    match event {
        E::Renamed { guid, name } => {
            if let Some(i) = find(tracks, guid) {
                tracks[i].name.clone_from(name);
            }
        }
        E::MuteChanged { guid, muted } => {
            if let Some(i) = find(tracks, guid) {
                tracks[i].muted = *muted;
            }
        }
        E::SoloChanged { guid, soloed } => {
            if let Some(i) = find(tracks, guid) {
                tracks[i].soloed = *soloed;
            }
        }
        E::ArmChanged { guid, armed } => {
            if let Some(i) = find(tracks, guid) {
                tracks[i].armed = *armed;
            }
        }
        E::SelectionChanged { guid, selected } => {
            if let Some(i) = find(tracks, guid) {
                tracks[i].selected = *selected;
            }
        }
        E::VolumeChanged { guid, volume } => {
            if let Some(i) = find(tracks, guid) {
                tracks[i].volume = *volume;
            }
        }
        E::PanChanged { guid, pan } => {
            if let Some(i) = find(tracks, guid) {
                tracks[i].pan = *pan;
            }
        }
        E::ColorChanged { guid, color } => {
            if let Some(i) = find(tracks, guid) {
                tracks[i].color = *color;
            }
        }
        E::PhaseInvertedChanged { guid, inverted } => {
            if let Some(i) = find(tracks, guid) {
                tracks[i].phase_inverted = *inverted;
            }
        }
        E::FxCountChanged {
            guid,
            fx_count,
            input_fx_count,
        } => {
            if let Some(i) = find(tracks, guid) {
                tracks[i].fx_count = *fx_count;
                tracks[i].input_fx_count = *input_fx_count;
            }
        }
        // Added and Removed change the track LIST, not a track — the
        // rows, the offsets and the recorded scenes all follow from it,
        // so it is a rebuild rather than a field to poke. Ignored here
        // and handled by whoever owns the scene.
        _ => {}
    }
}

#[cfg(test)]
mod watch_tests {
    use super::apply_event;
    use daw_proto::Track;
    use daw_proto::track::TrackEvent as E;

    fn tracks() -> Vec<Track> {
        ["a", "b"]
            .into_iter()
            .map(|g| Track {
                guid: g.to_owned(),
                volume: 1.0,
                ..Track::default()
            })
            .collect()
    }

    /// The engine's word overwrites the window's guess — including a
    /// guess that was wrong, which is the entire point of subscribing.
    #[test]
    fn the_engine_corrects_a_bad_prediction() {
        let mut t = tracks();
        // The window predicted a mute.
        t[0].muted = true;
        // The engine says otherwise — a solo-exclusive elsewhere, say.
        apply_event(
            &mut t,
            &E::MuteChanged {
                guid: "a".into(),
                muted: false,
            },
        );
        assert!(!t[0].muted);
    }

    /// An event for a track this window does not have is ignored rather
    /// than panicking. The engine's list and the window's can differ for
    /// a frame after a change.
    #[test]
    fn an_unknown_track_is_ignored() {
        let mut t = tracks();
        apply_event(
            &mut t,
            &E::VolumeChanged {
                guid: "gone".into(),
                volume: 0.5,
            },
        );
        assert!((t[0].volume - 1.0).abs() < f64::EPSILON);
    }

    /// Every field the UI draws is reachable, or a change made
    /// elsewhere would never show.
    #[test]
    fn every_drawn_field_can_be_corrected() {
        let mut t = tracks();
        apply_event(
            &mut t,
            &E::Renamed {
                guid: "a".into(),
                name: "Kick".into(),
            },
        );
        apply_event(
            &mut t,
            &E::SoloChanged {
                guid: "a".into(),
                soloed: true,
            },
        );
        apply_event(
            &mut t,
            &E::ArmChanged {
                guid: "a".into(),
                armed: true,
            },
        );
        apply_event(
            &mut t,
            &E::VolumeChanged {
                guid: "a".into(),
                volume: 0.25,
            },
        );
        apply_event(
            &mut t,
            &E::PanChanged {
                guid: "a".into(),
                pan: -0.5,
            },
        );
        apply_event(
            &mut t,
            &E::SelectionChanged {
                guid: "a".into(),
                selected: true,
            },
        );
        assert_eq!(t[0].name, "Kick");
        assert!(t[0].soloed && t[0].armed && t[0].selected);
        assert!((t[0].volume - 0.25).abs() < f64::EPSILON);
        assert!((t[0].pan + 0.5).abs() < f64::EPSILON);
    }

    /// One track's event does not touch another's.
    #[test]
    fn events_do_not_leak_between_tracks() {
        let mut t = tracks();
        apply_event(
            &mut t,
            &E::MuteChanged {
                guid: "b".into(),
                muted: true,
            },
        );
        assert!(!t[0].muted && t[1].muted);
    }
}

#[cfg(test)]
mod continuous_tests {
    use super::continuous_for;
    use daw_proto::track::TrackEvent as E;

    /// A fader and a pan are the two things a hand can still be holding
    /// when the engine answers, so they are the two the window may hold
    /// back.
    #[test]
    fn only_the_draggable_ones_name_a_track() {
        assert_eq!(
            continuous_for(&E::VolumeChanged {
                guid: "a".to_owned(),
                volume: 0.5,
            }),
            Some("a")
        );
        assert_eq!(
            continuous_for(&E::PanChanged {
                guid: "b".to_owned(),
                pan: -0.25,
            }),
            Some("b")
        );
    }

    /// A mute is over the moment it is clicked, so the engine's word on
    /// it is never in competition with a hand.
    #[test]
    fn a_toggle_is_not_continuous() {
        assert_eq!(
            continuous_for(&E::MuteChanged {
                guid: "a".to_owned(),
                muted: true,
            }),
            None
        );
        assert_eq!(
            continuous_for(&E::Renamed {
                guid: "a".to_owned(),
                name: "Kick".to_owned(),
            }),
            None
        );
    }
}

/// The engine's live meter levels.
///
/// A separate subscription from [`Watch`] because it is a different
/// KIND of fact. A track event is a change the window must not miss —
/// a mute it never heard about leaves the strip wrong until something
/// else happens. A meter frame is a measurement, and the next one is
/// thirty milliseconds away: missing one costs nothing, and queueing
/// them costs a backlog of readings that were true a second ago.
///
/// So this keeps the LATEST frame and drops the rest, where `Watch`
/// keeps every event in order. Same transport, opposite policy, and the
/// policy is the reason they are not one type.
pub struct Meters {
    latest: std::sync::Arc<std::sync::Mutex<Vec<daw_proto::TrackLevels>>>,
}

impl Meters {
    /// Subscribe. `None` if the facade is not up, in which case the
    /// meters stay at rest rather than the window failing to open.
    #[must_use]
    pub fn start() -> Option<Self> {
        let runtime = crate::open::runtime()?;
        let latest = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let into_thread = std::sync::Arc::clone(&latest);
        std::thread::Builder::new()
            .name("session-daw-meters".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let Some(daw) = daw::rpc::Daw::try_get() else {
                        return;
                    };
                    let Ok(project) = daw.current_project().await else {
                        return;
                    };
                    let mut stream = project.meter_events();
                    while let Ok(Some(frame)) = stream.recv().await {
                        let frame = frame.get();
                        // Overwrite rather than append: the only frame
                        // worth drawing is the one that just arrived.
                        let Ok(mut slot) = into_thread.lock() else {
                            break;
                        };
                        slot.clear();
                        slot.extend_from_slice(&frame.tracks);
                    }
                });
            })
            .ok()?;
        Some(Self { latest })
    }

    /// The most recent frame, copied out for this redraw.
    ///
    /// Copied rather than borrowed because the lock must not be held
    /// across a frame: the pump publishes at 30 Hz and a renderer
    /// holding its mutex would stall the engine's thread, which is the
    /// one thread in this program that has a deadline.
    #[must_use]
    pub fn levels(&self) -> Vec<daw_proto::TrackLevels> {
        self.latest
            .lock()
            .map(|frame| frame.clone())
            .unwrap_or_default()
    }
}

/// A meter's height, from a linear peak.
///
/// The meter is drawn from a fraction of its own height, so the level
/// has to be mapped the way an ear reads it rather than the way the
/// sample buffer holds it: linear `0..1` puts −20 dBFS — a perfectly
/// ordinary track — at two per cent of the well, which reads as
/// silence.
///
/// The same scale the fader uses, so a track sitting at unity with a
/// hot signal lights the meter to about where its own cap sits. That
/// correspondence is what makes a mixer readable at a glance.
#[must_use]
pub fn meter_fraction(peak: f32) -> f64 {
    let db = 20.0 * f64::from(peak.max(1e-6)).log10();
    // The METER's scale, which runs past unity: a signal over 0 dBFS
    // has to land above the 0 mark rather than pinning to the same full
    // bar a clean −1 draws.
    daw_theme_art::paint::tcp::meter_norm(db)
}

#[cfg(test)]
mod meter_tests {
    use super::meter_fraction;

    /// Silence empties it, 0 dBFS lands on the 0 mark rather than at
    /// the top, and there is room above for the overs — which is the
    /// one distinction a meter exists to make and the one a scale
    /// ending at unity cannot.
    #[test]
    fn the_meter_reads_in_decibels() {
        use daw_theme_art::paint::tcp::{METER_TOP_DB, meter_norm};
        assert!(meter_fraction(0.0) < 1e-6, "silence is empty");
        let unity = meter_fraction(1.0);
        assert!(
            (unity - meter_norm(0.0)).abs() < 1e-6,
            "0 dBFS landed at {unity}, not on the 0 mark"
        );
        assert!(
            unity < 1.0,
            "0 dBFS filled the column, leaving no room for an over"
        );
        // And an over goes ABOVE it rather than pinning to the same bar.
        assert!(
            meter_fraction(1.5) > unity,
            "an over did not rise past unity"
        );
        // Up to the ceiling, where it stops rather than running off.
        let way_over = meter_fraction(10.0);
        assert!(
            way_over <= 1.0,
            "a loud over ran off the column: {way_over}"
        );
        assert!((METER_TOP_DB - 12.0).abs() < f64::EPSILON);
    }

    /// A denormal or a zero must not produce a NaN height — the meter
    /// would vanish rather than read empty.
    #[test]
    fn silence_is_a_number() {
        assert!(meter_fraction(0.0).is_finite());
        assert!(meter_fraction(f32::MIN_POSITIVE).is_finite());
    }
}
