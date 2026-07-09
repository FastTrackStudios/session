//! Separation by shelling out to an installed CLI — the zero-build-deps path.
//!
//! This wraps a `demucs`-compatible command (the reference Python `demucs`, or
//! anything that writes `vocals.wav`/`drums.wav`/`bass.wav`/`other.wav` into a
//! per-track output folder). It's the backend `kf sync` uses when built without
//! the `onnx` feature: no native lib in the binary, at the cost of requiring
//! `demucs` on PATH. Great for development and for benchmarking the canonical
//! implementation against our embedded one.

use std::path::PathBuf;
use std::process::Command;

use crate::audio::AudioBuffer;
use crate::error::{Result, SyncError};

use super::{StemSelection, StemSeparator, Stems};

/// Shell-out separator. Defaults to the reference `demucs` CLI invocation:
/// `demucs -n <model> -o <out> <input.wav>`, which writes
/// `<out>/<model>/<track>/<stem>.wav`.
#[derive(Debug, Clone)]
pub struct ExternalSeparator {
    /// Executable to run (default `"demucs"`).
    pub program: String,
    /// Model name passed via `-n` (default `"htdemucs"`).
    pub model: String,
    /// Working directory for outputs (a temp dir if `None`).
    pub work_dir: Option<PathBuf>,
}

impl Default for ExternalSeparator {
    fn default() -> Self {
        Self {
            program: "demucs".to_string(),
            model: "htdemucs".to_string(),
            work_dir: None,
        }
    }
}

impl ExternalSeparator {
    pub fn with_model(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..Default::default()
        }
    }
}

impl StemSeparator for ExternalSeparator {
    fn name(&self) -> &str {
        &self.model
    }

    fn separate(&self, audio: &AudioBuffer, selection: StemSelection) -> Result<Stems> {
        // demucs reads files, so stage the input as WAV in a work dir.
        let work = match &self.work_dir {
            Some(d) => d.clone(),
            None => std::env::temp_dir().join(format!("keyflow-sync-{}", std::process::id())),
        };
        std::fs::create_dir_all(&work)?;
        let input = work.join("input.wav");
        audio.write_wav(&input)?;
        let out = work.join("out");

        let mut cmd = Command::new(&self.program);
        cmd.arg("-n").arg(&self.model).arg("-o").arg(&out);
        if selection == StemSelection::Two {
            // demucs two-stem mode isolates one source against the rest.
            cmd.arg("--two-stems").arg("vocals");
        }
        cmd.arg(&input);

        let status = cmd.status().map_err(|e| {
            SyncError::Separation(format!(
                "failed to run `{}` (is it installed and on PATH?): {e}",
                self.program
            ))
        })?;
        if !status.success() {
            return Err(SyncError::Separation(format!(
                "`{}` exited with {status}",
                self.program
            )));
        }

        let track_dir = out.join(&self.model).join("input");
        let load = |name: &str| -> Option<AudioBuffer> {
            let p = track_dir.join(format!("{name}.wav"));
            p.exists().then(|| AudioBuffer::load_wav(&p).ok()).flatten()
        };

        let mut stems = Stems {
            vocals: load("vocals"),
            ..Default::default()
        };
        if selection == StemSelection::Four {
            stems.drums = load("drums");
            stems.bass = load("bass");
            stems.other = load("other");
        } else {
            // two-stem writes vocals.wav + no_vocals.wav; keep the latter as `other`.
            stems.other = load("no_vocals");
        }

        if stems.vocals.is_none() {
            return Err(SyncError::Separation(format!(
                "no vocals.wav under {}; did the model name match?",
                track_dir.display()
            )));
        }
        Ok(stems)
    }
}
