//! The timing sidecar — the single source of *when*, kept separate from the
//! `.kf`'s single source of *what* (rhythm + lyrics).
//!
//! Alignment never copies the lyrics back into the chart and never edits the
//! `.kf`. It emits this sidecar: a flat list of word timings, each addressed by
//! `(section, word index within that section's lyric line)` so it can be joined
//! back onto a parsed `Chart` without duplicating a single syllable. Slides,
//! karaoke, and MIDI lyric events are all derivations of `chart + sidecar`.

use facet::Facet;

use crate::align::WordTiming;
use crate::error::{Result, SyncError};

/// On-disk format version, bumped on breaking layout changes.
pub const SIDECAR_VERSION: u32 = 1;

/// A confidence bucket — gates which sections are trustworthy enough to drive
/// karaoke / CloneHero export, and which need a human pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum Rating {
    /// High confidence; usable as-is.
    Verified,
    /// Plausible but worth a glance.
    Review,
    /// Low confidence; alignment likely wrong here.
    Failed,
}

/// Confidence cutoffs for the rating buckets. The right values depend on the
/// acoustic model and the material: speech ASR on read speech scores high
/// (~0.7+ for good hits), but the same model on *singing* scores far lower even
/// when the alignment is correct, so singing needs lower cutoffs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RatingThresholds {
    pub verified: f32,
    pub review: f32,
}

impl RatingThresholds {
    /// Speech defaults (clean read speech).
    pub const SPEECH: RatingThresholds = RatingThresholds {
        verified: 0.70,
        review: 0.40,
    };
    /// Singing defaults — calibrated against isolated-vocal CTC scores, which
    /// run several × lower than speech for correct alignments.
    pub const SINGING: RatingThresholds = RatingThresholds {
        verified: 0.30,
        review: 0.15,
    };

    pub fn rate(&self, c: f32) -> Rating {
        if c >= self.verified {
            Rating::Verified
        } else if c >= self.review {
            Rating::Review
        } else {
            Rating::Failed
        }
    }
}

impl Default for RatingThresholds {
    fn default() -> Self {
        RatingThresholds::SPEECH
    }
}

impl Rating {
    /// Bucket a confidence with the default (speech) thresholds.
    pub fn from_confidence(c: f32) -> Self {
        RatingThresholds::default().rate(c)
    }
}

/// One aligned word, addressed back to the chart.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct WordEntry {
    /// Index into `Chart.sections`.
    pub section: u32,
    /// Index of this word within the section's lyric line (in word order).
    pub index: u32,
    /// The matched word text — denormalized for human readability and
    /// debugging only. The `.kf` remains the source of truth.
    pub text: String,
    /// Start time in seconds.
    pub start: f32,
    /// End time in seconds.
    pub end: f32,
    /// Mean CTC probability across the word, `0.0..=1.0`.
    pub confidence: f32,
    pub rating: Rating,
}

/// The full sidecar for one (audio, chart) pair.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct TimingMap {
    pub version: u32,
    /// Source audio path / identifier the alignment was computed against.
    pub audio: String,
    /// Aligner model id (provenance — re-runs with a new model overwrite).
    pub model: String,
    /// Acoustic-model frame rate used; lets consumers reason about resolution.
    pub frames_per_second: f32,
    pub words: Vec<WordEntry>,
}

impl TimingMap {
    pub fn new(audio: impl Into<String>, model: impl Into<String>, fps: f32) -> Self {
        Self {
            version: SIDECAR_VERSION,
            audio: audio.into(),
            model: model.into(),
            frames_per_second: fps,
            words: Vec::new(),
        }
    }

    /// Append a section's aligned words, tagging each with the section index and
    /// rating it against `thresholds`.
    pub fn push_section(
        &mut self,
        section: u32,
        words: &[WordTiming],
        thresholds: RatingThresholds,
    ) {
        for (i, w) in words.iter().enumerate() {
            self.words.push(WordEntry {
                section,
                index: i as u32,
                text: w.word.clone(),
                start: w.start,
                end: w.end,
                confidence: w.confidence,
                rating: thresholds.rate(w.confidence),
            });
        }
    }

    /// Counts per rating bucket — the headline of a `kf sync` run.
    pub fn rating_summary(&self) -> (usize, usize, usize) {
        let mut verified = 0;
        let mut review = 0;
        let mut failed = 0;
        for w in &self.words {
            match w.rating {
                Rating::Verified => verified += 1,
                Rating::Review => review += 1,
                Rating::Failed => failed += 1,
            }
        }
        (verified, review, failed)
    }

    pub fn to_json(&self) -> Result<String> {
        facet_json::to_string_pretty(self)
            .map_err(|e| SyncError::Sidecar(format!("serialize: {e}")))
    }

    pub fn from_json(s: &str) -> Result<Self> {
        facet_json::from_str(s).map_err(|e| SyncError::Sidecar(format!("parse: {e}")))
    }

    pub fn write_file(&self, path: impl AsRef<std::path::Path>) -> Result<()> {
        std::fs::write(path, self.to_json()?)?;
        Ok(())
    }

    pub fn read_file(path: impl AsRef<std::path::Path>) -> Result<Self> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wt(word: &str, start: f32, end: f32, c: f32) -> WordTiming {
        WordTiming {
            word: word.into(),
            start,
            end,
            confidence: c,
        }
    }

    #[test]
    fn rating_thresholds() {
        assert_eq!(Rating::from_confidence(0.9), Rating::Verified);
        assert_eq!(Rating::from_confidence(0.5), Rating::Review);
        assert_eq!(Rating::from_confidence(0.1), Rating::Failed);
    }

    #[test]
    fn json_round_trips() {
        let mut map = TimingMap::new("song.wav", "mms_fa", 49.95);
        map.push_section(
            0,
            &[wt("hello", 0.0, 0.5, 0.9), wt("world", 0.5, 1.0, 0.3)],
            RatingThresholds::default(),
        );
        let json = map.to_json().unwrap();
        let back = TimingMap::from_json(&json).unwrap();
        assert_eq!(map, back);
        let (v, r, f) = back.rating_summary();
        assert_eq!((v, r, f), (1, 0, 1));
    }
}
