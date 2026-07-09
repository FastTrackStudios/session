//! Forced alignment: known lyrics + audio → per-word (and per-syllable) timing
//! with confidence.
//!
//! Two layers:
//! * [`trellis`] — the pure-Rust CTC Viterbi alignment (no model).
//! * [`EmissionModel`] — the acoustic model that turns audio into the emission
//!   matrix the trellis consumes. The real implementation (wav2vec2 via ONNX
//!   Runtime) lives in [`wav2vec2`] behind the `onnx` feature; this trait keeps
//!   the orchestration testable with a stub.

pub mod tokenizer;
pub mod trellis;

#[cfg(feature = "onnx")]
pub mod wav2vec2;

use crate::error::Result;
use tokenizer::{Vocab, tokenize};
use trellis::{Emission, forced_align};

/// An acoustic model: audio (mono, at [`Self::sample_rate`]) → CTC emissions.
pub trait EmissionModel {
    /// Sample rate the model expects, e.g. 16000.
    fn sample_rate(&self) -> u32;
    /// The model's label set.
    fn vocab(&self) -> &Vocab;
    /// Emission frames produced per second of audio (model stride). For
    /// wav2vec2 this is ~49.95 (20 ms stride). Used to convert frame indices to
    /// seconds.
    fn frames_per_second(&self) -> f32;
    /// Run the model: `samples` are mono `f32` at [`Self::sample_rate`].
    fn emit(&self, samples: &[f32]) -> Result<Emission>;
}

/// One word's aligned timing in seconds, with a confidence in `0.0..=1.0`.
#[derive(Debug, Clone, PartialEq)]
pub struct WordTiming {
    pub word: String,
    pub start: f32,
    pub end: f32,
    /// Mean CTC probability across the word's tokens — the raw rating signal.
    pub confidence: f32,
}

/// Align `lyrics` against `samples` using `model`. `samples` must already be
/// mono at `model.sample_rate()` (use [`crate::audio::AudioBuffer::resample`]).
pub fn align_words(
    model: &dyn EmissionModel,
    samples: &[f32],
    lyrics: &str,
) -> Result<Vec<WordTiming>> {
    align_words_star(model, samples, lyrics, None)
}

/// Like [`align_words`], but with an optional `<star>` token. When
/// `star_score` is `Some(s)`, an extra emission column with constant
/// log-probability `s` is appended and star tokens are inserted at the start,
/// end, and between words. The star absorbs audio that matches no lyric token
/// (instrumental intros/interludes, ad-libs, breaths, repeats not in the
/// transcript), so a local mismatch can't drag neighbouring words off — the
/// standard MMS robustness trick. `s` must sit between a genuine match
/// (≈ -0.4..-2 log-prob) and a non-match (≈ -5 or lower): ~ -2.5 is a good
/// start. Too high (→0) and star greedily steals real frames; too low and it
/// has no effect.
pub fn align_words_star(
    model: &dyn EmissionModel,
    samples: &[f32],
    lyrics: &str,
    star_score: Option<f32>,
) -> Result<Vec<WordTiming>> {
    let vocab = model.vocab();
    let tok = tokenize(lyrics, vocab)?;
    let emission = model.emit(samples)?;
    let fps = model.frames_per_second();

    match star_score {
        None => {
            let spans = forced_align(&emission, &tok.tokens, vocab.blank)?;
            Ok(group_into_words(&spans, &tok, fps))
        }
        Some(s) => {
            let star_id = emission.vocab as u32;
            let aug = augment_with_star(&emission, s)?;
            let starred = insert_star(&tok, star_id);
            let spans = forced_align(&aug, &starred.tokens, vocab.blank)?;
            Ok(group_into_words(&spans, &starred, fps))
        }
    }
}

/// Transcribe audio by CTC greedy decoding (no transcript) — independent ASR
/// for comparison against forced alignment.
///
/// Takes the per-frame argmax, collapses consecutive repeats, drops blanks, and
/// groups characters into words at the separator token. Returns each decoded
/// word with its frame-derived timing and mean character probability. Requires
/// a model whose vocab has a word separator (wav2vec2 `|`); MMS has none, so its
/// CTC output can't be segmented into words this way.
pub fn transcribe(model: &dyn EmissionModel, samples: &[f32]) -> Result<Vec<WordTiming>> {
    let vocab = model.vocab();
    let sep = vocab.separator.ok_or_else(|| {
        crate::error::SyncError::Tokenize(
            "transcribe needs a model with a word-separator token (e.g. w2v2-960h, not mms-fa)"
                .into(),
        )
    })?;
    let em = model.emit(samples)?;
    let fps = model.frames_per_second();
    let v = em.vocab;

    let mut words: Vec<WordTiming> = Vec::new();
    let mut cur = String::new();
    let mut start_f: Option<usize> = None;
    let mut end_f = 0usize;
    let mut prob_sum = 0.0f64;
    let mut chars = 0usize;
    let mut prev_raw = u32::MAX;

    let flush = |words: &mut Vec<WordTiming>,
                 cur: &mut String,
                 start_f: &mut Option<usize>,
                 end_f: usize,
                 prob_sum: &mut f64,
                 chars: &mut usize| {
        if let Some(sf) = *start_f {
            if !cur.is_empty() {
                words.push(WordTiming {
                    word: std::mem::take(cur),
                    start: sf as f32 / fps,
                    end: end_f as f32 / fps,
                    confidence: if *chars > 0 {
                        (*prob_sum / *chars as f64) as f32
                    } else {
                        0.0
                    },
                });
            }
        }
        *start_f = None;
        cur.clear();
        *prob_sum = 0.0;
        *chars = 0;
    };

    for f in 0..em.frames {
        let row = &em.data[f * v..(f + 1) * v];
        let (tok, &logp) = row
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, p)| (i as u32, p))
            .unwrap();
        if tok == prev_raw {
            continue; // collapse consecutive duplicates
        }
        prev_raw = tok;
        if tok == vocab.blank {
            continue;
        }
        if tok == sep {
            flush(
                &mut words,
                &mut cur,
                &mut start_f,
                end_f,
                &mut prob_sum,
                &mut chars,
            );
            continue;
        }
        // a real character
        if start_f.is_none() {
            start_f = Some(f);
        }
        end_f = f + 1;
        cur.push_str(&vocab.label(tok).to_lowercase());
        prob_sum += (logp as f64).exp();
        chars += 1;
    }
    flush(
        &mut words,
        &mut cur,
        &mut start_f,
        end_f,
        &mut prob_sum,
        &mut chars,
    );
    Ok(words)
}

/// Append a star column (constant log-prob `score`) to every emission frame.
fn augment_with_star(em: &Emission, score: f32) -> Result<Emission> {
    let v = em.vocab;
    let nv = v + 1;
    let mut data = Vec::with_capacity(em.frames * nv);
    for f in 0..em.frames {
        data.extend_from_slice(&em.data[f * v..(f + 1) * v]);
        data.push(score);
    }
    Emission::new(data, em.frames, nv)
}

/// Rebuild a token sequence with star tokens at the start, end, and every word
/// boundary. Existing separator tokens (wav2vec2) are dropped — star replaces
/// them. Star tokens carry `word = None` so [`group_into_words`] skips them.
fn insert_star(tok: &tokenizer::Tokenized, star_id: u32) -> tokenizer::Tokenized {
    let mut tokens = vec![star_id];
    let mut token_word: Vec<Option<usize>> = vec![None];
    let mut prev: Option<usize> = None;
    for (i, &t) in tok.tokens.iter().enumerate() {
        let w = tok.token_word[i];
        let Some(wi) = w else { continue }; // drop existing separators
        if let Some(p) = prev {
            if wi != p {
                tokens.push(star_id);
                token_word.push(None);
            }
        }
        tokens.push(t);
        token_word.push(Some(wi));
        prev = Some(wi);
    }
    tokens.push(star_id);
    token_word.push(None);
    tokenizer::Tokenized {
        tokens,
        token_word,
        words: tok.words.clone(),
    }
}

/// Regroup per-token spans into per-word timings. Separator tokens (word=None)
/// are skipped; each word spans from its first token's start to its last
/// token's end, weighting confidence by token length.
fn group_into_words(
    spans: &[trellis::TokenSpan],
    tok: &tokenizer::Tokenized,
    fps: f32,
) -> Vec<WordTiming> {
    let mut out: Vec<WordTiming> = Vec::with_capacity(tok.words.len());
    // spans align 1:1 with tok.tokens by construction of forced_align.
    let mut i = 0usize;
    while i < spans.len() {
        let Some(word_idx) = tok.token_word[i] else {
            i += 1;
            continue;
        };
        let start_frame = spans[i].start;
        let mut end_frame = spans[i].end;
        let mut weighted = 0.0f64;
        let mut frames = 0usize;
        // Consume all consecutive tokens of this same word.
        while i < spans.len() && tok.token_word[i] == Some(word_idx) {
            let len = spans[i].len().max(1);
            weighted += spans[i].score as f64 * len as f64;
            frames += len;
            end_frame = spans[i].end;
            i += 1;
        }
        let confidence = if frames > 0 {
            (weighted / frames as f64) as f32
        } else {
            0.0
        };
        out.push(WordTiming {
            word: tok.words[word_idx].clone(),
            start: start_frame as f32 / fps,
            end: end_frame as f32 / fps,
            confidence,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokenizer::Vocab;
    use trellis::Emission;

    /// A stub model that emits a hand-built matrix; lets us test the full
    /// audio→words path without ONNX.
    struct StubModel {
        vocab: Vocab,
        emission: Emission,
    }

    fn vocab() -> Vocab {
        let mut labels = vec!["<blank>".to_string(), "|".to_string()];
        for c in 'a'..='z' {
            labels.push(c.to_string());
        }
        Vocab::new(labels, "<blank>", Some("|"), true).unwrap()
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
        fn emit(&self, _samples: &[f32]) -> Result<Emission> {
            Ok(self.emission.clone())
        }
    }

    // Build a near-one-hot log-prob emission from a sequence of winning labels.
    fn emit_rows(winners: &[u32], vocab_len: usize) -> Emission {
        let mut data = Vec::new();
        for &w in winners {
            let logits: Vec<f32> = (0..vocab_len)
                .map(|v| if v as u32 == w { 6.0 } else { 0.0 })
                .collect();
            let max = logits.iter().cloned().fold(f32::MIN, f32::max);
            let sum: f32 = logits.iter().map(|l| (l - max).exp()).sum();
            let logz = max + sum.ln();
            data.extend(logits.iter().map(|l| l - logz));
        }
        Emission::new(data, winners.len(), vocab_len).unwrap()
    }

    #[test]
    fn aligns_two_words_to_time() {
        let v = vocab();
        let id = |c: char| v_id(&v, c);
        // "hi" then "ok": frames h i | o k  (10 fps → 0.1s/frame)
        let rows = vec![id('h'), id('i'), v.separator.unwrap(), id('o'), id('k')];
        let emission = emit_rows(&rows, v.len());
        let model = StubModel { vocab: v, emission };
        let words = align_words(&model, &[0.0; 16_000], "hi ok").unwrap();
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].word, "hi");
        assert_eq!(words[1].word, "ok");
        assert!(words[0].start < words[1].start);
        assert!(words[0].confidence > 0.5);
    }

    fn v_id(v: &Vocab, c: char) -> u32 {
        // tokenizer has no public id lookup; rebuild via tokenize of a single char.
        let t = tokenize(&c.to_string(), v).unwrap();
        t.tokens[0]
    }
}
