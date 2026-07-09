//! wav2vec2 / MMS CTC acoustic model via ONNX Runtime (`onnx` feature).
//!
//! Produces the emission matrix the pure-Rust [`super::trellis`] consumes. This
//! is the "no external tools" alignment path. It targets the `ort` 2.0 API; the
//! input/output tensor names below match a standard `transformers` wav2vec2
//! export (`input_values` → `logits`). If your export names them differently,
//! adjust [`IN_NAME`]/[`OUT_NAME`] — that's the only model-specific seam.

use ndarray::Array2;
use ort::session::Session;
use ort::value::Tensor;

use super::EmissionModel;
use super::tokenizer::Vocab;
use super::trellis::Emission;
use crate::error::{Result, SyncError};

const IN_NAME: &str = "input_values";
const OUT_NAME: &str = "logits";

/// wav2vec2 conv stride: 320 input samples per output frame (16 kHz → 50 fps).
const STRIDE: usize = 320;
/// Inference window: 30 s (1500 frames). Keeps attention memory bounded so long
/// songs run on a GPU. Whole multiple of STRIDE.
const WINDOW_SAMPLES: usize = STRIDE * 1500;
/// Overlap between windows: 4 s (200 frames). Whole multiple of STRIDE; half is
/// trimmed from each inner boundary to avoid edge artifacts and duplication.
const OVERLAP_SAMPLES: usize = STRIDE * 200;
const OVERLAP_FRAMES: usize = OVERLAP_SAMPLES / STRIDE;

/// An ONNX wav2vec2/MMS model ready to emit CTC log-probabilities.
///
/// `ort`'s `Session::run` takes `&mut self`, but [`EmissionModel::emit`] is
/// `&self` (and we want `&dyn EmissionModel` shareable), so the session lives
/// behind a `Mutex`. Inference is the bottleneck anyway; lock contention is moot.
pub struct Wav2Vec2Onnx {
    session: std::sync::Mutex<Session>,
    vocab: Vocab,
    sample_rate: u32,
    frames_per_second: f32,
}

impl Wav2Vec2Onnx {
    /// Load a model. `frames_per_second` is the model stride in Hz
    /// (~49.95 for 16 kHz wav2vec2 with a 320-sample hop).
    pub fn load(
        model_path: impl AsRef<std::path::Path>,
        vocab: Vocab,
        sample_rate: u32,
        frames_per_second: f32,
    ) -> Result<Self> {
        let session = crate::onnx_util::build_session(model_path.as_ref())?;
        Ok(Self {
            session: std::sync::Mutex::new(session),
            vocab,
            sample_rate,
            frames_per_second,
        })
    }
}

/// Per-row log-softmax in place over a `[frames, vocab]` flat buffer.
fn log_softmax_rows(data: &mut [f32], frames: usize, vocab: usize) {
    for f in 0..frames {
        let row = &mut data[f * vocab..(f + 1) * vocab];
        let max = row.iter().cloned().fold(f32::MIN, f32::max);
        let mut sum = 0.0f32;
        for v in row.iter() {
            sum += (v - max).exp();
        }
        let logz = max + sum.ln();
        for v in row.iter_mut() {
            *v -= logz;
        }
    }
}

impl EmissionModel for Wav2Vec2Onnx {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
    fn vocab(&self) -> &Vocab {
        &self.vocab
    }
    fn frames_per_second(&self) -> f32 {
        self.frames_per_second
    }

    fn emit(&self, samples: &[f32]) -> Result<Emission> {
        if samples.is_empty() {
            return Err(SyncError::Audio("empty audio for emission".into()));
        }
        // A single forward over a whole song is O(seq²) in attention memory and
        // OOMs a GPU (it only fits in big CPU RAM). Process in overlapping
        // windows and stitch the emissions, dropping half the overlap on each
        // inner boundary so frames stay contiguous at the model's frame rate.
        // Window/overlap are whole multiples of STRIDE so frame counts line up.
        if samples.len() <= WINDOW_SAMPLES {
            return self.emit_window(samples);
        }

        let hop = WINDOW_SAMPLES - OVERLAP_SAMPLES;
        let mut data: Vec<f32> = Vec::new();
        let mut total_frames = 0usize;
        let mut vocab;
        let mut start = 0usize;
        loop {
            let end = (start + WINDOW_SAMPLES).min(samples.len());
            let em = self.emit_window(&samples[start..end])?;
            vocab = em.vocab;
            let drop_left = if start == 0 { 0 } else { OVERLAP_FRAMES / 2 };
            let last = end == samples.len();
            let drop_right = if last { 0 } else { OVERLAP_FRAMES / 2 };
            let keep_end = em.frames.saturating_sub(drop_right);
            if keep_end > drop_left {
                data.extend_from_slice(&em.data[drop_left * vocab..keep_end * vocab]);
                total_frames += keep_end - drop_left;
            }
            if last {
                break;
            }
            start += hop;
        }
        Emission::new(data, total_frames, vocab)
    }
}

impl Wav2Vec2Onnx {
    /// Run the model over one window (assumed small enough to fit in memory) and
    /// return its log-softmax emission `[frames, vocab]`.
    fn emit_window(&self, samples: &[f32]) -> Result<Emission> {
        // wav2vec2 expects zero-mean / unit-variance input_values [batch, seq].
        let mean = samples.iter().sum::<f32>() / samples.len() as f32;
        let var = samples.iter().map(|s| (s - mean).powi(2)).sum::<f32>() / samples.len() as f32;
        let std = var.sqrt().max(1e-7);
        let normed: Vec<f32> = samples.iter().map(|s| (s - mean) / std).collect();

        let input = Array2::from_shape_vec((1, normed.len()), normed)
            .map_err(|e| SyncError::Shape(format!("input shape: {e}")))?;
        let tensor = Tensor::from_array(input)
            .map_err(|e| SyncError::Shape(format!("input tensor: {e}")))?;

        let mut session = self
            .session
            .lock()
            .map_err(|_| SyncError::Model("session mutex poisoned".into()))?;
        let outputs = session
            .run(ort::inputs![IN_NAME => tensor])
            .map_err(|e| SyncError::Model(format!("inference: {e}")))?;

        // logits: [batch=1, frames, vocab]
        let (shape, data) = outputs[OUT_NAME]
            .try_extract_tensor::<f32>()
            .map_err(|e| SyncError::Model(format!("extract logits: {e}")))?;
        if shape.len() != 3 {
            return Err(SyncError::Shape(format!(
                "expected 3D logits, got shape {shape:?}"
            )));
        }
        let frames = shape[1] as usize;
        let vocab = shape[2] as usize;
        if vocab != self.vocab.len() {
            return Err(SyncError::Shape(format!(
                "model vocab {vocab} != tokenizer vocab {}",
                self.vocab.len()
            )));
        }
        let mut buf = data.to_vec();
        log_softmax_rows(&mut buf, frames, vocab);
        Emission::new(buf, frames, vocab)
    }
}
