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
    /// REAPER 7's fixed lanes: how many, what each is called, which play.
    pub lanes: LaneState,
    /// Its part in each track group: a family's lead or follow mask by
    /// name (`recarm_lead`, `media_edit_follow`, …), the non-zero ones.
    pub grouping: BTreeMap<String, u128>,
    /// What extensions and the template wrote on it (`FTS:kind` →
    /// `piece`): REAPER's `P_EXT`, by key.
    pub ext: BTreeMap<String, String>,
    /// Its FX, in chain order.
    pub fx: Vec<FxState>,
    /// Its sends, in order.
    pub sends: Vec<SendState>,
    /// Its automation, by what is automated ([`EnvelopeState`]).
    pub envelopes: BTreeMap<String, EnvelopeState>,
}

/// A track's fixed lanes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaneState {
    pub count: u32,
    pub names: Vec<String>,
    /// Bit `n`: lane `n` plays.
    pub play_mask: u64,
}

/// One FX on a track.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FxState {
    pub guid: String,
    /// What the engine adds it by: a built-in's name, a plugin's id or
    /// bundle.
    pub plugin: String,
    /// As shown.
    pub name: String,
    pub enabled: bool,
    pub offline: bool,
    /// Everything it holds (its parameters, a sampler's kit, a rig's
    /// capture), as a blob's name in the project's store — kept once,
    /// however many versions hold it.
    pub state: Option<String>,
}

/// One send from a track.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SendState {
    /// The destination track's guid.
    pub dest: String,
    /// `post_fader`, `pre_fx`, `post_fx`.
    pub mode: String,
    pub volume: f64,
    pub pan: f64,
    pub muted: bool,
    /// The source's first channel (0: its main pair, 2: its 3/4, 4: 5/6).
    pub from: u32,
    /// The destination's first channel (2: its sidechain).
    pub to: u32,
}

/// One automation envelope: what plays it, and its points.
///
/// Keyed on its track by what it automates: `volume`, `pan`, `mute`,
/// `volume_prefx`, `pan_prefx`, `width`, or `fx:<fx guid>:<param>`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EnvelopeState {
    /// `read`, `trim`, `touch`, `latch`, `latch_preview`, `write`, `off`.
    pub mode: String,
    pub visible: bool,
    pub armed: bool,
    pub points: Vec<EnvelopePointState>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EnvelopePointState {
    /// Seconds.
    pub at: f64,
    pub value: f64,
    /// REAPER's shape number: 0 linear, 1 square, 2 slow start/end, 3
    /// fast start, 4 fast end, 5 bezier.
    pub shape: u8,
    pub tension: f64,
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
    /// The fixed lane it sits on, on a track with lanes.
    pub fixed_lane: Option<u32>,
    /// Its source loops across it.
    pub loop_source: bool,
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
    /// A MIDI take (its notes are [`Self::notes`]), not audio.
    pub midi: bool,
    /// Its stretch markers, in order.
    pub stretch: Vec<StretchPoint>,
    /// A MIDI take's notes, in order.
    pub notes: Vec<NoteState>,
}

/// A stretch marker: where in the take it sits and what source moment
/// it holds there.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StretchPoint {
    /// Seconds into the take.
    pub at: f64,
    /// Seconds into the source.
    pub source: f64,
    pub slope: f64,
}

/// One MIDI note, in the take's quarter notes.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct NoteState {
    pub start: f64,
    pub length: f64,
    pub pitch: u8,
    pub velocity: u8,
    pub channel: u8,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct MarkerState {
    pub at: f64,
    pub name: String,
    pub color: Option<u32>,
    /// Which ruler lane it sits on (none: the default).
    pub lane: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RegionState {
    pub start: f64,
    pub end: f64,
    pub name: String,
    pub color: Option<u32>,
    /// Which ruler lane it sits on (none: the default) — a song's region
    /// on one, its sections on another.
    pub lane: Option<u32>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct TempoPoint {
    /// Seconds.
    pub at: f64,
    pub bpm: f64,
    pub beats_per_bar: u32,
    pub beat_unit: u32,
}
