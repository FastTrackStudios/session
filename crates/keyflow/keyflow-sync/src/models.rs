//! Model registry, cache, and download.
//!
//! Weights never ship in the binary — they're pulled on demand into a cache dir
//! and reused. The registry is *data*: a built-in default list you can override
//! with a `models.json` in the cache dir (or `KEYFLOW_MODELS_JSON`), so URLs and
//! checksums can be corrected without a recompile.
//!
//! NOTE: the built-in URLs are best-effort starting points for the ONNX exports
//! discussed in the design notes. Verify/replace them for your deployment — the
//! download machinery doesn't care what the URL is.

use std::path::PathBuf;

use facet::Facet;

use crate::error::{Result, SyncError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum ModelKind {
    /// CTC acoustic model for forced alignment (wav2vec2 / MMS).
    Aligner,
    /// Source-separation model (HT-Demucs / MDX-Net).
    Separator,
}

#[derive(Debug, Clone, PartialEq, Facet)]
pub struct ModelEntry {
    /// Stable id used on the CLI and in sidecar provenance.
    pub id: String,
    pub kind: ModelKind,
    /// Download URL for the ONNX file (or archive).
    pub url: String,
    /// Optional lowercase-hex SHA-256 for integrity verification.
    pub sha256: Option<String>,
    /// Local filename under the cache dir.
    pub filename: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Registry {
    pub models: Vec<ModelEntry>,
}

/// Built-in defaults. Replace URLs/checksums via `models.json` as needed.
pub fn builtin_registry() -> Registry {
    Registry {
        models: vec![
            ModelEntry {
                // Real, working entry: the official ONNX export of
                // facebook/wav2vec2-base-960h (input_values -> logits, uppercase
                // 960h CTC vocab — matches `Vocab::wav2vec2_en_960h`).
                id: "w2v2-960h".into(),
                kind: ModelKind::Aligner,
                // Pinned to the revision that carries the onnx/ export (not on main).
                url: "https://huggingface.co/facebook/wav2vec2-base-960h/resolve/6d2b9ffaac8aabc45934584ee608c5fb5ee34a4e/onnx/model.onnx".into(),
                sha256: None,
                filename: "wav2vec2-base-960h.onnx".into(),
                description: "wav2vec2-base-960h CTC aligner (English, speech-trained)".into(),
            },
            ModelEntry {
                // MMS-300M forced aligner (MahmoudAshraf port of torchaudio MMS_FA),
                // ONNX by onnx-community. 158 languages, lowercase/romanized char
                // vocab, no separator token. Far more robust on real/sung audio than
                // wav2vec2-960h. NOTE: weights are CC-BY-NC-4.0 (non-commercial).
                id: "mms-fa".into(),
                kind: ModelKind::Aligner,
                url: "https://huggingface.co/onnx-community/mms-300m-1130-forced-aligner-ONNX/resolve/main/onnx/model.onnx".into(),
                sha256: None,
                filename: "mms-fa.onnx".into(),
                description: "MMS-300M forced aligner (158 langs, CTC; CC-BY-NC)".into(),
            },
            ModelEntry {
                // Real, working entry: HT-Demucs FT vocals specialist, ONNX export
                // by StemSplitio. Fixed I/O `mix`[1,2,343980] -> `stems`[1,4,2,343980],
                // source order [drums, bass, other, vocals]; we read the vocals row.
                id: "htdemucs-vocals".into(),
                kind: ModelKind::Separator,
                url: "https://huggingface.co/StemSplitio/htdemucs-ft-vocals-onnx/resolve/main/htdemucs_ft_vocals.onnx".into(),
                sha256: None,
                filename: "htdemucs-ft-vocals.onnx".into(),
                description: "HT-Demucs FT vocals separation (44.1kHz, 7.8s windows)".into(),
            },
        ],
    }
}

/// Cache dir: `$KEYFLOW_MODELS_DIR`, else `$XDG_CACHE_HOME/keyflow/models`,
/// else `$HOME/.cache/keyflow/models`.
pub fn models_dir() -> PathBuf {
    if let Ok(d) = std::env::var("KEYFLOW_MODELS_DIR") {
        return PathBuf::from(d);
    }
    let base = std::env::var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join(".cache")
        });
    base.join("keyflow").join("models")
}

/// Load the registry, preferring a user override file over the built-in list.
pub fn load_registry() -> Result<Registry> {
    let path = std::env::var("KEYFLOW_MODELS_JSON")
        .map(PathBuf::from)
        .unwrap_or_else(|_| models_dir().join("models.json"));
    if path.exists() {
        let text = std::fs::read_to_string(&path)?;
        return facet_json::from_str(&text)
            .map_err(|e| SyncError::Model(format!("parse {}: {e}", path.display())));
    }
    Ok(builtin_registry())
}

pub fn find(registry: &Registry, id: &str) -> Option<ModelEntry> {
    registry.models.iter().find(|m| m.id == id).cloned()
}

/// Absolute path a model resolves to in the cache (whether or not present).
pub fn local_path(entry: &ModelEntry) -> PathBuf {
    models_dir().join(&entry.filename)
}

pub fn is_present(entry: &ModelEntry) -> bool {
    local_path(entry).exists()
}

/// Download a model into the cache if not already present. Verifies SHA-256
/// when the registry supplies one. Requires the `download` feature.
#[cfg(feature = "download")]
pub fn pull(entry: &ModelEntry) -> Result<PathBuf> {
    use sha2::{Digest, Sha256};
    use std::io::Read;

    let dest = local_path(entry);
    if dest.exists() {
        return Ok(dest);
    }
    std::fs::create_dir_all(models_dir())?;
    tracing::info!("downloading {} <- {}", entry.id, entry.url);

    let resp = ureq::get(&entry.url)
        .call()
        .map_err(|e| SyncError::Model(format!("GET {}: {e}", entry.url)))?;
    let mut bytes = Vec::new();
    resp.into_reader()
        .read_to_end(&mut bytes)
        .map_err(|e| SyncError::Model(format!("read body: {e}")))?;

    if let Some(expected) = &entry.sha256 {
        let got = format!("{:x}", Sha256::digest(&bytes));
        if !got.eq_ignore_ascii_case(expected) {
            return Err(SyncError::Model(format!(
                "checksum mismatch for {}: expected {expected}, got {got}",
                entry.id
            )));
        }
    }

    // Write atomically via a temp file then rename.
    let tmp = dest.with_extension("part");
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &dest)?;
    Ok(dest)
}

#[cfg(not(feature = "download"))]
pub fn pull(_entry: &ModelEntry) -> Result<PathBuf> {
    Err(SyncError::FeatureDisabled("download"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_round_trips_and_lookups() {
        let reg = builtin_registry();
        assert!(find(&reg, "mms-fa").is_some());
        assert!(find(&reg, "nope").is_none());
        // facet round-trip so a user override file is parseable.
        let json = facet_json::to_string_pretty(&reg).unwrap();
        let back: Registry = facet_json::from_str(&json).unwrap();
        assert_eq!(reg, back);
    }

    #[test]
    fn models_dir_honors_env() {
        std::env::set_var("KEYFLOW_MODELS_DIR", "/tmp/kf-models-test");
        assert_eq!(models_dir(), PathBuf::from("/tmp/kf-models-test"));
        std::env::remove_var("KEYFLOW_MODELS_DIR");
    }
}
