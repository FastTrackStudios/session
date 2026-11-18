//! The song structure and the generic timeline: the tempo map, bars and
//! beats, markers, regions, lanes and timed items.
//!
//! Everything timed is placed by a [`Time`], in musical time (quarter
//! notes: it moves when the tempo changes, as a chord on bar 5 must) or in
//! absolute time (seconds: it stays, as a lighting cue on a hit point or a
//! recorded take does). The [`TempoMap`] turns either into the other.

use crate::id::{ExtensionId, LaneId};
use crate::time::{TempoMapEngine, TempoPoint, TimeSignature};
use crate::value::Value;

/// A point on the timeline, in the time base it was placed in.
#[derive(Debug, Clone, Copy, PartialEq, facet::Facet)]
#[repr(u8)]
pub enum Time {
    /// Seconds from the session's start: does not move with the tempo.
    Seconds(f64),
    /// Quarter notes from the session's start: moves with the tempo.
    QuarterNotes(f64),
}

impl Time {
    /// Where this is, in seconds, under `tempo`.
    #[must_use]
    pub fn seconds(self, tempo: &TempoMap) -> f64 {
        match self {
            Self::Seconds(s) => s,
            Self::QuarterNotes(qn) => tempo.engine.quarter_notes_to_seconds(qn),
        }
    }

    /// Where this is, in quarter notes, under `tempo`.
    #[must_use]
    pub fn quarter_notes(self, tempo: &TempoMap) -> f64 {
        match self {
            Self::Seconds(s) => tempo.engine.seconds_to_quarter_notes(s),
            Self::QuarterNotes(qn) => qn,
        }
    }

    /// Where this is in bars and beats, under `tempo`.
    #[must_use]
    pub fn bar_beat(self, tempo: &TempoMap) -> BarBeat {
        tempo.bar_beat(self.quarter_notes(tempo))
    }
}

/// A place in bars and beats: bar 1 beat 1 is the session's start.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarBeat {
    /// 1-based.
    pub bar: i32,
    /// 1-based, in the meter's beat unit (an eighth in 6/8).
    pub beat: i32,
    /// How far into the beat, `0.0..1.0`.
    pub fraction: f64,
}

/// The tempo and meter map: a default tempo and meter, and the changes.
///
/// One value on purpose: a tempo map is only meaningful whole, so two
/// people editing it at once is last-writer-wins for the whole of it, not a
/// merge of halves neither of them wrote (as Studio's `SessionDoc` already
/// decided).
#[derive(Debug, Clone)]
pub struct TempoMap {
    default_bpm: f64,
    default_meter: TimeSignature,
    engine: TempoMapEngine,
}

impl TempoMap {
    /// A map at `default_bpm` in `default_meter`, changing at `points`
    /// (positioned in seconds; any order).
    #[must_use]
    pub fn new(default_bpm: f64, default_meter: TimeSignature, points: Vec<TempoPoint>) -> Self {
        Self {
            default_bpm,
            default_meter,
            engine: TempoMapEngine::new(default_bpm, default_meter, points),
        }
    }

    #[must_use]
    pub const fn default_bpm(&self) -> f64 {
        self.default_bpm
    }

    #[must_use]
    pub const fn default_meter(&self) -> TimeSignature {
        self.default_meter
    }

    /// The changes, ordered by position.
    #[must_use]
    pub fn points(&self) -> &[TempoPoint] {
        self.engine.points()
    }

    /// The evaluation engine (`daw-proto`'s, copied).
    #[must_use]
    pub const fn engine(&self) -> &TempoMapEngine {
        &self.engine
    }

    /// The map compiled for the audio thread: every conversion a binary
    /// search and arithmetic, with no allocation, ramps included. Build it
    /// off the audio thread when the map changes; ask it from anywhere.
    #[must_use]
    pub fn plan(&self) -> crate::time::TempoPlan {
        crate::time::TempoPlan::compile(&self.engine, self.default_bpm, self.default_meter)
    }

    /// The tempo at `seconds`, BPM (quarter notes a minute).
    #[must_use]
    pub fn bpm_at(&self, seconds: f64) -> f64 {
        self.engine.tempo_at(seconds)
    }

    /// The meter at `seconds`.
    #[must_use]
    pub fn meter_at(&self, seconds: f64) -> TimeSignature {
        self.engine.time_signature_at(seconds)
    }

    /// `quarter_notes` in bars and beats.
    #[must_use]
    pub fn bar_beat(&self, quarter_notes: f64) -> BarBeat {
        let (bar, beat, fraction) = self.engine.quarter_notes_to_musical(quarter_notes);
        BarBeat {
            bar,
            beat,
            fraction,
        }
    }

    /// Bar `bar`, beat `beat` (both 1-based), as a musical [`Time`].
    #[must_use]
    pub fn at_bar(&self, bar: i32, beat: i32, fraction: f64) -> Time {
        Time::QuarterNotes(self.engine.musical_to_quarter_notes(bar, beat, fraction))
    }

    /// A change at `seconds`: `bpm`, and a new meter when `meter` is some.
    #[must_use]
    pub fn point(seconds: f64, bpm: f64, meter: Option<TimeSignature>) -> TempoPoint {
        let mut point = TempoPoint::from_seconds(seconds, bpm);
        point.time_signature = meter;
        point
    }
}

impl Default for TempoMap {
    fn default() -> Self {
        Self::new(120.0, TimeSignature::COMMON_TIME, Vec::new())
    }
}

impl PartialEq for TempoMap {
    fn eq(&self, other: &Self) -> bool {
        self.default_bpm.total_cmp(&other.default_bpm).is_eq()
            && self.default_meter == other.default_meter
            && self.points() == other.points()
    }
}

/// A named point in the song: "Verse 2", "Key change", "Drop".
#[derive(Debug, Clone, PartialEq, facet::Facet)]
pub struct Marker {
    pub at: Time,
    pub name: String,
    /// `0xRRGGBB`; `None` for the default.
    pub color: Option<u32>,
    /// Which ruler it sits on ([`crate::Session::rulers`]); `None`: the
    /// default one.
    pub ruler: Option<u32>,
}

/// A named span of the song: a section, a song in a set, a loop range.
#[derive(Debug, Clone, PartialEq, facet::Facet)]
pub struct Region {
    pub start: Time,
    pub end: Time,
    pub name: String,
    pub color: Option<u32>,
    pub ruler: Option<u32>,
}

/// A lane of the timeline: a row something is drawn and played on.
///
/// The session gives lanes no meaning. The DAW's audio track is a lane of
/// kind `daw.track` owned by `daw`; Ignition's cue list is a lane of kind
/// `ignition.cues`; Keyflow's chord row a lane of kind `keyflow.chords`.
#[derive(Debug, Clone, PartialEq, facet::Facet)]
pub struct Lane {
    /// The extension that made it and owns its meaning.
    pub owner: ExtensionId,
    /// What it is, namespaced by its owner (`daw.track`).
    pub kind: String,
    pub name: String,
    /// Sort key among its siblings (a fractional index: insert between
    /// two lanes by taking a value between theirs, so a concurrent move
    /// and insert both land).
    pub order: f64,
    /// The lane it is nested in (a folder track, a cue list's group).
    pub parent: Option<LaneId>,
}

/// Something at a time on a lane: a clip, a cue, a chord, a lyric line, a
/// patch change.
#[derive(Debug, Clone, PartialEq, facet::Facet)]
pub struct Item {
    /// The extension that made it and interprets it.
    pub owner: ExtensionId,
    pub lane: LaneId,
    /// What it is, namespaced by its owner (`ignition.cue`).
    pub kind: String,
    pub at: Time,
    /// Where it ends; `None` for an instant (a cue, a patch change).
    pub end: Option<Time>,
    /// What the owner stores; the session does not read it.
    pub payload: Value,
}

impl Item {
    /// Where it starts and ends, in seconds, under `tempo`.
    #[must_use]
    pub fn seconds(&self, tempo: &TempoMap) -> (f64, Option<f64>) {
        (
            self.at.seconds(tempo),
            self.end.map(|end| end.seconds(tempo)),
        )
    }
}
