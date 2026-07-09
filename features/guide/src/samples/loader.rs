//! wav file loading (replaces the legacy symphonium-based `SampleLoader`).

use std::path::Path;

use crate::GuideError;

use super::AudioSample;

/// Load a wav file as planar f32 PCM, resampled to `target_sample_rate`.
///
/// Supports 16/24/32-bit integer and 32-bit float wav.
pub fn load_wav(path: &Path, target_sample_rate: u32) -> Result<AudioSample, GuideError> {
    let mut reader = hound::WavReader::open(path)
        .map_err(|e| GuideError::SampleLoad(format!("{}: {e}", path.display())))?;
    let spec = reader.spec();
    let channels = spec.channels.max(1) as usize;

    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| GuideError::SampleLoad(format!("{}: {e}", path.display())))?,
        hound::SampleFormat::Int => {
            let scale = 1.0f32 / (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<_, _>>()
                .map_err(|e| GuideError::SampleLoad(format!("{}: {e}", path.display())))?
        }
    };

    let frames = interleaved.len() / channels;
    let mut data = vec![Vec::with_capacity(frames); channels];
    for (i, s) in interleaved.iter().enumerate() {
        data[i % channels].push(*s);
    }

    let sample = AudioSample {
        data,
        sample_rate: spec.sample_rate,
    };
    Ok(sample.resampled_to(target_sample_rate))
}
