//! HT-Demucs source separation via ONNX Runtime (`onnx` feature).
//!
//! This is the embedded, no-external-tools separation path. HT-Demucs is a
//! hybrid (waveform + spectrogram) model; a working ONNX export bakes the
//! STFT/ISTFT in, exposing a simple contract:
//!
//! * input  `mix`:   `f32` `[batch=1, channels=2, length]`
//! * output `stems`: `f32` `[batch=1, 4, channels=2, length]`, sources ordered
//!   `[drums, bass, other, vocals]` (the Demucs convention).
//!
//! Long songs must be processed in overlapping windows (Demucs is trained on
//! ~7.8 s segments); here we run the model on fixed-size chunks with linear
//! cross-fade overlap-add. Adjust [`IN_NAME`]/[`OUT_NAME`]/[`SOURCE_ORDER`] to
//! match your specific export — that's the only model-specific seam.

use ndarray::Array3;
use ort::session::Session;
use ort::value::Tensor;

use crate::audio::AudioBuffer;
use crate::error::{Result, SyncError};

use super::{StemSelection, StemSeparator, Stems};

const IN_NAME: &str = "mix";
const OUT_NAME: &str = "stems";
/// Source index order in the model output.
const SOURCE_ORDER: [&str; 4] = ["drums", "bass", "other", "vocals"];
/// HT-Demucs native sample rate.
const MODEL_SR: u32 = 44_100;
/// Chunk length in samples (~7.8 s) and overlap for cross-fade.
const CHUNK: usize = 343_980;
const OVERLAP: usize = CHUNK / 4;

pub struct DemucsOnnx {
    // `ort::Session::run` needs `&mut self`; the trait method is `&self`. See
    // the note on `Wav2Vec2Onnx`.
    session: std::sync::Mutex<Session>,
    name: String,
}

impl DemucsOnnx {
    pub fn load(model_path: impl AsRef<std::path::Path>, name: impl Into<String>) -> Result<Self> {
        let session = crate::onnx_util::build_session(model_path.as_ref())?;
        Ok(Self {
            session: std::sync::Mutex::new(session),
            name: name.into(),
        })
    }

    /// Run one chunk through the model. The export has a **fixed** input shape
    /// `[1, 2, CHUNK]`, so a chunk shorter than `CHUNK` (the final window) is
    /// zero-padded and the outputs trimmed back to `chunk.len()`. `chunk` is
    /// mono, duplicated into both stereo channels. Returns `[4][chunk.len()]`
    /// mono stems (channels averaged), ordered per [`SOURCE_ORDER`].
    fn run_chunk(&self, chunk: &[f32]) -> Result<[Vec<f32>; 4]> {
        let len = chunk.len();
        debug_assert!(len <= CHUNK);
        // Always feed the fixed CHUNK length; pad the tail with zeros.
        let mut input = Array3::<f32>::zeros((1, 2, CHUNK));
        for (i, &s) in chunk.iter().enumerate() {
            input[[0, 0, i]] = s;
            input[[0, 1, i]] = s;
        }
        let tensor =
            Tensor::from_array(input).map_err(|e| SyncError::Shape(format!("mix tensor: {e}")))?;
        let mut session = self
            .session
            .lock()
            .map_err(|_| SyncError::Separation("session mutex poisoned".into()))?;
        let outputs = session
            .run(ort::inputs![IN_NAME => tensor])
            .map_err(|e| SyncError::Separation(format!("inference: {e}")))?;
        let (shape, data) = outputs[OUT_NAME]
            .try_extract_tensor::<f32>()
            .map_err(|e| SyncError::Separation(format!("extract stems: {e}")))?;
        // expect [1, 4, 2, CHUNK]
        if shape.len() != 4 || shape[1] != 4 {
            return Err(SyncError::Shape(format!(
                "unexpected stems shape {shape:?}"
            )));
        }
        let ch = shape[2] as usize;
        let out_len = shape[3] as usize;
        // Trim back to the real (unpadded) chunk length.
        let keep = len.min(out_len);
        let mut stems: [Vec<f32>; 4] = Default::default();
        for (s, stem) in stems.iter_mut().enumerate() {
            *stem = (0..keep)
                .map(|i| {
                    // average channels back to mono
                    let mut acc = 0.0f32;
                    for c in 0..ch {
                        let idx = ((s * ch + c) * out_len) + i;
                        acc += data[idx];
                    }
                    acc / ch as f32
                })
                .collect();
        }
        Ok(stems)
    }
}

impl StemSeparator for DemucsOnnx {
    fn name(&self) -> &str {
        &self.name
    }

    fn separate(&self, audio: &AudioBuffer, selection: StemSelection) -> Result<Stems> {
        let src = audio.resample(MODEL_SR);
        let n = src.samples.len();
        let mut acc: [Vec<f32>; 4] = [vec![0.0; n], vec![0.0; n], vec![0.0; n], vec![0.0; n]];
        let mut weight = vec![0.0f32; n];

        let step = CHUNK - OVERLAP;
        let mut start = 0usize;
        while start < n {
            let end = (start + CHUNK).min(n);
            let chunk = &src.samples[start..end];
            let stems = self.run_chunk(chunk)?;
            // triangular window for the cross-fade overlap-add.
            for i in 0..(end - start) {
                let pos = i as f32 / (end - start).max(1) as f32;
                let w = 1.0 - (2.0 * pos - 1.0).abs(); // peak in the middle
                let w = w.max(0.05);
                for s in 0..4 {
                    if i < stems[s].len() {
                        acc[s][start + i] += stems[s][i] * w;
                    }
                }
                weight[start + i] += w;
            }
            if end == n {
                break;
            }
            start += step;
        }
        for source in &mut acc {
            for i in 0..n {
                if weight[i] > 0.0 {
                    source[i] /= weight[i];
                }
            }
        }

        let mut buf = |idx: usize| AudioBuffer::new(std::mem::take(&mut acc[idx]), MODEL_SR);
        let vocals_idx = SOURCE_ORDER
            .iter()
            .position(|&s| s == "vocals")
            .unwrap_or(3);
        let mut stems = Stems::default();
        // Take vocals first (it's the one alignment needs).
        stems.vocals = Some(buf(vocals_idx));
        if selection == StemSelection::Four {
            for (i, &name) in SOURCE_ORDER.iter().enumerate() {
                if i == vocals_idx {
                    continue;
                }
                let b = Some(buf(i));
                match name {
                    "drums" => stems.drums = b,
                    "bass" => stems.bass = b,
                    _ => stems.other = b,
                }
            }
        }
        Ok(stems)
    }
}
