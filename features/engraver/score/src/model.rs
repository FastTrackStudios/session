//! Thin notation IR — the written score as engraving input.
//!
//! This is deliberately NOT an editor DOM (see docs/spec/score-engraving.md):
//! it preserves exactly what layout needs from MusicXML — written pitch
//! spelling, voices, rests, clefs/keys/meters, tuplets, ties — in an
//! immutable, measure-partwise shape. Playback concerns (QN timelines, CC
//! curves) stay in `keyflow-orchestra`; chart concerns stay in
//! `keyflow-musicxml`.

use std::collections::BTreeSet;

/// A full score: ordered parts, each with the same measure grid.
#[derive(Debug, Clone, Default)]
pub struct Score {
    pub work_title: Option<String>,
    pub movement_title: Option<String>,
    pub composer: Option<String>,
    pub arranger: Option<String>,
    pub lyricist: Option<String>,
    pub parts: Vec<Part>,
}

/// One instrument part (may span multiple staves, e.g. piano/harp = 2).
#[derive(Debug, Clone)]
pub struct Part {
    /// MusicXML part id (`P1`, …).
    pub id: String,
    /// Display name from `<part-list>` (entities decoded).
    pub name: String,
    /// Abbreviated name for subsequent systems (`Vln.`, `Tpt.`).
    pub abbreviation: Option<String>,
    /// Highest `<staves>` value seen (1 for single-staff parts).
    pub staves: u8,
    /// Written transposition in effect (B♭/F parts), if any.
    pub transpose: Option<Transposition>,
    pub measures: Vec<Measure>,
}

/// Written transposition (`<transpose>`): sounding = written + chromatic
/// + 12·octave_change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transposition {
    pub diatonic: i8,
    pub chromatic: i8,
    pub octave_change: i8,
}

/// One measure of one part: attribute changes + timed events.
#[derive(Debug, Clone, Default)]
pub struct Measure {
    /// MusicXML `number` attribute (usually numeric; "0" for pickups).
    pub number: String,
    /// Pickup / non-counting measure (`implicit="yes"`).
    pub implicit: bool,
    /// Attribute changes in this measure, in document order.
    pub attributes: Vec<AttrChange>,
    /// Notes/rests, each with a tick onset from measure start.
    pub events: Vec<Event>,
    /// Divisions-per-quarter in effect for this measure's ticks.
    pub divisions: u32,
    /// Measure length in ticks (from the inherited meter; content may
    /// overflow on malformed input — layout clamps).
    pub len_ticks: u32,
}

/// A mid-stream attribute change (clef/key/time/divisions/staves).
#[derive(Debug, Clone, Default)]
pub struct AttrChange {
    /// Tick position within the measure (usually 0).
    pub tick: u32,
    pub divisions: Option<u32>,
    /// Key signature as circle-of-fifths count.
    pub key_fifths: Option<i8>,
    /// Time signature (beats, beat-type).
    pub time: Option<(u32, u32)>,
    pub staves: Option<u8>,
    /// Clef changes: (staff number 1-based, clef).
    pub clefs: Vec<(u8, Clef)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Clef {
    pub sign: ClefSign,
    /// Staff line (2 = treble G, 4 = bass F, 3 = alto C…). None = sign default.
    pub line: Option<u8>,
    /// Octave transposition marks (−1 = tenor-voice G clef with 8 below).
    pub octave_change: i8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClefSign {
    G,
    F,
    C,
    Percussion,
    Tab,
    None,
}

/// A timed measure event: a chord (≥1 notes sharing a stem) or a rest.
#[derive(Debug, Clone)]
pub enum Event {
    Chord(Chord),
    Rest(Rest),
}

impl Event {
    #[must_use]
    pub fn tick(&self) -> u32 {
        match self {
            Event::Chord(c) => c.tick,
            Event::Rest(r) => r.tick,
        }
    }

    #[must_use]
    pub fn voice(&self) -> u32 {
        match self {
            Event::Chord(c) => c.voice,
            Event::Rest(r) => r.voice,
        }
    }
}

/// Written duration info shared by chords and rests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WrittenDuration {
    /// Written note type (`<type>`), if given. Whole=1 … 1024th.
    pub note_type: Option<NoteTypeValue>,
    pub dots: u8,
    /// Tuplet time modification (actual, normal), e.g. (3, 2) for triplets.
    pub tuplet: Option<(u8, u8)>,
}

/// One stem's worth of notes (MusicXML principal note + `<chord>` followers).
#[derive(Debug, Clone)]
pub struct Chord {
    /// Tick onset from measure start.
    pub tick: u32,
    /// Duration in ticks (0 for grace notes).
    pub duration: u32,
    pub voice: u32,
    /// Staff within the part, 1-based.
    pub staff: u8,
    pub written: WrittenDuration,
    pub grace: bool,
    /// Stacked notes, in document order.
    pub notes: Vec<Note>,
    pub slur_start: bool,
    pub slur_stop: bool,
    /// Open articulation tag set (staccato, accent, fermata, …), MusicXML
    /// kebab-case child names.
    pub articulations: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct Note {
    pub pitch: WrittenPitch,
    pub tie_start: bool,
    pub tie_stop: bool,
    /// Printed accidental (`<accidental>` text: "sharp", "natural", …),
    /// when the file states one explicitly.
    pub accidental: Option<String>,
}

/// Written pitch spelling: step letter + alteration + octave. Never reduced
/// to MIDI — F♯ and G♭ engrave differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrittenPitch {
    pub step: Step,
    /// Chromatic alteration: −2 (𝄫) … +2 (𝄪).
    pub alter: i8,
    /// Scientific octave (4 = middle-C octave).
    pub octave: i8,
}

impl WrittenPitch {
    /// Sounding MIDI note number (before part transposition).
    #[must_use]
    pub fn midi(&self) -> i32 {
        let step = match self.step {
            Step::C => 0,
            Step::D => 2,
            Step::E => 4,
            Step::F => 5,
            Step::G => 7,
            Step::A => 9,
            Step::B => 11,
        };
        (i32::from(self.octave) + 1) * 12 + step + i32::from(self.alter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
}

/// Written note type values (`<type>`), longest to shortest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteTypeValue {
    Maxima,
    Long,
    Breve,
    Whole,
    Half,
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
    SixtyFourth,
    OneHundredTwentyEighth,
    TwoHundredFiftySixth,
    FiveHundredTwelfth,
    OneThousandTwentyFourth,
}

#[derive(Debug, Clone)]
pub struct Rest {
    pub tick: u32,
    pub duration: u32,
    pub voice: u32,
    pub staff: u8,
    pub written: WrittenDuration,
    /// `<rest measure="yes"/>` — a whole-measure rest regardless of meter.
    pub measure_rest: bool,
}
