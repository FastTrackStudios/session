//! ChordPro 6.07 AST.
//!
//! See the [crate docs](crate) for usage.

use std::fmt;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Byte range in the original source document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Span {
    pub start: usize,
    pub len: usize,
}

impl Span {
    pub const fn new(start: usize, len: usize) -> Self {
        Self { start, len }
    }
    pub const fn end(&self) -> usize {
        self.start + self.len
    }
    pub const fn empty(at: usize) -> Self {
        Self { start: at, len: 0 }
    }
}

/// Top-level parsed ChordPro document.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Document {
    /// Lines, in source order. Includes directives, lyrics, comments, and
    /// blanks. Section grouping is available via [`Document::sections`].
    pub lines: Vec<Line>,
}

impl Document {
    pub fn new() -> Self {
        Self { lines: Vec::new() }
    }

    /// Iterate over every directive in the document, in source order.
    pub fn directives(&self) -> impl Iterator<Item = &Directive> {
        self.lines.iter().filter_map(|l| match l {
            Line::Directive(d) => Some(d),
            _ => None,
        })
    }

    /// Find the first directive matching `name` (case-insensitive). Use the
    /// canonical (long) directive name; aliases are normalized at parse time.
    pub fn find_directive(&self, name: &str) -> Option<&Directive> {
        self.directives()
            .find(|d| d.name().eq_ignore_ascii_case(name))
    }

    /// Convenience: title from the `{title:}` directive.
    pub fn title(&self) -> Option<&str> {
        self.find_directive("title").map(|d| d.value())
    }
    pub fn artist(&self) -> Option<&str> {
        self.find_directive("artist").map(|d| d.value())
    }
    pub fn key(&self) -> Option<&str> {
        self.find_directive("key").map(|d| d.value())
    }
    pub fn capo(&self) -> Option<&str> {
        self.find_directive("capo").map(|d| d.value())
    }

    /// Group lines into sections delimited by `{start_of_*}` /
    /// `{end_of_*}`. Lines not inside any explicit environment are placed in
    /// an implicit lead-in section (label = `None`, environment = `None`).
    pub fn sections(&self) -> Vec<Section<'_>> {
        let mut out: Vec<Section<'_>> = Vec::new();
        let mut current = Section {
            environment: None,
            label: None,
            lines: Vec::new(),
        };
        for line in &self.lines {
            match line {
                Line::Directive(d) => match &d.kind {
                    DirectiveKind::StartOfEnvironment { env, label } => {
                        if !current.lines.is_empty() || current.environment.is_some() {
                            out.push(std::mem::replace(
                                &mut current,
                                Section {
                                    environment: None,
                                    label: None,
                                    lines: Vec::new(),
                                },
                            ));
                        }
                        current.environment = Some(*env);
                        current.label = label.clone();
                    }
                    DirectiveKind::EndOfEnvironment { .. } => {
                        out.push(std::mem::replace(
                            &mut current,
                            Section {
                                environment: None,
                                label: None,
                                lines: Vec::new(),
                            },
                        ));
                    }
                    _ => current.lines.push(line),
                },
                _ => current.lines.push(line),
            }
        }
        if !current.lines.is_empty() || current.environment.is_some() {
            out.push(current);
        }
        out
    }
}

/// A logical section of a document, delimited by `{start_of_*}` /
/// `{end_of_*}` or implicit (no environment).
#[derive(Debug, Clone)]
pub struct Section<'a> {
    pub environment: Option<Environment>,
    pub label: Option<String>,
    pub lines: Vec<&'a Line>,
}

/// One source line.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Line {
    Directive(Directive),
    /// `[C]Twinkle, [F]little [C]star`
    Lyric {
        chunks: Vec<ChordChunk>,
        span: Span,
    },
    /// `# this comment is ignored`
    HashComment {
        text: String,
        span: Span,
    },
    /// Blank or whitespace-only line.
    Empty {
        span: Span,
    },
}

impl Line {
    pub fn span(&self) -> Span {
        match self {
            Line::Directive(d) => d.span,
            Line::Lyric { span, .. } | Line::HashComment { span, .. } | Line::Empty { span } => {
                *span
            }
        }
    }
}

/// One contiguous run of `[chord]` / `[*annotation]` markers and the text
/// that follows them.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ChordChunk {
    /// Optional chord symbol (e.g. `C`, `Gmaj7`, `Am/G`).
    pub chord: Option<String>,
    /// Optional inline annotation (`[*pp]`, `[*ritardando]`).
    pub annotation: Option<Annotation>,
    /// Lyric text following the chord/annotation. May be empty (e.g. two
    /// adjacent chords with no lyric in between).
    pub text: String,
    /// Source byte span covering both the marker and the text.
    pub span: Span,
}

/// An inline annotation (`[*…]`).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Annotation {
    pub text: String,
    pub span: Span,
}

/// A ChordPro environment (the X in `{start_of_X}` / `{end_of_X}`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Environment {
    Verse,
    Chorus,
    Bridge,
    Tab,
    Grid,
    /// Generic `{start_of_section}`. Label disambiguates.
    Section,
}

impl Environment {
    /// Map a directive base name to the environment it begins/ends.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "verse" | "sov" | "eov" => Self::Verse,
            "chorus" | "soc" | "eoc" => Self::Chorus,
            "bridge" | "sob" | "eob" => Self::Bridge,
            "tab" | "sot" | "eot" => Self::Tab,
            "grid" | "sog" | "eog" => Self::Grid,
            "section" => Self::Section,
            _ => return None,
        })
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Verse => "verse",
            Self::Chorus => "chorus",
            Self::Bridge => "bridge",
            Self::Tab => "tab",
            Self::Grid => "grid",
            Self::Section => "section",
        }
    }
}

/// One ChordPro directive.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Directive {
    /// Strongly-typed directive variant.
    pub kind: DirectiveKind,
    /// Conditional selector (`{title-en: …}` → `Some("en")`).
    pub condition: Option<String>,
    /// Source byte span of the entire `{…}` directive, including braces.
    pub span: Span,
}

impl Directive {
    /// Canonical (long) name of this directive.
    pub fn name(&self) -> &str {
        self.kind.canonical_name()
    }
    /// Primary stringified value. For directives without a meaningful single
    /// value (e.g. `Define`) this returns an empty string; use the typed
    /// variant for richer access.
    pub fn value(&self) -> &str {
        self.kind.value()
    }
}

/// Strongly-typed directive variants.
///
/// The cheat sheet's full set is covered. Aliases (`t` → `title`, `eoc` →
/// `end_of_chorus`, …) are normalized at parse time. `meta foo`-style
/// sub-directives are folded into [`DirectiveKind::Meta`].
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum DirectiveKind {
    // ---- text / metadata ----
    Title(String),
    Subtitle(String),
    Comment(String),
    CommentBox(String),
    CommentItalic(String),
    /// `{highlight: …}` (alias of comment).
    Highlight(String),
    /// `{meta: <item> <value>}` — generic catch-all for typed metadata.
    Meta(MetaItem),

    // ---- environments ----
    /// `{start_of_chorus}`, `{start_of_verse: My Tag}`, etc.
    StartOfEnvironment {
        env: Environment,
        label: Option<String>,
    },
    /// `{end_of_chorus}`, `{end_of_section}`, etc.
    EndOfEnvironment {
        env: Environment,
    },
    /// `{chorus: optional-label}` — recall the most recent chorus.
    ChorusRecall {
        label: Option<String>,
    },

    // ---- pagination ----
    NewPage,
    NewPhysicalPage,
    NewSong {
        toc: Option<bool>,
    },
    ColumnBreak,
    Columns(u32),
    PageType(String),

    // ---- formatting / styling (color/font/size; we keep raw strings) ----
    /// One of: titles flush direction, `{title_*}`, `{subtitle_*}`,
    /// `{tab*}`, `{text*}`, `{toc*}`, `{chord*}`, `{footer*}`, `{label*}`,
    /// `{chorus_*}`, etc. Keeps the raw `(name, value)` to avoid a 60-line
    /// enum of every styling sub-directive.
    Style {
        name: String,
        value: String,
    },
    TitlesFlush(String),

    // ---- chord definition / display ----
    /// `{chord …}` inline display; `{define …}` adds to library.
    Define {
        def: ChordDefinition,
        is_define: bool,
    },
    /// `{diagrams: on|off|grid}` and obsolete `{grid}` / `{no_grid}`.
    Diagrams(String),

    // ---- transposition ----
    Transpose(i32),

    // ---- images ----
    /// `{image src=… width=… …}` — kept as a free-form key/value list.
    Image(Vec<(String, String)>),

    // ---- custom & unknown ----
    /// Custom user directive (`{x_lyrics_only: 1}` etc.) plus any unknown
    /// directive name. Keeps the raw string so consumers can opt in.
    Custom {
        name: String,
        value: Option<String>,
    },
}

impl DirectiveKind {
    /// Stable canonical (long) name.
    pub fn canonical_name(&self) -> &str {
        match self {
            Self::Title(_) => "title",
            Self::Subtitle(_) => "subtitle",
            Self::Comment(_) => "comment",
            Self::CommentBox(_) => "comment_box",
            Self::CommentItalic(_) => "comment_italic",
            Self::Highlight(_) => "highlight",
            // For meta directives we expose the *item* as the canonical name
            // so `find_directive("artist")` resolves `{artist: …}` correctly.
            Self::Meta(m) => m.item.as_str(),
            Self::StartOfEnvironment { env, .. } => match env {
                Environment::Verse => "start_of_verse",
                Environment::Chorus => "start_of_chorus",
                Environment::Bridge => "start_of_bridge",
                Environment::Tab => "start_of_tab",
                Environment::Grid => "start_of_grid",
                Environment::Section => "start_of_section",
            },
            Self::EndOfEnvironment { env } => match env {
                Environment::Verse => "end_of_verse",
                Environment::Chorus => "end_of_chorus",
                Environment::Bridge => "end_of_bridge",
                Environment::Tab => "end_of_tab",
                Environment::Grid => "end_of_grid",
                Environment::Section => "end_of_section",
            },
            Self::ChorusRecall { .. } => "chorus",
            Self::NewPage => "new_page",
            Self::NewPhysicalPage => "new_physical_page",
            Self::NewSong { .. } => "new_song",
            Self::ColumnBreak => "column_break",
            Self::Columns(_) => "columns",
            Self::PageType(_) => "pagetype",
            Self::Style { name, .. } => name.as_str(),
            Self::TitlesFlush(_) => "titles",
            Self::Define { is_define, .. } => {
                if *is_define {
                    "define"
                } else {
                    "chord"
                }
            }
            Self::Diagrams(_) => "diagrams",
            Self::Transpose(_) => "transpose",
            Self::Image(_) => "image",
            Self::Custom { name, .. } => name.as_str(),
        }
    }

    fn value(&self) -> &str {
        match self {
            Self::Title(s)
            | Self::Subtitle(s)
            | Self::Comment(s)
            | Self::CommentBox(s)
            | Self::CommentItalic(s)
            | Self::Highlight(s)
            | Self::PageType(s)
            | Self::TitlesFlush(s)
            | Self::Diagrams(s) => s.as_str(),
            Self::Style { value, .. } => value.as_str(),
            Self::Custom { value, .. } => value.as_deref().unwrap_or(""),
            Self::Meta(m) => &m.value,
            _ => "",
        }
    }
}

/// `{meta: <item> <value>}` and aliased `{album: …}` / `{tempo: …}` /
/// `{time: …}` etc.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MetaItem {
    pub item: String,
    pub value: String,
}

/// Body of `{define name …}` / `{chord name …}`.
///
/// Kept as a structured set of optional fields rather than a sub-AST so the
/// long tail of fingering/diagram options can be added incrementally.
#[derive(Debug, Clone, PartialEq, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ChordDefinition {
    pub name: String,
    pub base_fret: Option<i32>,
    pub frets: Option<Vec<String>>,
    pub fingers: Option<Vec<String>>,
    pub keys: Option<Vec<String>>,
    pub display: Option<String>,
    pub diagram: Option<String>,
    pub format: Option<String>,
    /// Anything we don't recognize — preserved for round-trip.
    pub extra: Vec<(String, String)>,
}

// ---------------- Errors ----------------

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub message: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ParseErrorKind {
    UnclosedBrace,
    UnclosedBracket,
    InvalidDirective,
    InvalidEscape,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}
impl std::error::Error for ParseError {}

impl fmt::Display for ParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnclosedBrace => "unclosed brace",
            Self::UnclosedBracket => "unclosed bracket",
            Self::InvalidDirective => "invalid directive",
            Self::InvalidEscape => "invalid escape",
        })
    }
}
