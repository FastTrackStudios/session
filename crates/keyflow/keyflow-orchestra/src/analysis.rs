//! Score analysis reports — instrumentation, articulation inventory, ranges.

use std::collections::BTreeSet;

use crate::profile::{ProfileKind, detect_profile};
use crate::score::{MarkingKind, Part, Score};

/// Per-part analysis summary.
#[derive(Debug, Clone)]
pub struct PartReport {
    pub id: String,
    pub name: String,
    pub profile: ProfileKind,
    pub note_count: usize,
    pub voices: Vec<u32>,
    /// Max simultaneous same-voice notes (divisi width) across all voices.
    pub max_chord_width: usize,
    /// MIDI pitch range (min, max).
    pub range: Option<(i32, i32)>,
    /// Every articulation/technical/ornament tag seen.
    pub articulations: BTreeSet<String>,
    /// Dynamic mark names seen (p, mf, sfz, …).
    pub dynamics: BTreeSet<String>,
    pub has_slurs: bool,
    pub has_ties: bool,
    pub has_grace: bool,
    pub has_gliss: bool,
    pub first_note_qn: Option<f64>,
    pub last_note_end_qn: Option<f64>,
}

/// Whole-score analysis summary.
#[derive(Debug, Clone)]
pub struct ScoreReport {
    pub title: Option<String>,
    pub parts: Vec<PartReport>,
    pub tempo_changes: usize,
    pub bpm_range: Option<(f64, f64)>,
    pub meters: Vec<(u32, u32)>,
    pub rehearsal_marks: Vec<String>,
    /// Total length in QN (max over parts).
    pub length_qn: f64,
}

pub fn analyze(score: &Score) -> ScoreReport {
    let parts: Vec<PartReport> = score.parts.iter().map(analyze_part).collect();
    let bpm_range = score.meta.tempos.iter().map(|t| t.bpm).fold(
        None,
        |acc: Option<(f64, f64)>, b| match acc {
            None => Some((b, b)),
            Some((lo, hi)) => Some((lo.min(b), hi.max(b))),
        },
    );
    let mut meters: Vec<(u32, u32)> = Vec::new();
    for m in &score.meta.meters {
        let sig = (m.beats, m.beat_type);
        if meters.last() != Some(&sig) {
            meters.push(sig);
        }
    }
    ScoreReport {
        title: score.work_title.clone(),
        length_qn: parts
            .iter()
            .filter_map(|p| p.last_note_end_qn)
            .fold(0.0, f64::max),
        tempo_changes: score.meta.tempos.len(),
        bpm_range,
        meters,
        rehearsal_marks: score
            .meta
            .rehearsals
            .iter()
            .filter_map(|m| match &m.kind {
                MarkingKind::Rehearsal(t) => Some(t.clone()),
                _ => None,
            })
            .collect(),
        parts,
    }
}

fn analyze_part(part: &Part) -> PartReport {
    let mut voices = BTreeSet::new();
    let mut articulations = BTreeSet::new();
    let mut range: Option<(i32, i32)> = None;
    let mut has_slurs = false;
    let mut has_ties = false;
    let mut has_grace = false;
    let mut has_gliss = false;
    for n in &part.notes {
        voices.insert(n.voice);
        for t in n.art.iter() {
            articulations.insert(t.to_string());
        }
        has_slurs |= n.slur_start || n.slur_stop;
        has_ties |= n.tie_start || n.tie_stop;
        has_grace |= n.has("grace");
        has_gliss |= n.has("glissando");
        range = Some(match range {
            None => (n.pitch, n.pitch),
            Some((lo, hi)) => (lo.min(n.pitch), hi.max(n.pitch)),
        });
    }

    // Divisi width: max count of same-voice notes sounding at any onset.
    let mut max_width = if part.notes.is_empty() { 0 } else { 1 };
    for n in &part.notes {
        let sounding = part
            .notes
            .iter()
            .filter(|m| {
                m.voice == n.voice && m.onset <= n.onset + 1e-6 && m.onset + m.dur > n.onset + 1e-6
            })
            .count();
        max_width = max_width.max(sounding);
    }

    let dynamics = part
        .markings
        .iter()
        .filter_map(|m| match &m.kind {
            MarkingKind::Dynamic {
                name: Some(name), ..
            } => Some(name.clone()),
            _ => None,
        })
        .collect();

    PartReport {
        id: part.id.clone(),
        name: part.name.clone(),
        profile: detect_profile(&part.name),
        note_count: part.notes.len(),
        voices: voices.into_iter().collect(),
        max_chord_width: max_width,
        range,
        articulations,
        dynamics,
        has_slurs,
        has_ties,
        has_grace,
        has_gliss,
        first_note_qn: part
            .notes
            .iter()
            .map(|n| n.onset)
            .fold(None, |a: Option<f64>, o| Some(a.map_or(o, |a| a.min(o)))),
        last_note_end_qn: part
            .notes
            .iter()
            .map(|n| n.onset + n.dur)
            .fold(None, |a: Option<f64>, e| Some(a.map_or(e, |a| a.max(e)))),
    }
}
