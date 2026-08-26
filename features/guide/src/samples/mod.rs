//! Preloaded PCM samples and wav loading.
//!
//! The guide engine mixes preloaded PCM at scheduled sample positions
//! (same approach as the legacy fts-guide plugin). Decoding goes through
//! the shared `fts-sample` loader (its `load` feature only, keeping this
//! crate iOS-lean); the small linear resampler below stays for in-memory
//! rate changes — no platform audio I/O.

mod bank;
mod loader;

pub use bank::{
    get_guide_key, section_to_guide_filename, ClickSamplePaths, ClickSound, SampleBank,
};
pub use loader::load_wav;

/// Decoded, planar (per-channel) f32 PCM.
///
/// Mirrors the shape of symphonium's `DecodedAudioF32` that the legacy
/// players consumed: `data[channel][frame]`.
#[derive(Debug, Clone, Default)]
pub struct AudioSample {
    /// Planar channel data: `data[channel][frame]`.
    pub data: Vec<Vec<f32>>,
    /// Sample rate this PCM is at.
    pub sample_rate: u32,
}

impl AudioSample {
    /// Create a mono sample from raw PCM.
    pub fn mono(samples: Vec<f32>, sample_rate: u32) -> Self {
        Self {
            data: vec![samples],
            sample_rate,
        }
    }

    /// Number of frames (samples per channel).
    pub fn frames(&self) -> usize {
        self.data.first().map(|c| c.len()).unwrap_or(0)
    }

    /// Number of channels.
    pub fn channels(&self) -> usize {
        self.data.len()
    }

    /// Linear-resample to `target_rate` (no-op if already there).
    ///
    /// Linear interpolation is plenty for click/voice cues; heavy resamplers
    /// stay out of the processing core.
    pub fn resampled_to(&self, target_rate: u32) -> Self {
        if self.sample_rate == target_rate || self.sample_rate == 0 || self.frames() == 0 {
            let mut out = self.clone();
            out.sample_rate = target_rate.max(out.sample_rate);
            return out;
        }
        let ratio = f64::from(self.sample_rate) / f64::from(target_rate);
        let out_frames = ((self.frames() as f64) * f64::from(target_rate)
            / f64::from(self.sample_rate))
        .round() as usize;
        let data = self
            .data
            .iter()
            .map(|ch| {
                (0..out_frames)
                    .map(|i| {
                        let src = i as f64 * ratio;
                        let i0 = src.floor() as usize;
                        let i1 = (i0 + 1).min(ch.len().saturating_sub(1));
                        let frac = (src - i0 as f64) as f32;
                        let a = ch.get(i0).copied().unwrap_or(0.0);
                        let b = ch.get(i1).copied().unwrap_or(0.0);
                        a + (b - a) * frac
                    })
                    .collect()
            })
            .collect();
        Self {
            data,
            sample_rate: target_rate,
        }
    }
}
