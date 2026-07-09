//! Error type for the sync pipeline. Hand-written (rather than derive_more) so
//! the lean default build pulls no extra proc-macro deps.

use std::fmt;

pub type Result<T> = std::result::Result<T, SyncError>;

#[derive(Debug)]
pub enum SyncError {
    /// I/O while reading audio, writing a sidecar, or fetching a model.
    Io(std::io::Error),
    /// Audio decode / unsupported format.
    Audio(String),
    /// Tensor / matrix shape mismatch.
    Shape(String),
    /// Forced-alignment failure (e.g. audio shorter than the lyrics).
    Align(String),
    /// Tokenization: a character has no vocabulary entry, empty text, etc.
    Tokenize(String),
    /// Stem separation backend failure.
    Separation(String),
    /// Sidecar (de)serialization.
    Sidecar(String),
    /// Model registry / download.
    Model(String),
    /// A backend was requested whose feature wasn't compiled in.
    FeatureDisabled(&'static str),
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncError::Io(e) => write!(f, "io error: {e}"),
            SyncError::Audio(m) => write!(f, "audio error: {m}"),
            SyncError::Shape(m) => write!(f, "shape error: {m}"),
            SyncError::Align(m) => write!(f, "alignment error: {m}"),
            SyncError::Tokenize(m) => write!(f, "tokenize error: {m}"),
            SyncError::Separation(m) => write!(f, "separation error: {m}"),
            SyncError::Sidecar(m) => write!(f, "sidecar error: {m}"),
            SyncError::Model(m) => write!(f, "model error: {m}"),
            SyncError::FeatureDisabled(feat) => write!(
                f,
                "this backend needs keyflow-sync to be built with the `{feat}` feature"
            ),
        }
    }
}

impl std::error::Error for SyncError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SyncError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for SyncError {
    fn from(e: std::io::Error) -> Self {
        SyncError::Io(e)
    }
}
