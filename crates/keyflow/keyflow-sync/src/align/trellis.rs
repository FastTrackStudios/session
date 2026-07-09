//! Pure-Rust CTC forced alignment.
//!
//! This is the one piece of the lyrics-to-audio pipeline that no off-the-shelf
//! crate hands you: given a frame-by-frame emission matrix from a CTC acoustic
//! model (wav2vec2 / MMS) and a *known* token sequence (the lyrics, already
//! spelled into vocabulary ids), find the most likely monotonic alignment of
//! tokens to frames. It is a Viterbi pass over the CTC trellis with optional
//! blank frames between tokens, ported from the torchaudio "Forced Alignment
//! with Wav2Vec2" tutorial.
//!
//! The acoustic model itself lives behind the `ort` feature; this module only
//! needs the emission matrix it produces, so it compiles with zero heavy deps
//! and is independently unit-testable.

use crate::error::{Result, SyncError};

/// A frame-major emission matrix of **log-probabilities**, shape `[frames, vocab]`.
///
/// `data[t * vocab + v]` is `log P(token v | frame t)`. Rows are expected to be
/// log-softmax normalized (each row sums to 1 in probability space), which is
/// what a CTC head produces after `log_softmax`.
#[derive(Debug, Clone)]
pub struct Emission {
    pub data: Vec<f32>,
    pub frames: usize,
    pub vocab: usize,
}

impl Emission {
    pub fn new(data: Vec<f32>, frames: usize, vocab: usize) -> Result<Self> {
        if data.len() != frames * vocab {
            return Err(SyncError::Shape(format!(
                "emission data len {} != frames {} * vocab {}",
                data.len(),
                frames,
                vocab
            )));
        }
        Ok(Self {
            data,
            frames,
            vocab,
        })
    }

    #[inline]
    fn at(&self, frame: usize, token: u32) -> f32 {
        self.data[frame * self.vocab + token as usize]
    }
}

/// One token's aligned span, in **frames** (convert to seconds with the model's
/// frames-per-second). `score` is the average per-frame probability (0..=1)
/// over the span — this is the raw signal the rating system aggregates.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenSpan {
    pub token: u32,
    /// First frame (inclusive).
    pub start: usize,
    /// Last frame (exclusive).
    pub end: usize,
    /// Mean probability across the span, in `0.0..=1.0`.
    pub score: f32,
}

impl TokenSpan {
    pub fn len(&self) -> usize {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }
}

/// Run forced alignment of `tokens` against `emission`.
///
/// `tokens` is the transcript already spelled into vocabulary ids (no blanks
/// inserted — blanks are handled internally). `blank` is the CTC blank id
/// (conventionally `0`). Returns one [`TokenSpan`] per input token, in order.
///
/// This is the standard CTC forced-alignment Viterbi over the blank-extended
/// state sequence `[blank, t0, blank, t1, ..., t_{L-1}, blank]` (`2L+1`
/// states). At each frame the path may **stay** in a state, **advance** to the
/// next, or **skip** a blank between two *different* tokens. Each token state is
/// necessarily visited, so every token gets at least one frame; blank states
/// between tokens are gaps and aren't assigned to any token, so the returned
/// spans are ordered and non-overlapping but need not be contiguous.
pub fn forced_align(emission: &Emission, tokens: &[u32], blank: u32) -> Result<Vec<TokenSpan>> {
    let num_frame = emission.frames;
    let num_tokens = tokens.len();

    if num_tokens == 0 {
        return Err(SyncError::Align("empty token sequence".into()));
    }
    if num_frame < num_tokens {
        return Err(SyncError::Align(format!(
            "audio too short: {num_frame} frames for {num_tokens} tokens"
        )));
    }
    for (i, &t) in tokens.iter().enumerate() {
        if t as usize >= emission.vocab {
            return Err(SyncError::Shape(format!(
                "token #{i} id {t} out of vocab {}",
                emission.vocab
            )));
        }
    }

    // Extended state sequence: even states are blank, odd state `2k+1` is token k.
    let num_states = 2 * num_tokens + 1;
    let state_label = |p: usize| -> u32 {
        if p.is_multiple_of(2) {
            blank
        } else {
            tokens[(p - 1) / 2]
        }
    };

    const NEG_INF: f32 = f32::NEG_INFINITY;
    let mut dp = vec![NEG_INF; num_frame * num_states];
    // backpointer: 0 = stay (p), 1 = advance (p-1), 2 = skip (p-2)
    let mut bp = vec![0u8; num_frame * num_states];
    let at = |t: usize, p: usize| t * num_states + p;

    // Frame 0: only the leading blank or the first token are reachable.
    dp[at(0, 0)] = emission.at(0, blank);
    dp[at(0, 1)] = emission.at(0, tokens[0]);

    for t in 1..num_frame {
        for p in 0..num_states {
            let mut best = dp[at(t - 1, p)];
            let mut from = 0u8;
            if p >= 1 && dp[at(t - 1, p - 1)] > best {
                best = dp[at(t - 1, p - 1)];
                from = 1;
            }
            // Skip the blank between two *different* tokens.
            if p >= 2
                && !p.is_multiple_of(2)
                && tokens[(p - 1) / 2] != tokens[(p - 3) / 2]
                && dp[at(t - 1, p - 2)] > best
            {
                best = dp[at(t - 1, p - 2)];
                from = 2;
            }
            if best != NEG_INF {
                dp[at(t, p)] = best + emission.at(t, state_label(p));
                bp[at(t, p)] = from;
            }
        }
    }

    // The path may end on the final token or the trailing blank.
    let last_blank = num_states - 1;
    let last_tok = num_states - 2;
    let (mut p, end_score) = {
        let a = dp[at(num_frame - 1, last_blank)];
        let b = dp[at(num_frame - 1, last_tok)];
        if a >= b {
            (last_blank, a)
        } else {
            (last_tok, b)
        }
    };
    if end_score == NEG_INF {
        return Err(SyncError::Align(
            "no valid alignment (audio too short or silent for the given lyrics)".into(),
        ));
    }

    // Backtrack: record which state each frame occupied.
    let mut state_of_frame = vec![0usize; num_frame];
    for t in (0..num_frame).rev() {
        state_of_frame[t] = p;
        if t == 0 {
            break;
        }
        p = match bp[at(t, p)] {
            1 => p - 1,
            2 => p - 2,
            _ => p,
        };
    }

    // Each token k owns the (contiguous, by monotonicity) run of frames in its
    // state 2k+1. Score = mean emission probability of that token over its run.
    let mut spans = Vec::with_capacity(num_tokens);
    for (k, &token) in tokens.iter().enumerate() {
        let state = 2 * k + 1;
        let mut start = None;
        let mut end = 0usize;
        let mut acc = 0.0f64;
        let mut count = 0usize;
        for (t, &s) in state_of_frame.iter().enumerate() {
            if s == state {
                if start.is_none() {
                    start = Some(t);
                }
                end = t + 1;
                acc += emission.at(t, token).exp() as f64;
                count += 1;
            }
        }
        let start = start.ok_or_else(|| {
            SyncError::Align(format!("token #{k} received no frames during alignment"))
        })?;
        spans.push(TokenSpan {
            token,
            start,
            end,
            score: (acc / count as f64) as f32,
        });
    }

    Ok(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a toy emission where each frame strongly favors a chosen token.
    /// vocab = {0: blank, 1: a, 2: b}. log-probs via a crude softmax.
    fn emit(rows: &[u32], vocab: usize) -> Emission {
        let mut data = Vec::new();
        for &winner in rows {
            // logits: winner gets 5.0, others 0.0, then log_softmax.
            let logits: Vec<f32> = (0..vocab)
                .map(|v| if v as u32 == winner { 5.0 } else { 0.0 })
                .collect();
            let max = logits.iter().cloned().fold(f32::MIN, f32::max);
            let sum: f32 = logits.iter().map(|l| (l - max).exp()).sum();
            let logz = max + sum.ln();
            for l in logits {
                data.push(l - logz);
            }
        }
        Emission::new(data, rows.len(), vocab).unwrap()
    }

    #[test]
    fn aligns_two_tokens_cleanly() {
        // frames: a a blank b b  -> tokens [a=1, b=2]
        let e = emit(&[1, 1, 0, 2, 2], 3);
        let spans = forced_align(&e, &[1, 2], 0).unwrap();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].token, 1);
        assert_eq!(spans[1].token, 2);
        // token a should own the early frames, b the late ones.
        assert!(spans[0].start < spans[1].start);
        assert!(spans[0].score > 0.5);
        // contiguous, full coverage.
        assert_eq!(spans[0].start, 0);
        assert_eq!(spans.last().unwrap().end, 5);
    }

    #[test]
    fn rejects_audio_shorter_than_tokens() {
        let e = emit(&[1], 3);
        assert!(forced_align(&e, &[1, 2, 1], 0).is_err());
    }

    #[test]
    fn handles_repeated_tokens() {
        // "a a" — same token twice, must be separated by a blank frame.
        let e = emit(&[1, 0, 1], 3);
        let spans = forced_align(&e, &[1, 1], 0).unwrap();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].token, 1);
        assert_eq!(spans[1].token, 1);
    }
}
