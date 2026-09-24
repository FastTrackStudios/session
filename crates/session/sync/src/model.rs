//! The session, as plain data: what the document holds and what the
//! engine is told.
//!
//! Deliberately not daw-proto's types. The document is a file format —
//! it outlives every refactor of the service types — so what goes into
//! it is written down here, field by field, and nothing lands in it by
//! accident because a wire struct grew a member.

use std::collections::BTreeMap;

/// Everything collaborators share about one song's session.
///
/// Selection, the edit cursor and the play position are NOT here: they
/// are one person's, and travel as presence.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionModel {
    /// Project order, folders included — a depth-first walk of the
    /// track tree, each track naming its folder.
    pub tracks: Vec<TrackState>,
    /// By item guid.
    pub items: BTreeMap<String, ItemState>,
    /// By marker guid.
    pub markers: BTreeMap<String, MarkerState>,
    /// By region guid.
    pub regions: BTreeMap<String, RegionState>,
    /// Tempo and meter changes, in time order.
    pub tempo: Vec<TempoPoint>,
    /// The song's keyflow chart, as text.
    pub chart: String,
}

impl SessionModel {
    /// The track with this guid.
    #[must_use]
    pub fn track(&self, guid: &str) -> Option<&TrackState> {
        self.tracks.iter().find(|t| t.guid == guid)
    }
}

// The engine's own booleans, one field each: the doc merges them one by
// one, which an enum of states could not do.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackState {
    pub guid: String,
    /// The folder this track sits in; `None` at the top level.
    pub parent: Option<String>,
    pub name: String,
    /// `0xRRGGBB`; `None` is the theme's default.
    pub color: Option<u32>,
    /// A gain, 1.0 = unity — the engine's own unit.
    pub volume: f64,
    /// -1.0 (left) to 1.0 (right).
    pub pan: f64,
    pub muted: bool,
    pub soloed: bool,
    pub phase_inverted: bool,
    /// Whether it feeds its folder.
    pub parent_send: bool,
    pub visible_in_tcp: bool,
    pub visible_in_mixer: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ItemState {
    /// The guid of the track it is on.
    pub track: String,
    /// Seconds from the project start.
    pub position: f64,
    /// Seconds.
    pub length: f64,
    pub snap_offset: f64,
    pub muted: bool,
    pub locked: bool,
    pub volume: f64,
    pub fade_in: f64,
    pub fade_out: f64,
    pub fade_in_shape: String,
    pub fade_out_shape: String,
    /// The item's note — what the ruler lanes name chords, keys and
    /// lyric lines by.
    pub label: String,
    pub color: Option<u32>,
    pub take: TakeState,
}

/// The item's active take: what it plays.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TakeState {
    pub name: String,
    /// The media file, relative to the session folder. `None` for an
    /// empty (label-only) or MIDI take.
    pub source: Option<String>,
    /// Seconds into the source where the item starts.
    pub start_offset: f64,
    pub playrate: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MarkerState {
    pub at: f64,
    pub name: String,
    pub color: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RegionState {
    pub start: f64,
    pub end: f64,
    pub name: String,
    pub color: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TempoPoint {
    /// Seconds.
    pub at: f64,
    pub bpm: f64,
    pub beats_per_bar: u32,
    pub beat_unit: u32,
}
