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
    /// Send this track to that one.
    ///
    /// Two guids, because a send is a relation and not a property: the
    /// source owns it and the destination is a track the engine has to
    /// resolve. The index it comes back with is REAPER's, and the
    /// window does not guess it — the read that follows says what it
    /// was.
    AddSend(String, String),
    /// Take one of this track's sends away, by its index.
    ///
    /// Removing renumbers every send after it, which is why nothing
    /// here patches a list after one: the panel forgets the track and
    /// reads it again.
    RemoveSend(String, u32),
    /// How much of the track goes down one of its sends.
    ///
    /// A gain, the way the track's own volume is: 1.0 is unity.
    SetSendVolume(String, u32, f64),
    /// Where the send sits, -1.0 to 1.0.
    SetSendPan(String, u32, f64),
    /// Whether the send passes anything at all.
    SetSendMute(String, u32, bool),
    /// Where the send is tapped from: post-fader, pre-FX, post-FX.
    ///
    /// The difference between a reverb that follows the fader and one
    /// that does not, which is the first thing anybody changes about a
    /// send and the thing REAPER hides in a menu.
    SetSendMode(String, u32, daw_proto::routing::SendMode),
    /// The track's colour, as REAPER stores it (0xRRGGBB).
    ///
    /// Carries the value rather than cycling, because a colour is
    /// chosen and not stepped through, and the window is the thing
    /// holding the palette the user picked from.
    SetColor(String, u32),
    /// Trim, read, touch, write, latch, latch-preview.
    ///
    /// Governs whether moving a control writes envelope points while
    /// the transport runs, so it changes what every OTHER edit on this
    /// track means. That is why it is a value and not a toggle: there
    /// is no safe "next" mode to guess at.
    SetAutomationMode(String, daw_proto::primitives::AutomationMode),
    /// What the track records from — a hardware channel, or a MIDI
    /// device and channel.
    ///
    /// The one an engineer changes while tracking, which is exactly
    /// when a second screen showing the old answer is worst.
    SetRecordInput(String, daw_proto::track::RecordInput),
    /// Whether the track is drawn in the arrange panel, the mixer, or
    /// both. Both flags travel together because REAPER sets them
    /// together, and sending one would silently re-assert the other.
    SetVisibility(String, bool, bool),
    /// How tall the track's panel is drawn, in pixels.
    ///
    /// A view concern that lives in the project, so a height set here
    /// is a height REAPER opens with — which is the whole point of a
    /// scene that arranges the session rather than only this window.
    SetHeight(String, u32),
    /// Folder depth: positive opens a folder, negative closes one or
    /// more levels, zero is an ordinary track.
    ///
    /// The one edit that changes the SHAPE of the session rather than a
    /// value in it, so a window applying it has to re-read rather than
    /// patch — every depth below it moves.
    SetFolderDepth(String, i32),
    /// Join or leave one of REAPER's 128 group slots, in every family
    /// at once.
    SetGroupMembership(String, u32, bool),
    /// The track's role — lead, follow, neither — in ONE family of one
    /// slot. What a VCA actually is.
    SetGroupFlags(
        String,
        u32,
        daw_proto::track::GroupFamily,
        daw_proto::track::GroupRole,
    ),
    /// One of the per-slot modifiers that is not a lead/follow pair:
    /// reversed volume, no-lead-when-follow, and the rest.
    SetGroupModifier(String, u32, daw_proto::track::GroupModifier, bool),
    // ── The ruler ────────────────────────────────────────────────
    //
    // Markers and regions are addressed by REAPER's own number rather
    // than by a guid, because that is the only identity the live API
    // offers — there IS a guid in the project file and the live API
    // never fills it. The number is not stable across REAPER's
    // renumber action, which is why the event stream reports a
    // renumbering as one rather than as a deletion: a window holding
    // one of these can follow it rather than lose it.
    //
    // The guid field every other edit carries is empty here. These are
    // not about a track.
    /// Put a marker at a time, in a ruler lane.
    AddMarker(String, f64, u32),
    /// Move a marker to a time.
    MoveMarker(String, u32, f64),
    /// Rename one.
    RenameMarker(String, u32, String),
    /// Take it away.
    RemoveMarker(String, u32),
    /// Make a region spanning two times, in a ruler lane.
    ///
    /// A region in the SECTIONS lane IS a song section — the song
    /// model reads the arrangement from that lane — so this is how a
    /// verse comes into being.
    AddRegion(String, f64, f64, u32),
    /// Set both of a region's bounds. Both together because REAPER's
    /// setter takes both, and because dragging one end of a band the
    /// user can see is a change to the band.
    SetRegionBounds(String, u32, f64, f64),
    RenameRegion(String, u32, String),
    RemoveRegion(String, u32),
    /// Set the tempo of the stretch starting at a time, adding a tempo
    /// marker there if there is not one.
    ///
    /// What tempo mapping writes. The guid is empty: a tempo belongs to
    /// the project, not to a track.
    SetTempo(String, f64, f64),
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
            | Self::AddSend(g, _)
            | Self::RemoveSend(g, _)
            | Self::SetSendVolume(g, ..)
            | Self::SetSendPan(g, ..)
            | Self::SetSendMute(g, ..)
            | Self::SetSendMode(g, ..)
            | Self::SetColor(g, _)
            | Self::SetAutomationMode(g, _)
            | Self::SetRecordInput(g, _)
            | Self::SetVisibility(g, ..)
            | Self::SetHeight(g, _)
            | Self::SetFolderDepth(g, _)
            | Self::SetGroupMembership(g, ..)
            | Self::SetGroupFlags(g, ..)
            | Self::SetGroupModifier(g, ..)
            | Self::AddMarker(g, ..)
            | Self::MoveMarker(g, ..)
            | Self::RenameMarker(g, ..)
            | Self::RemoveMarker(g, ..)
            | Self::AddRegion(g, ..)
            | Self::SetRegionBounds(g, ..)
            | Self::RenameRegion(g, ..)
            | Self::RemoveRegion(g, ..)
            | Self::SetTempo(g, ..)
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
    /// Whether this edit is about the ruler rather than a track or an
    /// item. Its guid field is empty; it is addressed by number.
    #[must_use]
    pub const fn is_ruler(&self) -> bool {
        matches!(
            self,
            Self::SetTempo(..)
                | Self::AddMarker(..)
                | Self::MoveMarker(..)
                | Self::RenameMarker(..)
                | Self::RemoveMarker(..)
                | Self::AddRegion(..)
                | Self::SetRegionBounds(..)
                | Self::RenameRegion(..)
                | Self::RemoveRegion(..)
        )
    }

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
                | Self::SetSendVolume(..)
                | Self::SetSendPan(..)
                | Self::SetFadeIn(..)
                | Self::SetFadeOut(..)
                | Self::MoveItem(..)
                | Self::TrimItem(..)
        )
    }

    /// Whether `other` is the same control on the same track.
    ///
    /// A send's number is part of its identity here. Without it two
    /// sends on one track are one control, and dragging the reverb
    /// send would eat the delay send's last position out of the queue
    /// — a coalescing bug that looks like a routing bug.
    #[must_use]
    fn same_control_as(&self, other: &Self) -> bool {
        self.guid() == other.guid()
            && std::mem::discriminant(self) == std::mem::discriminant(other)
            && self.route() == other.route()
    }

    /// Which of the track's sends this edit is about, if any.
    #[must_use]
    const fn route(&self) -> Option<u32> {
        match self {
            Self::RemoveSend(_, index)
            | Self::SetSendVolume(_, index, _)
            | Self::SetSendPan(_, index, _)
            | Self::SetSendMute(_, index, _)
            | Self::SetSendMode(_, index, _) => Some(*index),
            _ => None,
        }
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

    /// And per SEND. Two sends on one track are two controls: without
    /// the number in the identity, dragging the reverb send would eat
    /// the delay send's last position out of the queue, and the delay
    /// would end up wherever the reverb was left.
    #[test]
    fn coalescing_does_not_cross_sends() {
        let mut q = Queue::default();
        q.push(Edit::SetSendVolume("kick".into(), 0, 0.5));
        q.push(Edit::SetSendVolume("kick".into(), 1, 0.5));
        assert_eq!(q.len(), 2);
        q.push(Edit::SetSendVolume("kick".into(), 0, 0.9));
        assert_eq!(q.len(), 2, "send 0 should have been replaced");
        assert_eq!(q.pop(), Some(Edit::SetSendVolume("kick".into(), 0, 0.9)));
        assert_eq!(q.pop(), Some(Edit::SetSendVolume("kick".into(), 1, 0.5)));
    }

    /// Taking a send away is a click, not a drag: two removes are two
    /// removes, because the second one is about a different send —
    /// removing renumbers everything after it.
    #[test]
    fn removing_a_send_never_collapses() {
        let mut q = Queue::default();
        q.push(Edit::RemoveSend("kick".into(), 0));
        q.push(Edit::RemoveSend("kick".into(), 0));
        assert_eq!(q.len(), 2);
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
    if edit.is_ruler() {
        let markers = project.markers();
        let regions = project.regions();
        let outcome = match edit {
            Edit::AddMarker(_, at, lane) => match markers.add(*at, "").await {
                // The lane is a second call because REAPER's add takes
                // no lane. A marker that landed in the wrong lane would
                // be a mark in the sections row, which is a section
                // that is not one.
                Ok(id) => markers.set_lane(id, Some(*lane)).await,
                Err(error) => Err(error),
            },
            Edit::MoveMarker(_, id, at) => markers.move_to(*id, at.max(0.0)).await,
            Edit::RenameMarker(_, id, name) => markers.rename(*id, name).await,
            Edit::RemoveMarker(_, id) => markers.remove(*id).await,
            Edit::AddRegion(_, from, to, lane) => {
                let (from, to) = ordered(*from, *to);
                match regions.add(from, to, "").await {
                    Ok(id) => regions.set_lane(id, Some(*lane)).await,
                    Err(error) => Err(error),
                }
            }
            Edit::SetRegionBounds(_, id, from, to) => {
                let (from, to) = ordered(*from, *to);
                regions.set_bounds(*id, from, to).await
            }
            Edit::RenameRegion(_, id, name) => regions.rename(*id, name).await,
            Edit::RemoveRegion(_, id) => regions.remove(*id).await,
            Edit::SetTempo(_, at, bpm) => set_tempo(&project, *at, *bpm).await,
            _ => Ok(()),
        };
        if let Err(error) = outcome {
            tracing::warn!(error = %error, edit = ?edit, "the engine refused a ruler edit");
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
        // A send is addressed through the track that owns it, by the
        // number REAPER gave it. `by_index` costs a round trip of its
        // own to prove the route is still there — worth it, because
        // the alternative is writing a level into whatever now holds
        // that number after somebody removed the one before it.
        Edit::AddSend(_, dest) => track.sends().add_to(dest).await.map(|_| ()),
        Edit::RemoveSend(_, index) => {
            on_send(&track, *index, |route| async move { route.remove().await }).await
        }
        Edit::SetSendVolume(_, index, v) => {
            let v = *v;
            on_send(&track, *index, move |route| async move {
                route.set_volume(v).await
            })
            .await
        }
        Edit::SetSendPan(_, index, p) => {
            let p = *p;
            on_send(&track, *index, move |route| async move { route.set_pan(p).await }).await
        }
        Edit::SetSendMute(_, index, muted) => {
            let muted = *muted;
            on_send(&track, *index, move |route| async move {
                if muted { route.mute().await } else { route.unmute().await }
            })
            .await
        }
        Edit::SetSendMode(_, index, mode) => {
            let mode = *mode;
            on_send(&track, *index, move |route| async move {
                route.set_send_mode(mode).await
            })
            .await
        }
        Edit::SetColor(_, color) => track.set_color(*color).await,
        Edit::SetAutomationMode(_, mode) => track.set_automation_mode(*mode).await,
        Edit::SetRecordInput(_, input) => track.set_record_input(*input).await,
        Edit::SetVisibility(_, tcp, mixer) => track.set_visibility(*tcp, *mixer).await,
        Edit::SetHeight(_, pixels) => track.set_tcp_height(*pixels).await,
        Edit::SetFolderDepth(_, depth) => track.set_folder_depth(*depth).await,
        Edit::SetGroupMembership(_, slot, member) => {
            track.set_group_membership(*slot, *member).await
        }
        Edit::SetGroupFlags(_, slot, family, role) => {
            track.set_group_flags(*slot, *family, *role).await
        }
        Edit::SetGroupModifier(_, slot, modifier, on) => {
            track.set_group_modifier(*slot, *modifier, *on).await
        }
        Edit::SetFadeIn(..)
        | Edit::SetFadeOut(..)
        | Edit::SelectItem(..)
        | Edit::DeselectAllItems(_)
        | Edit::SelectAllItems(_)
        | Edit::MoveItem(..)
        | Edit::TrimItem(..)
        | Edit::SplitItem(..)
        | Edit::DeleteItem(_)
        // Already handled above, where they did not need a track.
        | Edit::AddMarker(..)
        | Edit::MoveMarker(..)
        | Edit::RenameMarker(..)
        | Edit::RemoveMarker(..)
        | Edit::AddRegion(..)
        | Edit::SetRegionBounds(..)
        | Edit::RenameRegion(..)
        | Edit::RemoveRegion(..)
        | Edit::SetTempo(..) => Ok(()),
    };
    if let Err(error) = outcome {
        // One line, because a failed edit is a thing the user did that
        // did not happen — silence here is how a mixer starts lying.
        tracing::warn!(error = %error, edit = ?edit, "the engine refused an edit");
    }
}

/// Write a tempo at a time, moving the marker there if one is close
/// enough to be the same one.
///
/// "Close enough" is a millisecond. Tempo mapping puts a marker on a
/// bar line and then corrects it, and a correction that added a second
/// marker a thousandth of a second from the first would leave a tempo
/// map full of pairs — and pairs mean a bar that is two tempos, which
/// is a bar with no tempo at all.
async fn set_tempo(project: &daw_control::Project, at: f64, bpm: f64) -> daw_control::Result<()> {
    const SAME_POINT: f64 = 0.001;
    let map = project.tempo_map();
    let existing = map.points().await.unwrap_or_default();
    let found = existing.iter().position(|point| {
        point
            .position
            .time
            .as_ref()
            .is_some_and(|t| (t.as_seconds() - at).abs() < SAME_POINT)
    });
    match found {
        Some(index) => {
            map.set_tempo_at(u32::try_from(index).unwrap_or(0), bpm)
                .await
        }
        None => map.add_point(at.max(0.0), bpm).await.map(|_| ()),
    }
}

/// Two times, in order, with a floor at zero.
///
/// A region dragged right to left is the region the user drew; a
/// region with its end before its start is one REAPER will refuse.
fn ordered(a: f64, b: f64) -> (f64, f64) {
    let (from, to) = if a <= b { (a, b) } else { (b, a) };
    (from.max(0.0), to.max(0.0))
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

/// Do something to one of a track's sends.
///
/// The resolve is half the cost of a send edit — `by_index` reads the
/// route to prove it exists before handing back a handle — so it is in
/// one place rather than repeated per arm, and a send that has gone is
/// a warning rather than a silent nothing: a level written into a
/// number nobody holds any more is exactly the bug that would otherwise
/// look like the panel not working.
async fn on_send<F, Fut>(
    track: &daw_control::TrackHandle,
    index: u32,
    act: F,
) -> daw_control::Result<()>
where
    F: FnOnce(daw_control::RouteHandle) -> Fut,
    Fut: std::future::Future<Output = daw_control::Result<()>>,
{
    match track.sends().by_index(index).await {
        Ok(Some(route)) => act(route).await,
        Ok(None) => {
            tracing::warn!(route.index = index, "the send this edit names is gone");
            Ok(())
        }
        Err(error) => Err(error),
    }
}

/// The engine's account of the ROUTING, as it changes.
///
/// A second subscription, and it has to be: what a send is doing —
/// its level, its pan, its mute — travels on the routing stream, and
/// the track stream carries only how MANY there are. A strip that
/// draws a lane does not need the first; a panel that draws the sends
/// needs nothing else.
///
/// Taken from the cross-domain bus rather than a routing stream of its
/// own, because there is no routing stream client: the bus is the only
/// client-side path to a [`RoutingEvent`]. One filter, one domain — the
/// rest are dropped before they reach this channel.
pub struct Bus {
    changes: std::sync::mpsc::Receiver<daw_proto::routing::RoutingEvent>,
    alive: Alive,
}

impl Bus {
    /// Subscribe. `None` if the facade is not up.
    #[must_use]
    pub fn start() -> Option<Self> {
        let runtime = crate::open::runtime()?;
        let (tx, rx) = std::sync::mpsc::channel();
        let alive = Alive::new();
        let mine = alive.clone();
        std::thread::Builder::new()
            .name("session-daw-routing".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let _end = scopeguard(move || mine.ended());
                    let Some(daw) = daw::rpc::Daw::try_get() else {
                        return;
                    };
                    let filter = daw_proto::event_bus::BusFilter {
                        routing: true,
                        ..daw_proto::event_bus::BusFilter::default()
                    };
                    let Ok(mut stream) = daw.events().subscribe(filter).await else {
                        return;
                    };
                    while let Ok(Some(event)) = stream.recv().await {
                        let daw_proto::event_bus::DawEvent::Routing(routing) = event.get() else {
                            continue;
                        };
                        if tx.send(routing.clone()).is_err() {
                            break;
                        }
                    }
                });
            })
            .ok()?;
        Some(Self { changes: rx, alive })
    }

    /// Everything that has happened since the last frame.
    pub fn drain(&self) -> impl Iterator<Item = daw_proto::routing::RoutingEvent> + '_ {
        self.changes.try_iter()
    }

    /// Is this subscription still connected?
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.alive.is_live()
    }
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
    alive: Alive,
}

impl Watch {
    /// Subscribe. `None` if the facade is not up.
    #[must_use]
    pub fn start() -> Option<Self> {
        let runtime = crate::open::runtime()?;
        let (tx, rx) = std::sync::mpsc::channel();
        let alive = Alive::new();
        let mine = alive.clone();
        std::thread::Builder::new()
            .name("session-daw-watch".into())
            .spawn(move || {
                runtime.block_on(async move {
                    // Every exit from here marks the handle dead,
                    // including the ones that never got a stream at
                    // all: a subscribe that failed is a window with no
                    // corrections coming, which looks exactly like a
                    // project where nothing is happening.
                    let _end = scopeguard(move || mine.ended());
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
        Some(Self { changes: rx, alive })
    }

    /// Everything that has happened since the last frame.
    ///
    /// Drained rather than waited on: the window asks once per frame
    /// and applies whatever arrived. A frame that finds nothing has
    /// nothing to do, which is most of them.
    pub fn drain(&self) -> impl Iterator<Item = daw_proto::track::TrackEvent> + '_ {
        self.changes.try_iter()
    }

    /// Is this subscription still connected?
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.alive.is_live()
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

/// What an event changed.
///
/// Worth distinguishing because the two cost different things. A field
/// is a poke: the row it belongs to redraws and nothing else moves. The
/// LIST changing invalidates every row map, every offset and the scene
/// resolved against it, so the caller has to rebuild — and it needs
/// telling, because it cannot see inside this function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied {
    /// One track's field. Redraw it.
    Field,
    /// The track list itself. Rebuild the rows.
    Structure,
    /// Nothing this window keeps. Most often an event for a field the
    /// window does not draw.
    Nothing,
}

/// Apply one event to the window's track list.
///
/// Takes the `Vec` rather than a slice, and that is the whole reason
/// structure events used to be dropped here: a slice cannot grow, so
/// `Added` had nowhere to go and was quietly ignored. A track created
/// in REAPER did not appear until the window was restarted, and nothing
/// said so.
pub fn apply_event(
    tracks: &mut Vec<daw_proto::Track>,
    event: &daw_proto::track::TrackEvent,
) -> Applied {
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
        E::RouteCountsChanged {
            guid,
            send_count,
            receive_count,
        } => {
            if let Some(i) = find(tracks, guid) {
                tracks[i].send_count = *send_count;
                tracks[i].receive_count = *receive_count;
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
        // The fields this window WRITES and then never heard back
        // about. A prediction that is never confirmed is only right
        // while nothing rejects it: a parent send refused by the engine,
        // or toggled by somebody else in REAPER, stayed wrong here for
        // the life of the window with nothing to say so.
        E::ParentSendChanged { guid, enabled } => {
            if let Some(i) = find(tracks, guid) {
                tracks[i].parent_send = *enabled;
            }
        }
        E::InputMonitorChanged { guid, monitor } => {
            if let Some(i) = find(tracks, guid) {
                tracks[i].input_monitor = *monitor;
            }
        }
        E::RecordInputChanged { guid, input } => {
            if let Some(i) = find(tracks, guid) {
                tracks[i].record_input = *input;
            }
        }
        E::AutomationModeChanged { guid, mode } => {
            if let Some(i) = find(tracks, guid) {
                tracks[i].automation_mode = *mode;
            }
        }
        E::GroupingChanged { guid, grouping } => {
            if let Some(i) = find(tracks, guid) {
                tracks[i].grouping = grouping.clone();
            }
        }
        E::HeightChanged { guid, height } => {
            let Some(i) = find(tracks, guid) else {
                return Applied::Nothing;
            };
            tracks[i].height = *height;
            // A row's height is the LAYOUT, so every row below it moves
            // — the same reason a track appearing is structural. A
            // window that repainted only this row would leave every
            // other one drawn at its old offset.
            return Applied::Structure;
        }
        E::LanesChanged {
            guid,
            lane_count,
            lane_play_mask,
            lane_names,
            lane_display,
        } => {
            let Some(i) = find(tracks, guid) else {
                return Applied::Nothing;
            };
            let track = &mut tracks[i];
            track.lane_count = *lane_count;
            track.lane_play_mask = *lane_play_mask;
            track.lane_names.clone_from(lane_names);
            track.lane_display = *lane_display;
            // Lanes are rows too: turning them on gives a track three
            // times the height it had.
            return Applied::Structure;
        }
        E::FolderDepthChanged { guid, folder_depth } => {
            let Some(i) = find(tracks, guid) else {
                return Applied::Nothing;
            };
            tracks[i].folder_depth = *folder_depth;
            tracks[i].is_folder = *folder_depth > 0;
            // Depth is RELATIVE, so this one track's change re-parents
            // every track below it, and the parent guids this window
            // holds are now wrong for tracks it was told nothing about.
            // Only a re-read can fix that, which is what Structure asks
            // for — patching the tree from here would be guessing.
            return Applied::Structure;
        }
        // Visibility is the list as drawn rather than the list as
        // stored, so it re-derives the rows: hiding a track in REAPER
        // and leaving its row on screen is the same failure as leaving
        // a removed one there.
        E::TcpVisibilityChanged { guid, visible } => {
            let Some(i) = find(tracks, guid) else {
                return Applied::Nothing;
            };
            tracks[i].visible_in_tcp = *visible;
            return Applied::Structure;
        }
        E::MixerVisibilityChanged { guid, visible } => {
            let Some(i) = find(tracks, guid) else {
                return Applied::Nothing;
            };
            tracks[i].visible_in_mixer = *visible;
            return Applied::Structure;
        }
        // ── the list itself ──────────────────────────────────────
        E::Added(track) => {
            // Inserted at the track's own index rather than pushed:
            // REAPER numbers tracks by position, so a track added in the
            // middle would otherwise sit at the end and every row below
            // it would name the wrong track.
            let at = usize::try_from(track.index).unwrap_or(tracks.len());
            let at = at.min(tracks.len());
            tracks.insert(at, track.clone());
            reindex(tracks);
            return Applied::Structure;
        }
        E::Removed(guid) => {
            let Some(i) = find(tracks, guid) else {
                return Applied::Nothing;
            };
            tracks.remove(i);
            reindex(tracks);
            return Applied::Structure;
        }
        E::Moved {
            guid, new_index, ..
        } => {
            let Some(from) = find(tracks, guid) else {
                return Applied::Nothing;
            };
            let to = usize::try_from(*new_index)
                .unwrap_or(from)
                .min(tracks.len().saturating_sub(1));
            let track = tracks.remove(from);
            tracks.insert(to, track);
            reindex(tracks);
            return Applied::Structure;
        } // Deliberately exhaustive. The catch-all that used to sit here
          // meant a field added to the event upstream was dropped in
          // silence, which is exactly how five of the arms above went
          // missing for months. A new variant should break this build.
    }
    Applied::Field
}

/// Renumber after the list changed.
///
/// The index is a track's position, so every track below an insertion
/// or a removal has a new one. Leaving them stale is worse than it
/// sounds: the scene engine resolves rows by index, so one stale number
/// shows the wrong track at the right place — which reads as a
/// rendering bug rather than a bookkeeping one.
fn reindex(tracks: &mut [daw_proto::Track]) {
    for (at, track) in tracks.iter_mut().enumerate() {
        track.index = u32::try_from(at).unwrap_or(track.index);
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
                // As a track arrives from either backend, not as
                // `Default` leaves it: a fixture that starts with the
                // parent send off and the track hidden cannot show that
                // turning either one off works.
                parent_send: true,
                visible_in_tcp: true,
                visible_in_mixer: true,
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

    /// The fields this window writes and predicts locally.
    ///
    /// A prediction is only right until something rejects it, so the
    /// event that confirms it is what makes the local guess safe. This
    /// test is the negative control for that: every field here was
    /// already being written and drawn, and none of them could be
    /// corrected — a parent send toggled by anybody else stayed wrong
    /// for the life of the window.
    #[test]
    fn a_prediction_this_window_made_can_be_corrected() {
        use daw_proto::primitives::AutomationMode;
        use daw_proto::track::{InputMonitoringMode, RecordInput};

        let mut t = tracks();
        assert!(t[0].parent_send, "the control needs somewhere to fall from");

        apply_event(
            &mut t,
            &E::ParentSendChanged {
                guid: "a".into(),
                enabled: false,
            },
        );
        apply_event(
            &mut t,
            &E::InputMonitorChanged {
                guid: "a".into(),
                monitor: InputMonitoringMode::NotWhenPlaying,
            },
        );
        apply_event(
            &mut t,
            &E::RecordInputChanged {
                guid: "a".into(),
                input: RecordInput::Audio { channel: 7 },
            },
        );
        apply_event(
            &mut t,
            &E::AutomationModeChanged {
                guid: "a".into(),
                mode: AutomationMode::Latch,
            },
        );

        assert!(!t[0].parent_send);
        assert_eq!(t[0].input_monitor, InputMonitoringMode::NotWhenPlaying);
        assert_eq!(t[0].record_input, RecordInput::Audio { channel: 7 });
        assert_eq!(t[0].automation_mode, AutomationMode::Latch);
        assert!(t[1].parent_send, "the other track was not touched");
    }

    /// Hiding a track changes the rows, not just a field.
    ///
    /// The rows are derived from the list, so a visibility change has to
    /// say the list changed — otherwise a track hidden in REAPER keeps
    /// its row on screen, which is the same failure as leaving a removed
    /// one there.
    #[test]
    fn hiding_a_track_redraws_the_rows() {
        use super::Applied;
        let mut t = tracks();
        assert_eq!(
            apply_event(
                &mut t,
                &E::TcpVisibilityChanged {
                    guid: "a".into(),
                    visible: false,
                },
            ),
            Applied::Structure
        );
        assert!(!t[0].visible_in_tcp);

        // An event for a track this window never heard of changes
        // nothing and must not ask for a redraw.
        assert_eq!(
            apply_event(
                &mut t,
                &E::MixerVisibilityChanged {
                    guid: "nobody".into(),
                    visible: false,
                },
            ),
            Applied::Nothing
        );
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
    alive: Alive,
}

impl Meters {
    /// Subscribe. `None` if the facade is not up, in which case the
    /// meters stay at rest rather than the window failing to open.
    #[must_use]
    pub fn start() -> Option<Self> {
        let runtime = crate::open::runtime()?;
        let latest = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let into_thread = std::sync::Arc::clone(&latest);
        let alive = Alive::new();
        let mine = alive.clone();
        std::thread::Builder::new()
            .name("session-daw-meters".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let _end = scopeguard(move || mine.ended());
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
        Some(Self { latest, alive })
    }

    /// Is this subscription still connected?
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.alive.is_live()
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

#[cfg(test)]
mod structure_tests {
    use super::{Applied, apply_event};
    use daw_proto::Track;
    use daw_proto::track::TrackEvent as E;

    fn kit() -> Vec<Track> {
        ["Kick", "Snare", "OH"]
            .iter()
            .enumerate()
            .map(|(i, name)| {
                Track::new(
                    (*name).to_owned(),
                    u32::try_from(i).unwrap_or(0),
                    (*name).to_owned(),
                )
            })
            .collect()
    }

    /// **A track created in REAPER appears.** The applier used to take a
    /// slice, so `Added` had nowhere to go and was dropped — the window
    /// only caught up when something else triggered a full re-fetch.
    #[test]
    fn a_track_added_in_the_daw_lands_in_the_list() {
        let mut tracks = kit();
        let added = Track::new("Room".to_owned(), 3, "Room".to_owned());
        assert_eq!(
            apply_event(&mut tracks, &E::Added(added)),
            Applied::Structure
        );
        assert_eq!(tracks.len(), 4);
        assert_eq!(tracks[3].name, "Room");
    }

    /// **And at its own position, not the end.** REAPER numbers tracks
    /// by position, so a track inserted in the middle that got appended
    /// would leave every row below it naming the wrong track.
    #[test]
    fn a_track_added_in_the_middle_lands_in_the_middle() {
        let mut tracks = kit();
        let added = Track::new("Sub".to_owned(), 1, "Sub".to_owned());
        apply_event(&mut tracks, &E::Added(added));
        let names: Vec<&str> = tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Kick", "Sub", "Snare", "OH"]);
    }

    /// The index is a position, so everything below a change is
    /// renumbered. A stale index shows the wrong track at the right
    /// place, which reads as a rendering bug rather than a bookkeeping
    /// one.
    #[test]
    fn the_list_is_renumbered_after_it_changes() {
        let mut tracks = kit();
        apply_event(
            &mut tracks,
            &E::Added(Track::new("Sub".to_owned(), 1, "Sub".to_owned())),
        );
        let indices: Vec<u32> = tracks.iter().map(|t| t.index).collect();
        assert_eq!(indices, [0, 1, 2, 3]);

        apply_event(&mut tracks, &E::Removed("Kick".to_owned()));
        let indices: Vec<u32> = tracks.iter().map(|t| t.index).collect();
        assert_eq!(indices, [0, 1, 2], "removal left a hole in the numbering");
    }

    /// A track removed in the DAW goes.
    #[test]
    fn a_track_removed_in_the_daw_leaves_the_list() {
        let mut tracks = kit();
        assert_eq!(
            apply_event(&mut tracks, &E::Removed("Snare".to_owned())),
            Applied::Structure
        );
        let names: Vec<&str> = tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Kick", "OH"]);
    }

    /// A reorder in the DAW reorders here.
    #[test]
    fn a_moved_track_moves() {
        let mut tracks = kit();
        apply_event(
            &mut tracks,
            &E::Moved {
                guid: "OH".to_owned(),
                old_index: 2,
                new_index: 0,
            },
        );
        let names: Vec<&str> = tracks.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["OH", "Kick", "Snare"]);
    }

    /// **An event for a track this window has never heard of is not an
    /// error.** Attaching to a REAPER mid-session means the first events
    /// can name anything, and a window that panicked or resynced on each
    /// one would be unusable for the first second of every attach.
    #[test]
    fn an_event_for_an_unknown_track_is_ignored() {
        let mut tracks = kit();
        let before = tracks.len();
        assert_eq!(
            apply_event(&mut tracks, &E::Removed("nonesuch".to_owned())),
            Applied::Nothing
        );
        assert_eq!(tracks.len(), before);
    }

    /// A field change says so, so the window redraws a row instead of
    /// rebuilding every row map it has.
    #[test]
    fn a_field_change_is_not_a_rebuild() {
        let mut tracks = kit();
        assert_eq!(
            apply_event(
                &mut tracks,
                &E::MuteChanged {
                    guid: "Kick".to_owned(),
                    muted: true,
                }
            ),
            Applied::Field
        );
    }
}

/// Run a closure when the scope ends, however it ends.
///
/// Every stream loop here has several ways out — no facade, no project,
/// a refused subscribe, a closed stream — and each of them has to mark
/// the handle dead. Writing that at four exits is writing it at three
/// and forgetting the fourth, which in this file means a window that
/// believes it is connected.
fn scopeguard<F: FnOnce()>(f: F) -> impl Drop {
    struct Guard<F: FnOnce()>(Option<F>);
    impl<F: FnOnce()> Drop for Guard<F> {
        fn drop(&mut self) {
            if let Some(f) = self.0.take() {
                f();
            }
        }
    }
    Guard(Some(f))
}

/// Whether a subscription is still connected.
///
/// Every stream in this file ends the same way: REAPER quits, the
/// socket closes, `recv` returns `None` and the thread falls out of its
/// loop. Nothing used to notice. The window kept its handle, the
/// handle's restart check only fires when the handle is *missing*, and
/// so a REAPER quit and reopened left a window drawing the last thing
/// it heard — connected in appearance and dead in fact, which is the
/// failure a mirror must never have.
///
/// Shared rather than returned, because the thread is the only one that
/// knows and the window is the only one that can act.
#[derive(Clone)]
pub struct Alive(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Alive {
    fn new() -> Self {
        Self(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(
            true,
        )))
    }

    fn ended(&self) {
        self.0.store(false, std::sync::atomic::Ordering::Relaxed);
    }

    /// Is the stream behind this handle still delivering?
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Everything in the project that is not a track.
///
/// Items, takes, markers, regions and the tempo map are all read as a
/// snapshot and drawn from it, and until now nothing told the window
/// they had moved. An item dragged in REAPER stayed where it used to be
/// until some unrelated track was added or removed and the resulting
/// re-read happened to bring it along — which is a mirror that is right
/// only by accident.
///
/// It is a FLAG rather than a queue of events, and that is the whole
/// design. The window does not keep items, takes, markers or the tempo
/// map as anything it could patch: they are derived, in one read, from
/// the project. So the useful thing to know is "the snapshot is old",
/// and knowing it twice is the same as knowing it once — which is
/// exactly what a flag says and a queue does not. A drag in REAPER
/// emits an event per frame and this coalesces every one of them into
/// a single re-read.
///
/// Tracks are the exception and keep their own [`Watch`], because the
/// window DOES hold them and can correct a single field without paying
/// for a whole read.
pub struct Refresh {
    stale: std::sync::Arc<std::sync::atomic::AtomicBool>,
    alive: Alive,
}

impl Refresh {
    /// Subscribe. `None` if the facade is not up.
    #[must_use]
    pub fn start() -> Option<Self> {
        use daw_proto::event_bus::BusFilter;

        let runtime = crate::open::runtime()?;
        let stale = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer = std::sync::Arc::clone(&stale);
        let alive = Alive::new();
        let mine = alive.clone();
        std::thread::Builder::new()
            .name("session-daw-refresh".into())
            .spawn(move || {
                runtime.block_on(async move {
                    let _end = scopeguard(move || mine.ended());
                    let Some(daw) = daw::rpc::Daw::try_get() else {
                        return;
                    };
                    // Named field by field rather than `all()` minus a
                    // couple, because every domain switched on here
                    // costs a whole project read. Tracks are out
                    // because `Watch` has them and patches them in
                    // place. The position tick and the play state are
                    // out because they change constantly and change
                    // nothing that is drawn from the snapshot. FX and
                    // routing are out because nothing here draws
                    // them — an automated parameter would otherwise
                    // re-read the session at audio rate.
                    let filter = BusFilter {
                        items: true,
                        takes: true,
                        markers: true,
                        regions: true,
                        tempo_map: true,
                        projects: true,
                        tracks: false,
                        fx: false,
                        routing: false,
                        transport_state: false,
                        transport_position: false,
                        project_guid: None,
                    };
                    let Ok(mut stream) = daw.events().subscribe(filter).await else {
                        return;
                    };
                    while let Ok(Some(_)) = stream.recv().await {
                        writer.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                });
            })
            .ok()?;
        Some(Self { stale, alive })
    }

    /// Is this subscription still connected?
    #[must_use]
    pub fn is_live(&self) -> bool {
        self.alive.is_live()
    }

    /// Is the snapshot old?
    ///
    /// Asking does not clear it. Reading and clearing in one step reads
    /// better and is wrong here: a caller that cannot act yet — because
    /// the last read is still in flight — would throw away the news
    /// that arrived while it was reading, and the last edit of a drag
    /// is exactly the one that would go missing.
    #[must_use]
    pub fn pending(&self) -> bool {
        self.stale.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Say the snapshot has been re-read. Call this only when a read
    /// actually started.
    pub fn settled(&self) {
        self.stale
            .store(false, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(test)]
mod refresh_tests {
    use super::Refresh;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    fn refresh() -> (Refresh, Arc<AtomicBool>) {
        let stale = Arc::new(AtomicBool::new(false));
        (
            Refresh {
                stale: Arc::clone(&stale),
                alive: super::Alive::new(),
            },
            stale,
        )
    }

    /// Many events, one read.
    ///
    /// A drag in REAPER emits an event per frame, and every one of them
    /// says the same thing: the snapshot is old. Re-reading the project
    /// once per event would re-read it sixty times for one gesture.
    #[test]
    fn a_burst_of_changes_asks_for_one_read() {
        let (refresh, stale) = refresh();
        for _ in 0..60 {
            stale.store(true, Ordering::Relaxed);
        }
        assert!(refresh.pending());
        refresh.settled();
        assert!(!refresh.pending(), "one read answered all sixty");
    }

    /// A handle whose stream ended reads as dead.
    ///
    /// This is the whole of the REAPER-went-away detection. The guard
    /// fires however the thread leaves — no facade, no project, a
    /// refused subscribe, a closed stream — because writing the same
    /// line at four exits is writing it at three.
    #[test]
    fn a_stream_that_ends_marks_its_handle_dead() {
        let alive = super::Alive::new();
        assert!(alive.is_live());

        let seen = alive.clone();
        {
            let mine = alive.clone();
            let _end = super::scopeguard(move || mine.ended());
            assert!(seen.is_live(), "still connected inside the loop");
        }
        assert!(!seen.is_live(), "the window can see the connection go");
    }

    /// Asking does not clear.
    ///
    /// This is the whole reason `pending` and `settled` are two calls.
    /// A frame that cannot act — because the last read is still in
    /// flight — asks and does nothing, and the news has to still be
    /// there on the next frame. Collapsing them into one read-and-clear
    /// would lose exactly the change that arrives while the window is
    /// busy reading, which in a drag is the one that says where the
    /// item ended up.
    #[test]
    fn asking_does_not_clear() {
        let (refresh, stale) = refresh();
        stale.store(true, Ordering::Relaxed);
        assert!(refresh.pending());
        assert!(refresh.pending(), "a frame that could not act kept it");
        assert!(refresh.pending());
        refresh.settled();
        assert!(!refresh.pending(), "only a read that happened clears it");
    }
}
