//! Staff bracket and brace layout module.
//!
//! Implements layout for staff brackets and braces following MuseScore's
//! bracket layout algorithm. Brackets group multiple staves together,
//! indicating they should be read together (e.g., piano grand staff,
//! orchestral sections).
//!
//! Reference: MuseScore `bracket.cpp` and `tlayout.cpp::layoutBracket`

use kurbo::{BezPath, Point, Rect};
use peniko::Color;

use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::{PaintCommand, SceneNode};

/// SMuFL codepoints for bracket and brace symbols.
pub mod smufl {
    /// Brace (standard)
    pub const BRACE: char = '\u{E000}';
    /// Bracket (thin vertical with serifs)
    pub const BRACKET: char = '\u{E002}';
    /// Bracket top (serif/hook at top)
    pub const BRACKET_TOP: char = '\u{E003}';
    /// Bracket bottom (serif/hook at bottom)
    pub const BRACKET_BOTTOM: char = '\u{E004}';

    // Bravura stylistic alternates for different spans
    /// Brace (small) - for single staff span
    pub const BRACE_SMALL: char = '\u{F400}';
    /// Brace (large) - for 3 staff span
    pub const BRACE_LARGE: char = '\u{F401}';
    /// Brace (larger) - for 4+ staff span
    pub const BRACE_LARGER: char = '\u{F402}';
}

/// Type of bracket connecting staves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BracketType {
    /// Curly brace (default for piano grand staff, etc.)
    #[default]
    Brace,
    /// Normal bracket with serifs/hooks at top and bottom
    Normal,
    /// Square bracket (L-shaped at top and bottom)
    Square,
    /// Simple vertical line
    Line,
    /// No bracket (used internally)
    None,
}

impl BracketType {
    /// Get a descriptive name for the bracket type.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Brace => "Brace",
            Self::Normal => "Bracket",
            Self::Square => "Square bracket",
            Self::Line => "Line",
            Self::None => "None",
        }
    }

    /// Whether this bracket type uses a glyph (vs. path-based drawing).
    #[must_use]
    pub const fn uses_glyph(self) -> bool {
        matches!(self, Self::Brace | Self::Normal)
    }
}

/// Configuration for bracket layout.
#[derive(Debug, Clone)]
pub struct BracketConfig {
    /// Width of a brace in spatiums (default: 1.3)
    pub brace_width: f64,
    /// Width of a bracket in spatiums (default: 0.45)
    pub bracket_width: f64,
    /// Distance from bracket to barline in spatiums (default: 0.25)
    pub bracket_distance: f64,
    /// Distance from brace to barline in spatiums (default: 0.4)
    pub brace_distance: f64,
    /// Staff line width for square bracket in spatiums (default: 0.1)
    pub line_width: f64,
    /// Extra extension beyond staff top/bottom in spatiums (default: 0.25)
    pub bracket_extension: f64,
    /// Color for the bracket
    pub color: Color,
    /// Use path-based brace (Emmentaler/Gonville style) instead of glyph
    pub use_path_brace: bool,
}

impl Default for BracketConfig {
    fn default() -> Self {
        Self {
            brace_width: 1.3,
            bracket_width: 0.45,
            bracket_distance: 0.25,
            brace_distance: 0.4,
            line_width: 0.1,
            bracket_extension: 0.25,
            color: Color::BLACK,
            use_path_brace: false,
        }
    }
}

/// Input data for bracket layout.
#[derive(Debug, Clone)]
pub struct BracketInput {
    /// Type of bracket
    pub bracket_type: BracketType,
    /// X position for the bracket (left edge)
    pub x: f64,
    /// Y position of the top of the first staff
    pub first_staff_y: f64,
    /// Y position of the bottom of the last staff
    pub last_staff_y: f64,
    /// Number of staves spanned
    pub staff_span: usize,
    /// Bracket column (for nested brackets, 0 = innermost)
    pub column: usize,
    /// Unique identifier
    pub id: u64,
}

impl BracketInput {
    /// Create a new bracket input.
    #[must_use]
    pub fn new(
        bracket_type: BracketType,
        x: f64,
        first_staff_y: f64,
        last_staff_y: f64,
        staff_span: usize,
    ) -> Self {
        Self {
            bracket_type,
            x,
            first_staff_y,
            last_staff_y,
            staff_span,
            column: 0,
            id: 0,
        }
    }

    /// Set the bracket column.
    #[must_use]
    pub fn with_column(mut self, column: usize) -> Self {
        self.column = column;
        self
    }

    /// Set the ID.
    #[must_use]
    pub fn with_id(mut self, id: u64) -> Self {
        self.id = id;
        self
    }

    /// Get the height of the bracket.
    #[must_use]
    pub fn height(&self) -> f64 {
        self.last_staff_y - self.first_staff_y
    }
}

/// Result of bracket layout.
#[derive(Debug, Clone)]
pub struct BracketLayout {
    /// Bounding box of the bracket
    pub bbox: Rect,
    /// Width of the bracket (including distance to barline)
    pub bracket_width: f64,
    /// Paint commands for rendering
    pub commands: Vec<PaintCommand>,
    /// Scene node for the bracket
    pub scene: SceneNode,
    /// Optional path for path-based brackets (brace, square)
    pub path: Option<BezPath>,
    /// Glyph character if using glyph-based rendering
    pub glyph: Option<char>,
}

/// Select the appropriate brace glyph based on staff span.
///
/// Follows MuseScore's logic for selecting brace size variants.
#[must_use]
pub fn select_brace_glyph(staff_span: usize) -> char {
    match staff_span {
        1 => smufl::BRACE_SMALL,
        2 => smufl::BRACE,
        3 => smufl::BRACE_LARGE,
        _ => smufl::BRACE_LARGER,
    }
}

/// Calculate the magnification factor for a brace based on staff span.
///
/// Based on MuseScore's "magic" formula: `v + ((v - 1) * 1.625)`
/// where 1.625 is derived from akkoladeDistance/4.0.
#[must_use]
pub fn brace_magnification(staff_span: usize) -> f64 {
    let v = staff_span.max(1) as f64;
    v + (v - 1.0) * 1.625
}

/// Create a Bezier path for a brace (Emmentaler/Gonville style).
///
/// This creates the classic curly brace shape using cubic Bezier curves.
/// The path is scaled to fit the given width and height.
#[must_use]
pub fn create_brace_path(width: f64, height: f64) -> BezPath {
    let h2 = height * 0.5;

    // Coordinate transformation macros from MuseScore
    // XM: x coordinate scaled from design space (-700 to 0) to actual width
    // YM: y coordinate scaled from design space (-7100 to 7100) to actual height
    let xm = |a: f64| (a + 700.0) * width / 700.0;
    let ym = |a: f64| (a + 7100.0) * h2 / 7100.0;

    let mut path = BezPath::new();

    // Start at top inner curve
    path.move_to((xm(-8.0), ym(-2048.0)));

    // Upper half (going up and back)
    path.curve_to(
        (xm(-8.0), ym(-3192.0)),
        (xm(-360.0), ym(-4304.0)),
        (xm(-360.0), ym(-5400.0)),
    );
    path.curve_to(
        (xm(-360.0), ym(-5952.0)),
        (xm(-264.0), ym(-6488.0)),
        (xm(32.0), ym(-6968.0)),
    );
    path.curve_to(
        (xm(36.0), ym(-6974.0)),
        (xm(38.0), ym(-6984.0)),
        (xm(38.0), ym(-6990.0)),
    );
    path.curve_to(
        (xm(38.0), ym(-7008.0)),
        (xm(16.0), ym(-7024.0)),
        (xm(0.0), ym(-7024.0)),
    );
    path.curve_to(
        (xm(-8.0), ym(-7024.0)),
        (xm(-22.0), ym(-7022.0)),
        (xm(-32.0), ym(-7008.0)),
    );
    path.curve_to(
        (xm(-416.0), ym(-6392.0)),
        (xm(-544.0), ym(-5680.0)),
        (xm(-544.0), ym(-4960.0)),
    );
    path.curve_to(
        (xm(-544.0), ym(-3800.0)),
        (xm(-168.0), ym(-2680.0)),
        (xm(-168.0), ym(-1568.0)),
    );
    path.curve_to(
        (xm(-168.0), ym(-1016.0)),
        (xm(-264.0), ym(-496.0)),
        (xm(-560.0), ym(-16.0)),
    );

    // Center point
    path.line_to((xm(-560.0), ym(0.0)));
    path.line_to((xm(-560.0), ym(16.0)));

    // Lower half (mirror of upper)
    path.curve_to(
        (xm(-264.0), ym(496.0)),
        (xm(-168.0), ym(1016.0)),
        (xm(-168.0), ym(1568.0)),
    );
    path.curve_to(
        (xm(-544.0), ym(3800.0)),
        (xm(-544.0), ym(3800.0)),
        (xm(-544.0), ym(4960.0)),
    );
    path.curve_to(
        (xm(-544.0), ym(5680.0)),
        (xm(-416.0), ym(6392.0)),
        (xm(-32.0), ym(7008.0)),
    );
    path.curve_to(
        (xm(-22.0), ym(7022.0)),
        (xm(-8.0), ym(7024.0)),
        (xm(0.0), ym(7024.0)),
    );
    path.curve_to(
        (xm(16.0), ym(7024.0)),
        (xm(38.0), ym(7008.0)),
        (xm(38.0), ym(6990.0)),
    );
    path.curve_to(
        (xm(38.0), ym(6984.0)),
        (xm(36.0), ym(6974.0)),
        (xm(32.0), ym(6968.0)),
    );
    path.curve_to(
        (xm(-264.0), ym(6488.0)),
        (xm(-360.0), ym(5952.0)),
        (xm(-360.0), ym(5400.0)),
    );
    path.curve_to(
        (xm(-360.0), ym(4304.0)),
        (xm(-8.0), ym(3192.0)),
        (xm(-8.0), ym(2048.0)),
    );

    // Close back to start via center
    path.curve_to(
        (xm(-8.0), ym(1320.0)),
        (xm(-136.0), ym(624.0)),
        (xm(-512.0), ym(0.0)),
    );
    path.curve_to(
        (xm(-136.0), ym(-624.0)),
        (xm(-8.0), ym(-1320.0)),
        (xm(-8.0), ym(-2048.0)),
    );

    path.close_path();
    path
}

/// Create a square bracket path.
#[must_use]
pub fn create_square_bracket_path(width: f64, height: f64, line_width: f64) -> BezPath {
    let mut path = BezPath::new();
    let hw = line_width * 0.5;

    // Top horizontal
    path.move_to((-hw, -hw));
    path.line_to((width, -hw));
    path.line_to((width, hw));
    path.line_to((hw, hw));

    // Vertical
    path.line_to((hw, height - hw));

    // Bottom horizontal
    path.line_to((width, height - hw));
    path.line_to((width, height + hw));
    path.line_to((-hw, height + hw));

    path.close_path();
    path
}

/// Layout a bracket.
///
/// Positions the bracket to the left of the barline, spanning the given staves.
///
/// # Arguments
/// * `input` - Bracket input data
/// * `spatium` - Staff space height in pixels/points
/// * `config` - Configuration options
///
/// # Returns
/// Layout result with position and rendering commands.
#[must_use]
pub fn layout_bracket(input: &BracketInput, spatium: f64, config: &BracketConfig) -> BracketLayout {
    let height = input.height();
    let extension = config.bracket_extension * spatium;

    match input.bracket_type {
        BracketType::Brace => {
            if config.use_path_brace {
                layout_brace_path(input, spatium, config, height, extension)
            } else {
                layout_brace_glyph(input, spatium, config, height, extension)
            }
        }
        BracketType::Normal => layout_normal_bracket(input, spatium, config, height, extension),
        BracketType::Square => layout_square_bracket(input, spatium, config, height, extension),
        BracketType::Line => layout_line_bracket(input, spatium, config, height, extension),
        BracketType::None => layout_no_bracket(input),
    }
}

/// Layout a brace using a glyph.
fn layout_brace_glyph(
    input: &BracketInput,
    spatium: f64,
    config: &BracketConfig,
    height: f64,
    extension: f64,
) -> BracketLayout {
    let glyph = select_brace_glyph(input.staff_span);
    let width = config.brace_width * spatium;
    let bracket_width = width + config.brace_distance * spatium;

    // Position at center of brace height
    let center_y = input.first_staff_y + height / 2.0;
    let x = input.x - bracket_width;

    // Approximate glyph dimensions (scaled by magnification)
    let mag = brace_magnification(input.staff_span);
    let glyph_height = height + 2.0 * extension;
    let scale = glyph_height / (4.0 * spatium * mag);

    let bbox = Rect::new(
        x,
        input.first_staff_y - extension,
        x + width,
        input.last_staff_y + extension,
    );

    // Create paint command
    let glyph_pos = Point::new(x + width / 2.0, center_y);
    let commands = vec![PaintCommand::glyph(
        glyph,
        glyph_pos,
        spatium * scale,
        config.color,
    )];

    // Create scene node
    let mut scene = SceneNode::group(
        SemanticId::new(ElementType::Custom, input.id)
            .with_attribute("bracket-type", "brace")
            .with_attribute("column", input.column.to_string()),
    );
    scene.commands = commands.clone();
    scene.bounds = bbox;

    BracketLayout {
        bbox,
        bracket_width,
        commands,
        scene,
        path: None,
        glyph: Some(glyph),
    }
}

/// Layout a brace using a Bezier path.
fn layout_brace_path(
    input: &BracketInput,
    spatium: f64,
    config: &BracketConfig,
    height: f64,
    extension: f64,
) -> BracketLayout {
    let width = config.brace_width * spatium;
    let bracket_width = width + config.brace_distance * spatium;

    let path = create_brace_path(width, height + 2.0 * extension);

    let x = input.x - bracket_width;
    let center_y = input.first_staff_y + height / 2.0;

    let bbox = Rect::new(
        x,
        input.first_staff_y - extension,
        x + width,
        input.last_staff_y + extension,
    );

    // Translate path to position
    let translated_path = kurbo::Affine::translate((x + width, center_y)) * path.clone();

    let commands = vec![PaintCommand::filled_path(
        translated_path.clone(),
        config.color,
    )];

    let mut scene = SceneNode::group(
        SemanticId::new(ElementType::Custom, input.id)
            .with_attribute("bracket-type", "brace-path")
            .with_attribute("column", input.column.to_string()),
    );
    scene.commands = commands.clone();
    scene.bounds = bbox;

    BracketLayout {
        bbox,
        bracket_width,
        commands,
        scene,
        path: Some(translated_path),
        glyph: None,
    }
}

/// Layout a normal bracket with serifs.
fn layout_normal_bracket(
    input: &BracketInput,
    spatium: f64,
    config: &BracketConfig,
    _height: f64,
    extension: f64,
) -> BracketLayout {
    let width = config.bracket_width * spatium;
    let bracket_width = width + config.bracket_distance * spatium;

    let x = input.x - bracket_width;
    let top_y = input.first_staff_y - extension;
    let bottom_y = input.last_staff_y + extension;

    let bbox = Rect::new(x, top_y, x + width, bottom_y);

    let mut commands = Vec::new();

    // Top serif glyph
    let top_pos = Point::new(x + width / 2.0, top_y);
    commands.push(PaintCommand::glyph(
        smufl::BRACKET_TOP,
        top_pos,
        spatium,
        config.color,
    ));

    // Bottom serif glyph
    let bottom_pos = Point::new(x + width / 2.0, bottom_y);
    commands.push(PaintCommand::glyph(
        smufl::BRACKET_BOTTOM,
        bottom_pos,
        spatium,
        config.color,
    ));

    // Vertical line
    let line_path = {
        let mut path = BezPath::new();
        let hw = config.line_width * spatium * 0.5;
        path.move_to((x + width / 2.0 - hw, top_y));
        path.line_to((x + width / 2.0 + hw, top_y));
        path.line_to((x + width / 2.0 + hw, bottom_y));
        path.line_to((x + width / 2.0 - hw, bottom_y));
        path.close_path();
        path
    };
    commands.push(PaintCommand::filled_path(line_path, config.color));

    let mut scene = SceneNode::group(
        SemanticId::new(ElementType::Custom, input.id)
            .with_attribute("bracket-type", "normal")
            .with_attribute("column", input.column.to_string()),
    );
    scene.commands = commands.clone();
    scene.bounds = bbox;

    BracketLayout {
        bbox,
        bracket_width,
        commands,
        scene,
        path: None,
        glyph: None,
    }
}

/// Layout a square bracket.
fn layout_square_bracket(
    input: &BracketInput,
    spatium: f64,
    config: &BracketConfig,
    height: f64,
    extension: f64,
) -> BracketLayout {
    let line_width = config.line_width * spatium;
    let hook_width = 0.5 * spatium + 3.0 * line_width;
    let bracket_width = hook_width + config.bracket_distance * spatium;

    let x = input.x - bracket_width;
    let top_y = input.first_staff_y - extension;
    let bottom_y = input.last_staff_y + extension;

    let bbox = Rect::new(
        x,
        top_y - line_width / 2.0,
        x + hook_width,
        bottom_y + line_width / 2.0,
    );

    // Create square bracket path
    let path = create_square_bracket_path(hook_width, height + 2.0 * extension, line_width);
    let translated_path = kurbo::Affine::translate((x, top_y)) * path;

    let commands = vec![PaintCommand::filled_path(
        translated_path.clone(),
        config.color,
    )];

    let mut scene = SceneNode::group(
        SemanticId::new(ElementType::Custom, input.id)
            .with_attribute("bracket-type", "square")
            .with_attribute("column", input.column.to_string()),
    );
    scene.commands = commands.clone();
    scene.bounds = bbox;

    BracketLayout {
        bbox,
        bracket_width,
        commands,
        scene,
        path: Some(translated_path),
        glyph: None,
    }
}

/// Layout a line bracket.
fn layout_line_bracket(
    input: &BracketInput,
    spatium: f64,
    config: &BracketConfig,
    _height: f64,
    extension: f64,
) -> BracketLayout {
    let line_width = 0.67 * config.bracket_width * spatium;
    let bracket_width = line_width + config.bracket_distance * spatium;

    let x = input.x - bracket_width;
    let top_y = input.first_staff_y - extension;
    let bottom_y = input.last_staff_y + extension;

    let bbox = Rect::new(x, top_y, x + line_width, bottom_y);

    // Simple vertical line
    let line_path = {
        let mut path = BezPath::new();
        path.move_to((x, top_y));
        path.line_to((x + line_width, top_y));
        path.line_to((x + line_width, bottom_y));
        path.line_to((x, bottom_y));
        path.close_path();
        path
    };

    let commands = vec![PaintCommand::filled_path(line_path.clone(), config.color)];

    let mut scene = SceneNode::group(
        SemanticId::new(ElementType::Custom, input.id)
            .with_attribute("bracket-type", "line")
            .with_attribute("column", input.column.to_string()),
    );
    scene.commands = commands.clone();
    scene.bounds = bbox;

    BracketLayout {
        bbox,
        bracket_width,
        commands,
        scene,
        path: Some(line_path),
        glyph: None,
    }
}

/// Layout for no bracket (placeholder).
fn layout_no_bracket(input: &BracketInput) -> BracketLayout {
    let bbox = Rect::new(input.x, input.first_staff_y, input.x, input.last_staff_y);

    BracketLayout {
        bbox,
        bracket_width: 0.0,
        commands: Vec::new(),
        scene: SceneNode::group(SemanticId::new(ElementType::Custom, input.id)),
        path: None,
        glyph: None,
    }
}

/// Layout multiple brackets for a system.
///
/// Handles nested brackets by offsetting each column appropriately.
#[must_use]
pub fn layout_brackets(
    inputs: &[BracketInput],
    spatium: f64,
    config: &BracketConfig,
) -> Vec<BracketLayout> {
    inputs
        .iter()
        .map(|input| layout_bracket(input, spatium, config))
        .collect()
}

/// Calculate total width for all bracket columns.
///
/// Given multiple nested brackets, calculates the total horizontal space needed.
#[must_use]
pub fn total_brackets_width(inputs: &[BracketInput], spatium: f64, config: &BracketConfig) -> f64 {
    inputs
        .iter()
        .map(|input| layout_bracket(input, spatium, config).bracket_width)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape;

    #[test]
    fn test_bracket_type_names() {
        assert_eq!(BracketType::Brace.name(), "Brace");
        assert_eq!(BracketType::Normal.name(), "Bracket");
        assert_eq!(BracketType::Square.name(), "Square bracket");
        assert_eq!(BracketType::Line.name(), "Line");
    }

    #[test]
    fn test_bracket_type_uses_glyph() {
        assert!(BracketType::Brace.uses_glyph());
        assert!(BracketType::Normal.uses_glyph());
        assert!(!BracketType::Square.uses_glyph());
        assert!(!BracketType::Line.uses_glyph());
    }

    #[test]
    fn test_select_brace_glyph() {
        assert_eq!(select_brace_glyph(1), smufl::BRACE_SMALL);
        assert_eq!(select_brace_glyph(2), smufl::BRACE);
        assert_eq!(select_brace_glyph(3), smufl::BRACE_LARGE);
        assert_eq!(select_brace_glyph(4), smufl::BRACE_LARGER);
        assert_eq!(select_brace_glyph(10), smufl::BRACE_LARGER);
    }

    #[test]
    fn test_brace_magnification() {
        // For 1 staff: 1 + (0 * 1.625) = 1.0
        assert!((brace_magnification(1) - 1.0).abs() < 0.01);
        // For 2 staves: 2 + (1 * 1.625) = 3.625
        assert!((brace_magnification(2) - 3.625).abs() < 0.01);
        // For 3 staves: 3 + (2 * 1.625) = 6.25
        assert!((brace_magnification(3) - 6.25).abs() < 0.01);
    }

    #[test]
    fn test_brace_path_creation() {
        let path = create_brace_path(10.0, 100.0);
        let bounds = path.bounding_box();

        // Path should have non-zero bounds
        assert!(bounds.width() > 0.0);
        assert!(bounds.height() > 0.0);
    }

    #[test]
    fn test_square_bracket_path() {
        let path = create_square_bracket_path(5.0, 50.0, 1.0);
        let bounds = path.bounding_box();

        // Should span the height plus line width
        assert!(bounds.height() >= 50.0);
        assert!(bounds.width() > 0.0);
    }

    #[test]
    fn test_layout_brace() {
        let input = BracketInput::new(BracketType::Brace, 100.0, 0.0, 80.0, 2);
        let config = BracketConfig::default();
        let result = layout_bracket(&input, 5.0, &config);

        // Should have a glyph
        assert!(result.glyph.is_some());
        // Should have non-zero width
        assert!(result.bracket_width > 0.0);
        // Should have commands
        assert!(!result.commands.is_empty());
        // Bounding box should span the staves
        assert!(result.bbox.height() >= 80.0);
    }

    #[test]
    fn test_layout_brace_path() {
        let input = BracketInput::new(BracketType::Brace, 100.0, 0.0, 80.0, 2);
        let config = BracketConfig {
            use_path_brace: true,
            ..Default::default()
        };
        let result = layout_bracket(&input, 5.0, &config);

        // Should have a path, not glyph
        assert!(result.path.is_some());
        assert!(result.glyph.is_none());
    }

    #[test]
    fn test_layout_normal_bracket() {
        let input = BracketInput::new(BracketType::Normal, 100.0, 0.0, 80.0, 2);
        let config = BracketConfig::default();
        let result = layout_bracket(&input, 5.0, &config);

        // Normal bracket uses serif glyphs plus a line
        assert!(result.bracket_width > 0.0);
        // Should have multiple commands (top glyph, bottom glyph, line)
        assert!(result.commands.len() >= 2);
    }

    #[test]
    fn test_layout_square_bracket() {
        let input = BracketInput::new(BracketType::Square, 100.0, 0.0, 80.0, 2);
        let config = BracketConfig::default();
        let result = layout_bracket(&input, 5.0, &config);

        // Square bracket uses a path
        assert!(result.path.is_some());
        assert!(result.bracket_width > 0.0);
    }

    #[test]
    fn test_layout_line_bracket() {
        let input = BracketInput::new(BracketType::Line, 100.0, 0.0, 80.0, 2);
        let config = BracketConfig::default();
        let result = layout_bracket(&input, 5.0, &config);

        // Line bracket is simple
        assert!(result.path.is_some());
        // Should be thinner than other brackets
        assert!(result.bracket_width < config.bracket_width * 5.0 + config.bracket_distance * 5.0);
    }

    #[test]
    fn test_layout_no_bracket() {
        let input = BracketInput::new(BracketType::None, 100.0, 0.0, 80.0, 2);
        let config = BracketConfig::default();
        let result = layout_bracket(&input, 5.0, &config);

        // No bracket has zero width and no commands
        assert!((result.bracket_width - 0.0).abs() < 0.001);
        assert!(result.commands.is_empty());
    }

    #[test]
    fn test_bracket_input_height() {
        let input = BracketInput::new(BracketType::Brace, 100.0, 10.0, 90.0, 2);
        assert!((input.height() - 80.0).abs() < 0.01);
    }

    #[test]
    fn test_bracket_input_with_column() {
        let input = BracketInput::new(BracketType::Brace, 100.0, 0.0, 80.0, 2).with_column(1);
        assert_eq!(input.column, 1);
    }

    #[test]
    fn test_multiple_brackets() {
        let inputs = vec![
            BracketInput::new(BracketType::Brace, 100.0, 0.0, 40.0, 1).with_id(1),
            BracketInput::new(BracketType::Normal, 100.0, 0.0, 80.0, 2)
                .with_column(1)
                .with_id(2),
        ];

        let config = BracketConfig::default();
        let results = layout_brackets(&inputs, 5.0, &config);

        assert_eq!(results.len(), 2);
        // Each should have valid output
        for result in &results {
            assert!(result.bracket_width > 0.0);
        }
    }

    #[test]
    fn test_total_brackets_width() {
        let inputs = vec![
            BracketInput::new(BracketType::Brace, 100.0, 0.0, 80.0, 2),
            BracketInput::new(BracketType::Normal, 100.0, 0.0, 80.0, 2),
        ];

        let config = BracketConfig::default();
        let total_width = total_brackets_width(&inputs, 5.0, &config);

        // Total should be sum of individual widths
        let individual_sum: f64 = inputs
            .iter()
            .map(|i| layout_bracket(i, 5.0, &config).bracket_width)
            .sum();
        assert!((total_width - individual_sum).abs() < 0.01);
    }

    #[test]
    fn test_smufl_codepoints() {
        // Verify SMuFL codepoints match expected values
        assert_eq!(smufl::BRACE, '\u{E000}');
        assert_eq!(smufl::BRACKET, '\u{E002}');
        assert_eq!(smufl::BRACKET_TOP, '\u{E003}');
        assert_eq!(smufl::BRACKET_BOTTOM, '\u{E004}');
        assert_eq!(smufl::BRACE_SMALL, '\u{F400}');
        assert_eq!(smufl::BRACE_LARGE, '\u{F401}');
        assert_eq!(smufl::BRACE_LARGER, '\u{F402}');
    }

    #[test]
    fn test_bracket_with_extension() {
        let input = BracketInput::new(BracketType::Normal, 100.0, 10.0, 50.0, 2);
        let config = BracketConfig {
            bracket_extension: 1.0, // 1 spatium extension
            ..Default::default()
        };

        let result = layout_bracket(&input, 5.0, &config);

        // Bounding box should extend beyond staff positions
        assert!(result.bbox.y0 < 10.0);
        assert!(result.bbox.y1 > 50.0);
    }
}
