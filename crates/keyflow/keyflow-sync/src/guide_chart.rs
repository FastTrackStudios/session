//! Recover a keyflow section arrangement from spoken guide-track cues.
//!
//! A worship "Guide"/"Cue" stem speaks the upcoming section names and count-ins
//! ("…Refrain… Pre-chorus, 3, 4, Chorus, all in…"). Given those words with
//! timestamps (from Whisper + forced alignment upstream) and a [`ClickGrid`],
//! this snaps each recognized section cue to a bar and emits keyflow chart
//! text: one section per line with its length in bars.
//!
//! This module is pure (no audio/ML deps) so it unit-tests without models; the
//! CLI supplies the transcribed, aligned cues.

use crate::click::ClickGrid;

/// One transcribed word with its aligned time and confidence.
#[derive(Debug, Clone)]
pub struct SectionCue {
    pub word: String,
    pub time_sec: f32,
    pub confidence: f32,
}

/// Map a spoken word (lowercased, punctuation-stripped) to a keyflow section
/// abbreviation, or `None` if it isn't a section keyword. Tolerant of common
/// Whisper mishearings (e.g. "pricorus" for "pre-chorus").
fn keyword_to_kf(word: &str) -> Option<&'static str> {
    let w: String = word
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .collect();
    let m = match w.as_str() {
        _ if w.contains("refrain") => "Refrain",
        _ if w.contains("prechorus") || w.contains("pricorus") => "PRE",
        _ if w.contains("verse") => "VS",
        _ if w.contains("chorus") || w.contains("corus") => "CH",
        _ if w.contains("bridge") => "BR",
        _ if w.contains("interlude") => "Interlude",
        _ if w.contains("instrumental") => "INST",
        _ if w.contains("breakdown") => "Breakdown",
        _ if w.contains("intro") => "In",
        _ if w.contains("outro") || w.contains("ending") => "Outro",
        _ if w.contains("vamp") => "Vamp",
        _ if w.contains("tag") => "Tag",
        _ => return None,
    };
    Some(m)
}

/// A recovered section: its keyflow abbreviation, the bar it starts on, and its
/// length in bars (gap to the next section).
#[derive(Debug, Clone, PartialEq)]
pub struct RecoveredSection {
    pub kf: &'static str,
    pub start_bar: u32,
    pub bars: u32,
}

/// Turn timestamped guide cues into recovered sections, snapping each to the
/// click grid. `min_confidence` drops low-confidence hallucinations. Cues within
/// the same bar collapse to the first (a name is often repeated as it's sung).
pub fn recover_sections(
    cues: &[SectionCue],
    grid: &ClickGrid,
    min_confidence: f32,
) -> Vec<RecoveredSection> {
    // Keep recognized, confident cues in time order, snapped to a bar.
    let mut hits: Vec<(u32, &'static str)> = cues
        .iter()
        .filter(|c| c.confidence >= min_confidence)
        .filter_map(|c| keyword_to_kf(&c.word).map(|kf| (grid.bar_at(c.time_sec), kf)))
        .collect();
    hits.sort_by_key(|(bar, _)| *bar);

    // Collapse duplicates landing on the same bar (repeated cue words).
    hits.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);

    let mut out: Vec<RecoveredSection> = Vec::new();
    for i in 0..hits.len() {
        let (start_bar, kf) = hits[i];
        let bars = hits
            .get(i + 1)
            .map(|(next, _)| next.saturating_sub(start_bar))
            .unwrap_or(0)
            .max(1);
        // Skip a same-section run that produced a zero-gap duplicate.
        if let Some(prev) = out.last() {
            if prev.start_bar == start_bar {
                continue;
            }
        }
        out.push(RecoveredSection {
            kf,
            start_bar,
            bars,
        });
    }
    out
}

/// Render recovered sections as keyflow chart text. `header` (optional) is
/// prepended verbatim (e.g. `"Song\n#A 127bpm 4/4\n"`); the last section is
/// emitted without a bar count (unknown length).
pub fn to_keyflow(sections: &[RecoveredSection], header: Option<&str>) -> String {
    let mut s = String::new();
    if let Some(h) = header {
        s.push_str(h);
        if !h.ends_with('\n') {
            s.push('\n');
        }
        s.push('\n');
    }
    for (i, sec) in sections.iter().enumerate() {
        let last = i + 1 == sections.len();
        if last {
            s.push_str(sec.kf);
        } else {
            s.push_str(&format!("{} {}", sec.kf, sec.bars));
        }
        s.push('\n');
    }
    s
}

/// Convenience: cues + grid → keyflow text in one call.
pub fn cues_to_keyflow(
    cues: &[SectionCue],
    grid: &ClickGrid,
    min_confidence: f32,
    header: Option<&str>,
) -> String {
    let sections = recover_sections(cues, grid, min_confidence);
    to_keyflow(&sections, header)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid_120() -> ClickGrid {
        ClickGrid {
            bpm: 120.0,
            first_beat_sec: 0.0,
            beats_per_bar: 4,
            beats: vec![],
            downbeats: vec![],
        }
    }

    fn cue(word: &str, t: f32, c: f32) -> SectionCue {
        SectionCue {
            word: word.to_string(),
            time_sec: t,
            confidence: c,
        }
    }

    #[test]
    fn keyword_mapping_tolerates_mishearings() {
        assert_eq!(keyword_to_kf("Refrain,"), Some("Refrain"));
        assert_eq!(keyword_to_kf("pricorus"), Some("PRE"));
        assert_eq!(keyword_to_kf("Chorus"), Some("CH"));
        assert_eq!(keyword_to_kf("verse"), Some("VS"));
        assert_eq!(keyword_to_kf("the"), None);
        assert_eq!(keyword_to_kf("1234"), None);
    }

    #[test]
    fn recovers_sections_with_bar_lengths() {
        // 120 bpm 4/4 → bar = 2 s. Cues: Intro@0, Refrain@8s(bar4),
        // Chorus@24s(bar12).
        let grid = grid_120();
        let cues = vec![
            cue("intro", 0.0, 0.9),
            cue("refrain", 8.0, 0.8),
            cue("chorus", 24.0, 0.7),
        ];
        let secs = recover_sections(&cues, &grid, 0.3);
        assert_eq!(
            secs,
            vec![
                RecoveredSection { kf: "In", start_bar: 0, bars: 4 },
                RecoveredSection { kf: "Refrain", start_bar: 4, bars: 8 },
                RecoveredSection { kf: "CH", start_bar: 12, bars: 1 },
            ]
        );
    }

    #[test]
    fn drops_low_confidence_and_renders() {
        let grid = grid_120();
        let cues = vec![
            cue("refrain", 0.0, 0.8),
            cue("bloomberg", 30.0, 0.9), // not a section word
            cue("chorus", 4.0, 0.05),    // too low-confidence → dropped
            cue("verse", 8.0, 0.6),
        ];
        let kf = cues_to_keyflow(&cues, &grid, 0.3, Some("Song\n#C 120bpm 4/4"));
        // chorus dropped (low conf); bloomberg ignored (not a section).
        assert!(kf.contains("Song\n#C 120bpm 4/4"));
        assert!(kf.contains("Refrain 4\n"));
        assert!(kf.trim_end().ends_with("VS"));
        assert!(!kf.contains("CH"));
    }
}
