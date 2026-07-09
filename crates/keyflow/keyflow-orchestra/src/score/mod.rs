//! Score analysis primitives — parse `.mxl`/`.musicxml` into a QN-domain model.
//!
//! Port of `css_engine.lua` §2–4 (`getParts` / `parsePart` / `getScoreMeta`) on
//! top of the `musicxml` crate. Everything is in **quarter-notes from score
//! start**; MusicXML `divisions` ticks never escape this module.

mod parse;

use std::collections::BTreeSet;

pub use parse::load;

/// A parsed score: every part fully extracted, plus the unioned score-wide
/// timeline (tempo map / meters / rehearsal marks).
#[derive(Debug, Clone)]
pub struct Score {
    pub work_title: Option<String>,
    pub parts: Vec<Part>,
    pub meta: ScoreMeta,
}

/// One `<part>` — raw events in QN, before any engine processing.
#[derive(Debug, Clone)]
pub struct Part {
    pub id: String,
    pub name: String,
    pub notes: Vec<RawNote>,
    pub dynamics: Vec<DynamicPoint>,
    pub tempos: Vec<TempoPoint>,
    pub meters: Vec<MeterPoint>,
    pub markings: Vec<Marking>,
    pub harmonies: Vec<HarmonyPoint>,
}

/// Score-wide timeline, unioned across all parts and deduped — different
/// parts may carry different markings, but the project has one tempo map.
#[derive(Debug, Clone, Default)]
pub struct ScoreMeta {
    pub tempos: Vec<TempoPoint>,
    pub meters: Vec<MeterPoint>,
    pub rehearsals: Vec<Marking>,
}

/// A raw score note (or expanded grace note), straight from the measure walk.
#[derive(Debug, Clone)]
pub struct RawNote {
    /// Onset in QN from score start.
    pub onset: f64,
    /// Duration in QN.
    pub dur: f64,
    /// MIDI pitch.
    pub pitch: i32,
    /// MusicXML voice number (1 if unmarked).
    pub voice: u32,
    pub tie_start: bool,
    pub tie_stop: bool,
    /// Open set of articulation/technical/ornament tags on the note
    /// (`staccato`, `strong-accent`, `tremolo`, `grace`, `glissando`, …).
    pub art: ArtSet,
    pub slur_start: bool,
    pub slur_stop: bool,
    /// Onset position within its bar (QN).
    pub beat_qn: f64,
    /// Meter in effect (numerator / denominator).
    pub beats: u32,
    pub beat_type: u32,
    /// Key signature in effect (circle-of-fifths count, for harp gliss scales).
    pub fifths: i32,
}

impl RawNote {
    pub fn has(&self, tag: &str) -> bool {
        self.art.has(tag)
    }
}

/// Open articulation-tag set (MusicXML child-tag names, kebab-case).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtSet(pub BTreeSet<String>);

impl ArtSet {
    pub fn has(&self, tag: &str) -> bool {
        self.0.contains(tag)
    }
    pub fn insert(&mut self, tag: impl Into<String>) {
        self.0.insert(tag.into());
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn union_with(&mut self, other: &ArtSet) {
        for t in &other.0 {
            self.0.insert(t.clone());
        }
    }
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(String::as_str)
    }
    /// Comma-joined tag list (annotation format, matches the Lua `scoreArt`).
    pub fn joined(&self) -> String {
        self.0.iter().cloned().collect::<Vec<_>>().join(",")
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicPoint {
    pub qn: f64,
    /// CC-domain dynamics value 0–127.
    pub value: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoPoint {
    pub qn: f64,
    pub bpm: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeterPoint {
    pub qn_x10000: i64, // exact-dedup key; see `MeterPoint::qn()`
    pub beats: u32,
    pub beat_type: u32,
}

impl MeterPoint {
    pub fn new(qn: f64, beats: u32, beat_type: u32) -> Self {
        Self {
            qn_x10000: (qn * 10000.0).round() as i64,
            beats,
            beat_type,
        }
    }
    pub fn qn(&self) -> f64 {
        self.qn_x10000 as f64 / 10000.0
    }
}

/// A score marking kept for annotation/reporting.
#[derive(Debug, Clone, PartialEq)]
pub struct Marking {
    pub qn: f64,
    pub kind: MarkingKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MarkingKind {
    Tempo(f64),
    Rehearsal(String),
    Words(String),
    Dynamic { name: Option<String>, value: f64 },
}

/// A `<harmony>` chord symbol (for harp gliss pitch sets).
#[derive(Debug, Clone, PartialEq)]
pub struct HarmonyPoint {
    pub qn: f64,
    pub root_pc: i32,
    pub kind: String,
    pub degrees: Vec<HarmonyDegree>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HarmonyDegree {
    pub value: i32,
    pub alter: i32,
    pub kind: DegreeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegreeKind {
    Add,
    Alter,
    Subtract,
}

/// BPM in effect at a given QN (port of `bpmAt`).
pub fn bpm_at(tempos: &[TempoPoint], qn: f64) -> f64 {
    let mut bpm = tempos.first().map(|t| t.bpm).unwrap_or(120.0);
    for t in tempos {
        if t.qn <= qn + 1e-9 {
            bpm = t.bpm;
        } else {
            break;
        }
    }
    bpm
}

/// Pitch-class set of a harmony (root + kind + degree edits) — port of `chordPCs`.
pub fn chord_pcs(h: &HarmonyPoint) -> [bool; 12] {
    let intervals: &[i32] = match h.kind.as_str() {
        "major" => &[0, 4, 7],
        "minor" => &[0, 3, 7],
        "augmented" => &[0, 4, 8],
        "diminished" => &[0, 3, 6],
        "dominant" | "dominant-seventh" => &[0, 4, 7, 10],
        "major-seventh" => &[0, 4, 7, 11],
        "minor-seventh" => &[0, 3, 7, 10],
        "diminished-seventh" => &[0, 3, 6, 9],
        "half-diminished" => &[0, 3, 6, 10],
        "major-minor" | "minor-major" => &[0, 3, 7, 11],
        "major-sixth" => &[0, 4, 7, 9],
        "minor-sixth" => &[0, 3, 7, 9],
        "suspended-fourth" => &[0, 5, 7],
        "suspended-second" => &[0, 2, 7],
        "power" => &[0, 7],
        "dominant-ninth" => &[0, 4, 7, 10, 2],
        "major-ninth" => &[0, 4, 7, 11, 2],
        "minor-ninth" => &[0, 3, 7, 10, 2],
        _ => &[0, 4, 7],
    };
    let mut set = [false; 12];
    for iv in intervals {
        set[((h.root_pc + iv).rem_euclid(12)) as usize] = true;
    }
    for d in &h.degrees {
        let base = degree_semitones(d.value);
        match d.kind {
            DegreeKind::Alter => {
                set[((h.root_pc + base).rem_euclid(12)) as usize] = false;
                set[((h.root_pc + base + d.alter).rem_euclid(12)) as usize] = true;
            }
            DegreeKind::Add => {
                set[((h.root_pc + base + d.alter).rem_euclid(12)) as usize] = true;
            }
            DegreeKind::Subtract => {
                set[((h.root_pc + base + d.alter).rem_euclid(12)) as usize] = false;
            }
        }
    }
    set
}

fn degree_semitones(value: i32) -> i32 {
    match value {
        1 => 0,
        2 | 9 => 2,
        3 => 4,
        4 | 11 => 5,
        5 => 7,
        6 | 13 => 9,
        7 => 11,
        _ => 0,
    }
}

/// Fallback CC value when a `<dynamics>` mark has no numeric `<sound dynamics>`
/// (port of `DYN_MARK`).
pub fn dynamic_mark_value(name: &str) -> Option<f64> {
    Some(match name {
        "pppp" => 8.0,
        "ppp" => 16.0,
        "pp" => 33.0,
        "p" => 49.0,
        "mp" => 64.0,
        "mf" => 80.0,
        "f" | "fp" | "rf" | "rfz" => 96.0,
        "ff" | "sffz" => 112.0,
        "fff" => 120.0,
        "ffff" => 127.0,
        "sf" => 100.0,
        "sfz" | "fz" => 105.0,
        _ => return None,
    })
}

/// Recognise a dynamic level typed as plain `<words>` text (port of `DYN_WORD` +
/// `dynamicFromText`): scans whole words, incl. spelled-out and Italian forms.
pub fn dynamic_from_text(text: &str) -> Option<(f64, String)> {
    let lower = text.to_lowercase();
    // Words = maximal runs of [a-z-] starting with a letter, like the Lua
    // pattern "[%a][%a%-]*".
    let mut word = String::new();
    let mut out: Option<(f64, String)> = None;
    let mut chars = lower.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_alphabetic() {
            word.push(c);
            while let Some(&n) = chars.peek() {
                if n.is_ascii_alphabetic() || n == '-' {
                    word.push(n);
                    chars.next();
                } else {
                    break;
                }
            }
            if let Some(v) = dyn_word_value(&word) {
                out = Some((v, std::mem::take(&mut word)));
                break;
            }
            word.clear();
        }
    }
    out
}

fn dyn_word_value(word: &str) -> Option<f64> {
    Some(match word {
        "pppp" => 8.0,
        "ppp" | "pianississimo" => 16.0,
        "pp" | "pianissimo" => 33.0,
        "p" | "piano" => 49.0,
        "mp" | "mezzo-piano" | "mezzopiano" => 64.0,
        "mf" | "mezzo-forte" | "mezzoforte" => 80.0,
        "f" | "forte" | "fp" | "forte-piano" | "fortepiano" | "rf" | "rfz" => 96.0,
        "ff" | "fortissimo" | "sffz" => 112.0,
        "fff" | "fortississimo" => 120.0,
        "ffff" => 127.0,
        "sf" => 100.0,
        "sfz" | "fz" => 105.0,
        "sfp" => 90.0,
        _ => return None,
    })
}
