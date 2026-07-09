//! Pluggable stem separation.
//!
//! Separating the vocal stem before alignment is the single biggest quality
//! win — speech acoustic models fall apart on a full mix. But which separator
//! is "best" is exactly the thing you want to benchmark over time (HT-Demucs vs
//! MDX-Net vs an online API). So separation is a trait, not a hard-coded call,
//! and every backend produces the same [`Stems`] shape so [`crate::bench`] can
//! score them head-to-head.

use crate::audio::AudioBuffer;
use crate::error::Result;

pub mod external;

#[cfg(feature = "onnx")]
pub mod demucs;

/// The standard 4-stem decomposition (HT-Demucs layout). `other` is everything
/// not captured by the named stems. A 2-stem backend leaves drums/bass empty
/// and puts the backing track in `other`.
#[derive(Debug, Clone, Default)]
pub struct Stems {
    pub vocals: Option<AudioBuffer>,
    pub drums: Option<AudioBuffer>,
    pub bass: Option<AudioBuffer>,
    pub other: Option<AudioBuffer>,
}

impl Stems {
    /// The vocal stem if present — what the aligner wants.
    pub fn vocals(&self) -> Option<&AudioBuffer> {
        self.vocals.as_ref()
    }
}

/// Which stems a caller wants. Backends may produce more than requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StemSelection {
    /// vocals + accompaniment (fastest, all that alignment needs).
    Two,
    /// vocals + drums + bass + other.
    Four,
}

/// A stem separator. Implementations: [`external::ExternalSeparator`] (shell
/// out to a `demucs`/`audio-separator` install), [`demucs::DemucsOnnx`]
/// (embedded, `onnx` feature), and — later — an online API client.
pub trait StemSeparator {
    /// Human-readable id used in benchmark tables ("htdemucs_ft", "demucs-onnx",
    /// "stemsplit-api", ...).
    fn name(&self) -> &str;
    /// Separate `audio` into stems.
    fn separate(&self, audio: &AudioBuffer, selection: StemSelection) -> Result<Stems>;
}
