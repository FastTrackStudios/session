//! Error types for the engraver crate.
//!
//! This module provides a unified error type for all engraver operations,
//! replacing scattered panics with proper Result-based error handling.

use derive_more::{Display, Error as DeriveError, From};

// region:    --- Result Alias

/// Result type alias for engraver operations.
pub type Result<T> = core::result::Result<T, Error>;

// endregion: --- Result Alias

// region:    --- Error Types

/// Unified error type for engraver operations.
///
/// Categorized by subsystem for clear error handling and reporting.
#[derive(Debug, Display, DeriveError, From)]
pub enum Error {
    // -- Layout Errors
    /// Beam group has no notes
    #[display("beam group is empty")]
    EmptyBeamGroup,

    /// Chord has no notes
    #[display("chord is empty")]
    EmptyChord,

    /// Tuplet has no notes
    #[display("tuplet is empty")]
    EmptyTuplet,

    /// Invalid measure index
    #[display("invalid measure index: {_0}")]
    #[error(ignore)]
    InvalidMeasureIndex(usize),

    /// Invalid staff index
    #[display("invalid staff index: {_0}")]
    #[error(ignore)]
    InvalidStaffIndex(usize),

    /// Invalid voice index
    #[display("invalid voice index: {_0}")]
    #[error(ignore)]
    InvalidVoiceIndex(usize),

    /// Layout configuration error
    #[display("layout configuration error: {_0}")]
    #[error(ignore)]
    LayoutConfig(String),

    // -- Font Errors
    /// Font not loaded
    #[display("font not loaded")]
    FontNotLoaded,

    /// Missing glyph in font
    #[display("missing glyph '{name}' (U+{codepoint:04X})")]
    #[error(ignore)]
    MissingGlyph { name: String, codepoint: u32 },

    /// Font metadata error
    #[display("font metadata error: {_0}")]
    #[error(ignore)]
    FontMetadata(String),

    // -- Style Errors
    /// Invalid style property
    #[display("invalid style property: {_0}")]
    #[error(ignore)]
    InvalidStyleProperty(String),

    /// Style value type mismatch
    #[display("style value type mismatch for {property}: expected {expected}, got {actual}")]
    #[error(ignore)]
    StyleTypeMismatch {
        property: String,
        expected: String,
        actual: String,
    },

    // -- Scene Errors
    /// Invalid scene node
    #[display("invalid scene node: {_0}")]
    #[error(ignore)]
    InvalidSceneNode(String),

    /// Scene graph cycle detected
    #[display("scene graph cycle detected")]
    SceneGraphCycle,

    // -- Render Errors
    /// GPU device error
    #[display("GPU device error: {_0}")]
    #[error(ignore)]
    GpuDevice(String),

    /// Texture creation error
    #[display("texture creation error: {_0}")]
    #[error(ignore)]
    TextureCreation(String),

    // -- IO Errors
    /// Standard IO error
    #[from]
    Io(std::io::Error),

    /// JSON serialization/deserialization error
    #[from]
    Json(serde_json::Error),

    // -- Import Errors
    /// MIDI parse error
    #[display("MIDI parse error: {_0}")]
    #[error(ignore)]
    MidiParse(String),

    // -- Generic Errors
    /// Custom error with message
    #[display("{_0}")]
    #[error(ignore)]
    Custom(String),
}

// endregion: --- Error Types

// region:    --- Error Conversions

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Self::Custom(s.to_string())
    }
}

impl From<&String> for Error {
    fn from(s: &String) -> Self {
        Self::Custom(s.clone())
    }
}

// endregion: --- Error Conversions

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    type TestResult<T> = core::result::Result<T, Box<dyn std::error::Error>>;

    #[test]
    fn test_error_display_empty_beam() -> TestResult<()> {
        let err = Error::EmptyBeamGroup;
        assert_eq!(err.to_string(), "beam group is empty");
        Ok(())
    }

    #[test]
    fn test_error_display_missing_glyph() -> TestResult<()> {
        let err = Error::MissingGlyph {
            name: "noteheadBlack".to_string(),
            codepoint: 0xE0A4,
        };
        assert_eq!(err.to_string(), "missing glyph 'noteheadBlack' (U+E0A4)");
        Ok(())
    }

    #[test]
    fn test_error_from_str() -> TestResult<()> {
        let err: Error = "custom error message".into();
        assert_eq!(err.to_string(), "custom error message");
        Ok(())
    }

    #[test]
    fn test_error_from_io() -> TestResult<()> {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let err: Error = io_err.into();
        assert!(matches!(err, Error::Io(_)));
        Ok(())
    }

    #[test]
    fn test_result_type_alias() -> TestResult<()> {
        fn returns_ok() -> Result<i32> {
            Ok(42)
        }

        fn returns_err() -> Result<i32> {
            Err(Error::EmptyChord)
        }

        assert_eq!(returns_ok()?, 42);
        assert!(returns_err().is_err());
        Ok(())
    }
}

// endregion: --- Tests
