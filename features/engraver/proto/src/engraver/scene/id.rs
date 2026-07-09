//! Semantic identification for scene graph nodes.
//!
//! Provides a hierarchical identification system that maps scene graph nodes
//! back to their source music elements. This enables:
//! - SVG export with semantic `data-*` attributes
//! - Hit testing that returns meaningful music element references
//! - Round-trip editing (edit SVG → update model)

use serde::{Deserialize, Serialize};
use std::fmt;

/// Type of music element in the scene graph.
///
/// Used for semantic identification and SVG `data-type` attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ElementType {
    // Container elements
    /// A page of the score
    Page,
    /// A system (horizontal line of music)
    System,
    /// A measure (bar)
    Measure,
    /// A segment (vertical slice at a tick position)
    Segment,
    /// A staff
    Staff,

    // Note-related elements
    /// A chord (group of simultaneous notes)
    Chord,
    /// A single note
    Note,
    /// A rest
    Rest,
    /// A stem
    Stem,
    /// A beam connecting notes
    Beam,
    /// A flag on a stem
    Flag,
    /// An accidental (sharp, flat, natural)
    Accidental,
    /// A notehead
    NoteHead,
    /// A ledger line
    LedgerLine,
    /// A dot (augmentation)
    Dot,

    // Staff elements
    /// Staff lines
    StaffLines,
    /// A clef
    Clef,
    /// A key signature
    KeySignature,
    /// A time signature
    TimeSignature,
    /// A barline
    Barline,

    // Chord symbols and text
    /// A chord symbol (harmony)
    ChordSymbol,
    /// A rehearsal mark
    RehearsalMark,
    /// A measure number
    MeasureNumber,
    /// Generic text
    Text,
    /// Lyrics
    Lyrics,
    /// Tempo marking
    Tempo,

    // Expressions and articulations
    /// A dynamic marking (p, f, etc.)
    Dynamic,
    /// An articulation (staccato, accent, etc.)
    Articulation,
    /// A fermata (pause/hold sign)
    Fermata,
    /// A slur
    Slur,
    /// A tie
    Tie,
    /// A hairpin (crescendo/decrescendo)
    Hairpin,

    // Other
    /// A tuplet bracket and number
    Tuplet,
    /// A rhythm slash
    RhythmSlash,
    /// A fret diagram
    FretDiagram,
    /// Custom/unknown element
    Custom,
}

impl ElementType {
    /// Get the SVG `data-type` attribute value for this element type.
    #[must_use]
    pub const fn svg_type_name(&self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::System => "system",
            Self::Measure => "measure",
            Self::Segment => "segment",
            Self::Staff => "staff",
            Self::Chord => "chord",
            Self::Note => "note",
            Self::Rest => "rest",
            Self::Stem => "stem",
            Self::Beam => "beam",
            Self::Flag => "flag",
            Self::Accidental => "accidental",
            Self::NoteHead => "notehead",
            Self::LedgerLine => "ledger-line",
            Self::Dot => "dot",
            Self::StaffLines => "staff-lines",
            Self::Clef => "clef",
            Self::KeySignature => "key-signature",
            Self::TimeSignature => "time-signature",
            Self::Barline => "barline",
            Self::ChordSymbol => "chord-symbol",
            Self::RehearsalMark => "rehearsal-mark",
            Self::MeasureNumber => "measure-number",
            Self::Text => "text",
            Self::Lyrics => "lyrics",
            Self::Tempo => "tempo",
            Self::Dynamic => "dynamic",
            Self::Articulation => "articulation",
            Self::Fermata => "fermata",
            Self::Slur => "slur",
            Self::Tie => "tie",
            Self::Hairpin => "hairpin",
            Self::Tuplet => "tuplet",
            Self::RhythmSlash => "rhythm-slash",
            Self::FretDiagram => "fret-diagram",
            Self::Custom => "custom",
        }
    }
}

impl fmt::Display for ElementType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.svg_type_name())
    }
}

/// Semantic identifier linking scene graph nodes to source music elements.
///
/// Provides hierarchical identification with parent references for nested elements.
/// Generates SVG `data-*` attributes for editable vector export.
///
/// # Example
///
/// ```ignore
/// let note_id = SemanticId::new(ElementType::Note, 42)
///     .with_parent(SemanticId::new(ElementType::Chord, 10))
///     .with_attribute("pitch", "C4")
///     .with_attribute("duration", "quarter");
///
/// // Generates SVG attributes:
/// // data-type="note" data-id="42" data-pitch="C4" data-duration="quarter"
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SemanticId {
    /// Type of element
    pub element_type: ElementType,
    /// Unique identifier within this type
    pub id: u64,
    /// Optional parent reference (e.g., note's parent chord)
    pub parent: Option<Box<SemanticId>>,
    /// Additional attributes for SVG export (e.g., pitch="C4", measure="1")
    attributes: Vec<(String, String)>,
}

impl SemanticId {
    /// Create a new semantic ID.
    #[must_use]
    pub const fn new(element_type: ElementType, id: u64) -> Self {
        Self {
            element_type,
            id,
            parent: None,
            attributes: Vec::new(),
        }
    }

    /// Add a parent reference.
    #[must_use]
    pub fn with_parent(mut self, parent: SemanticId) -> Self {
        self.parent = Some(Box::new(parent));
        self
    }

    /// Add a custom attribute for SVG export.
    #[must_use]
    pub fn with_attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.push((key.into(), value.into()));
        self
    }

    /// Add multiple attributes.
    #[must_use]
    pub fn with_attributes(
        mut self,
        attrs: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        for (k, v) in attrs {
            self.attributes.push((k.into(), v.into()));
        }
        self
    }

    /// Generate SVG `data-*` attributes for this element.
    ///
    /// Returns a list of (attribute_name, value) pairs suitable for SVG export.
    #[must_use]
    pub fn to_svg_attributes(&self) -> Vec<(String, String)> {
        let mut attrs = Vec::with_capacity(2 + self.attributes.len());

        // Core attributes
        attrs.push(("data-type".to_string(), self.element_type.to_string()));
        attrs.push(("data-id".to_string(), self.id.to_string()));

        // Custom attributes
        for (key, value) in &self.attributes {
            attrs.push((format!("data-{key}"), value.clone()));
        }

        attrs
    }

    /// Generate an SVG `id` attribute value.
    ///
    /// Format: `{type}-{id}` (e.g., "note-42", "chord-10")
    #[must_use]
    pub fn svg_id(&self) -> String {
        format!("{}-{}", self.element_type.svg_type_name(), self.id)
    }
}

impl fmt::Display for SemanticId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.element_type, self.id)
    }
}

// Builder methods for common element types
impl SemanticId {
    /// Create a page ID.
    #[must_use]
    pub fn page(page_num: u64) -> Self {
        Self::new(ElementType::Page, page_num).with_attribute("page", page_num.to_string())
    }

    /// Create a system ID.
    #[must_use]
    pub fn system(system_num: u64) -> Self {
        Self::new(ElementType::System, system_num).with_attribute("system", system_num.to_string())
    }

    /// Create a measure ID.
    #[must_use]
    pub fn measure(measure_num: u64) -> Self {
        Self::new(ElementType::Measure, measure_num)
            .with_attribute("measure", measure_num.to_string())
    }

    /// Create a segment ID with tick position.
    #[must_use]
    pub fn segment(id: u64, tick: i32) -> Self {
        Self::new(ElementType::Segment, id).with_attribute("tick", tick.to_string())
    }

    /// Create a chord ID.
    #[must_use]
    pub fn chord(id: u64) -> Self {
        Self::new(ElementType::Chord, id)
    }

    /// Create a note ID with pitch information.
    #[must_use]
    pub fn note(id: u64, pitch: impl Into<String>) -> Self {
        Self::new(ElementType::Note, id).with_attribute("pitch", pitch)
    }

    /// Create a rest ID.
    #[must_use]
    pub fn rest(id: u64) -> Self {
        Self::new(ElementType::Rest, id)
    }

    /// Create a chord symbol ID with harmony text.
    #[must_use]
    pub fn chord_symbol(id: u64, harmony: impl Into<String>) -> Self {
        Self::new(ElementType::ChordSymbol, id).with_attribute("harmony", harmony)
    }

    /// Create a rehearsal mark ID.
    #[must_use]
    pub fn rehearsal_mark(id: u64, text: impl Into<String>) -> Self {
        Self::new(ElementType::RehearsalMark, id).with_attribute("text", text)
    }

    /// Create a rhythm slash ID.
    #[must_use]
    pub fn rhythm_slash(id: u64, beat: u8) -> Self {
        Self::new(ElementType::RhythmSlash, id).with_attribute("beat", beat.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_element_type_svg_name() {
        assert_eq!(ElementType::Note.svg_type_name(), "note");
        assert_eq!(ElementType::ChordSymbol.svg_type_name(), "chord-symbol");
        assert_eq!(ElementType::TimeSignature.svg_type_name(), "time-signature");
    }

    #[test]
    fn test_semantic_id_basic() {
        let id = SemanticId::new(ElementType::Note, 42);
        assert_eq!(id.element_type, ElementType::Note);
        assert_eq!(id.id, 42);
        assert!(id.parent.is_none());
    }

    #[test]
    fn test_semantic_id_with_parent() {
        let chord_id = SemanticId::chord(10);
        let note_id = SemanticId::note(42, "C4").with_parent(chord_id);

        assert!(note_id.parent.is_some());
        let parent = note_id.parent.as_ref().unwrap();
        assert_eq!(parent.element_type, ElementType::Chord);
        assert_eq!(parent.id, 10);
    }

    #[test]
    fn test_svg_attributes() {
        let id = SemanticId::note(42, "C4").with_attribute("duration", "quarter");

        let attrs = id.to_svg_attributes();
        assert!(attrs.contains(&("data-type".to_string(), "note".to_string())));
        assert!(attrs.contains(&("data-id".to_string(), "42".to_string())));
        assert!(attrs.contains(&("data-pitch".to_string(), "C4".to_string())));
        assert!(attrs.contains(&("data-duration".to_string(), "quarter".to_string())));
    }

    #[test]
    fn test_svg_id() {
        let id = SemanticId::chord(123);
        assert_eq!(id.svg_id(), "chord-123");
    }

    #[test]
    fn test_builder_methods() {
        let page = SemanticId::page(1);
        let attrs = page.to_svg_attributes();
        assert!(attrs.contains(&("data-page".to_string(), "1".to_string())));

        let measure = SemanticId::measure(4);
        let attrs = measure.to_svg_attributes();
        assert!(attrs.contains(&("data-measure".to_string(), "4".to_string())));

        let segment = SemanticId::segment(100, 480);
        let attrs = segment.to_svg_attributes();
        assert!(attrs.contains(&("data-tick".to_string(), "480".to_string())));
    }

    #[test]
    fn test_display() {
        let id = SemanticId::note(42, "C4");
        assert_eq!(format!("{id}"), "note:42");
    }
}
