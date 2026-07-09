//! Scene graph node for hierarchical rendering.
//!
//! The scene graph represents the visual structure of a music score,
//! with nodes for pages, systems, measures, and individual elements.
//! Each node can have paint commands and child nodes.

use kurbo::{Affine, Point, Rect, Shape};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::HashMap;

use super::id::SemanticId;
use super::paint::PaintCommand;

/// Metadata key constants for common element attributes.
pub mod metadata_keys {
    /// Source text span (JSON-serialized TextSpan)
    pub const SOURCE_SPAN: &str = "source_span";
    /// Chart position (JSON-serialized ChartPosition)
    pub const CHART_POSITION: &str = "chart_position";
    /// Semantic roles (JSON-serialized Vec<SemanticRole>)
    pub const SEMANTIC_ROLES: &str = "semantic_roles";
    /// Source link (JSON-serialized SourceLink)
    pub const SOURCE_LINK: &str = "source_link";
    /// Page number
    pub const PAGE: &str = "page";
    /// System index
    pub const SYSTEM: &str = "system";
    /// Measure index
    pub const MEASURE: &str = "measure";
    /// Beat index
    pub const BEAT: &str = "beat";
    /// Voice number
    pub const VOICE: &str = "voice";
    /// Section type
    pub const SECTION_TYPE: &str = "section_type";
    /// Section number
    pub const SECTION_NUMBER: &str = "section_number";
    /// Element type (chord, barline, clef, etc.)
    pub const ELEMENT_TYPE: &str = "element_type";
    /// Glyph information (JSON-serialized GlyphInfo)
    pub const GLYPH_INFO: &str = "glyph_info";
    /// Font family name
    pub const FONT_FAMILY: &str = "font_family";
}

/// Type of glyph being rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GlyphType {
    /// SMuFL music font glyph (noteheads, clefs, accidentals, etc.)
    Smufl,
    /// Regular text character(s)
    Text,
    /// Unicode symbol (arrows, special characters)
    Symbol,
    /// Custom/unknown glyph type
    Custom,
}

impl std::fmt::Display for GlyphType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GlyphType::Smufl => write!(f, "smufl"),
            GlyphType::Text => write!(f, "text"),
            GlyphType::Symbol => write!(f, "symbol"),
            GlyphType::Custom => write!(f, "custom"),
        }
    }
}

/// Information about a rendered glyph.
///
/// Tracks what glyph is being rendered and from which font,
/// useful for debugging, accessibility, and font management.
///
/// # Example
///
/// ```ignore
/// // SMuFL quarter note
/// let glyph_info = GlyphInfo::smufl('\u{E0A4}', "Bravura")
///     .with_smufl_name("noteQuarterUp");
///
/// // Text chord symbol
/// let text_info = GlyphInfo::text("Gmaj7", "Arial");
///
/// // Store in node metadata
/// node.set_glyph_info(&glyph_info);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlyphInfo {
    /// Type of glyph (SMuFL, text, symbol, etc.)
    pub glyph_type: GlyphType,

    /// The rendered content.
    /// For SMuFL: single character codepoint (e.g., '\u{E0A4}')
    /// For text: the full string (e.g., "Gmaj7")
    pub content: String,

    /// Unicode codepoint (for single-character glyphs)
    /// None for multi-character text
    pub codepoint: Option<u32>,

    /// Font family used to render this glyph
    pub font_family: String,

    /// SMuFL glyph name (e.g., "noteQuarterUp", "gClef")
    /// Only populated for SMuFL glyphs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smufl_name: Option<String>,

    /// SMuFL glyph class (e.g., "noteheads", "clefs", "accidentals")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smufl_class: Option<String>,

    /// Additional description or context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl GlyphInfo {
    /// Create a new SMuFL glyph info.
    ///
    /// # Arguments
    /// * `codepoint` - The SMuFL Unicode codepoint
    /// * `font_family` - The music font name (e.g., "Bravura", "Leland")
    #[must_use]
    pub fn smufl(codepoint: char, font_family: impl Into<String>) -> Self {
        Self {
            glyph_type: GlyphType::Smufl,
            content: codepoint.to_string(),
            codepoint: Some(codepoint as u32),
            font_family: font_family.into(),
            smufl_name: None,
            smufl_class: None,
            description: None,
        }
    }

    /// Create a new text glyph info.
    ///
    /// # Arguments
    /// * `text` - The text content
    /// * `font_family` - The font name (e.g., "Arial", "Times New Roman")
    #[must_use]
    pub fn text(text: impl Into<String>, font_family: impl Into<String>) -> Self {
        let text = text.into();
        let codepoint = if text.chars().count() == 1 {
            text.chars().next().map(|c| c as u32)
        } else {
            None
        };
        Self {
            glyph_type: GlyphType::Text,
            content: text,
            codepoint,
            font_family: font_family.into(),
            smufl_name: None,
            smufl_class: None,
            description: None,
        }
    }

    /// Create a new symbol glyph info.
    ///
    /// # Arguments
    /// * `symbol` - The symbol character
    /// * `font_family` - The font name
    #[must_use]
    pub fn symbol(symbol: char, font_family: impl Into<String>) -> Self {
        Self {
            glyph_type: GlyphType::Symbol,
            content: symbol.to_string(),
            codepoint: Some(symbol as u32),
            font_family: font_family.into(),
            smufl_name: None,
            smufl_class: None,
            description: None,
        }
    }

    /// Set the SMuFL glyph name (e.g., "noteQuarterUp").
    #[must_use]
    pub fn with_smufl_name(mut self, name: impl Into<String>) -> Self {
        self.smufl_name = Some(name.into());
        self
    }

    /// Set the SMuFL glyph class (e.g., "noteheads").
    #[must_use]
    pub fn with_smufl_class(mut self, class: impl Into<String>) -> Self {
        self.smufl_class = Some(class.into());
        self
    }

    /// Set a description for this glyph.
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Check if this is a SMuFL glyph.
    #[must_use]
    pub fn is_smufl(&self) -> bool {
        self.glyph_type == GlyphType::Smufl
    }

    /// Check if this is text.
    #[must_use]
    pub fn is_text(&self) -> bool {
        self.glyph_type == GlyphType::Text
    }

    /// Get the codepoint as a char (if single character).
    #[must_use]
    pub fn as_char(&self) -> Option<char> {
        self.codepoint.and_then(char::from_u32)
    }

    /// Get the codepoint in U+XXXX format.
    #[must_use]
    pub fn codepoint_string(&self) -> Option<String> {
        self.codepoint.map(|cp| format!("U+{cp:04X}"))
    }
}

/// A node in the scene graph.
///
/// Scene nodes form a tree structure where each node can have:
/// - A semantic ID linking to the source music element
/// - A local transform (position, rotation, scale)
/// - Paint commands for rendering
/// - Child nodes
///
/// # Example
///
/// ```ignore
/// // Create a measure group with child chord
/// let mut measure = SceneNode::group(SemanticId::measure(1));
/// measure.add_child(
///     SceneNode::leaf(
///         SemanticId::chord(1),
///         vec![PaintCommand::glyph('\u{E0A4}', Point::ZERO, 1.0, Color::BLACK)],
///     )
///     .with_position(Point::new(50.0, 100.0))
/// );
/// ```
#[derive(Debug, Clone)]
pub struct SceneNode {
    /// Semantic identifier linking to source music element.
    /// None for anonymous grouping nodes.
    pub id: Option<SemanticId>,

    /// Local transform applied before rendering this node and children.
    pub transform: Affine,

    /// Bounding box in local coordinates.
    /// Computed from paint commands and children.
    pub bounds: Rect,

    /// Paint commands for this node (rendered before children).
    pub commands: Vec<PaintCommand>,

    /// Child nodes (rendered after this node's commands).
    pub children: Vec<SceneNode>,

    /// Whether this node is visible.
    /// Invisible nodes and their children are skipped during rendering.
    pub visible: bool,

    /// User-defined metadata (for extensibility).
    /// Can store additional attributes for SVG export.
    pub metadata: HashMap<String, String>,
}

impl Default for SceneNode {
    fn default() -> Self {
        Self {
            id: None,
            transform: Affine::IDENTITY,
            bounds: Rect::ZERO,
            commands: Vec::new(),
            children: Vec::new(),
            visible: true,
            metadata: HashMap::new(),
        }
    }
}

impl SceneNode {
    /// Create a new empty scene node.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a group node (no paint commands, just children).
    #[must_use]
    pub fn group(id: SemanticId) -> Self {
        Self {
            id: Some(id),
            ..Default::default()
        }
    }

    /// Create an anonymous group node (no semantic ID).
    #[must_use]
    pub fn anonymous_group() -> Self {
        Self::default()
    }

    /// Create a leaf node with paint commands.
    #[must_use]
    pub fn leaf(id: SemanticId, commands: Vec<PaintCommand>) -> Self {
        let bounds = compute_commands_bounds(&commands);
        Self {
            id: Some(id),
            commands,
            bounds,
            ..Default::default()
        }
    }

    /// Create an anonymous leaf node (no semantic ID).
    #[must_use]
    pub fn anonymous_leaf(commands: Vec<PaintCommand>) -> Self {
        let bounds = compute_commands_bounds(&commands);
        Self {
            commands,
            bounds,
            ..Default::default()
        }
    }

    /// Set the semantic ID.
    #[must_use]
    pub fn with_id(mut self, id: SemanticId) -> Self {
        self.id = Some(id);
        self
    }

    /// Set the local transform.
    #[must_use]
    pub fn with_transform(mut self, transform: Affine) -> Self {
        self.transform = transform;
        self
    }

    /// Set position (translation only).
    #[must_use]
    pub fn with_position(mut self, position: Point) -> Self {
        self.transform = Affine::translate((position.x, position.y));
        self
    }

    /// Set visibility.
    #[must_use]
    pub fn with_visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    /// Set bounding box explicitly.
    #[must_use]
    pub fn with_bounds(mut self, bounds: Rect) -> Self {
        self.bounds = bounds;
        self
    }

    /// Add metadata.
    #[must_use]
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Add a child node.
    pub fn add_child(&mut self, child: SceneNode) {
        self.children.push(child);
    }

    /// Add multiple children.
    pub fn add_children(&mut self, children: impl IntoIterator<Item = SceneNode>) {
        self.children.extend(children);
    }

    /// Add a paint command.
    pub fn add_command(&mut self, command: PaintCommand) {
        self.commands.push(command);
    }

    /// Add multiple paint commands.
    pub fn add_commands(&mut self, commands: impl IntoIterator<Item = PaintCommand>) {
        self.commands.extend(commands);
    }

    /// Compute the bounds of this node in local coordinates.
    /// Includes the bounds of all children transformed by their local transforms.
    #[must_use]
    pub fn compute_bounds(&self) -> Rect {
        let mut bounds = self.bounds;

        for child in &self.children {
            if !child.visible {
                continue;
            }

            let child_bounds = child.compute_bounds();
            if child_bounds.is_zero_area() {
                continue;
            }

            // Transform child bounds by child's transform
            let transformed = child.transform.transform_rect_bbox(child_bounds);
            bounds = bounds.union(transformed);
        }

        bounds
    }

    /// Compute world-space bounds by applying all ancestor transforms.
    #[must_use]
    pub fn world_bounds(&self, parent_transform: Affine) -> Rect {
        let world_transform = parent_transform * self.transform;
        let local_bounds = self.compute_bounds();
        world_transform.transform_rect_bbox(local_bounds)
    }

    /// Get all nodes matching a predicate (depth-first).
    pub fn find_all(&self, predicate: impl Fn(&SceneNode) -> bool) -> Vec<&SceneNode> {
        let mut results = Vec::new();
        self.find_all_recursive(&predicate, &mut results);
        results
    }

    fn find_all_recursive<'a>(
        &'a self,
        predicate: &impl Fn(&SceneNode) -> bool,
        results: &mut Vec<&'a SceneNode>,
    ) {
        if predicate(self) {
            results.push(self);
        }
        for child in &self.children {
            child.find_all_recursive(predicate, results);
        }
    }

    /// Find the first node matching a predicate (depth-first).
    #[must_use]
    pub fn find_first(&self, predicate: impl Fn(&SceneNode) -> bool) -> Option<&SceneNode> {
        self.find_first_recursive(&predicate)
    }

    fn find_first_recursive(&self, predicate: &impl Fn(&SceneNode) -> bool) -> Option<&SceneNode> {
        if predicate(self) {
            return Some(self);
        }
        for child in &self.children {
            if let Some(found) = child.find_first_recursive(predicate) {
                return Some(found);
            }
        }
        None
    }

    /// Find a node by semantic ID.
    #[must_use]
    pub fn find_by_id(&self, id: &SemanticId) -> Option<&SceneNode> {
        self.find_first(|node| node.id.as_ref() == Some(id))
    }

    /// Hit-test the subtree rooted at this node and return the deepest visible
    /// descendant whose world-space `bounds` contain `point`.
    ///
    /// Walks depth-first; the first leaf-most match wins. Invisible nodes (and
    /// their subtrees) are skipped. Nodes with zero-area `bounds` are skipped
    /// for the containment check but their children are still traversed.
    #[must_use]
    pub fn hit_test(&self, point: Point) -> Option<&SceneNode> {
        self.hit_test_recursive(point, kurbo::Affine::IDENTITY)
    }

    fn hit_test_recursive(
        &self,
        point: Point,
        parent_transform: kurbo::Affine,
    ) -> Option<&SceneNode> {
        if !self.visible {
            return None;
        }
        let world_transform = parent_transform * self.transform;

        // Recurse into children first so deeper hits take precedence over ancestors.
        for child in self.children.iter().rev() {
            if let Some(found) = child.hit_test_recursive(point, world_transform) {
                return Some(found);
            }
        }

        if self.bounds.width() > 0.0 && self.bounds.height() > 0.0 {
            let world_bounds = world_transform.transform_rect_bbox(self.bounds);
            if world_bounds.contains(point) {
                return Some(self);
            }
        }
        None
    }

    /// Count total number of nodes in the subtree.
    #[must_use]
    pub fn node_count(&self) -> usize {
        1 + self
            .children
            .iter()
            .map(SceneNode::node_count)
            .sum::<usize>()
    }

    /// Count total number of paint commands in the subtree.
    #[must_use]
    pub fn command_count(&self) -> usize {
        self.commands.len()
            + self
                .children
                .iter()
                .map(SceneNode::command_count)
                .sum::<usize>()
    }

    /// Check if this node or any descendant has a semantic ID.
    #[must_use]
    pub fn has_semantic_ids(&self) -> bool {
        self.id.is_some() || self.children.iter().any(SceneNode::has_semantic_ids)
    }

    // =========================================================================
    // JSON Metadata Helpers
    // =========================================================================

    /// Store a JSON-serializable value in metadata.
    ///
    /// Use this for complex types like SourceLink, ChartPosition, etc.
    /// Returns the node for chaining.
    ///
    /// # Example
    /// ```ignore
    /// use crate::{TextSpan, ChartPosition};
    /// node.set_json_metadata(metadata_keys::SOURCE_SPAN, &span);
    /// node.set_json_metadata(metadata_keys::CHART_POSITION, &position);
    /// ```
    pub fn set_json_metadata<T: Serialize>(&mut self, key: &str, value: &T) -> &mut Self {
        if let Ok(json) = serde_json::to_string(value) {
            self.metadata.insert(key.to_string(), json);
        }
        self
    }

    /// Set JSON metadata with builder pattern (consumes and returns self).
    #[must_use]
    pub fn with_json_metadata<T: Serialize>(mut self, key: &str, value: &T) -> Self {
        self.set_json_metadata(key, value);
        self
    }

    /// Retrieve a JSON-serializable value from metadata.
    ///
    /// Returns None if the key doesn't exist or deserialization fails.
    ///
    /// # Example
    /// ```ignore
    /// if let Some(span) = node.get_json_metadata::<TextSpan>(metadata_keys::SOURCE_SPAN) {
    ///     println!("Source: line {}, column {}", span.line, span.column);
    /// }
    /// ```
    #[must_use]
    pub fn get_json_metadata<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        self.metadata
            .get(key)
            .and_then(|json| serde_json::from_str(json).ok())
    }

    /// Check if a metadata key exists.
    #[must_use]
    pub fn has_metadata(&self, key: &str) -> bool {
        self.metadata.contains_key(key)
    }

    /// Get raw string metadata value.
    #[must_use]
    pub fn get_metadata(&self, key: &str) -> Option<&str> {
        self.metadata.get(key).map(String::as_str)
    }

    /// Remove a metadata key and return the value if it existed.
    pub fn remove_metadata(&mut self, key: &str) -> Option<String> {
        self.metadata.remove(key)
    }

    // =========================================================================
    // Convenience Metadata Accessors
    // =========================================================================

    /// Set the page number for this node.
    pub fn set_page(&mut self, page: u32) -> &mut Self {
        self.metadata
            .insert(metadata_keys::PAGE.to_string(), page.to_string());
        self
    }

    /// Get the page number for this node.
    #[must_use]
    pub fn get_page(&self) -> Option<u32> {
        self.metadata
            .get(metadata_keys::PAGE)
            .and_then(|s| s.parse().ok())
    }

    /// Set the system index for this node.
    pub fn set_system(&mut self, system: u32) -> &mut Self {
        self.metadata
            .insert(metadata_keys::SYSTEM.to_string(), system.to_string());
        self
    }

    /// Get the system index for this node.
    #[must_use]
    pub fn get_system(&self) -> Option<u32> {
        self.metadata
            .get(metadata_keys::SYSTEM)
            .and_then(|s| s.parse().ok())
    }

    /// Set the measure index for this node.
    pub fn set_measure(&mut self, measure: u32) -> &mut Self {
        self.metadata
            .insert(metadata_keys::MEASURE.to_string(), measure.to_string());
        self
    }

    /// Get the measure index for this node.
    #[must_use]
    pub fn get_measure(&self) -> Option<u32> {
        self.metadata
            .get(metadata_keys::MEASURE)
            .and_then(|s| s.parse().ok())
    }

    /// Set the beat index for this node.
    pub fn set_beat(&mut self, beat: u32) -> &mut Self {
        self.metadata
            .insert(metadata_keys::BEAT.to_string(), beat.to_string());
        self
    }

    /// Get the beat index for this node.
    #[must_use]
    pub fn get_beat(&self) -> Option<u32> {
        self.metadata
            .get(metadata_keys::BEAT)
            .and_then(|s| s.parse().ok())
    }

    /// Set the voice number for this node.
    pub fn set_voice(&mut self, voice: u8) -> &mut Self {
        self.metadata
            .insert(metadata_keys::VOICE.to_string(), voice.to_string());
        self
    }

    /// Get the voice number for this node.
    #[must_use]
    pub fn get_voice(&self) -> Option<u8> {
        self.metadata
            .get(metadata_keys::VOICE)
            .and_then(|s| s.parse().ok())
    }

    /// Set the section type for this node.
    pub fn set_section_type(&mut self, section_type: &str) -> &mut Self {
        self.metadata.insert(
            metadata_keys::SECTION_TYPE.to_string(),
            section_type.to_string(),
        );
        self
    }

    /// Get the section type for this node.
    #[must_use]
    pub fn get_section_type(&self) -> Option<&str> {
        self.metadata
            .get(metadata_keys::SECTION_TYPE)
            .map(String::as_str)
    }

    /// Set the element type for this node.
    pub fn set_element_type(&mut self, element_type: &str) -> &mut Self {
        self.metadata.insert(
            metadata_keys::ELEMENT_TYPE.to_string(),
            element_type.to_string(),
        );
        self
    }

    /// Get the element type for this node.
    #[must_use]
    pub fn get_element_type(&self) -> Option<&str> {
        self.metadata
            .get(metadata_keys::ELEMENT_TYPE)
            .map(String::as_str)
    }

    /// Set the font family for this node.
    pub fn set_font_family(&mut self, font_family: &str) -> &mut Self {
        self.metadata.insert(
            metadata_keys::FONT_FAMILY.to_string(),
            font_family.to_string(),
        );
        self
    }

    /// Get the font family for this node.
    #[must_use]
    pub fn get_font_family(&self) -> Option<&str> {
        self.metadata
            .get(metadata_keys::FONT_FAMILY)
            .map(String::as_str)
    }

    // =========================================================================
    // Glyph Information Accessors
    // =========================================================================

    /// Set glyph information for this node.
    ///
    /// Use this to track what glyph is being rendered and from which font.
    ///
    /// # Example
    /// ```ignore
    /// // SMuFL quarter note from Bravura
    /// let glyph = GlyphInfo::smufl('\u{E0A4}', "Bravura")
    ///     .with_smufl_name("noteQuarterUp")
    ///     .with_smufl_class("noteheads");
    /// node.set_glyph_info(&glyph);
    ///
    /// // Text chord symbol
    /// node.set_glyph_info(&GlyphInfo::text("Gmaj7", "Arial"));
    /// ```
    pub fn set_glyph_info(&mut self, info: &GlyphInfo) -> &mut Self {
        self.set_json_metadata(metadata_keys::GLYPH_INFO, info)
    }

    /// Get glyph information for this node.
    #[must_use]
    pub fn get_glyph_info(&self) -> Option<GlyphInfo> {
        self.get_json_metadata(metadata_keys::GLYPH_INFO)
    }

    /// Set glyph info with builder pattern (consumes and returns self).
    #[must_use]
    pub fn with_glyph_info(mut self, info: &GlyphInfo) -> Self {
        self.set_glyph_info(info);
        self
    }

    /// Convenience: Set SMuFL glyph info directly.
    ///
    /// Also sets the `FONT_FAMILY` metadata for easy querying.
    ///
    /// # Example
    /// ```ignore
    /// node.set_smufl_glyph('\u{E0A4}', "Bravura", Some("noteQuarterUp"));
    /// ```
    pub fn set_smufl_glyph(
        &mut self,
        codepoint: char,
        font_family: &str,
        smufl_name: Option<&str>,
    ) -> &mut Self {
        let mut info = GlyphInfo::smufl(codepoint, font_family);
        if let Some(name) = smufl_name {
            info = info.with_smufl_name(name);
        }
        self.set_font_family(font_family);
        self.set_glyph_info(&info)
    }

    /// Convenience: Set text glyph info directly.
    ///
    /// Also sets the `FONT_FAMILY` metadata for easy querying.
    ///
    /// # Example
    /// ```ignore
    /// node.set_text_glyph("Gmaj7", "Arial");
    /// ```
    pub fn set_text_glyph(&mut self, text: &str, font_family: &str) -> &mut Self {
        self.set_font_family(font_family);
        self.set_glyph_info(&GlyphInfo::text(text, font_family))
    }

    /// Check if this node renders a SMuFL glyph.
    #[must_use]
    pub fn is_smufl_glyph(&self) -> bool {
        self.get_glyph_info().is_some_and(|info| info.is_smufl())
    }

    /// Check if this node renders text.
    #[must_use]
    pub fn is_text_glyph(&self) -> bool {
        self.get_glyph_info().is_some_and(|info| info.is_text())
    }

    // =========================================================================
    // Query Methods for Finding Nodes by Metadata
    // =========================================================================

    /// Find all nodes with a specific page number.
    pub fn find_by_page(&self, page: u32) -> Vec<&SceneNode> {
        let page_str = page.to_string();
        self.find_all(|node| node.metadata.get(metadata_keys::PAGE) == Some(&page_str))
    }

    /// Find all nodes with a specific system index.
    pub fn find_by_system(&self, system: u32) -> Vec<&SceneNode> {
        let system_str = system.to_string();
        self.find_all(|node| node.metadata.get(metadata_keys::SYSTEM) == Some(&system_str))
    }

    /// Find all nodes at a specific measure.
    pub fn find_by_measure(&self, measure: u32) -> Vec<&SceneNode> {
        let measure_str = measure.to_string();
        self.find_all(|node| node.metadata.get(metadata_keys::MEASURE) == Some(&measure_str))
    }

    /// Find all nodes at a specific measure and beat.
    pub fn find_by_position(&self, measure: u32, beat: u32) -> Vec<&SceneNode> {
        let measure_str = measure.to_string();
        let beat_str = beat.to_string();
        self.find_all(|node| {
            node.metadata.get(metadata_keys::MEASURE) == Some(&measure_str)
                && node.metadata.get(metadata_keys::BEAT) == Some(&beat_str)
        })
    }

    /// Find all nodes with a specific element type.
    pub fn find_by_element_type(&self, element_type: &str) -> Vec<&SceneNode> {
        self.find_all(|node| {
            node.metadata
                .get(metadata_keys::ELEMENT_TYPE)
                .is_some_and(|t| t == element_type)
        })
    }

    /// Find all nodes in a specific section type.
    pub fn find_by_section_type(&self, section_type: &str) -> Vec<&SceneNode> {
        self.find_all(|node| {
            node.metadata
                .get(metadata_keys::SECTION_TYPE)
                .is_some_and(|t| t == section_type)
        })
    }

    /// Find all nodes using a specific font family.
    pub fn find_by_font_family(&self, font_family: &str) -> Vec<&SceneNode> {
        self.find_all(|node| {
            node.metadata
                .get(metadata_keys::FONT_FAMILY)
                .is_some_and(|f| f == font_family)
        })
    }

    /// Find all SMuFL glyph nodes.
    pub fn find_smufl_glyphs(&self) -> Vec<&SceneNode> {
        self.find_all(|node| node.is_smufl_glyph())
    }

    /// Find all text glyph nodes.
    pub fn find_text_glyphs(&self) -> Vec<&SceneNode> {
        self.find_all(|node| node.is_text_glyph())
    }

    /// Find all nodes with a specific glyph type.
    pub fn find_by_glyph_type(&self, glyph_type: GlyphType) -> Vec<&SceneNode> {
        self.find_all(|node| {
            node.get_glyph_info()
                .is_some_and(|info| info.glyph_type == glyph_type)
        })
    }

    // =========================================================================
    // ChartIndex Integration (requires keyflow-import feature)
    // =========================================================================

    /// Build a ChartIndex from this scene graph.
    ///
    /// Traverses the scene graph and collects all nodes that have source link
    /// metadata, building an index for bidirectional lookups.
    ///
    /// Each node is assigned a unique ID based on its semantic ID hash or
    /// a generated counter for anonymous nodes.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let scene = render_chart(&chart);
    /// let index = scene.build_chart_index();
    ///
    /// // Find elements at a specific source offset (click handling)
    /// let elements = index.find_by_source_offset(42);
    ///
    /// // Find elements at a specific musical position
    /// let elements = index.find_by_position(4, 2);
    /// ```
    #[cfg(feature = "svg")]
    #[must_use]
    pub fn build_chart_index(&self) -> crate::ChartIndex {
        let mut index = crate::ChartIndex::new();
        let mut counter: u64 = 0;
        self.build_chart_index_recursive(&mut index, &mut counter);
        index
    }

    #[cfg(feature = "svg")]
    fn build_chart_index_recursive(&self, index: &mut crate::ChartIndex, counter: &mut u64) {
        // Try to get source link from metadata
        if let Some(source_link) =
            self.get_json_metadata::<crate::SourceLink>(metadata_keys::SOURCE_LINK)
        {
            // Generate element ID
            let element_id = self.id.as_ref().map_or_else(
                || {
                    *counter += 1;
                    *counter
                },
                |sid| {
                    use std::hash::{Hash, Hasher};
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    sid.hash(&mut hasher);
                    hasher.finish()
                },
            );

            index.add_element(element_id, source_link);
        } else {
            // Check for position-only metadata
            if let Some(position) =
                self.get_json_metadata::<crate::ChartPosition>(metadata_keys::CHART_POSITION)
            {
                let element_id = self.id.as_ref().map_or_else(
                    || {
                        *counter += 1;
                        *counter
                    },
                    |sid| {
                        use std::hash::{Hash, Hasher};
                        let mut hasher = std::collections::hash_map::DefaultHasher::new();
                        sid.hash(&mut hasher);
                        hasher.finish()
                    },
                );

                index.add_element_position(element_id, position);
            }
        }

        // Recurse into children
        for child in &self.children {
            child.build_chart_index_recursive(index, counter);
        }
    }

    /// Find all nodes whose source spans contain a specific byte offset.
    ///
    /// This is useful for click-to-highlight: find rendered elements at a
    /// specific position in the source text.
    #[cfg(feature = "svg")]
    pub fn find_by_source_offset(&self, offset: usize) -> Vec<&SceneNode> {
        self.find_all(|node| {
            node.get_json_metadata::<crate::TextSpan>(metadata_keys::SOURCE_SPAN)
                .is_some_and(|span| span.contains(offset))
        })
    }

    /// Find all nodes whose source spans overlap with a byte range.
    ///
    /// This is useful for selection handling: find rendered elements that
    /// correspond to a selected range of source text.
    #[cfg(feature = "svg")]
    pub fn find_by_source_range(&self, start: usize, end: usize) -> Vec<&SceneNode> {
        self.find_all(|node| {
            node.get_json_metadata::<crate::TextSpan>(metadata_keys::SOURCE_SPAN)
                .is_some_and(|span| span.overlaps_range(start, end))
        })
    }

    /// Find all nodes at a specific chart position (measure and beat).
    #[cfg(feature = "svg")]
    pub fn find_by_chart_position(&self, measure: u32, beat: u32) -> Vec<&SceneNode> {
        self.find_all(|node| {
            node.get_json_metadata::<crate::ChartPosition>(metadata_keys::CHART_POSITION)
                .is_some_and(|pos| pos.measure == measure && pos.beat == beat)
        })
    }
}

/// Compute bounding box from paint commands.
fn compute_commands_bounds(commands: &[PaintCommand]) -> Rect {
    let mut bounds = Rect::ZERO;

    for cmd in commands {
        let cmd_bounds = match cmd {
            PaintCommand::Fill { path, .. } | PaintCommand::Stroke { path, .. } => {
                path.bounding_box()
            }
            PaintCommand::Glyph { position, size, .. } => {
                // Approximate glyph bounds (actual bounds come from font metrics)
                Rect::new(
                    position.x,
                    position.y - size,
                    position.x + size,
                    position.y + size * 0.25,
                )
            }
            PaintCommand::Text {
                position,
                font_size,
                text,
                ..
            } => {
                // Approximate text bounds (actual bounds come from font metrics)
                let width = text.len() as f64 * font_size * 0.5;
                Rect::new(
                    position.x,
                    position.y - font_size,
                    position.x + width,
                    position.y + font_size * 0.25,
                )
            }
            PaintCommand::Line {
                start, end, width, ..
            } => {
                let half = width / 2.0;
                Rect::new(
                    start.x.min(end.x) - half,
                    start.y.min(end.y) - half,
                    start.x.max(end.x) + half,
                    start.y.max(end.y) + half,
                )
            }
            PaintCommand::Rect {
                rect, stroke_width, ..
            } => {
                let half = stroke_width / 2.0;
                rect.inflate(half, half)
            }
            PaintCommand::Circle {
                center,
                radius,
                stroke_width,
                ..
            } => {
                let r = radius + stroke_width / 2.0;
                Rect::new(center.x - r, center.y - r, center.x + r, center.y + r)
            }
            PaintCommand::Ellipse {
                center,
                radius_x,
                radius_y,
                stroke_width,
                ..
            } => {
                let half = stroke_width / 2.0;
                Rect::new(
                    center.x - radius_x - half,
                    center.y - radius_y - half,
                    center.x + radius_x + half,
                    center.y + radius_y + half,
                )
            }
        };

        if bounds.is_zero_area() {
            bounds = cmd_bounds;
        } else {
            bounds = bounds.union(cmd_bounds);
        }
    }

    bounds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engraver::scene::id::ElementType;
    use peniko::Color;

    #[test]
    fn test_new_node() {
        let node = SceneNode::new();
        assert!(node.id.is_none());
        assert_eq!(node.transform, Affine::IDENTITY);
        assert!(node.commands.is_empty());
        assert!(node.children.is_empty());
        assert!(node.visible);
    }

    #[test]
    fn test_group_node() {
        let node = SceneNode::group(SemanticId::measure(1));
        assert!(node.id.is_some());
        assert_eq!(node.id.as_ref().unwrap().element_type, ElementType::Measure);
    }

    #[test]
    fn test_leaf_node() {
        let commands = vec![PaintCommand::line(
            Point::new(0.0, 0.0),
            Point::new(100.0, 0.0),
            Color::BLACK,
            1.0,
        )];
        let node = SceneNode::leaf(SemanticId::chord(1), commands);
        assert_eq!(node.commands.len(), 1);
        assert!(!node.bounds.is_zero_area());
    }

    #[test]
    fn test_hit_test_returns_deepest_match() {
        // Outer 0..100 x 0..100, child at (10,10)..(30,30) (positioned via transform).
        let mut outer =
            SceneNode::group(SemanticId::measure(1)).with_bounds(Rect::new(0.0, 0.0, 100.0, 100.0));
        let inner = SceneNode::leaf(
            SemanticId::chord(1),
            vec![PaintCommand::line(
                Point::new(0.0, 0.0),
                Point::new(20.0, 20.0),
                Color::BLACK,
                1.0,
            )],
        )
        .with_bounds(Rect::new(0.0, 0.0, 20.0, 20.0))
        .with_transform(Affine::translate((10.0, 10.0)));
        outer.add_child(inner);

        // Hit inside the inner child's world bounds → deepest match wins.
        let hit = outer.hit_test(Point::new(15.0, 15.0)).unwrap();
        assert_eq!(
            hit.id.as_ref().unwrap().element_type,
            ElementType::Chord,
            "expected chord (deepest hit), got {:?}",
            hit.id
        );

        // Hit only in outer (outside child) → outer matches.
        let hit = outer.hit_test(Point::new(80.0, 80.0)).unwrap();
        assert_eq!(hit.id.as_ref().unwrap().element_type, ElementType::Measure);

        // Miss entirely.
        assert!(outer.hit_test(Point::new(200.0, 200.0)).is_none());
    }

    #[test]
    fn test_hit_test_skips_invisible() {
        let mut outer =
            SceneNode::group(SemanticId::measure(1)).with_bounds(Rect::new(0.0, 0.0, 100.0, 100.0));
        let invisible_child = SceneNode::leaf(SemanticId::chord(1), vec![])
            .with_bounds(Rect::new(0.0, 0.0, 50.0, 50.0))
            .with_visible(false);
        outer.add_child(invisible_child);

        // Click would hit the invisible child first, but it's skipped → falls back to outer.
        let hit = outer.hit_test(Point::new(10.0, 10.0)).unwrap();
        assert_eq!(hit.id.as_ref().unwrap().element_type, ElementType::Measure);
    }

    #[test]
    fn test_with_position() {
        let node = SceneNode::new().with_position(Point::new(50.0, 100.0));
        let expected = Affine::translate((50.0, 100.0));
        assert_eq!(node.transform, expected);
    }

    #[test]
    fn test_add_child() {
        let mut parent = SceneNode::group(SemanticId::measure(1));
        let child = SceneNode::group(SemanticId::chord(1));
        parent.add_child(child);
        assert_eq!(parent.children.len(), 1);
    }

    #[test]
    fn test_node_count() {
        let mut root = SceneNode::new();
        let mut child1 = SceneNode::new();
        child1.add_child(SceneNode::new());
        child1.add_child(SceneNode::new());
        root.add_child(child1);
        root.add_child(SceneNode::new());

        assert_eq!(root.node_count(), 5); // root + child1 + 2 grandchildren + child2
    }

    #[test]
    fn test_find_by_id() {
        let mut root = SceneNode::group(SemanticId::page(1));
        let mut measure = SceneNode::group(SemanticId::measure(1));
        measure.add_child(SceneNode::group(SemanticId::chord(42)));
        root.add_child(measure);

        let found = root.find_by_id(&SemanticId::chord(42));
        assert!(found.is_some());
        assert_eq!(
            found.unwrap().id.as_ref().unwrap().element_type,
            ElementType::Chord
        );
    }

    #[test]
    fn test_compute_bounds() {
        let mut root = SceneNode::new();

        let child1 = SceneNode::anonymous_leaf(vec![PaintCommand::filled_rect(
            Rect::new(0.0, 0.0, 50.0, 50.0),
            Color::BLACK,
        )]);

        let child2 = SceneNode::anonymous_leaf(vec![PaintCommand::filled_rect(
            Rect::new(0.0, 0.0, 30.0, 30.0),
            Color::BLACK,
        )])
        .with_position(Point::new(100.0, 100.0));

        root.add_child(child1);
        root.add_child(child2);

        let bounds = root.compute_bounds();
        // Should encompass both children
        assert!(bounds.x0 <= 0.0);
        assert!(bounds.y0 <= 0.0);
        assert!(bounds.x1 >= 130.0); // 100 + 30
        assert!(bounds.y1 >= 130.0); // 100 + 30
    }

    #[test]
    fn test_invisible_node_excluded_from_bounds() {
        let mut root = SceneNode::new();

        let visible = SceneNode::anonymous_leaf(vec![PaintCommand::filled_rect(
            Rect::new(0.0, 0.0, 50.0, 50.0),
            Color::BLACK,
        )]);

        let invisible = SceneNode::anonymous_leaf(vec![PaintCommand::filled_rect(
            Rect::new(1000.0, 1000.0, 2000.0, 2000.0),
            Color::BLACK,
        )])
        .with_visible(false);

        root.add_child(visible);
        root.add_child(invisible);

        let bounds = root.compute_bounds();
        // Should not include the invisible child
        assert!(bounds.x1 <= 100.0);
        assert!(bounds.y1 <= 100.0);
    }

    #[test]
    fn test_json_metadata() {
        use serde::{Deserialize, Serialize};

        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        struct TestPosition {
            measure: u32,
            beat: u32,
        }

        let mut node = SceneNode::new();
        let pos = TestPosition {
            measure: 4,
            beat: 2,
        };

        node.set_json_metadata("test_position", &pos);

        let retrieved: Option<TestPosition> = node.get_json_metadata("test_position");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap(), pos);
    }

    #[test]
    fn test_convenience_metadata_accessors() {
        let mut node = SceneNode::new();

        node.set_page(3);
        node.set_system(1);
        node.set_measure(8);
        node.set_beat(2);
        node.set_voice(0);
        node.set_element_type("chord");
        node.set_section_type("Verse");

        assert_eq!(node.get_page(), Some(3));
        assert_eq!(node.get_system(), Some(1));
        assert_eq!(node.get_measure(), Some(8));
        assert_eq!(node.get_beat(), Some(2));
        assert_eq!(node.get_voice(), Some(0));
        assert_eq!(node.get_element_type(), Some("chord"));
        assert_eq!(node.get_section_type(), Some("Verse"));
    }

    #[test]
    fn test_with_json_metadata_builder() {
        use serde::{Deserialize, Serialize};

        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        struct TestSpan {
            start: usize,
            len: usize,
        }

        let span = TestSpan { start: 10, len: 5 };
        let node = SceneNode::new().with_json_metadata("span", &span);

        let retrieved: Option<TestSpan> = node.get_json_metadata("span");
        assert_eq!(retrieved, Some(span));
    }

    #[test]
    fn test_find_by_metadata() {
        let mut root = SceneNode::new();

        // Create children with different metadata
        let mut chord1 = SceneNode::group(SemanticId::chord(1));
        chord1.set_measure(0);
        chord1.set_beat(0);
        chord1.set_page(1);

        let mut chord2 = SceneNode::group(SemanticId::chord(2));
        chord2.set_measure(0);
        chord2.set_beat(1);
        chord2.set_page(1);

        let mut chord3 = SceneNode::group(SemanticId::chord(3));
        chord3.set_measure(1);
        chord3.set_beat(0);
        chord3.set_page(2);

        root.add_child(chord1);
        root.add_child(chord2);
        root.add_child(chord3);

        // Find by page
        let page1_nodes = root.find_by_page(1);
        assert_eq!(page1_nodes.len(), 2);

        // Find by measure
        let measure0_nodes = root.find_by_measure(0);
        assert_eq!(measure0_nodes.len(), 2);

        // Find by position
        let pos_nodes = root.find_by_position(0, 1);
        assert_eq!(pos_nodes.len(), 1);
    }

    #[test]
    fn test_metadata_keys_constants() {
        // Verify metadata keys are consistent
        assert_eq!(metadata_keys::PAGE, "page");
        assert_eq!(metadata_keys::SYSTEM, "system");
        assert_eq!(metadata_keys::MEASURE, "measure");
        assert_eq!(metadata_keys::BEAT, "beat");
        assert_eq!(metadata_keys::SOURCE_SPAN, "source_span");
        assert_eq!(metadata_keys::CHART_POSITION, "chart_position");
        assert_eq!(metadata_keys::SOURCE_LINK, "source_link");
    }

    // =========================================================================
    // Example Usage Tests - Demonstrating Real-World Workflows
    // =========================================================================

    /// Example: Building a scene graph with full metadata for a chart page.
    ///
    /// This shows how the chart layout engine would construct a scene graph
    /// with metadata for click-to-highlight and navigation features.
    #[test]
    fn example_scene_graph_with_metadata() {
        // Build a scene graph representing:
        // Page 1, System 0: | G | Am | C | D |
        // Page 1, System 1: | Em | F | G | C |

        let mut page = SceneNode::group(SemanticId::page(1));
        page.set_page(1);

        // System 0
        let mut system0 = SceneNode::group(SemanticId::system(0));
        system0.set_page(1);
        system0.set_system(0);

        let chord_symbols = [("G", 0u32, 0u32), ("Am", 0, 1), ("C", 0, 2), ("D", 0, 3)];

        for (idx, (symbol, system, measure)) in chord_symbols.iter().enumerate() {
            // Use chord_symbol for chord display nodes
            let mut chord = SceneNode::leaf(
                SemanticId::chord_symbol(idx as u64, *symbol),
                vec![PaintCommand::text(
                    *symbol,
                    "sans-serif",
                    24.0,
                    Point::new(*measure as f64 * 100.0 + 50.0, 50.0),
                    Color::BLACK,
                )],
            );
            chord.set_page(1);
            chord.set_system(*system);
            chord.set_measure(*measure);
            chord.set_beat(0);
            chord.set_element_type("chord");
            chord.set_section_type("Verse");
            system0.add_child(chord);
        }
        page.add_child(system0);

        // System 1
        let mut system1 = SceneNode::group(SemanticId::system(1));
        system1.set_page(1);
        system1.set_system(1);

        let chorus_chords = [("Em", 1u32, 4u32), ("F", 1, 5), ("G", 1, 6), ("C", 1, 7)];

        for (idx, (symbol, system, measure)) in chorus_chords.iter().enumerate() {
            let mut chord = SceneNode::leaf(
                SemanticId::chord_symbol((idx + 4) as u64, *symbol),
                vec![PaintCommand::text(
                    *symbol,
                    "sans-serif",
                    24.0,
                    Point::new((*measure - 4) as f64 * 100.0 + 50.0, 150.0),
                    Color::BLACK,
                )],
            );
            chord.set_page(1);
            chord.set_system(*system);
            chord.set_measure(*measure);
            chord.set_beat(0);
            chord.set_element_type("chord");
            chord.set_section_type("Chorus");
            system1.add_child(chord);
        }
        page.add_child(system1);

        // Query the scene graph

        // Find all chords on page 1
        let all_chords = page.find_by_element_type("chord");
        assert_eq!(all_chords.len(), 8);

        // Find chords in system 0 (verse)
        let verse_chords = page.find_by_system(0);
        assert_eq!(verse_chords.len(), 5); // system group + 4 chords

        // Find the chord at measure 5
        let measure5 = page.find_by_measure(5);
        assert_eq!(measure5.len(), 1);
        assert_eq!(measure5[0].get_element_type(), Some("chord"));

        // Find all chorus chords
        let chorus = page.find_by_section_type("Chorus");
        assert_eq!(chorus.len(), 4);
    }

    /// Example: Hit testing and click-to-highlight with scene graph.
    ///
    /// Shows how to find a node at a screen position and look up its source.
    #[test]
    fn example_hit_test_and_highlight() {
        use serde::{Deserialize, Serialize};

        // Simulated source span (would normally come from keyflow)
        #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
        struct SourceSpan {
            start: usize,
            len: usize,
            line: u32,
            column: u32,
        }

        // Build a chord node with source span metadata
        let mut chord = SceneNode::leaf(
            SemanticId::chord_symbol(1, "Gmaj7"),
            vec![PaintCommand::text(
                "Gmaj7",
                "sans-serif",
                24.0,
                Point::new(100.0, 50.0),
                Color::BLACK,
            )],
        );

        // Add metadata for the chord
        chord.set_page(1);
        chord.set_measure(0);
        chord.set_beat(0);
        chord.set_element_type("chord");

        // Store the source span (linking back to "Gmaj7" in source text)
        let source_span = SourceSpan {
            start: 5, // byte offset in source
            len: 5,   // "Gmaj7" is 5 characters
            line: 2,
            column: 3,
        };
        chord.set_json_metadata(metadata_keys::SOURCE_SPAN, &source_span);

        // Simulate click handling:
        // 1. Hit test found this chord node
        // 2. Retrieve source span for highlighting

        let span: Option<SourceSpan> = chord.get_json_metadata(metadata_keys::SOURCE_SPAN);
        assert!(span.is_some());

        let span = span.unwrap();
        // Editor would highlight: line 2, columns 3-8
        assert_eq!(span.line, 2);
        assert_eq!(span.column, 3);
        assert_eq!(span.start, 5);
        assert_eq!(span.len, 5);
    }

    /// Example: Building a multi-page score navigation index.
    ///
    /// Shows how to query the scene graph for page-based navigation.
    #[test]
    fn example_multipage_navigation() {
        let mut root = SceneNode::new();

        // Build 3 pages with 2 systems each, 4 measures per system
        for page_num in 1..=3 {
            let mut page = SceneNode::group(SemanticId::page(page_num));
            page.set_page(page_num as u32);

            for system in 0..2 {
                let global_system = (page_num - 1) * 2 + system;
                let mut system_node = SceneNode::group(SemanticId::system(global_system));
                system_node.set_page(page_num as u32);
                system_node.set_system(global_system as u32);

                for beat in 0..4 {
                    let measure = global_system * 4 + beat;
                    let mut chord = SceneNode::group(SemanticId::chord(measure));
                    chord.set_page(page_num as u32);
                    chord.set_system(global_system as u32);
                    chord.set_measure(measure as u32);
                    chord.set_beat(0);
                    system_node.add_child(chord);
                }
                page.add_child(system_node);
            }
            root.add_child(page);
        }

        // Navigation queries:

        // 1. Go to page 2
        let page2_elements = root.find_by_page(2);
        assert!(!page2_elements.is_empty());

        // 2. Find all elements in measure 10 (should be on page 2)
        let measure10 = root.find_by_measure(10);
        assert_eq!(measure10.len(), 1);
        assert_eq!(measure10[0].get_page(), Some(2));

        // 3. Jump to system 4 (page 3, system 0)
        let system4 = root.find_by_system(4);
        assert!(!system4.is_empty());
        // All system 4 elements should be on page 3
        for node in &system4 {
            if node.get_page().is_some() {
                assert_eq!(node.get_page(), Some(3));
            }
        }
    }

    /// Example: Finding elements for playback cursor highlighting.
    ///
    /// During audio playback, highlight the current beat in the notation.
    #[test]
    fn example_playback_highlighting() {
        let mut root = SceneNode::new();

        // Create a measure with chord on beat 0 and slashes on beats 1-3
        let mut measure = SceneNode::group(SemanticId::measure(0));
        measure.set_measure(0);

        // Chord on beat 0
        let mut chord = SceneNode::group(SemanticId::chord(0));
        chord.set_measure(0);
        chord.set_beat(0);
        chord.set_element_type("chord");
        measure.add_child(chord);

        // Rhythm slashes on beats 1-3
        for beat in 1u64..4 {
            let mut slash = SceneNode::group(SemanticId::rhythm_slash(beat, beat as u8));
            slash.set_measure(0);
            slash.set_beat(beat as u32);
            slash.set_element_type("slash");
            measure.add_child(slash);
        }

        root.add_child(measure);

        // Simulate playback cursor at different positions

        // Beat 0: Should highlight the chord
        let beat0_elements = root.find_by_position(0, 0);
        assert_eq!(beat0_elements.len(), 1);
        assert_eq!(beat0_elements[0].get_element_type(), Some("chord"));

        // Beat 2: Should highlight a slash
        let beat2_elements = root.find_by_position(0, 2);
        assert_eq!(beat2_elements.len(), 1);
        assert_eq!(beat2_elements[0].get_element_type(), Some("slash"));

        // Find all elements in the current measure for context
        let all_in_measure = root.find_by_measure(0);
        assert_eq!(all_in_measure.len(), 5); // measure group + chord + 3 slashes
    }

    /// Example: Filtering by element type for selective rendering.
    ///
    /// Shows how to find specific element types for layer-based rendering.
    #[test]
    fn example_element_type_filtering() {
        let mut root = SceneNode::new();

        // Create various element types using anonymous nodes with metadata
        // (In real code, you'd use appropriate SemanticId constructors)

        // Chord at index 0
        let mut node0 = SceneNode::group(SemanticId::chord_symbol(0, "G"));
        node0.set_element_type("chord");
        node0.set_measure(0);
        root.add_child(node0);

        // Barline at index 1 (using segment as closest match)
        let mut node1 = SceneNode::group(SemanticId::segment(1, 0));
        node1.set_element_type("barline");
        node1.set_measure(0);
        root.add_child(node1);

        // Chord at index 2
        let mut node2 = SceneNode::group(SemanticId::chord_symbol(2, "Am"));
        node2.set_element_type("chord");
        node2.set_measure(1);
        root.add_child(node2);

        // Barline at index 3
        let mut node3 = SceneNode::group(SemanticId::segment(3, 0));
        node3.set_element_type("barline");
        node3.set_measure(1);
        root.add_child(node3);

        // Rehearsal mark at index 4
        let mut node4 = SceneNode::group(SemanticId::rehearsal_mark(4, "A"));
        node4.set_element_type("rehearsal_mark");
        node4.set_measure(2);
        root.add_child(node4);

        // Chord at index 5
        let mut node5 = SceneNode::group(SemanticId::chord_symbol(5, "C"));
        node5.set_element_type("chord");
        node5.set_measure(2);
        root.add_child(node5);

        // Find all chords (for chord-only display mode)
        let chords = root.find_by_element_type("chord");
        assert_eq!(chords.len(), 3);

        // Find all barlines (for barline layer)
        let barlines = root.find_by_element_type("barline");
        assert_eq!(barlines.len(), 2);

        // Find rehearsal marks (for navigation markers)
        let marks = root.find_by_element_type("rehearsal_mark");
        assert_eq!(marks.len(), 1);
    }

    // =========================================================================
    // GlyphInfo Tests
    // =========================================================================

    #[test]
    fn test_glyph_info_smufl() {
        // SMuFL quarter note (U+E0A4)
        let info = GlyphInfo::smufl('\u{E0A4}', "Bravura")
            .with_smufl_name("noteQuarterUp")
            .with_smufl_class("noteheads");

        assert!(info.is_smufl());
        assert!(!info.is_text());
        assert_eq!(info.glyph_type, GlyphType::Smufl);
        assert_eq!(info.content, "\u{E0A4}");
        assert_eq!(info.codepoint, Some(0xE0A4));
        assert_eq!(info.font_family, "Bravura");
        assert_eq!(info.smufl_name, Some("noteQuarterUp".to_string()));
        assert_eq!(info.smufl_class, Some("noteheads".to_string()));
        assert_eq!(info.codepoint_string(), Some("U+E0A4".to_string()));
        assert_eq!(info.as_char(), Some('\u{E0A4}'));
    }

    #[test]
    fn test_glyph_info_text() {
        let info = GlyphInfo::text("Gmaj7", "Arial");

        assert!(!info.is_smufl());
        assert!(info.is_text());
        assert_eq!(info.glyph_type, GlyphType::Text);
        assert_eq!(info.content, "Gmaj7");
        assert_eq!(info.codepoint, None); // Multi-char text has no single codepoint
        assert_eq!(info.font_family, "Arial");
    }

    #[test]
    fn test_glyph_info_single_char_text() {
        let info = GlyphInfo::text("G", "Arial");

        assert_eq!(info.codepoint, Some('G' as u32));
        assert_eq!(info.codepoint_string(), Some("U+0047".to_string()));
    }

    #[test]
    fn test_glyph_info_symbol() {
        let info = GlyphInfo::symbol('♯', "sans-serif").with_description("Sharp sign");

        assert_eq!(info.glyph_type, GlyphType::Symbol);
        assert_eq!(info.content, "♯");
        assert_eq!(info.description, Some("Sharp sign".to_string()));
    }

    #[test]
    fn test_node_glyph_info_accessors() {
        let mut node = SceneNode::new();

        // Set SMuFL glyph
        node.set_smufl_glyph('\u{E0A4}', "Bravura", Some("noteQuarterUp"));

        assert!(node.is_smufl_glyph());
        assert!(!node.is_text_glyph());

        let info = node.get_glyph_info().unwrap();
        assert_eq!(info.font_family, "Bravura");
        assert_eq!(info.smufl_name, Some("noteQuarterUp".to_string()));
    }

    #[test]
    fn test_node_text_glyph_accessor() {
        let mut node = SceneNode::new();
        node.set_text_glyph("Dm7", "Times New Roman");

        assert!(!node.is_smufl_glyph());
        assert!(node.is_text_glyph());

        let info = node.get_glyph_info().unwrap();
        assert_eq!(info.content, "Dm7");
        assert_eq!(info.font_family, "Times New Roman");
    }

    #[test]
    fn test_with_glyph_info_builder() {
        let glyph = GlyphInfo::smufl('\u{E050}', "Leland").with_smufl_name("gClef");

        let node = SceneNode::new().with_glyph_info(&glyph);

        let retrieved = node.get_glyph_info().unwrap();
        assert_eq!(retrieved.font_family, "Leland");
        assert_eq!(retrieved.smufl_name, Some("gClef".to_string()));
    }

    /// Example: Tracking glyphs for font debugging.
    ///
    /// Shows how to record glyph information for debugging font rendering issues.
    #[test]
    fn example_glyph_font_tracking() {
        let mut root = SceneNode::new();

        // Quarter note from Bravura
        let mut note = SceneNode::group(SemanticId::note(1, "C4"));
        note.set_smufl_glyph('\u{E0A4}', "Bravura", Some("noteQuarterUp"));
        note.set_element_type("notehead");
        root.add_child(note);

        // G clef from Bravura
        let mut clef = SceneNode::group(SemanticId::new(ElementType::Clef, 1));
        clef.set_smufl_glyph('\u{E050}', "Bravura", Some("gClef"));
        clef.set_element_type("clef");
        root.add_child(clef);

        // Chord symbol as text
        let mut chord = SceneNode::group(SemanticId::chord_symbol(1, "Gmaj7"));
        chord.set_text_glyph("Gmaj7", "Arial");
        chord.set_element_type("chord_symbol");
        root.add_child(chord);

        // Accidental from Bravura
        let mut accidental = SceneNode::group(SemanticId::new(ElementType::Accidental, 1));
        accidental.set_smufl_glyph('\u{E262}', "Bravura", Some("accidentalSharp"));
        accidental.set_element_type("accidental");
        root.add_child(accidental);

        // Query: Find all SMuFL glyphs (for font loading)
        let smufl_nodes = root.find_smufl_glyphs();
        assert_eq!(smufl_nodes.len(), 3); // note, clef, accidental

        // Query: Find all text glyphs (for text font loading)
        let text_nodes = root.find_text_glyphs();
        assert_eq!(text_nodes.len(), 1); // chord symbol

        // Query: Find all glyphs from Bravura
        let bravura_nodes = root.find_by_font_family("Bravura");
        assert_eq!(bravura_nodes.len(), 3);

        // Verify we can extract glyph details for debugging
        for node in &smufl_nodes {
            let info = node.get_glyph_info().unwrap();
            assert!(info.is_smufl());
            assert!(info.smufl_name.is_some());
            // Could log: "{} from {} (U+{:04X})", info.smufl_name, info.font_family, info.codepoint
        }
    }

    /// Example: Building a glyph inventory for font subsetting.
    ///
    /// Shows how to collect all unique glyphs used in a score.
    #[test]
    fn example_glyph_inventory() {
        let mut root = SceneNode::new();

        // Create nodes with various glyphs
        let glyphs = [
            ('\u{E0A4}', "noteQuarterUp", "Bravura"),
            ('\u{E0A3}', "noteHalfUp", "Bravura"),
            ('\u{E0A4}', "noteQuarterUp", "Bravura"), // duplicate
            ('\u{E050}', "gClef", "Bravura"),
            ('\u{E262}', "accidentalSharp", "Bravura"),
        ];

        for (idx, (codepoint, name, font)) in glyphs.iter().enumerate() {
            let mut node = SceneNode::new();
            let info = GlyphInfo::smufl(*codepoint, *font).with_smufl_name(*name);
            node.set_glyph_info(&info);
            node.set_element_type(&format!("element_{idx}"));
            root.add_child(node);
        }

        // Collect unique glyphs for font subsetting
        let smufl_nodes = root.find_smufl_glyphs();

        let mut unique_codepoints: std::collections::HashSet<u32> =
            std::collections::HashSet::new();
        for node in &smufl_nodes {
            if let Some(cp) = node.get_glyph_info().and_then(|info| info.codepoint) {
                unique_codepoints.insert(cp);
            }
        }

        // Should have 4 unique codepoints (noteQuarterUp appears twice)
        assert_eq!(unique_codepoints.len(), 4);
        assert!(unique_codepoints.contains(&0xE0A4)); // noteQuarterUp
        assert!(unique_codepoints.contains(&0xE0A3)); // noteHalfUp
        assert!(unique_codepoints.contains(&0xE050)); // gClef
        assert!(unique_codepoints.contains(&0xE262)); // accidentalSharp
    }

    /// Example: Verifying correct fonts are used for different element types.
    ///
    /// This test demonstrates a real-world scenario where:
    /// - Chord symbols use MuseJazz (jazz-style chord font)
    /// - Section labels use a bold sans-serif
    /// - Lyrics use a readable serif font
    /// - Music notation uses Bravura (SMuFL font)
    #[test]
    fn example_verify_font_assignments() {
        let mut root = SceneNode::new();

        // Chord symbols should use MuseJazz
        let chord_symbols = ["Gmaj7", "Dm7", "Cmaj7", "Am7"];
        for (idx, symbol) in chord_symbols.iter().enumerate() {
            let mut chord = SceneNode::group(SemanticId::chord_symbol(idx as u64, *symbol));
            chord.set_text_glyph(symbol, "MuseJazz");
            chord.set_element_type("chord_symbol");
            chord.set_measure(idx as u32);
            root.add_child(chord);
        }

        // Section labels should use bold sans-serif
        let sections = ["Verse 1", "Chorus", "Bridge"];
        for (idx, section) in sections.iter().enumerate() {
            let mut label =
                SceneNode::group(SemanticId::rehearsal_mark(idx as u64 + 100, *section));
            label.set_text_glyph(section, "Helvetica Bold");
            label.set_element_type("section_label");
            root.add_child(label);
        }

        // Lyrics should use readable serif
        let lyrics = ["Hel-", "lo", "world"];
        for word in lyrics.iter() {
            let mut lyric = SceneNode::new();
            lyric.set_text_glyph(word, "Times New Roman");
            lyric.set_element_type("lyric");
            root.add_child(lyric);
        }

        // Music notation should use Bravura
        let notes = ['\u{E0A4}', '\u{E0A3}', '\u{E0A2}']; // quarter, half, whole
        for (idx, &codepoint) in notes.iter().enumerate() {
            let mut note = SceneNode::group(SemanticId::note(idx as u64 + 200, "C4"));
            note.set_smufl_glyph(codepoint, "Bravura", None);
            note.set_element_type("notehead");
            root.add_child(note);
        }

        // ===== VERIFICATION =====

        // 1. All chord symbols should use MuseJazz
        let chord_nodes = root.find_by_element_type("chord_symbol");
        assert_eq!(chord_nodes.len(), 4);
        for node in &chord_nodes {
            let info = node.get_glyph_info().expect("Chord should have glyph info");
            assert_eq!(
                info.font_family, "MuseJazz",
                "Chord symbols must use MuseJazz"
            );
            assert!(info.is_text(), "Chord symbols should be text glyphs");
        }

        // 2. Section labels should use Helvetica Bold
        let section_nodes = root.find_by_element_type("section_label");
        assert_eq!(section_nodes.len(), 3);
        for node in &section_nodes {
            let info = node
                .get_glyph_info()
                .expect("Section label should have glyph info");
            assert_eq!(
                info.font_family, "Helvetica Bold",
                "Section labels must use Helvetica Bold"
            );
        }

        // 3. Lyrics should use Times New Roman
        let lyric_nodes = root.find_by_element_type("lyric");
        assert_eq!(lyric_nodes.len(), 3);
        for node in &lyric_nodes {
            let info = node.get_glyph_info().expect("Lyric should have glyph info");
            assert_eq!(
                info.font_family, "Times New Roman",
                "Lyrics must use Times New Roman"
            );
        }

        // 4. Noteheads should use Bravura SMuFL font
        let note_nodes = root.find_by_element_type("notehead");
        assert_eq!(note_nodes.len(), 3);
        for node in &note_nodes {
            let info = node.get_glyph_info().expect("Note should have glyph info");
            assert_eq!(info.font_family, "Bravura", "Noteheads must use Bravura");
            assert!(info.is_smufl(), "Noteheads should be SMuFL glyphs");
        }

        // 5. Verify we can find all nodes by font
        let musejazz_nodes = root.find_by_font_family("MuseJazz");
        assert_eq!(musejazz_nodes.len(), 4, "Should find 4 MuseJazz nodes");

        let bravura_nodes = root.find_by_font_family("Bravura");
        assert_eq!(bravura_nodes.len(), 3, "Should find 3 Bravura nodes");

        let times_nodes = root.find_by_font_family("Times New Roman");
        assert_eq!(times_nodes.len(), 3, "Should find 3 Times New Roman nodes");
    }

    /// Example: Font consistency check across a chart.
    ///
    /// Verifies that all elements of the same type use consistent fonts.
    #[test]
    fn example_font_consistency_check() {
        let mut root = SceneNode::new();

        // Build a scene with multiple systems, each having chord symbols
        for system in 0..3 {
            let mut system_node = SceneNode::group(SemanticId::system(system));

            for measure in 0..4 {
                let chord_text = format!("{}maj7", ['C', 'D', 'E', 'F'][measure as usize]);
                let mut chord =
                    SceneNode::group(SemanticId::chord_symbol(system * 4 + measure, &chord_text));
                // All chords should use MuseJazz
                chord.set_text_glyph(&chord_text, "MuseJazz");
                chord.set_element_type("chord_symbol");
                chord.set_system(system as u32);
                chord.set_measure(measure as u32);
                system_node.add_child(chord);
            }
            root.add_child(system_node);
        }

        // Verify font consistency: all chord symbols use the same font
        let all_chords = root.find_by_element_type("chord_symbol");
        assert_eq!(all_chords.len(), 12); // 3 systems × 4 measures

        let mut fonts_used: std::collections::HashSet<String> = std::collections::HashSet::new();
        for chord in &all_chords {
            if let Some(info) = chord.get_glyph_info() {
                fonts_used.insert(info.font_family.clone());
            }
        }

        // All chord symbols should use exactly one font
        assert_eq!(
            fonts_used.len(),
            1,
            "All chord symbols should use the same font, but found: {:?}",
            fonts_used
        );
        assert!(
            fonts_used.contains("MuseJazz"),
            "Chord symbols should use MuseJazz"
        );
    }

    /// Example: Detecting font mismatches (useful for debugging).
    ///
    /// Shows how to find elements that might be using the wrong font.
    #[test]
    fn example_detect_font_mismatches() {
        let mut root = SceneNode::new();

        // Intentionally create some nodes with "wrong" fonts for testing
        let mut chord1 = SceneNode::new();
        chord1.set_text_glyph("Gmaj7", "MuseJazz"); // Correct
        chord1.set_element_type("chord_symbol");
        root.add_child(chord1);

        let mut chord2 = SceneNode::new();
        chord2.set_text_glyph("Am7", "MuseJazz"); // Correct
        chord2.set_element_type("chord_symbol");
        root.add_child(chord2);

        let mut chord3 = SceneNode::new();
        chord3.set_text_glyph("Dm7", "Arial"); // WRONG - should be MuseJazz
        chord3.set_element_type("chord_symbol");
        root.add_child(chord3);

        let mut chord4 = SceneNode::new();
        chord4.set_text_glyph("Em7", "Comic Sans"); // WRONG - definitely wrong!
        chord4.set_element_type("chord_symbol");
        root.add_child(chord4);

        // Define expected font for chord symbols
        let expected_chord_font = "MuseJazz";

        // Find mismatched fonts
        let chord_nodes = root.find_by_element_type("chord_symbol");
        let mut mismatches: Vec<(&SceneNode, String, String)> = Vec::new();

        for node in &chord_nodes {
            if let Some(info) = node
                .get_glyph_info()
                .filter(|i| i.font_family != expected_chord_font)
            {
                mismatches.push((node, info.content.clone(), info.font_family.clone()));
            }
        }

        // Should detect 2 mismatches
        assert_eq!(mismatches.len(), 2, "Should detect 2 font mismatches");

        // Verify the mismatched fonts
        let mismatch_fonts: Vec<&str> = mismatches.iter().map(|(_, _, f)| f.as_str()).collect();
        assert!(mismatch_fonts.contains(&"Arial"));
        assert!(mismatch_fonts.contains(&"Comic Sans"));

        // In real code, you might log these:
        // for (node, content, font) in &mismatches {
        //     eprintln!("Font mismatch: '{}' uses '{}' instead of '{}'",
        //               content, font, expected_chord_font);
        // }
    }

    /// Example: Multi-font score with style configuration.
    ///
    /// Shows how a style system might configure and verify fonts.
    #[test]
    fn example_style_configured_fonts() {
        // Simulated style configuration
        struct FontConfig {
            chord_font: &'static str,
            lyric_font: &'static str,
            section_font: &'static str,
            music_font: &'static str,
        }

        let jazz_style = FontConfig {
            chord_font: "MuseJazz",
            lyric_font: "Georgia",
            section_font: "Futura Bold",
            music_font: "Bravura",
        };

        let classical_style = FontConfig {
            chord_font: "Times New Roman",
            lyric_font: "Garamond",
            section_font: "Didot",
            music_font: "Bravura",
        };

        // Function to build a scene with a given style
        fn build_scene_with_style(style: &FontConfig) -> SceneNode {
            let mut root = SceneNode::new();

            // Chord
            let mut chord = SceneNode::new();
            chord.set_text_glyph("Cmaj7", style.chord_font);
            chord.set_element_type("chord_symbol");
            root.add_child(chord);

            // Lyric
            let mut lyric = SceneNode::new();
            lyric.set_text_glyph("La la la", style.lyric_font);
            lyric.set_element_type("lyric");
            root.add_child(lyric);

            // Section
            let mut section = SceneNode::new();
            section.set_text_glyph("Verse", style.section_font);
            section.set_element_type("section_label");
            root.add_child(section);

            // Note
            let mut note = SceneNode::new();
            note.set_smufl_glyph('\u{E0A4}', style.music_font, Some("noteQuarterUp"));
            note.set_element_type("notehead");
            root.add_child(note);

            root
        }

        // Build scenes with different styles
        let jazz_scene = build_scene_with_style(&jazz_style);
        let classical_scene = build_scene_with_style(&classical_style);

        // Verify jazz style fonts
        let jazz_chord = jazz_scene.find_by_element_type("chord_symbol")[0];
        assert_eq!(jazz_chord.get_glyph_info().unwrap().font_family, "MuseJazz");

        let jazz_lyric = jazz_scene.find_by_element_type("lyric")[0];
        assert_eq!(jazz_lyric.get_glyph_info().unwrap().font_family, "Georgia");

        // Verify classical style fonts
        let classical_chord = classical_scene.find_by_element_type("chord_symbol")[0];
        assert_eq!(
            classical_chord.get_glyph_info().unwrap().font_family,
            "Times New Roman"
        );

        let classical_lyric = classical_scene.find_by_element_type("lyric")[0];
        assert_eq!(
            classical_lyric.get_glyph_info().unwrap().font_family,
            "Garamond"
        );

        // Both styles should use Bravura for music notation
        let jazz_note = jazz_scene.find_by_element_type("notehead")[0];
        let classical_note = classical_scene.find_by_element_type("notehead")[0];
        assert_eq!(jazz_note.get_glyph_info().unwrap().font_family, "Bravura");
        assert_eq!(
            classical_note.get_glyph_info().unwrap().font_family,
            "Bravura"
        );
    }
}
