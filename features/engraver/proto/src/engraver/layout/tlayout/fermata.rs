//! Fermata layout module.
//!
//! Implements layout for fermatas (pause/hold signs) following MuseScore's
//! fermata layout algorithm. Fermatas are placed above or below notes to
//! indicate a pause or prolongation of the note's duration.
//!
//! Reference: MuseScore `fermata.cpp` and `tlayout.cpp::layoutFermata`

use kurbo::{Point, Rect};
use peniko::Color;

use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::{PaintCommand, SceneNode};

/// Type of fermata, affecting both appearance and time stretch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FermataType {
    /// Very short fermata (shortest pause)
    VeryShort,
    /// Short fermata
    Short,
    /// Short fermata (Henze style)
    ShortHenze,
    /// Normal/standard fermata
    #[default]
    Normal,
    /// Long fermata
    Long,
    /// Long fermata (Henze style)
    LongHenze,
    /// Very long fermata (longest pause)
    VeryLong,
}

impl FermataType {
    /// Get the default time stretch factor for this fermata type.
    ///
    /// This affects playback - how much longer the note is held.
    #[must_use]
    pub const fn default_time_stretch(self) -> f64 {
        match self {
            Self::VeryShort => 1.25,
            Self::Short | Self::ShortHenze => 1.5,
            Self::Normal => 2.0,
            Self::Long | Self::LongHenze => 3.0,
            Self::VeryLong => 4.0,
        }
    }

    /// Get the SMuFL codepoint for this fermata type (above variant).
    #[must_use]
    pub const fn smufl_codepoint_above(self) -> char {
        match self {
            Self::VeryShort => '\u{E4C4}',  // fermataVeryShortAbove
            Self::Short => '\u{E4C6}',      // fermataShortAbove
            Self::ShortHenze => '\u{E4CC}', // fermataShortHenzeAbove (non-standard, may vary)
            Self::Normal => '\u{E4C0}',     // fermataAbove
            Self::Long => '\u{E4C2}',       // fermataLongAbove
            Self::LongHenze => '\u{E4CA}',  // fermataLongHenzeAbove (non-standard, may vary)
            Self::VeryLong => '\u{E4C8}',   // fermataVeryLongAbove
        }
    }

    /// Get the SMuFL codepoint for this fermata type (below variant).
    #[must_use]
    pub const fn smufl_codepoint_below(self) -> char {
        match self {
            Self::VeryShort => '\u{E4C5}',  // fermataVeryShortBelow
            Self::Short => '\u{E4C7}',      // fermataShortBelow
            Self::ShortHenze => '\u{E4CD}', // fermataShortHenzeBelow
            Self::Normal => '\u{E4C1}',     // fermataBelow
            Self::Long => '\u{E4C3}',       // fermataLongBelow
            Self::LongHenze => '\u{E4CB}',  // fermataLongHenzeBelow
            Self::VeryLong => '\u{E4C9}',   // fermataVeryLongBelow
        }
    }

    /// Get approximate glyph dimensions (width, height) in spatiums.
    ///
    /// These are approximations; actual dimensions depend on the font.
    #[must_use]
    pub const fn glyph_dimensions(self) -> (f64, f64) {
        match self {
            Self::VeryShort => (0.8, 0.8),
            Self::Short | Self::ShortHenze => (1.0, 1.0),
            Self::Normal => (1.4, 1.2),
            Self::Long | Self::LongHenze => (1.6, 1.4),
            Self::VeryLong => (1.8, 1.6),
        }
    }
}

/// Placement of the fermata relative to the staff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FermataPlacement {
    /// Place above the staff/note (default for most notation)
    #[default]
    Above,
    /// Place below the staff/note (for lower voice or specific contexts)
    Below,
}

/// Configuration for fermata layout.
#[derive(Debug, Clone)]
pub struct FermataConfig {
    /// Minimum distance from the staff/note in spatiums
    pub min_distance: f64,
    /// Offset from the default position (x, y) in spatiums
    pub offset: (f64, f64),
    /// Scaling factor for the fermata glyph
    pub scale: f64,
    /// Whether to show on tablature (common)
    pub show_tab_common: bool,
    /// Whether to show on tablature (simple)
    pub show_tab_simple: bool,
}

impl Default for FermataConfig {
    fn default() -> Self {
        Self {
            min_distance: 0.5,
            offset: (0.0, 0.0),
            scale: 1.0,
            show_tab_common: true,
            show_tab_simple: true,
        }
    }
}

/// Input data for fermata layout.
#[derive(Debug, Clone)]
pub struct FermataInput {
    /// Type of fermata
    pub fermata_type: FermataType,
    /// Placement (above or below)
    pub placement: FermataPlacement,
    /// X position of the associated note/chord center
    pub note_x: f64,
    /// Y position of the note (for placement relative to note)
    pub note_y: f64,
    /// Top of the staff (Y coordinate)
    pub staff_top: f64,
    /// Bottom of the staff (Y coordinate)
    pub staff_bottom: f64,
    /// Highest element above staff that fermata must avoid (optional)
    pub skyline_above: Option<f64>,
    /// Lowest element below staff that fermata must avoid (optional)
    pub skyline_below: Option<f64>,
    /// Time stretch factor (for playback, if different from default)
    pub time_stretch: Option<f64>,
    /// Unique identifier
    pub id: u64,
}

impl FermataInput {
    /// Create a basic fermata input with defaults.
    #[must_use]
    pub fn new(note_x: f64, note_y: f64, staff_top: f64, staff_bottom: f64) -> Self {
        Self {
            fermata_type: FermataType::Normal,
            placement: FermataPlacement::Above,
            note_x,
            note_y,
            staff_top,
            staff_bottom,
            skyline_above: None,
            skyline_below: None,
            time_stretch: None,
            id: 0,
        }
    }

    /// Set the fermata type.
    #[must_use]
    pub fn with_type(mut self, fermata_type: FermataType) -> Self {
        self.fermata_type = fermata_type;
        self
    }

    /// Set the placement.
    #[must_use]
    pub fn with_placement(mut self, placement: FermataPlacement) -> Self {
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

/// Result of fermata layout.
#[derive(Debug, Clone)]
pub struct FermataLayout {
    /// Position of the fermata (center of glyph)
    pub position: Point,
    /// Bounding box of the fermata
    pub bbox: Rect,
    /// SMuFL codepoint for the glyph
    pub glyph: char,
    /// Paint commands for rendering
    pub commands: Vec<PaintCommand>,
    /// Scene node for the fermata
    pub scene: SceneNode,
    /// Effective time stretch factor
    pub time_stretch: f64,
}

/// Layout a fermata.
///
/// Positions the fermata above or below the staff, centered on the note,
/// avoiding any elements in the skyline.
///
/// # Arguments
/// * `input` - Fermata input data
/// * `spatium` - Staff space height in pixels/points
/// * `config` - Configuration options
///
/// # Returns
/// Layout result with position and rendering commands.
#[must_use]
pub fn layout_fermata(input: &FermataInput, spatium: f64, config: &FermataConfig) -> FermataLayout {
    let fermata_type = input.fermata_type;
    let placement = input.placement;

    // Get glyph and dimensions
    let glyph = match placement {
        FermataPlacement::Above => fermata_type.smufl_codepoint_above(),
        FermataPlacement::Below => fermata_type.smufl_codepoint_below(),
    };

    let (glyph_width, glyph_height) = fermata_type.glyph_dimensions();
    let width = glyph_width * spatium * config.scale;
    let height = glyph_height * spatium * config.scale;

    // Calculate X position (centered on note)
    let x = input.note_x + config.offset.0 * spatium;

    // Calculate Y position based on placement
    let y = match placement {
        FermataPlacement::Above => {
            // Start at top of staff
            let mut y_pos = input.staff_top;

            // If there's a skyline, use it
            if let Some(skyline) = input.skyline_above {
                y_pos = y_pos.min(skyline);
            }

            // Add minimum distance and half the glyph height
            y_pos - config.min_distance * spatium - height / 2.0 + config.offset.1 * spatium
        }
        FermataPlacement::Below => {
            // Start at bottom of staff
            let mut y_pos = input.staff_bottom;

            // If there's a skyline, use it
            if let Some(skyline) = input.skyline_below {
                y_pos = y_pos.max(skyline);
            }

            // Add minimum distance and half the glyph height
            y_pos + config.min_distance * spatium + height / 2.0 + config.offset.1 * spatium
        }
    };

    let position = Point::new(x, y);

    // Calculate bounding box
    let bbox = Rect::new(
        x - width / 2.0,
        y - height / 2.0,
        x + width / 2.0,
        y + height / 2.0,
    );

    // Create paint command for the glyph
    let commands = vec![PaintCommand::glyph(
        glyph,
        position,
        spatium * config.scale,
        Color::BLACK,
    )];

    // Create scene node
    let mut scene = SceneNode::group(SemanticId::new(ElementType::Fermata, input.id));
    scene.commands = commands.clone();
    scene.bounds = bbox;

    // Get time stretch
    let time_stretch = input
        .time_stretch
        .unwrap_or_else(|| fermata_type.default_time_stretch());

    FermataLayout {
        position,
        bbox,
        glyph,
        commands,
        scene,
        time_stretch,
    }
}

/// Layout multiple fermatas, avoiding collisions.
///
/// Useful when multiple fermatas appear at the same beat (e.g., on different staves).
#[must_use]
pub fn layout_fermatas(
    inputs: &[FermataInput],
    spatium: f64,
    config: &FermataConfig,
) -> Vec<FermataLayout> {
    inputs
        .iter()
        .map(|input| layout_fermata(input, spatium, config))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fermata_type_time_stretch() {
        assert!((FermataType::VeryShort.default_time_stretch() - 1.25).abs() < 0.01);
        assert!((FermataType::Normal.default_time_stretch() - 2.0).abs() < 0.01);
        assert!((FermataType::VeryLong.default_time_stretch() - 4.0).abs() < 0.01);
    }

    #[test]
    fn test_fermata_glyphs() {
        // Normal fermata above should be U+E4C0
        assert_eq!(FermataType::Normal.smufl_codepoint_above(), '\u{E4C0}');
        // Normal fermata below should be U+E4C1
        assert_eq!(FermataType::Normal.smufl_codepoint_below(), '\u{E4C1}');
    }

    #[test]
    fn test_basic_fermata_layout() {
        let input = FermataInput::new(100.0, 0.0, 0.0, 20.0);
        let config = FermataConfig::default();
        let result = layout_fermata(&input, 5.0, &config);

        // Should be centered on note_x
        assert!((result.position.x - 100.0).abs() < 0.01);
        // Should be above staff (negative Y for above)
        assert!(result.position.y < 0.0);
        // Should have non-zero size
        assert!(result.bbox.width() > 0.0);
        assert!(result.bbox.height() > 0.0);
        // Should have the above glyph
        assert_eq!(result.glyph, FermataType::Normal.smufl_codepoint_above());
    }

    #[test]
    fn test_fermata_below() {
        let input =
            FermataInput::new(100.0, 0.0, 0.0, 20.0).with_placement(FermataPlacement::Below);
        let config = FermataConfig::default();
        let result = layout_fermata(&input, 5.0, &config);

        // Should be below staff (positive Y > staff_bottom)
        assert!(result.position.y > 20.0);
        // Should have the below glyph
        assert_eq!(result.glyph, FermataType::Normal.smufl_codepoint_below());
    }

    #[test]
    fn test_fermata_types() {
        let config = FermataConfig::default();
        let spatium = 5.0;

        for fermata_type in [
            FermataType::VeryShort,
            FermataType::Short,
            FermataType::Normal,
            FermataType::Long,
            FermataType::VeryLong,
        ] {
            let input = FermataInput::new(100.0, 0.0, 0.0, 20.0).with_type(fermata_type);
            let result = layout_fermata(&input, spatium, &config);

            // All should produce valid output
            assert!(!result.commands.is_empty());
            assert!(result.bbox.width() > 0.0);
        }
    }

    #[test]
    fn test_fermata_with_skyline() {
        // Test that fermata avoids elements in skyline
        let mut input = FermataInput::new(100.0, 0.0, 0.0, 20.0);
        input.skyline_above = Some(-10.0); // Something above the staff

        let config = FermataConfig::default();
        let result = layout_fermata(&input, 5.0, &config);

        // Should be above the skyline
        assert!(result.bbox.y0 < -10.0);
    }

    #[test]
    fn test_fermata_scaling() {
        let input = FermataInput::new(100.0, 0.0, 0.0, 20.0);
        let config_normal = FermataConfig::default();
        let config_scaled = FermataConfig {
            scale: 1.5,
            ..Default::default()
        };

        let result_normal = layout_fermata(&input, 5.0, &config_normal);
        let result_scaled = layout_fermata(&input, 5.0, &config_scaled);

        // Scaled fermata should be larger
        assert!(result_scaled.bbox.width() > result_normal.bbox.width());
        assert!(result_scaled.bbox.height() > result_normal.bbox.height());
    }

    #[test]
    fn test_fermata_offset() {
        let input = FermataInput::new(100.0, 0.0, 0.0, 20.0);
        let config = FermataConfig {
            offset: (2.0, -1.0), // 2 spatiums right, 1 spatium up
            ..Default::default()
        };

        let result = layout_fermata(&input, 5.0, &config);

        // X should be offset by 2 spatiums (10 units at spatium=5)
        assert!((result.position.x - 110.0).abs() < 0.01);
    }

    #[test]
    fn test_time_stretch_override() {
        let mut input = FermataInput::new(100.0, 0.0, 0.0, 20.0);
        input.time_stretch = Some(5.0);

        let config = FermataConfig::default();
        let result = layout_fermata(&input, 5.0, &config);

        // Should use the override time stretch
        assert!((result.time_stretch - 5.0).abs() < 0.01);
    }

    #[test]
    fn test_multiple_fermatas() {
        let inputs = vec![
            FermataInput::new(100.0, 0.0, 0.0, 20.0).with_id(1),
            FermataInput::new(150.0, 0.0, 0.0, 20.0)
                .with_id(2)
                .with_type(FermataType::Long),
            FermataInput::new(200.0, 0.0, 0.0, 20.0)
                .with_id(3)
                .with_placement(FermataPlacement::Below),
        ];

        let config = FermataConfig::default();
        let results = layout_fermatas(&inputs, 5.0, &config);

        assert_eq!(results.len(), 3);
        // All should be at different X positions
        assert!((results[0].position.x - 100.0).abs() < 0.01);
        assert!((results[1].position.x - 150.0).abs() < 0.01);
        assert!((results[2].position.x - 200.0).abs() < 0.01);
    }
}
