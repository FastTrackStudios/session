//! engraver-score — notation score model + MusicXML importer.
//!
//! P1 of the score-engraving effort (docs/spec/score-engraving.md, issue
//! #78): a thin written-notation IR ([`model::Score`]) and a MusicXML
//! importer that preserves pitch spelling, voices, rests, clefs, and meters
//! — everything the engraver needs to lay out full orchestral scores and
//! extracted parts. Playback flattening lives in `keyflow-orchestra`; chart
//! conversion lives in `keyflow-musicxml`.

pub mod import;
pub mod layout;
pub mod model;
pub mod render;

pub use import::{ImportError, import_bytes, import_file};
pub use model::Score;

use std::collections::BTreeSet;

/// Structural summary of one part — the P1 acceptance surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartInventory {
    pub id: String,
    pub name: String,
    pub staves: u8,
    pub measures: usize,
    /// Chord events (stems), grace chords included.
    pub chords: usize,
    /// Total stacked note heads across all chords.
    pub notes: usize,
    pub grace_chords: usize,
    pub rests: usize,
    pub measure_rests: usize,
    pub voices: BTreeSet<u32>,
    /// Distinct clefs seen, in first-seen order, as "G2"/"F4"/"C3"/"perc" strings.
    pub clefs: Vec<String>,
    /// Key changes as fifths values, in order, deduped consecutively.
    pub keys: Vec<i8>,
    /// Time signature changes, in order, deduped consecutively.
    pub times: Vec<(u32, u32)>,
}

/// Build per-part inventories for a score.
#[must_use]
pub fn inventory(score: &Score) -> Vec<PartInventory> {
    score
        .parts
        .iter()
        .map(|part| {
            let mut inv = PartInventory {
                id: part.id.clone(),
                name: part.name.clone(),
                staves: part.staves,
                measures: part.measures.len(),
                chords: 0,
                notes: 0,
                grace_chords: 0,
                rests: 0,
                measure_rests: 0,
                voices: BTreeSet::new(),
                clefs: Vec::new(),
                keys: Vec::new(),
                times: Vec::new(),
            };
            for measure in &part.measures {
                for change in &measure.attributes {
                    for (_, clef) in &change.clefs {
                        let s = clef_label(clef);
                        if !inv.clefs.contains(&s) {
                            inv.clefs.push(s);
                        }
                    }
                    if let Some(k) = change.key_fifths
                        && inv.keys.last() != Some(&k)
                    {
                        inv.keys.push(k);
                    }
                    if let Some(t) = change.time
                        && inv.times.last() != Some(&t)
                    {
                        inv.times.push(t);
                    }
                }
                for event in &measure.events {
                    inv.voices.insert(event.voice());
                    match event {
                        model::Event::Chord(c) => {
                            inv.chords += 1;
                            inv.notes += c.notes.len();
                            if c.grace {
                                inv.grace_chords += 1;
                            }
                        }
                        model::Event::Rest(r) => {
                            inv.rests += 1;
                            if r.measure_rest {
                                inv.measure_rests += 1;
                            }
                        }
                    }
                }
            }
            inv
        })
        .collect()
}

fn clef_label(clef: &model::Clef) -> String {
    use model::ClefSign;
    match clef.sign {
        ClefSign::G => format!("G{}", clef.line.unwrap_or(2)),
        ClefSign::F => format!("F{}", clef.line.unwrap_or(4)),
        ClefSign::C => format!("C{}", clef.line.unwrap_or(3)),
        ClefSign::Percussion => "perc".to_string(),
        ClefSign::Tab => "tab".to_string(),
        ClefSign::None => "none".to_string(),
    }
}
