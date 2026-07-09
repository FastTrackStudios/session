//! Minimal audio loading. WAV in, mono `f32` out, with a naive linear
//! resampler good enough to feed a 16 kHz acoustic model. Compressed inputs
//! (mp3/flac/m4a) are expected to be transcoded to WAV upstream — the docs
//! recommend `ffmpeg -i in.mp3 -ar 44100 -ac 2 out.wav`, because feeding lossy
//! sources to a separator/aligner is the #1 cause of metallic artifacts.

use std::path::Path;

use crate::error::{Result, SyncError};

/// PCM audio held as interleaved-free mono `f32` in `-1.0..=1.0`.
#[derive(Debug, Clone)]
pub struct AudioBuffer {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl AudioBuffer {
    pub fn new(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self {
            samples,
            sample_rate,
        }
    }

    pub fn duration_secs(&self) -> f32 {
        self.samples.len() as f32 / self.sample_rate as f32
    }

    /// Load a WAV file, downmixing to mono.
    pub fn load_wav(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let mut reader = hound::WavReader::open(path)
            .map_err(|e| SyncError::Audio(format!("open {}: {e}", path.display())))?;
        let spec = reader.spec();
        let channels = spec.channels.max(1) as usize;

        // Normalize every sample format to f32 in [-1, 1].
        let raw: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader
                .samples::<f32>()
                .collect::<std::result::Result<_, _>>()
                .map_err(|e| SyncError::Audio(format!("decode float: {e}")))?,
            hound::SampleFormat::Int => {
                let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
                reader
                    .samples::<i32>()
                    .map(|s| s.map(|v| v as f32 / max))
                    .collect::<std::result::Result<_, _>>()
                    .map_err(|e| SyncError::Audio(format!("decode int: {e}")))?
            }
        };

        // Downmix channels to mono by averaging.
        let mono: Vec<f32> = if channels == 1 {
            raw
        } else {
            raw.chunks(channels)
                .map(|frame| frame.iter().sum::<f32>() / channels as f32)
                .collect()
        };

        Ok(Self::new(mono, spec.sample_rate))
    }

    /// Linear-resample to `target` Hz. No-op when already at rate. This is a
    /// quality compromise (a windowed-sinc resampler would be cleaner) but it
    /// is dependency-free and adequate for forced alignment, where exact
    /// timbre matters far less than coarse frame timing.
    pub fn resample(&self, target: u32) -> AudioBuffer {
        if target == self.sample_rate || self.samples.is_empty() {
            return self.clone();
        }
        let ratio = target as f64 / self.sample_rate as f64;
        let out_len = ((self.samples.len() as f64) * ratio).round() as usize;
        let mut out = Vec::with_capacity(out_len);
        for i in 0..out_len {
            let src = i as f64 / ratio;
            let lo = src.floor() as usize;
            let hi = (lo + 1).min(self.samples.len() - 1);
            let frac = (src - lo as f64) as f32;
            out.push(self.samples[lo] * (1.0 - frac) + self.samples[hi] * frac);
        }
        AudioBuffer::new(out, target)
    }

    /// Write the buffer as a 16-bit PCM mono WAV (used when handing a separated
    /// stem to an external tool, or for debugging).
    pub fn write_wav(&self, path: impl AsRef<Path>) -> Result<()> {
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: self.sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path.as_ref(), spec)
            .map_err(|e| SyncError::Audio(format!("create wav: {e}")))?;
        for &s in &self.samples {
            let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer
                .write_sample(v)
                .map_err(|e| SyncError::Audio(format!("write sample: {e}")))?;
        }
        writer
            .finalize()
            .map_err(|e| SyncError::Audio(format!("finalize wav: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_halves_length_when_downsampling() {
        let buf = AudioBuffer::new(vec![0.0; 1000], 32_000);
        let down = buf.resample(16_000);
        assert_eq!(down.sample_rate, 16_000);
        assert!((down.samples.len() as i64 - 500).abs() <= 1);
    }

    #[test]
    fn resample_noop_at_same_rate() {
        let buf = AudioBuffer::new(vec![0.1, 0.2, 0.3], 16_000);
        let same = buf.resample(16_000);
        assert_eq!(same.samples, buf.samples);
    }
}
