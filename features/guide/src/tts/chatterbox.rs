//! Local Chatterbox TTS backend (bearcove's `chatterbox-rs`, ONNX / ort).
//!
//! Wraps `chatterbox_rs::chatterbox::Chatterbox`: models are fetched into
//! the Hugging Face cache on first use (`hf::download_chatterbox_assets`),
//! then `synthesize()` produces 24 kHz mono PCM. Loading the models takes
//! seconds and synthesis is far from realtime — use only through
//! [`CueBank`](super::CueBank) at setlist-build time.

use std::path::PathBuf;

use chatterbox_rs::chatterbox::Chatterbox;
use chatterbox_rs::hf::{self, ModelVariant};

use crate::GuideError;

use super::{TtsAudio, TtsRenderer};

/// Chatterbox model sample rate (`chatterbox_rs::audio::TARGET_SAMPLE_RATE`).
const CHATTERBOX_SAMPLE_RATE: u32 = 24_000;

/// Configuration for [`ChatterboxTts`].
#[derive(Debug, Clone)]
pub struct ChatterboxTtsConfig {
    /// Hugging Face repo of the ONNX export.
    pub repo_id: String,
    /// Repo revision.
    pub revision: String,
    /// Which quantization variant to download/load. Quantized is a good
    /// CPU default (~4x smaller than fp32).
    pub variant: ModelVariant,
    /// Reference voice wav (Chatterbox is a voice-cloning model; a short
    /// clean clip of the desired voice is required).
    pub voice_wav: PathBuf,
    /// Stable name for the voice, used in cue cache keys.
    pub voice_name: String,
    /// Generation cap (legacy cbx default: 512).
    pub max_new_tokens: usize,
    /// Repetition penalty (cbx default: 1.2).
    pub repetition_penalty: f32,
}

impl ChatterboxTtsConfig {
    /// Default model config for a given reference voice.
    pub fn with_voice(voice_wav: impl Into<PathBuf>, voice_name: impl Into<String>) -> Self {
        Self {
            repo_id: "ResembleAI/chatterbox-turbo-ONNX".to_string(),
            revision: "main".to_string(),
            variant: ModelVariant::Quantized,
            voice_wav: voice_wav.into(),
            voice_name: voice_name.into(),
            max_new_tokens: 512,
            repetition_penalty: 1.2,
        }
    }
}

/// A [`TtsRenderer`] backed by a locally-loaded Chatterbox model.
pub struct ChatterboxTts {
    model: Chatterbox,
    config: ChatterboxTtsConfig,
    voice_id: String,
}

impl ChatterboxTts {
    /// Download (into the HF cache) and load the model. Blocking and slow;
    /// never call from an audio thread.
    pub fn load(config: ChatterboxTtsConfig) -> Result<Self, GuideError> {
        let paths =
            hf::download_chatterbox_assets(&config.repo_id, &config.revision, config.variant)
                .map_err(|e| GuideError::Tts(format!("chatterbox asset download: {e:#}")))?;
        let model = Chatterbox::load(&paths)
            .map_err(|e| GuideError::Tts(format!("chatterbox model load: {e:#}")))?;
        let voice_id = format!(
            "chatterbox:{}@{}:{:?}:{}",
            config.repo_id, config.revision, config.variant, config.voice_name
        );
        Ok(Self {
            model,
            config,
            voice_id,
        })
    }
}

impl TtsRenderer for ChatterboxTts {
    fn voice_id(&self) -> &str {
        &self.voice_id
    }

    fn render(&mut self, text: &str) -> Result<TtsAudio, GuideError> {
        let samples = self
            .model
            .synthesize(
                text,
                &self.config.voice_wav,
                self.config.max_new_tokens,
                self.config.repetition_penalty,
            )
            .map_err(|e| GuideError::Tts(format!("chatterbox synthesize {text:?}: {e:#}")))?;
        Ok(TtsAudio {
            samples,
            sample_rate: CHATTERBOX_SAMPLE_RATE,
        })
    }
}
