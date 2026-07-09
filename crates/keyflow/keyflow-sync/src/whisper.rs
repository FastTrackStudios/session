//! Whisper ASR via candle (pure Rust) — an LM-backed transcription baseline to
//! compare against CTC forced alignment.
//!
//! Unlike greedy CTC (which produces gibberish on singing), Whisper's
//! autoregressive decoder with its language model recovers actual words, so its
//! transcript reveals the recording's real structure (extra repeats, ad-libs)
//! and gives phrase-level timestamps. This is a faithful-but-trimmed port of the
//! candle whisper example: greedy decoding (no temperature fallback), timestamps
//! on, English model (no language token).

use std::path::{Path, PathBuf};

use candle_core::{Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::whisper::{self as cw, Config, model::Whisper};
use tokenizers::Tokenizer;

use crate::error::{Result, SyncError};

fn err(e: impl std::fmt::Display) -> SyncError {
    SyncError::Model(format!("whisper: {e}"))
}

/// One transcribed phrase with its time span (seconds) and mean token
/// probability.
#[derive(Debug, Clone)]
pub struct Segment {
    pub text: String,
    pub start: f32,
    pub end: f32,
    pub confidence: f32,
}

struct DecodeResult {
    tokens: Vec<u32>,
    avg_logprob: f32,
    compression_ratio: f64,
    no_speech_prob: f32,
}

fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
}

fn argmax(v: &[f32]) -> u32 {
    v.iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .map(|(i, _)| i as u32)
        .unwrap()
}

/// Softmax of `logits / temperature` (temperature-scaled distribution).
fn softmax_vec(logits: &[f32], temperature: f32) -> Vec<f32> {
    let scaled: Vec<f32> = logits.iter().map(|x| x / temperature).collect();
    let max = scaled.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f32> = scaled.iter().map(|x| (x - max).exp()).collect();
    let sum: f32 = exps.iter().sum();
    exps.iter().map(|e| e / sum.max(1e-9)).collect()
}

/// A loaded Whisper model ready to transcribe.
pub struct WhisperAsr {
    model: Whisper,
    tokenizer: Tokenizer,
    config: Config,
    mel_filters: Vec<f32>,
    device: Device,
    sot: u32,
    transcribe: u32,
    eot: u32,
    no_timestamps: u32,
    no_speech: Option<u32>,
    suppress: Tensor,
    rng: rand::rngs::StdRng,
}

fn pick_device() -> Device {
    #[cfg(feature = "whisper-cuda")]
    {
        if let Ok(d) = Device::new_cuda(0) {
            return d;
        }
    }
    Device::Cpu
}

impl WhisperAsr {
    /// Load from a directory holding `config.json`, `tokenizer.json`,
    /// `model.safetensors`, and `melfilters.bytes` (see [`ensure_model`]).
    pub fn load(dir: &Path) -> Result<Self> {
        let device = pick_device();
        let config: Config =
            serde_json::from_str(&std::fs::read_to_string(dir.join("config.json"))?)
                .map_err(err)?;
        let tokenizer = Tokenizer::from_file(dir.join("tokenizer.json")).map_err(err)?;

        // mel filters: flat little-endian f32 [n_mels * (n_fft/2+1)].
        let raw = std::fs::read(dir.join("melfilters.bytes"))?;
        let mel_filters: Vec<f32> = raw
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();

        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(
                &[dir.join("model.safetensors")],
                cw::DTYPE,
                &device,
            )
            .map_err(err)?
        };
        let model = Whisper::load(&vb, config.clone()).map_err(err)?;

        let tok = |s: &str| -> Result<u32> {
            tokenizer
                .token_to_id(s)
                .ok_or_else(|| err(format!("missing special token {s}")))
        };
        let sot = tok(cw::SOT_TOKEN)?;
        let transcribe = tok(cw::TRANSCRIBE_TOKEN)?;
        let eot = tok(cw::EOT_TOKEN)?;
        let translate = tok(cw::TRANSLATE_TOKEN)?;
        let no_timestamps = tok(cw::NO_TIMESTAMPS_TOKEN)?;

        // Suppress degenerate / non-transcribing tokens so greedy decode stays
        // on text + timestamps.
        let mut mask = vec![0f32; config.vocab_size];
        for &t in &[sot, translate, no_timestamps] {
            mask[t as usize] = f32::NEG_INFINITY;
        }
        let mut no_speech = None;
        for s in cw::NO_SPEECH_TOKENS {
            if let Some(id) = tokenizer.token_to_id(s) {
                mask[id as usize] = f32::NEG_INFINITY;
                no_speech = no_speech.or(Some(id));
            }
        }
        let suppress = Tensor::new(mask.as_slice(), &device).map_err(err)?;

        Ok(Self {
            model,
            tokenizer,
            config,
            mel_filters,
            device,
            sot,
            transcribe,
            eot,
            no_timestamps,
            no_speech,
            suppress,
            rng: rand::SeedableRng::seed_from_u64(0),
        })
    }

    /// Transcribe mono 16 kHz `samples`, returning phrase segments in order.
    pub fn transcribe(&mut self, samples: &[f32]) -> Result<Vec<Segment>> {
        let mel = cw::audio::pcm_to_mel(&self.config, samples, &self.mel_filters);
        let n_mels = self.config.num_mel_bins;
        let n_frames = mel.len() / n_mels;
        let mel = Tensor::from_vec(mel, (1, n_mels, n_frames), &self.device).map_err(err)?;

        // Energy VAD: the loudest ~10 s block sets a reference; windows whose
        // vocal-stem RMS is far below it are instrumental (Demucs leaves them
        // near-silent) and are skipped, so Whisper never hallucinates over them.
        let block = 10 * cw::SAMPLE_RATE;
        let peak_rms = samples
            .chunks(block.max(1))
            .map(rms)
            .fold(0.0f32, f32::max)
            .max(1e-6);
        let vad_floor = 0.20 * peak_rms;

        let mut segments = Vec::new();
        let mut seek = 0usize; // mel frame index
        while seek < n_frames {
            let time_offset = (seek * cw::HOP_LENGTH) as f32 / cw::SAMPLE_RATE as f32;
            let size = (n_frames - seek).min(cw::N_FRAMES);
            // VAD: skip near-silent (instrumental) windows.
            let s0 = (seek * cw::HOP_LENGTH).min(samples.len());
            let s1 = ((seek + size) * cw::HOP_LENGTH).min(samples.len());
            if s1 > s0 && rms(&samples[s0..s1]) < vad_floor {
                seek += size;
                continue;
            }
            let mut window = mel.narrow(2, seek, size).map_err(err)?;
            // Pad the final window to the full 3000 frames the encoder expects.
            if size < cw::N_FRAMES {
                let pad = Tensor::zeros((1, n_mels, cw::N_FRAMES - size), cw::DTYPE, &self.device)
                    .map_err(err)?;
                window = Tensor::cat(&[&window, &pad], 2).map_err(err)?;
            }

            let dr = self.decode_with_fallback(&window)?;
            // No-speech gate: skip windows the model flags as silence/instrumental
            // (high <|nospeech|> prob and low confidence). Prevents the high-temp
            // fallback from hallucinating words over instrumental sections.
            const NO_SPEECH_THRESHOLD: f32 = 0.6;
            const LOGPROB_THRESHOLD: f32 = -1.0;
            if dr.no_speech_prob > NO_SPEECH_THRESHOLD && dr.avg_logprob < LOGPROB_THRESHOLD {
                seek += size;
                continue;
            }
            let advance =
                self.parse_segments(&dr.tokens, time_offset, dr.avg_logprob, &mut segments)?;
            // Advance by the consumed duration (last timestamp), else the full window.
            let frames = if advance > 0.0 {
                (advance * cw::SAMPLE_RATE as f32 / cw::HOP_LENGTH as f32) as usize
            } else {
                size
            };
            seek += frames.max(1).min(size);
        }
        Ok(segments)
    }

    /// Decode a window with temperature fallback (the OpenAI/candle strategy):
    /// try greedy (T=0); if the result is degenerate — too repetitive
    /// (compression ratio high) or low-probability (avg log-prob low) — retry at
    /// progressively higher temperatures with sampling. This robustly handles
    /// repetitive sung vamps instead of looping or truncating.
    fn decode_with_fallback(&mut self, mel: &Tensor) -> Result<DecodeResult> {
        const TEMPERATURES: [f64; 6] = [0.0, 0.2, 0.4, 0.6, 0.8, 1.0];
        const COMPRESSION_RATIO_THRESHOLD: f64 = 2.4;
        const LOGPROB_THRESHOLD: f64 = -1.0;
        let mut last = None;
        for (idx, &t) in TEMPERATURES.iter().enumerate() {
            let dr = self.decode_window(mel, t)?;
            let is_last = idx == TEMPERATURES.len() - 1;
            let good = dr.compression_ratio <= COMPRESSION_RATIO_THRESHOLD
                && (dr.avg_logprob as f64) > LOGPROB_THRESHOLD;
            if good || is_last {
                return Ok(dr);
            }
            last = Some(dr);
        }
        Ok(last.unwrap())
    }

    /// Decode one padded 30 s window at a given temperature. T=0 is greedy;
    /// T>0 samples from the temperature-scaled distribution.
    fn decode_window(&mut self, mel: &Tensor, temperature: f64) -> Result<DecodeResult> {
        use rand::distributions::Distribution;

        self.model.reset_kv_cache();
        let audio = self.model.encoder.forward(mel, true).map_err(err)?;

        let mut tokens = vec![self.sot, self.transcribe];
        let prefix = tokens.len();
        let mut logprob_sum = 0.0f64;
        let mut generated = 0usize;
        let mut no_speech_prob = 0.0f32;
        let max_len = self.config.max_target_positions.min(224);

        for i in 0..max_len {
            let t = Tensor::new(tokens.as_slice(), &self.device)
                .map_err(err)?
                .unsqueeze(0)
                .map_err(err)?;
            let ys = self
                .model
                .decoder
                .forward(&t, &audio, i == 0)
                .map_err(err)?;
            let seq_len = ys.dim(1).map_err(err)?;
            let logits = self
                .model
                .decoder
                .final_linear(&ys.i((.., seq_len - 1..)).map_err(err)?)
                .map_err(err)?
                .i(0)
                .map_err(err)?
                .i(0)
                .map_err(err)?;
            // No-speech probability: read the <|nospeech|> token's probability at
            // the first step (before suppression), Whisper's silence signal.
            if i == 0 {
                if let Some(ns) = self.no_speech {
                    let raw: Vec<f32> = candle_nn::ops::softmax(&logits, 0)
                        .map_err(err)?
                        .to_vec1()
                        .map_err(err)?;
                    no_speech_prob = raw[ns as usize];
                }
            }
            let logits = logits.broadcast_add(&self.suppress).map_err(err)?;
            // t=1 probabilities, used for the log-prob score regardless of temp.
            let probs: Vec<f32> = candle_nn::ops::softmax(&logits, 0)
                .map_err(err)?
                .to_vec1()
                .map_err(err)?;
            let next = if temperature == 0.0 {
                argmax(&probs)
            } else {
                let logits_v: Vec<f32> = logits.to_vec1().map_err(err)?;
                let scaled = softmax_vec(&logits_v, temperature as f32);
                let dist = rand::distributions::WeightedIndex::new(&scaled).map_err(err)?;
                dist.sample(&mut self.rng) as u32
            };
            let p = probs[next as usize];
            tokens.push(next);
            if next != self.eot {
                logprob_sum += (p.max(1e-9) as f64).ln();
                generated += 1;
            }
            if next == self.eot {
                break;
            }
        }
        let avg_logprob = if generated > 0 {
            (logprob_sum / generated as f64) as f32
        } else {
            -10.0
        };
        let out = tokens[prefix..].to_vec();
        let compression_ratio = self.compression_ratio(&out);
        Ok(DecodeResult {
            tokens: out,
            avg_logprob,
            compression_ratio,
            no_speech_prob,
        })
    }

    /// gzip compression ratio of the decoded text — high means repetitive
    /// (Whisper's degenerate-loop signature).
    fn compression_ratio(&self, tokens: &[u32]) -> f64 {
        use std::io::Write;
        let text_ids: Vec<u32> = tokens.iter().copied().filter(|&t| t < self.eot).collect();
        let text = self.tokenizer.decode(&text_ids, true).unwrap_or_default();
        if text.is_empty() {
            return 0.0;
        }
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        if enc.write_all(text.as_bytes()).is_err() {
            return 0.0;
        }
        match enc.finish() {
            Ok(c) if !c.is_empty() => text.len() as f64 / c.len() as f64,
            _ => 0.0,
        }
    }

    /// Walk the generated tokens, splitting text at timestamp tokens into
    /// [`Segment`]s with absolute times. Returns the last timestamp seen (for
    /// advancing the window), 0 if none.
    fn parse_segments(
        &self,
        tokens: &[u32],
        time_offset: f32,
        avg_logprob: f32,
        out: &mut Vec<Segment>,
    ) -> Result<f32> {
        let ts = |t: u32| -> f32 { (t as f32 - self.no_timestamps as f32 + 1.0) / 50.0 };
        let conf = avg_logprob.exp();
        let mut seg_start: Option<f32> = None;
        let mut text_ids: Vec<u32> = Vec::new();
        let mut last_ts = 0.0f32;

        let flush = |start: f32, end: f32, ids: &mut Vec<u32>, out: &mut Vec<Segment>| {
            if ids.is_empty() {
                return;
            }
            if let Ok(text) = self.tokenizer.decode(ids, true) {
                let text = text.trim().to_string();
                if !text.is_empty() {
                    out.push(Segment {
                        text,
                        start: time_offset + start,
                        end: time_offset + end,
                        confidence: conf,
                    });
                }
            }
            ids.clear();
        };

        for &t in tokens {
            if t == self.eot {
                break;
            }
            if t > self.no_timestamps {
                let time = ts(t);
                last_ts = time;
                match seg_start {
                    None => seg_start = Some(time),
                    Some(s) => {
                        flush(s, time, &mut text_ids, out);
                        seg_start = Some(time);
                    }
                }
            } else {
                text_ids.push(t);
            }
        }
        // Trailing text with an open start but no closing timestamp.
        if let Some(s) = seg_start {
            flush(s, last_ts.max(s), &mut text_ids, out);
        } else if !text_ids.is_empty() {
            flush(0.0, 0.0, &mut text_ids, out);
        }
        Ok(last_ts)
    }
}

// ---- model files ------------------------------------------------------------

/// Directory under the models cache where Whisper's files live.
pub fn model_dir() -> PathBuf {
    crate::models::models_dir().join("whisper-base.en")
}

/// Ensure all four Whisper files are present in [`model_dir`], downloading any
/// that are missing. Requires the `download` feature.
#[cfg(feature = "download")]
pub fn ensure_model() -> Result<PathBuf> {
    use std::io::Read;
    let dir = model_dir();
    std::fs::create_dir_all(&dir)?;
    let files = [
        (
            "config.json",
            "https://huggingface.co/openai/whisper-base.en/resolve/main/config.json",
        ),
        (
            "tokenizer.json",
            "https://huggingface.co/openai/whisper-base.en/resolve/main/tokenizer.json",
        ),
        (
            "model.safetensors",
            "https://huggingface.co/openai/whisper-base.en/resolve/main/model.safetensors",
        ),
        (
            "melfilters.bytes",
            "https://raw.githubusercontent.com/huggingface/candle/main/candle-examples/examples/whisper/melfilters.bytes",
        ),
    ];
    for (name, url) in files {
        let dest = dir.join(name);
        if dest.exists() {
            continue;
        }
        tracing::info!("downloading whisper/{name}");
        let resp = ureq::get(url)
            .call()
            .map_err(|e| err(format!("GET {url}: {e}")))?;
        let mut bytes = Vec::new();
        resp.into_reader().read_to_end(&mut bytes).map_err(err)?;
        let tmp = dest.with_extension("part");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &dest)?;
    }
    Ok(dir)
}

#[cfg(not(feature = "download"))]
pub fn ensure_model() -> Result<PathBuf> {
    Err(SyncError::FeatureDisabled("download"))
}
