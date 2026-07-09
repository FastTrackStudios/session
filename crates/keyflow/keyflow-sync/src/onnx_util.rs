//! Shared ONNX Runtime session construction (only built with `onnx`).
//!
//! Centralizes execution-provider setup so both the aligner and the separator
//! get GPU acceleration the same way. With the `cuda` feature, the CUDA
//! execution provider is registered first; ORT falls back to CPU automatically
//! if the CUDA libraries (onnxruntime CUDA provider + CUDA runtime + cuDNN)
//! aren't present at runtime, so the same binary works on GPU and CPU boxes.

use std::path::Path;

use ort::session::Session;

use crate::error::{Result, SyncError};

/// Build a session for `path`, preferring GPU when compiled and available.
pub fn build_session(path: &Path) -> Result<Session> {
    #[allow(unused_mut)]
    let mut builder =
        Session::builder().map_err(|e| SyncError::Model(format!("ort builder: {e}")))?;

    #[cfg(feature = "cuda")]
    {
        use ort::execution_providers::CUDAExecutionProvider;
        // Default: register CUDA, falling back to CPU if it can't initialize.
        // Set KEYFLOW_REQUIRE_CUDA=1 to make a failed CUDA init a hard error
        // (surfaces the real reason instead of a silent CPU fallback).
        let mut ep = CUDAExecutionProvider::default().build();
        if std::env::var("KEYFLOW_REQUIRE_CUDA").is_ok() {
            ep = ep.error_on_failure();
        }
        builder = builder
            .with_execution_providers([ep])
            .map_err(|e| SyncError::Model(format!("register CUDA EP: {e}")))?;
    }

    builder
        .commit_from_file(path)
        .map_err(|e| SyncError::Model(format!("load {}: {e}", path.display())))
}
