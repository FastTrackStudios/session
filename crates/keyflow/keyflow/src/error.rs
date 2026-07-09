//! Error types for the keyflow crate.

use derive_more::{Display, From};
use keyflow_proto::ParseError;

// region:    --- Result Alias

pub type Result<T> = core::result::Result<T, Error>;

// endregion: --- Result Alias

// region:    --- Error Types

#[derive(Debug, Display, From)]
#[display("{self:?}")]
pub enum Error {
    #[from(String, &String, &str)]
    Custom(String),

    // -- Parsing
    #[from]
    ChordParse(keyflow_proto::chord::ChordParseError),

    #[from]
    ChordParseMultiple(keyflow_proto::chord::ChordParseErrors),

    #[from]
    Parse(ParseError),

    // -- Externals
    #[from]
    Io(std::io::Error),
}

// endregion: --- Error Types

// region:    --- Custom

impl Error {
    pub fn custom(val: impl Into<String>) -> Self {
        Self::Custom(val.into())
    }
}

// endregion: --- Custom

// region:    --- Error Boilerplate

impl std::error::Error for Error {}

// endregion: --- Error Boilerplate
