//! Keyflow - Musical Chart Parser
//!
//! A trait-based system for parsing and manipulating musical charts.
//!
//! All chart types and parsing functionality is re-exported from `keyflow-proto`.

// Re-export all types from keyflow-proto for convenience
#[cfg(feature = "svg")]
pub use engraver;
#[cfg(feature = "live")]
pub use keyflow_live as live;
#[cfg(feature = "midi")]
pub use keyflow_midi as midi;
pub use keyflow_proto::*;
#[cfg(feature = "text")]
pub use keyflow_text as text;

pub mod daw_module;

#[derive(Debug, Clone)]
pub enum KeyflowSourceError {
    Text(String),
    Midi(String),
}

impl std::fmt::Display for KeyflowSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text(err) => write!(f, "text parse error: {err}"),
            Self::Midi(err) => write!(f, "midi parse error: {err}"),
        }
    }
}

impl std::error::Error for KeyflowSourceError {}

pub trait IntoChart {
    fn into_chart(self) -> std::result::Result<keyflow_proto::Chart, KeyflowSourceError>;
}

impl IntoChart for keyflow_proto::Chart {
    fn into_chart(self) -> std::result::Result<keyflow_proto::Chart, KeyflowSourceError> {
        Ok(self)
    }
}

#[cfg(feature = "text")]
impl IntoChart for &str {
    fn into_chart(self) -> std::result::Result<keyflow_proto::Chart, KeyflowSourceError> {
        parse_text_chart(self)
    }
}

#[cfg(feature = "text")]
impl IntoChart for String {
    fn into_chart(self) -> std::result::Result<keyflow_proto::Chart, KeyflowSourceError> {
        parse_text_chart(&self)
    }
}

#[cfg(feature = "text")]
impl IntoChart for &String {
    fn into_chart(self) -> std::result::Result<keyflow_proto::Chart, KeyflowSourceError> {
        parse_text_chart(self.as_str())
    }
}

/// Parse Keyflow source text into a `Chart`, honoring the document format.
///
/// Routes through `parse_document` so a file using `--- keyflow ---` /
/// `--- chordpro ---` delimiters has its blocks split correctly: only the
/// keyflow block becomes sections, and each ChordPro block is merged in as a
/// lyric layer. A plain file with no delimiters is parsed as a single keyflow
/// block. Calling the lower-level `parse_chart` here would feed ChordPro lyric
/// lines (e.g. `Verse 1:`) to the section parser and fail.
#[cfg(feature = "text")]
fn parse_text_chart(source: &str) -> std::result::Result<keyflow_proto::Chart, KeyflowSourceError> {
    keyflow_text::chart::parse_document(source)
        .map(|(chart, _doc)| chart)
        .map_err(KeyflowSourceError::Text)
}

#[cfg(feature = "midi")]
impl IntoChart for &[u8] {
    fn into_chart(self) -> std::result::Result<keyflow_proto::Chart, KeyflowSourceError> {
        keyflow_midi::parse_midi_bytes(self).map_err(KeyflowSourceError::Midi)
    }
}

#[cfg(feature = "midi")]
impl IntoChart for Vec<u8> {
    fn into_chart(self) -> std::result::Result<keyflow_proto::Chart, KeyflowSourceError> {
        keyflow_midi::parse_midi_bytes(&self).map_err(KeyflowSourceError::Midi)
    }
}

#[cfg(feature = "midi")]
impl IntoChart for &std::path::Path {
    fn into_chart(self) -> std::result::Result<keyflow_proto::Chart, KeyflowSourceError> {
        keyflow_midi::parse_midi_path(self).map_err(KeyflowSourceError::Midi)
    }
}

#[cfg(feature = "midi")]
impl IntoChart for std::path::PathBuf {
    fn into_chart(self) -> std::result::Result<keyflow_proto::Chart, KeyflowSourceError> {
        keyflow_midi::parse_midi_path(self.as_path()).map_err(KeyflowSourceError::Midi)
    }
}

#[cfg(feature = "text")]
pub trait KeyflowParseExt {
    fn keyflow_parse(&self) -> std::result::Result<keyflow_proto::Chart, KeyflowSourceError>;
}

#[cfg(feature = "text")]
impl KeyflowParseExt for str {
    fn keyflow_parse(&self) -> std::result::Result<keyflow_proto::Chart, KeyflowSourceError> {
        keyflow_text::chart::parse_chart(self).map_err(KeyflowSourceError::Text)
    }
}

pub fn parse<T: IntoChart>(
    source: T,
) -> std::result::Result<keyflow_proto::Chart, KeyflowSourceError> {
    source.into_chart()
}

/// Parse a .kf document (potentially multi-block) and return the Chart from the keyflow block.
#[cfg(feature = "text")]
pub fn parse_document(
    input: &str,
) -> std::result::Result<(keyflow_proto::Chart, keyflow_proto::document::KfDocument), String> {
    keyflow_text::chart::parse_document(input)
}

// Local modules
mod error;
pub use error::{Error, Result};

pub mod patterns;

#[cfg(all(test, feature = "text"))]
mod parse_routing_tests {
    use super::parse;

    /// `parse` must split `--- keyflow ---` / `--- chordpro ---` documents so
    /// ChordPro lyric lines (e.g. `Verse 1:`) are not fed to the section
    /// parser. Regression: it previously routed through the single-block
    /// `parse_chart`, so a document's ChordPro block was parsed as keyflow and
    /// failed with a section-length error.
    #[test]
    fn parse_routes_document_blocks() {
        let input = "\
--- keyflow ---
Doc
120bpm 4/4 #G

VS 1
| G C |

--- chordpro ---
Verse 1:
[G]Worthy of every [C]song
";
        let chart = parse(input).expect("document with chordpro block should parse");
        assert_eq!(
            chart.sections.len(),
            1,
            "only the keyflow block makes sections"
        );
    }
}
