//! Clean facade API for common Keyflow text parsing use cases.

/// Commonly used types for day-to-day chart work.
pub mod prelude {
    pub use keyflow_proto::chart::{Chart, ChartPosition, ChartSection, Measure};
    pub use keyflow_proto::chord::{Chord, ChordParseError, ChordRhythm, PushPullAmount};
    pub use keyflow_proto::key::Key;
    pub use keyflow_proto::metadata::SongMetadata;
    pub use keyflow_proto::sections::{Section, SectionType};
    pub use keyflow_proto::time::{MusicalDuration, MusicalPosition, Tempo, TimeSignature};
    pub use keyflow_proto::{DynamicMarking, TextCue};
    pub use keyflow_syntax::parsing::ParseError;
}

/// Parse helpers that accept raw strings and return domain objects.
pub mod parse {
    use keyflow_proto::chart::{Chart, Melody};
    use keyflow_proto::chord::Chord;
    use keyflow_proto::sections::SectionType;
    use keyflow_proto::{DynamicMarking, Key, TextCue};
    use keyflow_syntax::parsing::{Lexer, TokenType};

    use crate::chart::parse_chart;

    /// Parse full chart text into a `Chart`.
    pub fn chart(text: &str) -> Result<Chart, String> {
        parse_chart(text)
    }

    /// Parse a single chord token into a `Chord`.
    pub fn chord(text: &str) -> Result<Chord, keyflow_syntax::parsing::ParseError> {
        let mut lexer = Lexer::new(text.to_string());
        let mut tokens = lexer.tokenize();
        if matches!(tokens.last().map(|t| &t.token_type), Some(TokenType::Eof)) {
            tokens.pop();
        }
        Chord::parse(&tokens)
    }

    /// Parse a section type (e.g. "VS", "CH", "Bridge").
    pub fn section_type(text: &str) -> Result<SectionType, String> {
        SectionType::parse(text)
    }

    /// Parse a key signature (e.g. "C", "F#").
    pub fn key(text: &str) -> Result<Key, String> {
        Key::parse(text)
    }

    /// Parse a melody line into a `Melody`.
    pub fn melody(text: &str) -> Result<Melody, String> {
        Melody::parse(text)
    }

    /// Parse a text cue (e.g. `@keys "synth here"`).
    pub fn text_cue(text: &str) -> Result<TextCue, String> {
        TextCue::parse(text)
    }

    /// Parse a dynamic marking (e.g. `<Build>` or `<Hit>:3`).
    pub fn dynamic_marking(text: &str) -> Result<DynamicMarking, String> {
        DynamicMarking::parse(text)
    }
}
