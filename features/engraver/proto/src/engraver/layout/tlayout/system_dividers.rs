//! System divider layout module.
//!
//! Implements layout for system dividers (wing symbols) that appear between
//! systems to provide visual separation. Common in orchestral scores and
//! large ensemble music.
//!
//! Reference: MuseScore `systemdivider.cpp` and `tlayout.cpp`

use kurbo::{Point, Rect};
use peniko::Color;

use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::{PaintCommand, SceneNode};

/// SMuFL codepoints for system divider symbols.
pub mod smufl {
    /// Standard system divider (short wings)
    pub const SYSTEM_DIVIDER: char = '\u{E007}';
    /// Long system divider
    pub const SYSTEM_DIVIDER_LONG: char = '\u{E008}';
    /// Extra long system divider
    pub const SYSTEM_DIVIDER_EXTRA_LONG: char = '\u{E009}';
}

/// Type/style of system divider.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum SystemDividerStyle {
    /// Standard short wings
    #[default]
    Normal,
    /// Long wings
    Long,
    /// Extra long wings
    ExtraLong,
}

impl SystemDividerStyle {
    /// Get the SMuFL codepoint for this divider style.
    #[must_use]
    pub const fn codepoint(self) -> char {
        match self {
            Self::Normal => smufl::SYSTEM_DIVIDER,
            Self::Long => smufl::SYSTEM_DIVIDER_LONG,
            Self::ExtraLong => smufl::SYSTEM_DIVIDER_EXTRA_LONG,
        }
    }

    /// Get approximate glyph dimensions (width, height) in spatiums.
    #[must_use]
    pub const fn glyph_dimensions(self) -> (f64, f64) {
        match self {
            Self::Normal => (2.0, 4.0),
            Self::Long => (3.0, 4.0),
            Self::ExtraLong => (4.0, 4.0),
        }
    }

    /// Get the style name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Long => "Long",
            Self::ExtraLong => "Extra Long",
        }
    }
}

/// Side where the system divider appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DividerSide {
    /// Left side of the system
    #[default]
    Left,
    /// Right side of the system
    Right,
}

impl DividerSide {
    /// Get the side name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Left => "Left",
            Self::Right => "Right",
        }
    }
}

/// Configuration for system dividers.
#[derive(Debug, Clone)]
pub struct SystemDividerConfig {
    /// Whether to show left divider
    pub show_left: bool,
    /// Whether to show right divider
    pub show_right: bool,
    /// Style for left divider
    pub left_style: SystemDividerStyle,
    /// Style for right divider
    pub right_style: SystemDividerStyle,
    /// Size multiplier for left divider (default: 1.0)
    pub left_size: f64,
    /// Size multiplier for right divider (default: 1.0)
    pub right_size: f64,
    /// X offset for left divider in spatiums
    pub left_x_offset: f64,
    /// Y offset for left divider in spatiums
    pub left_y_offset: f64,
    /// X offset for right divider in spatiums
    pub right_x_offset: f64,
    /// Y offset for right divider in spatiums
    pub right_y_offset: f64,
    /// Color for dividers
    pub color: Color,
}

impl Default for SystemDividerConfig {
    fn default() -> Self {
        Self {
            show_left: true,
            show_right: true,
            left_style: SystemDividerStyle::Normal,
            right_style: SystemDividerStyle::Normal,
            left_size: 1.0,
            right_size: 1.0,
            left_x_offset: 0.0,
            left_y_offset: 0.0,
            right_x_offset: 0.0,
            right_y_offset: 0.0,
            color: Color::BLACK,
        }
    }
}

impl SystemDividerConfig {
    /// Create config showing only left divider.
    #[must_use]
    pub fn left_only() -> Self {
        Self {
            show_left: true,
            show_right: false,
            ..Default::default()
        }
    }

    /// Create config showing only right divider.
    #[must_use]
    pub fn right_only() -> Self {
        Self {
            show_left: false,
            show_right: true,
            ..Default::default()
        }
    }

    /// Create config with no dividers (disabled).
    #[must_use]
    pub fn none() -> Self {
        Self {
            show_left: false,
            show_right: false,
            ..Default::default()
        }
    }

    /// Set the style for both dividers.
    #[must_use]
    pub fn with_style(mut self, style: SystemDividerStyle) -> Self {
        self.left_style = style;
        self.right_style = style;
        self
    }

    /// Set the size for both dividers.
    #[must_use]
    pub fn with_size(mut self, size: f64) -> Self {
        self.left_size = size;
        self.right_size = size;
        self
    }
}

/// Input data for system divider layout.
#[derive(Debug, Clone)]
pub struct SystemDividerInput {
    /// Side of the system (left or right)
    pub side: DividerSide,
    /// Style of the divider
    pub style: SystemDividerStyle,
    /// X position (left edge for left divider, right edge for right divider)
    pub x: f64,
    /// Y position (vertical center between systems)
    pub y: f64,
    /// Size multiplier
    pub size: f64,
    /// X offset in spatiums
    pub x_offset: f64,
    /// Y offset in spatiums
    pub y_offset: f64,
    /// Unique identifier
    pub id: u64,
}

impl SystemDividerInput {
    /// Create a new system divider input.
    #[must_use]
    pub fn new(side: DividerSide, x: f64, y: f64) -> Self {
        Self {
            side,
            style: SystemDividerStyle::Normal,
            x,
            y,
            size: 1.0,
            x_offset: 0.0,
            y_offset: 0.0,
            id: 0,
        }
    }

    /// Set the style.
    #[must_use]
    pub fn with_style(mut self, style: SystemDividerStyle) -> Self {
        self.style = style;
        self
    }

    /// Set the size multiplier.
    #[must_use]
    pub fn with_size(mut self, size: f64) -> Self {
        self.size = size;
        self
    }

    /// Set the ID.
    #[must_use]
    pub fn with_id(mut self, id: u64) -> Self {
        self.id = id;
        self
    }

    /// Set offsets.
    #[must_use]
    pub fn with_offset(mut self, x_offset: f64, y_offset: f64) -> Self {
        self.x_offset = x_offset;
        self.y_offset = y_offset;
        self
    }
}

/// Result of system divider layout.
#[derive(Debug, Clone)]
pub struct SystemDividerLayout {
    /// Position of the divider (glyph origin)
    pub position: Point,
    /// Bounding box of the divider
    pub bbox: Rect,
    /// SMuFL codepoint for the glyph
    pub glyph: char,
    /// Paint commands for rendering
    pub commands: Vec<PaintCommand>,
    /// Scene node for the divider
    pub scene: SceneNode,
    /// Side of the divider
    pub side: DividerSide,
}

/// Layout a single system divider.
///
/// # Arguments
/// * `input` - Divider input data
/// * `spatium` - Staff space height in pixels/points
/// * `color` - Color for the divider
///
/// # Returns
/// Layout result with position and rendering commands.
#[must_use]
pub fn layout_system_divider(
    input: &SystemDividerInput,
    spatium: f64,
    color: Color,
) -> SystemDividerLayout {
    let glyph = input.style.codepoint();
    let (glyph_width, glyph_height) = input.style.glyph_dimensions();

    let width = glyph_width * spatium * input.size;
    let height = glyph_height * spatium * input.size;

    // Calculate position with offsets
    let x_offset = input.x_offset * spatium;
    let y_offset = input.y_offset * spatium;

    // Position based on side
    let x = match input.side {
        DividerSide::Left => input.x + x_offset,
        DividerSide::Right => input.x - width + x_offset,
    };
    let y = input.y + y_offset;

    let position = Point::new(x + width / 2.0, y);

    let bbox = Rect::new(x, y - height / 2.0, x + width, y + height / 2.0);

    // Create paint command for the glyph
    let commands = vec![PaintCommand::glyph(
        glyph,
        position,
        spatium * input.size,
        color,
    )];

    // Create scene node
    let mut scene = SceneNode::group(
        SemanticId::new(ElementType::Custom, input.id)
            .with_attribute("element-type", "system-divider")
            .with_attribute("side", input.side.name()),
    );
    scene.commands = commands.clone();
    scene.bounds = bbox;

    SystemDividerLayout {
        position,
        bbox,
        glyph,
        commands,
        scene,
        side: input.side,
    }
}

/// Layout system dividers for a system gap.
///
/// Creates dividers for both left and right sides based on configuration.
///
/// # Arguments
/// * `left_x` - X position of the left margin
/// * `right_x` - X position of the right margin
/// * `y` - Y position (vertical center between systems)
/// * `spatium` - Staff space height
/// * `config` - Configuration for dividers
/// * `base_id` - Base ID for the dividers (left gets base_id, right gets base_id + 1)
///
/// # Returns
/// Vector of layout results (0-2 dividers based on configuration).
#[must_use]
pub fn layout_system_dividers(
    left_x: f64,
    right_x: f64,
    y: f64,
    spatium: f64,
    config: &SystemDividerConfig,
    base_id: u64,
) -> Vec<SystemDividerLayout> {
    let mut results = Vec::with_capacity(2);

    if config.show_left {
        let input = SystemDividerInput::new(DividerSide::Left, left_x, y)
            .with_style(config.left_style)
            .with_size(config.left_size)
            .with_offset(config.left_x_offset, config.left_y_offset)
            .with_id(base_id);

        results.push(layout_system_divider(&input, spatium, config.color));
    }

    if config.show_right {
        let input = SystemDividerInput::new(DividerSide::Right, right_x, y)
            .with_style(config.right_style)
            .with_size(config.right_size)
            .with_offset(config.right_x_offset, config.right_y_offset)
            .with_id(base_id + 1);

        results.push(layout_system_divider(&input, spatium, config.color));
    }

    results
}

/// Calculate total width needed for system dividers.
///
/// Returns the maximum width of configured dividers (used for margin calculations).
#[must_use]
pub fn system_divider_width(config: &SystemDividerConfig, spatium: f64) -> (f64, f64) {
    let left_width = if config.show_left {
        let (w, _) = config.left_style.glyph_dimensions();
        w * spatium * config.left_size
    } else {
        0.0
    };

    let right_width = if config.show_right {
        let (w, _) = config.right_style.glyph_dimensions();
        w * spatium * config.right_size
    } else {
        0.0
    };

    (left_width, right_width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_divider_style_codepoints() {
        assert_eq!(SystemDividerStyle::Normal.codepoint(), '\u{E007}');
        assert_eq!(SystemDividerStyle::Long.codepoint(), '\u{E008}');
        assert_eq!(SystemDividerStyle::ExtraLong.codepoint(), '\u{E009}');
    }

    #[test]
    fn test_divider_style_names() {
        assert_eq!(SystemDividerStyle::Normal.name(), "Normal");
        assert_eq!(SystemDividerStyle::Long.name(), "Long");
        assert_eq!(SystemDividerStyle::ExtraLong.name(), "Extra Long");
    }

    #[test]
    fn test_divider_side_names() {
        assert_eq!(DividerSide::Left.name(), "Left");
        assert_eq!(DividerSide::Right.name(), "Right");
    }

    #[test]
    fn test_config_defaults() {
        let config = SystemDividerConfig::default();
        assert!(config.show_left);
        assert!(config.show_right);
        assert_eq!(config.left_style, SystemDividerStyle::Normal);
        assert_eq!(config.right_style, SystemDividerStyle::Normal);
    }

    #[test]
    fn test_config_left_only() {
        let config = SystemDividerConfig::left_only();
        assert!(config.show_left);
        assert!(!config.show_right);
    }

    #[test]
    fn test_config_right_only() {
        let config = SystemDividerConfig::right_only();
        assert!(!config.show_left);
        assert!(config.show_right);
    }

    #[test]
    fn test_config_none() {
        let config = SystemDividerConfig::none();
        assert!(!config.show_left);
        assert!(!config.show_right);
    }

    #[test]
    fn test_config_with_style() {
        let config = SystemDividerConfig::default().with_style(SystemDividerStyle::Long);
        assert_eq!(config.left_style, SystemDividerStyle::Long);
        assert_eq!(config.right_style, SystemDividerStyle::Long);
    }

    #[test]
    fn test_layout_left_divider() {
        let input = SystemDividerInput::new(DividerSide::Left, 10.0, 100.0);
        let result = layout_system_divider(&input, 5.0, Color::BLACK);

        assert_eq!(result.side, DividerSide::Left);
        assert_eq!(result.glyph, smufl::SYSTEM_DIVIDER);
        assert!(!result.commands.is_empty());
        // Left divider should be positioned at or after left_x
        assert!(result.bbox.x0 >= 10.0);
    }

    #[test]
    fn test_layout_right_divider() {
        let input = SystemDividerInput::new(DividerSide::Right, 500.0, 100.0);
        let result = layout_system_divider(&input, 5.0, Color::BLACK);

        assert_eq!(result.side, DividerSide::Right);
        // Right divider should be positioned at or before right_x
        assert!(result.bbox.x1 <= 500.0);
    }

    #[test]
    fn test_layout_divider_with_style() {
        let input = SystemDividerInput::new(DividerSide::Left, 10.0, 100.0)
            .with_style(SystemDividerStyle::ExtraLong);
        let result = layout_system_divider(&input, 5.0, Color::BLACK);

        assert_eq!(result.glyph, smufl::SYSTEM_DIVIDER_EXTRA_LONG);
    }

    #[test]
    fn test_layout_divider_with_size() {
        let input_normal = SystemDividerInput::new(DividerSide::Left, 10.0, 100.0);
        let input_large = SystemDividerInput::new(DividerSide::Left, 10.0, 100.0).with_size(2.0);

        let result_normal = layout_system_divider(&input_normal, 5.0, Color::BLACK);
        let result_large = layout_system_divider(&input_large, 5.0, Color::BLACK);

        // Larger size should result in larger bounding box
        assert!(result_large.bbox.width() > result_normal.bbox.width());
        assert!(result_large.bbox.height() > result_normal.bbox.height());
    }

    #[test]
    fn test_layout_divider_with_offset() {
        let input_no_offset = SystemDividerInput::new(DividerSide::Left, 10.0, 100.0);
        let input_with_offset =
            SystemDividerInput::new(DividerSide::Left, 10.0, 100.0).with_offset(2.0, 1.0);

        let result_no_offset = layout_system_divider(&input_no_offset, 5.0, Color::BLACK);
        let result_with_offset = layout_system_divider(&input_with_offset, 5.0, Color::BLACK);

        // X should be offset by 2 spatiums (10 units at spatium=5)
        assert!((result_with_offset.position.x - result_no_offset.position.x - 10.0).abs() < 0.01);
        // Y should be offset by 1 spatium (5 units at spatium=5)
        assert!((result_with_offset.position.y - result_no_offset.position.y - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_layout_system_dividers_both() {
        let config = SystemDividerConfig::default();
        let results = layout_system_dividers(10.0, 500.0, 100.0, 5.0, &config, 1);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].side, DividerSide::Left);
        assert_eq!(results[1].side, DividerSide::Right);
    }

    #[test]
    fn test_layout_system_dividers_left_only() {
        let config = SystemDividerConfig::left_only();
        let results = layout_system_dividers(10.0, 500.0, 100.0, 5.0, &config, 1);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].side, DividerSide::Left);
    }

    #[test]
    fn test_layout_system_dividers_none() {
        let config = SystemDividerConfig::none();
        let results = layout_system_dividers(10.0, 500.0, 100.0, 5.0, &config, 1);

        assert!(results.is_empty());
    }

    #[test]
    fn test_system_divider_width() {
        let config = SystemDividerConfig::default();
        let (left_width, right_width) = system_divider_width(&config, 5.0);

        // Both should have non-zero width
        assert!(left_width > 0.0);
        assert!(right_width > 0.0);
    }

    #[test]
    fn test_system_divider_width_none() {
        let config = SystemDividerConfig::none();
        let (left_width, right_width) = system_divider_width(&config, 5.0);

        assert!((left_width - 0.0).abs() < 0.01);
        assert!((right_width - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_smufl_codepoints() {
        assert_eq!(smufl::SYSTEM_DIVIDER, '\u{E007}');
        assert_eq!(smufl::SYSTEM_DIVIDER_LONG, '\u{E008}');
        assert_eq!(smufl::SYSTEM_DIVIDER_EXTRA_LONG, '\u{E009}');
    }
}
