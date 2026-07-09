//! Repeat signs layout module.
//!
//! Implements layout for repeat signs including:
//! - **Markers**: Segno, Coda, Fine (destination/endpoint markers)
//! - **Jumps**: D.C., D.S., To Coda (navigation instructions)
//!
//! ## Marker Types
//!
//! Markers are placed at specific points in the music to mark destinations:
//! - `Segno` (𝄋) - Sign marker for D.S. (Dal Segno) jumps
//! - `Coda` (𝄌) - Coda sign marking the coda section
//! - `Fine` - End marker for D.C./D.S. al Fine
//!
//! ## Jump Types
//!
//! Jumps are navigation instructions placed at the end of sections:
//! - `D.C.` (Da Capo) - Return to the beginning
//! - `D.S.` (Dal Segno) - Return to the segno sign
//! - `To Coda` - Jump to the coda section
//! - Various combinations: D.C. al Fine, D.S. al Coda, etc.
//!
//! Reference: MuseScore `marker.cpp`, `jump.cpp`

use kurbo::{Point, Rect};
use peniko::Color;

use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::{PaintCommand, SceneNode};

// ============================================================================
// SMuFL Codepoints for Repeat Signs
// ============================================================================

/// SMuFL codepoints for repeat sign symbols.
///
/// Reference: SMuFL specification, Repeats range (U+E040–U+E04F)
pub mod smufl {
    /// Segno sign (𝄋) - U+E045
    pub const SEGNO: char = '\u{E045}';

    /// Segno serpent variant 1 - U+E04A
    pub const SEGNO_SERPENT1: char = '\u{E04A}';

    /// Segno serpent variant 2 - U+E04B
    pub const SEGNO_SERPENT2: char = '\u{E04B}';

    /// Coda sign (𝄌) - U+E046
    pub const CODA: char = '\u{E046}';

    /// Coda square variant - U+E047
    pub const CODA_SQUARE: char = '\u{E047}';

    /// Dal Segno symbol - U+E045 (same as segno, used in text)
    pub const DAL_SEGNO: char = '\u{E045}';

    /// Da Capo symbol (optional, often just text)
    pub const DA_CAPO: char = '\u{E046}';
}

// ============================================================================
// Marker Types
// ============================================================================

/// Type of repeat marker (destination/endpoint).
///
/// Markers are placed at specific points in the music to mark
/// destinations for jumps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum MarkerType {
    /// Segno sign (𝄋) - standard segno
    #[default]
    Segno,
    /// Variant segno (serpent style)
    VarSegno,
    /// Coda sign (𝄌) - standard coda
    Coda,
    /// Variant coda (square style)
    VarCoda,
    /// Codetta (double coda) - two coda symbols
    Codetta,
    /// Fine - end marker (text)
    Fine,
}

impl MarkerType {
    /// Get the SMuFL codepoint for this marker type.
    ///
    /// Returns `None` for text-only markers like Fine.
    #[must_use]
    pub const fn smufl_codepoint(self) -> Option<char> {
        match self {
            Self::Segno => Some(smufl::SEGNO),
            Self::VarSegno => Some(smufl::SEGNO_SERPENT1),
            Self::Coda => Some(smufl::CODA),
            Self::VarCoda => Some(smufl::CODA_SQUARE),
            Self::Codetta => Some(smufl::CODA), // Rendered as two codas
            Self::Fine => None,
        }
    }

    /// Get the display text for this marker type.
    #[must_use]
    pub const fn display_text(self) -> &'static str {
        match self {
            Self::Segno => "",    // Symbol only
            Self::VarSegno => "", // Symbol only
            Self::Coda => "",     // Symbol only
            Self::VarCoda => "",  // Symbol only
            Self::Codetta => "",  // Symbol only (two codas)
            Self::Fine => "Fine",
        }
    }

    /// Get the label used for jump references.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Segno => "segno",
            Self::VarSegno => "varsegno",
            Self::Coda => "codab",
            Self::VarCoda => "varcoda",
            Self::Codetta => "codetta",
            Self::Fine => "fine",
        }
    }

    /// Whether this marker is a right-aligned marker.
    ///
    /// Right markers (Fine) are positioned at the end of the measure.
    #[must_use]
    pub const fn is_right_marker(self) -> bool {
        matches!(self, Self::Fine)
    }

    /// Whether this marker uses a symbol glyph.
    #[must_use]
    pub const fn has_symbol(self) -> bool {
        self.smufl_codepoint().is_some()
    }

    /// Approximate glyph dimensions (width, height) in spatiums.
    #[must_use]
    pub const fn glyph_dimensions(self) -> (f64, f64) {
        match self {
            Self::Segno | Self::VarSegno => (2.0, 2.8),
            Self::Coda | Self::VarCoda => (2.0, 2.5),
            Self::Codetta => (4.0, 2.5), // Two codas side by side
            Self::Fine => (2.5, 1.0),    // Text dimensions
        }
    }
}

// ============================================================================
// Jump Types
// ============================================================================

/// Type of repeat jump (navigation instruction).
///
/// Jumps are placed at the end of sections to instruct performers
/// where to go next.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum JumpType {
    /// D.C. (Da Capo) - return to start, play to end
    #[default]
    DaCapo,
    /// D.C. al Fine - return to start, play until Fine
    DaCapoAlFine,
    /// D.C. al Coda - return to start, play until To Coda, then jump to Coda
    DaCapoAlCoda,
    /// D.S. (Dal Segno) - return to segno, play to end
    DalSegno,
    /// D.S. al Fine - return to segno, play until Fine
    DalSegnoAlFine,
    /// D.S. al Coda - return to segno, play until To Coda, then jump to Coda
    DalSegnoAlCoda,
    /// D.C. al Doppia Coda
    DaCapoAlDblCoda,
    /// D.S. al Doppia Coda
    DalSegnoAlDblCoda,
    /// To Coda - jump to coda section
    ToCoda,
    /// To Coda with symbol
    ToCodaSym,
}

impl JumpType {
    /// Get the display text for this jump type.
    #[must_use]
    pub const fn display_text(self) -> &'static str {
        match self {
            Self::DaCapo => "D.C.",
            Self::DaCapoAlFine => "D.C. al Fine",
            Self::DaCapoAlCoda => "D.C. al Coda",
            Self::DalSegno => "D.S.",
            Self::DalSegnoAlFine => "D.S. al Fine",
            Self::DalSegnoAlCoda => "D.S. al Coda",
            Self::DaCapoAlDblCoda => "D.C. al Doppia Coda",
            Self::DalSegnoAlDblCoda => "D.S. al Doppia Coda",
            Self::ToCoda => "To Coda",
            Self::ToCodaSym => "To", // Followed by coda symbol
        }
    }

    /// Get the jump destination label.
    #[must_use]
    pub const fn jump_to(self) -> &'static str {
        match self {
            Self::DaCapo | Self::DaCapoAlFine | Self::DaCapoAlCoda | Self::DaCapoAlDblCoda => {
                "start"
            }
            Self::DalSegno
            | Self::DalSegnoAlFine
            | Self::DalSegnoAlCoda
            | Self::DalSegnoAlDblCoda => "segno",
            Self::ToCoda | Self::ToCodaSym => "coda",
        }
    }

    /// Get the play-until label.
    #[must_use]
    pub const fn play_until(self) -> &'static str {
        match self {
            Self::DaCapo | Self::DalSegno => "end",
            Self::DaCapoAlFine | Self::DalSegnoAlFine => "fine",
            Self::DaCapoAlCoda | Self::DalSegnoAlCoda => "coda",
            Self::DaCapoAlDblCoda | Self::DalSegnoAlDblCoda => "varcoda",
            Self::ToCoda | Self::ToCodaSym => "coda",
        }
    }

    /// Get the continue-at label (after jump).
    #[must_use]
    pub const fn continue_at(self) -> &'static str {
        match self {
            Self::DaCapoAlCoda
            | Self::DalSegnoAlCoda
            | Self::DaCapoAlDblCoda
            | Self::DalSegnoAlDblCoda => "codab",
            _ => "",
        }
    }

    /// Whether this jump includes a symbol in its rendering.
    #[must_use]
    pub const fn has_symbol(self) -> bool {
        matches!(self, Self::ToCodaSym)
    }

    /// Get the symbol codepoint if this jump includes one.
    #[must_use]
    pub const fn symbol_codepoint(self) -> Option<char> {
        match self {
            Self::ToCodaSym => Some(smufl::CODA),
            _ => None,
        }
    }
}

// ============================================================================
// Placement
// ============================================================================

/// Placement of repeat sign relative to staff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RepeatPlacement {
    /// Above the staff (default for most signs)
    #[default]
    Above,
    /// Below the staff
    Below,
}

// ============================================================================
// Configuration
// ============================================================================

/// Configuration for repeat sign layout.
#[derive(Debug, Clone)]
pub struct RepeatSignConfig {
    /// Font size for text elements (in points)
    pub text_size: f64,
    /// Symbol size multiplier
    pub symbol_scale: f64,
    /// Minimum distance from staff (in spatiums)
    pub min_distance: f64,
    /// Horizontal offset (in spatiums)
    pub x_offset: f64,
    /// Vertical offset (in spatiums)
    pub y_offset: f64,
    /// Whether to center symbol-based markers on the symbol
    pub center_on_symbol: bool,
    /// Text color
    pub color: Color,
    /// Use italic for text
    pub italic: bool,
    /// Use bold for text
    pub bold: bool,
}

impl Default for RepeatSignConfig {
    fn default() -> Self {
        Self {
            text_size: 12.0,
            symbol_scale: 1.0,
            min_distance: 1.0,
            x_offset: 0.0,
            y_offset: 0.0,
            center_on_symbol: true,
            color: Color::BLACK,
            italic: true,
            bold: false,
        }
    }
}

// ============================================================================
// Input Parameters
// ============================================================================

/// Input parameters for marker layout.
#[derive(Debug, Clone)]
pub struct MarkerInput {
    /// Type of marker
    pub marker_type: MarkerType,
    /// X position (measure position)
    pub x: f64,
    /// Staff top Y position
    pub staff_top: f64,
    /// Staff bottom Y position
    pub staff_bottom: f64,
    /// Placement (above/below)
    pub placement: RepeatPlacement,
    /// Unique identifier
    pub id: u64,
}

impl MarkerInput {
    /// Create a new marker input.
    #[must_use]
    pub fn new(marker_type: MarkerType, x: f64, staff_top: f64, staff_bottom: f64) -> Self {
        Self {
            marker_type,
            x,
            staff_top,
            staff_bottom,
            placement: RepeatPlacement::Above,
            id: 0,
        }
    }

    /// Set the placement.
    #[must_use]
    pub fn with_placement(mut self, placement: RepeatPlacement) -> Self {
        self.placement = placement;
        self
    }

    /// Set the ID.
    #[must_use]
    pub fn with_id(mut self, id: u64) -> Self {
        self.id = id;
        self
    }
}

/// Input parameters for jump layout.
#[derive(Debug, Clone)]
pub struct JumpInput {
    /// Type of jump
    pub jump_type: JumpType,
    /// X position (typically end of measure)
    pub x: f64,
    /// Staff top Y position
    pub staff_top: f64,
    /// Staff bottom Y position
    pub staff_bottom: f64,
    /// Placement (above/below)
    pub placement: RepeatPlacement,
    /// Unique identifier
    pub id: u64,
}

impl JumpInput {
    /// Create a new jump input.
    #[must_use]
    pub fn new(jump_type: JumpType, x: f64, staff_top: f64, staff_bottom: f64) -> Self {
        Self {
            jump_type,
            x,
            staff_top,
            staff_bottom,
            placement: RepeatPlacement::Above,
            id: 0,
        }
    }

    /// Set the placement.
    #[must_use]
    pub fn with_placement(mut self, placement: RepeatPlacement) -> Self {
        self.placement = placement;
        self
    }

    /// Set the ID.
    #[must_use]
    pub fn with_id(mut self, id: u64) -> Self {
        self.id = id;
        self
    }
}

// ============================================================================
// Layout Results
// ============================================================================

/// Result of marker layout.
#[derive(Debug, Clone)]
pub struct MarkerLayout {
    /// Position of the marker
    pub position: Point,
    /// Bounding box
    pub bbox: Rect,
    /// Paint commands for rendering
    pub commands: Vec<PaintCommand>,
    /// Scene node
    pub scene: SceneNode,
    /// Label for jump references
    pub label: String,
}

/// Result of jump layout.
#[derive(Debug, Clone)]
pub struct JumpLayout {
    /// Position of the jump
    pub position: Point,
    /// Bounding box
    pub bbox: Rect,
    /// Paint commands for rendering
    pub commands: Vec<PaintCommand>,
    /// Scene node
    pub scene: SceneNode,
    /// Jump destination
    pub jump_to: String,
    /// Play until marker
    pub play_until: String,
    /// Continue at marker
    pub continue_at: String,
}

// ============================================================================
// Layout Functions
// ============================================================================

/// Layout a repeat marker (Segno, Coda, Fine, etc.).
///
/// # Arguments
/// * `input` - Marker input parameters
/// * `spatium` - Staff space height in points
/// * `config` - Configuration options
#[must_use]
pub fn layout_marker(input: &MarkerInput, spatium: f64, config: &RepeatSignConfig) -> MarkerLayout {
    let marker_type = input.marker_type;
    let (glyph_width, glyph_height) = marker_type.glyph_dimensions();

    let width = glyph_width * spatium * config.symbol_scale;
    let height = glyph_height * spatium * config.symbol_scale;

    // Calculate position based on placement
    let x = input.x + config.x_offset * spatium;
    let y = match input.placement {
        RepeatPlacement::Above => {
            input.staff_top - config.min_distance * spatium - height / 2.0
                + config.y_offset * spatium
        }
        RepeatPlacement::Below => {
            input.staff_bottom
                + config.min_distance * spatium
                + height / 2.0
                + config.y_offset * spatium
        }
    };

    let position = Point::new(x, y);

    // Create bounding box
    let bbox = Rect::new(
        x - width / 2.0,
        y - height / 2.0,
        x + width / 2.0,
        y + height / 2.0,
    );

    // Create paint commands
    let mut commands = Vec::new();

    if let Some(glyph) = marker_type.smufl_codepoint() {
        if marker_type == MarkerType::Codetta {
            // Two coda symbols for codetta
            let symbol_width = spatium * config.symbol_scale;
            let offset = symbol_width * 0.6;
            commands.push(PaintCommand::glyph(
                glyph,
                Point::new(x - offset, y),
                spatium * config.symbol_scale,
                config.color,
            ));
            commands.push(PaintCommand::glyph(
                glyph,
                Point::new(x + offset, y),
                spatium * config.symbol_scale,
                config.color,
            ));
        } else {
            commands.push(PaintCommand::glyph(
                glyph,
                position,
                spatium * config.symbol_scale,
                config.color,
            ));
        }
    } else {
        // Text marker (Fine)
        commands.push(PaintCommand::text(
            marker_type.display_text().to_string(),
            "Times New Roman", // Standard music text font
            config.text_size,
            position,
            config.color,
        ));
    }

    // Create scene node
    let mut scene = SceneNode::group(
        SemanticId::new(ElementType::RehearsalMark, input.id)
            .with_attribute("marker-type", format!("{:?}", marker_type))
            .with_attribute("label", marker_type.label()),
    );
    scene.commands = commands.clone();
    scene.bounds = bbox;

    MarkerLayout {
        position,
        bbox,
        commands,
        scene,
        label: marker_type.label().to_string(),
    }
}

/// Layout a repeat jump (D.C., D.S., To Coda, etc.).
///
/// # Arguments
/// * `input` - Jump input parameters
/// * `spatium` - Staff space height in points
/// * `config` - Configuration options
#[must_use]
pub fn layout_jump(input: &JumpInput, spatium: f64, config: &RepeatSignConfig) -> JumpLayout {
    let jump_type = input.jump_type;
    let text = jump_type.display_text();

    // Estimate text dimensions (rough approximation)
    let char_width = config.text_size * 0.5;
    let text_width = text.len() as f64 * char_width;
    let text_height = config.text_size;

    // Add symbol width if present
    let symbol_width = if jump_type.has_symbol() {
        spatium * config.symbol_scale * 1.5
    } else {
        0.0
    };

    let total_width = text_width + symbol_width;

    // Calculate position - jumps are typically right-aligned
    let x = input.x + config.x_offset * spatium;
    let y = match input.placement {
        RepeatPlacement::Above => {
            input.staff_top - config.min_distance * spatium - text_height / 2.0
                + config.y_offset * spatium
        }
        RepeatPlacement::Below => {
            input.staff_bottom
                + config.min_distance * spatium
                + text_height / 2.0
                + config.y_offset * spatium
        }
    };

    let position = Point::new(x, y);

    // Create bounding box
    let bbox = Rect::new(
        x - total_width / 2.0,
        y - text_height / 2.0,
        x + total_width / 2.0,
        y + text_height / 2.0,
    );

    // Create paint commands
    let mut commands = Vec::new();

    // Add text
    let text_x = if jump_type.has_symbol() {
        x - symbol_width / 2.0
    } else {
        x
    };

    commands.push(PaintCommand::text(
        text.to_string(),
        "Times New Roman", // Standard music text font
        config.text_size,
        Point::new(text_x, y),
        config.color,
    ));

    // Add symbol if present
    if let Some(glyph) = jump_type.symbol_codepoint() {
        let symbol_x = text_x + text_width / 2.0 + symbol_width / 2.0;
        commands.push(PaintCommand::glyph(
            glyph,
            Point::new(symbol_x, y),
            spatium * config.symbol_scale,
            config.color,
        ));
    }

    // Create scene node
    let mut scene = SceneNode::group(
        SemanticId::new(ElementType::Text, input.id)
            .with_attribute("jump-type", format!("{:?}", jump_type))
            .with_attribute("jump-to", jump_type.jump_to())
            .with_attribute("play-until", jump_type.play_until()),
    );
    scene.commands = commands.clone();
    scene.bounds = bbox;

    JumpLayout {
        position,
        bbox,
        commands,
        scene,
        jump_to: jump_type.jump_to().to_string(),
        play_until: jump_type.play_until().to_string(),
        continue_at: jump_type.continue_at().to_string(),
    }
}

/// Layout multiple markers.
#[must_use]
pub fn layout_markers(
    inputs: &[MarkerInput],
    spatium: f64,
    config: &RepeatSignConfig,
) -> Vec<MarkerLayout> {
    inputs
        .iter()
        .map(|input| layout_marker(input, spatium, config))
        .collect()
}

/// Layout multiple jumps.
#[must_use]
pub fn layout_jumps(
    inputs: &[JumpInput],
    spatium: f64,
    config: &RepeatSignConfig,
) -> Vec<JumpLayout> {
    inputs
        .iter()
        .map(|input| layout_jump(input, spatium, config))
        .collect()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_marker_type_smufl() {
        assert_eq!(MarkerType::Segno.smufl_codepoint(), Some('\u{E045}'));
        assert_eq!(MarkerType::Coda.smufl_codepoint(), Some('\u{E046}'));
        assert_eq!(MarkerType::Fine.smufl_codepoint(), None);
    }

    #[test]
    fn test_marker_type_labels() {
        assert_eq!(MarkerType::Segno.label(), "segno");
        assert_eq!(MarkerType::Coda.label(), "codab");
        assert_eq!(MarkerType::Fine.label(), "fine");
    }

    #[test]
    fn test_jump_type_text() {
        assert_eq!(JumpType::DaCapo.display_text(), "D.C.");
        assert_eq!(JumpType::DalSegnoAlCoda.display_text(), "D.S. al Coda");
        assert_eq!(JumpType::ToCoda.display_text(), "To Coda");
    }

    #[test]
    fn test_jump_type_destinations() {
        assert_eq!(JumpType::DaCapo.jump_to(), "start");
        assert_eq!(JumpType::DalSegno.jump_to(), "segno");
        assert_eq!(JumpType::DaCapoAlFine.play_until(), "fine");
        assert_eq!(JumpType::DalSegnoAlCoda.continue_at(), "codab");
    }

    #[test]
    fn test_layout_segno_marker() {
        let input = MarkerInput::new(MarkerType::Segno, 100.0, 0.0, 20.0).with_id(1);
        let config = RepeatSignConfig::default();
        let result = layout_marker(&input, 5.0, &config);

        assert!(!result.commands.is_empty());
        assert!(result.bbox.width() > 0.0);
        assert_eq!(result.label, "segno");
        // Above staff, so position should be above staff_top
        assert!(result.position.y < 0.0);
    }

    #[test]
    fn test_layout_coda_marker() {
        let input = MarkerInput::new(MarkerType::Coda, 100.0, 0.0, 20.0).with_id(2);
        let config = RepeatSignConfig::default();
        let result = layout_marker(&input, 5.0, &config);

        assert!(!result.commands.is_empty());
        assert_eq!(result.label, "codab");
    }

    #[test]
    fn test_layout_fine_marker() {
        let input = MarkerInput::new(MarkerType::Fine, 100.0, 0.0, 20.0).with_id(3);
        let config = RepeatSignConfig::default();
        let result = layout_marker(&input, 5.0, &config);

        assert!(!result.commands.is_empty());
        assert_eq!(result.label, "fine");
    }

    #[test]
    fn test_layout_codetta_marker() {
        let input = MarkerInput::new(MarkerType::Codetta, 100.0, 0.0, 20.0).with_id(4);
        let config = RepeatSignConfig::default();
        let result = layout_marker(&input, 5.0, &config);

        // Codetta should have two glyph commands
        assert_eq!(result.commands.len(), 2);
    }

    #[test]
    fn test_layout_dc_jump() {
        let input = JumpInput::new(JumpType::DaCapo, 100.0, 0.0, 20.0).with_id(1);
        let config = RepeatSignConfig::default();
        let result = layout_jump(&input, 5.0, &config);

        assert!(!result.commands.is_empty());
        assert_eq!(result.jump_to, "start");
        assert_eq!(result.play_until, "end");
    }

    #[test]
    fn test_layout_ds_al_coda_jump() {
        let input = JumpInput::new(JumpType::DalSegnoAlCoda, 100.0, 0.0, 20.0).with_id(2);
        let config = RepeatSignConfig::default();
        let result = layout_jump(&input, 5.0, &config);

        assert!(!result.commands.is_empty());
        assert_eq!(result.jump_to, "segno");
        assert_eq!(result.play_until, "coda");
        assert_eq!(result.continue_at, "codab");
    }

    #[test]
    fn test_layout_to_coda_with_symbol() {
        let input = JumpInput::new(JumpType::ToCodaSym, 100.0, 0.0, 20.0).with_id(3);
        let config = RepeatSignConfig::default();
        let result = layout_jump(&input, 5.0, &config);

        // Should have text + symbol
        assert_eq!(result.commands.len(), 2);
    }

    #[test]
    fn test_marker_placement_below() {
        let input = MarkerInput::new(MarkerType::Segno, 100.0, 0.0, 20.0)
            .with_placement(RepeatPlacement::Below)
            .with_id(1);
        let config = RepeatSignConfig::default();
        let result = layout_marker(&input, 5.0, &config);

        // Below staff, so position should be below staff_bottom
        assert!(result.position.y > 20.0);
    }

    #[test]
    fn test_multiple_markers() {
        let inputs = vec![
            MarkerInput::new(MarkerType::Segno, 50.0, 0.0, 20.0).with_id(1),
            MarkerInput::new(MarkerType::Coda, 150.0, 0.0, 20.0).with_id(2),
        ];
        let config = RepeatSignConfig::default();
        let results = layout_markers(&inputs, 5.0, &config);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].label, "segno");
        assert_eq!(results[1].label, "codab");
    }

    #[test]
    fn test_multiple_jumps() {
        let inputs = vec![
            JumpInput::new(JumpType::DaCapoAlCoda, 100.0, 0.0, 20.0).with_id(1),
            JumpInput::new(JumpType::ToCoda, 200.0, 0.0, 20.0).with_id(2),
        ];
        let config = RepeatSignConfig::default();
        let results = layout_jumps(&inputs, 5.0, &config);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].jump_to, "start");
        assert_eq!(results[1].jump_to, "coda");
    }

    #[test]
    fn test_marker_right_marker() {
        assert!(MarkerType::Fine.is_right_marker());
        assert!(!MarkerType::Segno.is_right_marker());
        assert!(!MarkerType::Coda.is_right_marker());
    }

    #[test]
    fn test_config_defaults() {
        let config = RepeatSignConfig::default();
        assert_eq!(config.text_size, 12.0);
        assert_eq!(config.symbol_scale, 1.0);
        assert!(config.italic);
        assert!(config.center_on_symbol);
    }
}
