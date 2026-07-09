//! End-to-end orchestration: audio (+ optional separation) → forced alignment
//! → [`TimingMap`].
//!
//! Alignment is done in a single global pass over the whole song's lyrics (CTC
//! forced alignment is monotonic, so one pass is both correct and far more
//! robust than aligning each section against the full mix independently). The
//! resulting words are then partitioned back to their sections by count.

use crate::align::{EmissionModel, align_words_star, tokenizer::tokenize, transcribe};
use crate::audio::AudioBuffer;
use crate::error::Result;
use crate::separator::{StemSelection, StemSeparator};
use crate::timing::{RatingThresholds, TimingMap};

/// One section's lyrics, extracted from the chart by the caller. Keeping this a
/// plain struct (rather than taking a `Chart`) is what lets the core crate stay
/// free of the heavy keyflow graph; the `chart` feature provides the extractor.
#[derive(Debug, Clone)]
pub struct SectionLyrics {
    /// Index into `Chart.sections`.
    pub section: u32,
    /// The section's lyric text, words separated by whitespace.
    pub text: String,
}

/// Optionally isolate the vocal stem before alignment, returning the buffer the
/// aligner should run on plus a label for provenance.
pub fn prepare_vocals(
    audio: &AudioBuffer,
    separator: Option<&dyn StemSeparator>,
) -> Result<(AudioBuffer, Option<String>)> {
    match separator {
        None => Ok((audio.clone(), None)),
        Some(sep) => {
            let stems = sep.separate(audio, StemSelection::Two)?;
            let vocals = stems.vocals.ok_or_else(|| {
                crate::error::SyncError::Separation("separator produced no vocals".into())
            })?;
            Ok((vocals, Some(sep.name().to_string())))
        }
    }
}

/// Run the full alignment pass. `audio_buf` should be the buffer to align
/// against (already vocal-isolated if desired — see [`prepare_vocals`]).
pub fn run_alignment(
    model: &dyn EmissionModel,
    audio_buf: &AudioBuffer,
    sections: &[SectionLyrics],
    audio_id: &str,
    model_id: &str,
    thresholds: RatingThresholds,
    star_score: Option<f32>,
) -> Result<TimingMap> {
    let resampled = audio_buf.resample(model.sample_rate());

    // Build the combined transcript and remember how many *surviving* words
    // each section contributes, so we can slice the global result back apart.
    let vocab = model.vocab();
    let mut combined = String::new();
    let mut counts: Vec<(u32, usize)> = Vec::new();
    for s in sections {
        let n = match tokenize(&s.text, vocab) {
            Ok(t) => t.words.len(),
            Err(_) => 0, // section had no alignable words (e.g. instrumental)
        };
        if n == 0 {
            continue;
        }
        if !combined.is_empty() {
            combined.push(' ');
        }
        combined.push_str(&s.text);
        counts.push((s.section, n));
    }

    let mut map = TimingMap::new(audio_id, model_id, model.frames_per_second());
    if combined.is_empty() {
        return Ok(map);
    }

    let words = align_words_star(model, &resampled.samples, &combined, star_score)?;

    // Partition the flat word list back to sections by the per-section counts.
    let mut cursor = 0usize;
    for (section, n) in counts {
        let end = (cursor + n).min(words.len());
        map.push_section(section, &words[cursor..end], thresholds);
        cursor = end;
    }
    Ok(map)
}

/// Transcribe (CTC greedy ASR) the audio with no transcript — an independent
/// baseline to compare against forced alignment. Returns a [`TimingMap`] whose
/// words are what the model *heard* (one section, index 0), so it drops into the
/// same sidecar format and tooling.
pub fn run_transcription(
    model: &dyn EmissionModel,
    audio_buf: &AudioBuffer,
    audio_id: &str,
    model_id: &str,
    thresholds: RatingThresholds,
) -> Result<TimingMap> {
    let resampled = audio_buf.resample(model.sample_rate());
    let words = transcribe(model, &resampled.samples)?;
    let mut map = TimingMap::new(audio_id, model_id, model.frames_per_second());
    map.push_section(0, &words, thresholds);
    Ok(map)
}

/// Anchored per-section alignment — robust to recordings that repeat sections
/// more than the transcript lists (live worship, jam outros).
///
/// A single global pass forces every transcript word across the whole song in
/// one monotonic sweep, so one extra sung repeat throws off everything after
/// it. Here each section is aligned independently against the audio *from a
/// running anchor onward* (with `<star>` absorbing intros/instrumental/extra
/// repeats before it). After a section lands, the anchor advances past it, so
/// each canonical section matches its first sung occurrence and later sections
/// are searched only in the audio that remains.
pub fn run_alignment_anchored(
    model: &dyn EmissionModel,
    audio_buf: &AudioBuffer,
    sections: &[SectionLyrics],
    audio_id: &str,
    model_id: &str,
    thresholds: RatingThresholds,
    star_score: Option<f32>,
) -> Result<TimingMap> {
    let resampled = audio_buf.resample(model.sample_rate());
    let sr = model.sample_rate() as f32;
    let n = resampled.samples.len();
    let mut map = TimingMap::new(audio_id, model_id, model.frames_per_second());
    // Star is what makes a partial transcript align against a tail of audio, so
    // default it on for anchored mode if the caller didn't set it.
    let star = star_score.or(Some(-2.5));
    // Bounded search window per section: long enough to span a section plus
    // surrounding instrumental, short enough that a section can't spuriously
    // match far across the song (which would overshoot the anchor).
    let window = (45.0 * sr) as usize;
    // Don't let a confident section advance the anchor onto its own confident
    // span's *last* word if that word is an outlier; use the trusted span end.

    let mut anchor = 0usize; // sample index into `resampled`
    for s in sections {
        if anchor >= n {
            break;
        }
        let end = (anchor + window).min(n);
        let slice = &resampled.samples[anchor..end];
        let mut words = match align_words_star(model, slice, &s.text, star) {
            Ok(w) => w,
            Err(_) => continue, // section had no alignable words
        };
        let offset = anchor as f32 / sr;
        for w in &mut words {
            w.start += offset;
            w.end += offset;
        }
        // Advance past the section's *trusted* span — the end of the last word
        // at/above the verified cutoff (fallback: review, then a fixed nudge so
        // a weak section can't stall or overshoot the loop).
        let trusted_end = words
            .iter()
            .rev()
            .find(|w| w.confidence >= thresholds.verified)
            .or_else(|| {
                words
                    .iter()
                    .rev()
                    .find(|w| w.confidence >= thresholds.review)
            })
            .map(|w| w.end);
        let next = match trusted_end {
            Some(t) => (t * sr) as usize,
            None => anchor + window / 3, // nothing matched; step forward modestly
        };
        anchor = next.max(anchor + 1).min(n);
        map.push_section(s.section, &words, thresholds);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::align::tokenizer::Vocab;
    use crate::align::trellis::Emission;

    struct StubModel {
        vocab: Vocab,
        emission: Emission,
    }
    impl EmissionModel for StubModel {
        fn sample_rate(&self) -> u32 {
            16_000
        }
        fn vocab(&self) -> &Vocab {
            &self.vocab
        }
        fn frames_per_second(&self) -> f32 {
            10.0
        }
        fn emit(&self, _: &[f32]) -> Result<Emission> {
            Ok(self.emission.clone())
        }
    }

    fn vocab() -> Vocab {
        let mut labels = vec!["<blank>".to_string(), "|".to_string()];
        for c in 'a'..='z' {
            labels.push(c.to_string());
        }
        Vocab::new(labels, "<blank>", Some("|"), true).unwrap()
    }

    fn emit_rows(winners: &[u32], n: usize) -> Emission {
        let mut data = Vec::new();
        for &w in winners {
            let logits: Vec<f32> = (0..n)
                .map(|v| if v as u32 == w { 6.0 } else { 0.0 })
                .collect();
            let max = logits.iter().cloned().fold(f32::MIN, f32::max);
            let sum: f32 = logits.iter().map(|l| (l - max).exp()).sum();
            let logz = max + sum.ln();
            data.extend(logits.iter().map(|l| l - logz));
        }
        Emission::new(data, winners.len(), n).unwrap()
    }

    #[test]
    fn partitions_words_across_two_sections() {
        let v = vocab();
        let id = |c: char| tokenize(&c.to_string(), &v).unwrap().tokens[0];
        // "a | b | c": three single-letter words, sections [a] then [b c].
        let rows = vec![
            id('a'),
            v.separator.unwrap(),
            id('b'),
            v.separator.unwrap(),
            id('c'),
        ];
        let model = StubModel {
            emission: emit_rows(&rows, v.len()),
            vocab: v,
        };
        let sections = vec![
            SectionLyrics {
                section: 0,
                text: "a".into(),
            },
            SectionLyrics {
                section: 1,
                text: "b c".into(),
            },
        ];
        let map = run_alignment(
            &model,
            &AudioBuffer::new(vec![0.0; 16_000], 16_000),
            &sections,
            "x.wav",
            "stub",
            RatingThresholds::default(),
            None,
        )
        .unwrap();
        assert_eq!(map.words.len(), 3);
        assert_eq!(map.words[0].section, 0);
        assert_eq!(map.words[1].section, 1);
        assert_eq!(map.words[2].section, 1);
        // section-local word indices reset per section.
        assert_eq!(map.words[1].index, 0);
        assert_eq!(map.words[2].index, 1);
    }
}
