//! Articulation layout module.
//!
//! Implements layout for articulations (staccato, accent, tenuto, marcato, etc.)
//! following MuseScore's articulation layout algorithm. Articulations are placed
//! above or below notes and must be properly stacked when multiple articulations
//! appear on the same note.
//!
//! Reference: MuseScore `articulation.cpp` and `tlayout.cpp::layoutArticulation`

use kurbo::{Point, Rect};
use peniko::Color;

use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::{PaintCommand, SceneNode};

/// Type of articulation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ArticulationType {
    /// Staccato dot (detached)
    #[default]
    Staccato,
    /// Staccatissimo wedge (very detached)
    Staccatissimo,
    /// Accent (stress)
    Accent,
    /// Strong accent / marcato (^)
    Marcato,
    /// Tenuto line (held)
    Tenuto,
    /// Staccato + tenuto combination (portato)
    TenutoStaccato,
    /// Accent + staccato combination
    AccentStaccato,
    /// Marcato + staccato combination
    MarcatoStaccato,
    /// Marcato + tenuto combination
    MarcatoTenuto,
    /// Stress (strong down-bow like)
    Stress,
    /// Unstress
    Unstress,
    /// Soft accent
    SoftAccent,
    /// Laissez vibrer (let ring)
    LaissezVibrer,
}

impl ArticulationType {
    /// Get the SMuFL codepoint for this articulation (above variant).
    #[must_use]
    pub const fn smufl_codepoint_above(self) -> char {
        match self {
            Self::Staccato => '\u{E4A2}',        // articStaccatoAbove
            Self::Staccatissimo => '\u{E4A6}',   // articStaccatissimoAbove
            Self::Accent => '\u{E4A0}',          // articAccentAbove
            Self::Marcato => '\u{E4AC}',         // articMarcatoAbove
            Self::Tenuto => '\u{E4A4}',          // articTenutoAbove
            Self::TenutoStaccato => '\u{E4B2}',  // articTenutoStaccatoAbove
            Self::AccentStaccato => '\u{E4B0}',  // articAccentStaccatoAbove
            Self::MarcatoStaccato => '\u{E4AE}', // articMarcatoStaccatoAbove
            Self::MarcatoTenuto => '\u{E4B4}',   // articMarcatoTenutoAbove
            Self::Stress => '\u{E4B6}',          // articStressAbove
            Self::Unstress => '\u{E4B8}',        // articUnstressAbove
            Self::SoftAccent => '\u{E4BA}',      // articSoftAccentAbove
            Self::LaissezVibrer => '\u{E4BC}',   // articLaissezVibrerAbove
        }
    }

    /// Get the SMuFL codepoint for this articulation (below variant).
    #[must_use]
    pub const fn smufl_codepoint_below(self) -> char {
        match self {
            Self::Staccato => '\u{E4A3}',        // articStaccatoBelow
            Self::Staccatissimo => '\u{E4A7}',   // articStaccatissimoBelow
            Self::Accent => '\u{E4A1}',          // articAccentBelow
            Self::Marcato => '\u{E4AD}',         // articMarcatoBelow
            Self::Tenuto => '\u{E4A5}',          // articTenutoBelow
            Self::TenutoStaccato => '\u{E4B3}',  // articTenutoStaccatoBelow
            Self::AccentStaccato => '\u{E4B1}',  // articAccentStaccatoBelow
            Self::MarcatoStaccato => '\u{E4AF}', // articMarcatoStaccatoBelow
            Self::MarcatoTenuto => '\u{E4B5}',   // articMarcatoTenutoBelow
            Self::Stress => '\u{E4B7}',          // articStressBelow
            Self::Unstress => '\u{E4B9}',        // articUnstressBelow
            Self::SoftAccent => '\u{E4BB}',      // articSoftAccentBelow
            Self::LaissezVibrer => '\u{E4BD}',   // articLaissezVibrerBelow
        }
    }

    /// Get approximate glyph dimensions (width, height) in spatiums.
    #[must_use]
    pub const fn glyph_dimensions(self) -> (f64, f64) {
        match self {
            Self::Staccato => (0.4, 0.4),
            Self::Staccatissimo => (0.3, 0.8),
            Self::Accent => (1.0, 0.6),
            Self::Marcato => (0.8, 0.8),
            Self::Tenuto => (1.0, 0.2),
            Self::TenutoStaccato => (1.0, 0.6),
            Self::AccentStaccato => (1.0, 0.8),
            Self::MarcatoStaccato => (0.8, 1.0),
            Self::MarcatoTenuto => (1.0, 1.0),
            Self::Stress => (0.8, 0.8),
            Self::Unstress => (0.8, 0.8),
            Self::SoftAccent => (1.2, 0.6),
            Self::LaissezVibrer => (0.8, 0.6),
        }
    }

    /// Whether this articulation should be placed close to the notehead.
    ///
    /// "Inside" articulations (staccato, tenuto) are placed close to the note,
    /// while "outside" articulations (accent, marcato) are placed further away.
    #[must_use]
    pub const fn layout_close_to_note(self) -> bool {
        matches!(
            self,
            Self::Staccato | Self::Staccatissimo | Self::Tenuto | Self::TenutoStaccato
        )
    }

    /// Get the articulation category for stacking priority.
    ///
    /// Lower priority = closer to note. Articulations are stacked from
    /// closest to note (inside) to furthest (outside).
    #[must_use]
    pub const fn stacking_priority(self) -> u8 {
        match self {
            // Inside articulations - closest to note
            Self::Staccato => 0,
            Self::Staccatissimo => 0,
            Self::Tenuto => 1,
            Self::TenutoStaccato => 1,

            // Middle articulations
            Self::Accent => 2,
            Self::AccentStaccato => 2,
            Self::Stress => 2,
            Self::Unstress => 2,
            Self::SoftAccent => 2,

            // Outside articulations - furthest from note
            Self::Marcato => 3,
            Self::MarcatoStaccato => 3,
            Self::MarcatoTenuto => 3,
            Self::LaissezVibrer => 4,
        }
    }
}

/// Anchor position for articulation placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ArticulationAnchor {
    /// Always place above
    Top,
    /// Always place below
    Bottom,
    /// Auto: place opposite to stem direction
    #[default]
    Auto,
}

/// Horizontal alignment for stem-side articulations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ArticulationAlign {
    /// Center on stem
    Stem,
    /// Center on notehead
    #[default]
    Notehead,
    /// Average of stem and notehead positions
    Average,
}

/// Configuration for articulation layout.
#[derive(Debug, Clone)]
pub struct ArticulationConfig {
    /// Minimum distance from note/staff in spatiums
    pub min_distance: f64,
    /// Distance between stacked articulations in spatiums
    pub stacking_distance: f64,
    /// Distance for "inside" articulations (close to note)
    pub inside_distance: f64,
    /// Distance for "outside" articulations (further from note)
    pub outside_distance: f64,
    /// Horizontal alignment mode
    pub align: ArticulationAlign,
    /// Scaling factor
    pub scale: f64,
}

impl Default for ArticulationConfig {
    fn default() -> Self {
        Self {
            min_distance: 0.5,
            stacking_distance: 0.25,
            inside_distance: 0.25,
            outside_distance: 0.5,
            align: ArticulationAlign::Notehead,
            scale: 1.0,
        }
    }
}

/// Input data for articulation layout.
#[derive(Debug, Clone)]
pub struct ArticulationInput {
    /// Type of articulation
    pub articulation_type: ArticulationType,
    /// Anchor position preference
    pub anchor: ArticulationAnchor,
    /// Unique identifier
    pub id: u64,
}

impl ArticulationInput {
    /// Create a new articulation input.
    #[must_use]
    pub fn new(articulation_type: ArticulationType) -> Self {
        Self {
            articulation_type,
            anchor: ArticulationAnchor::Auto,
            id: 0,
        }
    }

    /// Set the anchor.
    #[must_use]
    pub fn with_anchor(mut self, anchor: ArticulationAnchor) -> Self {
        self.anchor = anchor;
        self
    }

    /// Set the ID.
    #[must_use]
    pub fn with_id(mut self, id: u64) -> Self {
        self.id = id;
        self
    }
}

/// Context for positioning articulations relative to a note/chord.
#[derive(Debug, Clone)]
pub struct ArticulationContext {
    /// X position of the notehead center
    pub notehead_x: f64,
    /// Y position of the notehead center
    pub notehead_y: f64,
    /// X position of the stem (if present)
    pub stem_x: Option<f64>,
    /// Whether stem goes up
    pub stem_up: bool,
    /// Top of the staff (Y coordinate)
    pub staff_top: f64,
    /// Bottom of the staff (Y coordinate)
    pub staff_bottom: f64,
    /// Top of the chord's bounding box
    pub chord_top: f64,
    /// Bottom of the chord's bounding box
    pub chord_bottom: f64,
    /// Width of the notehead
    pub notehead_width: f64,
}

impl ArticulationContext {
    /// Create a simple context for a single note.
    #[must_use]
    pub fn new(notehead_x: f64, notehead_y: f64, staff_top: f64, staff_bottom: f64) -> Self {
        Self {
            notehead_x,
            notehead_y,
            stem_x: None,
            stem_up: true,
            staff_top,
            staff_bottom,
            chord_top: notehead_y,
            chord_bottom: notehead_y,
            notehead_width: 1.18, // Default notehead width in spatiums
        }
    }

    /// Set stem information.
    #[must_use]
    pub fn with_stem(mut self, stem_x: f64, stem_up: bool) -> Self {
        self.stem_x = Some(stem_x);
        self.stem_up = stem_up;
        self
    }

    /// Set chord bounds.
    #[must_use]
    pub fn with_chord_bounds(mut self, top: f64, bottom: f64) -> Self {
        self.chord_top = top;
        self.chord_bottom = bottom;
        self
    }
}

/// Result of articulation layout.
#[derive(Debug, Clone)]
pub struct ArticulationLayout {
    /// Type of articulation
    pub articulation_type: ArticulationType,
    /// Position of the articulation (center of glyph)
    pub position: Point,
    /// Bounding box of the articulation
    pub bbox: Rect,
    /// SMuFL codepoint for the glyph
    pub glyph: char,
    /// Whether placed above (true) or below (false)
    pub above: bool,
    /// Paint commands for rendering
    pub commands: Vec<PaintCommand>,
    /// Scene node for the articulation
    pub scene: SceneNode,
}

/// Layout a single articulation.
#[must_use]
pub fn layout_articulation(
    input: &ArticulationInput,
    context: &ArticulationContext,
    spatium: f64,
    config: &ArticulationConfig,
) -> ArticulationLayout {
    // Determine direction based on anchor and stem
    let above = match input.anchor {
        ArticulationAnchor::Top => true,
        ArticulationAnchor::Bottom => false,
        ArticulationAnchor::Auto => !context.stem_up, // Opposite to stem
    };

    layout_articulation_at(input, context, above, 0.0, spatium, config)
}

/// Layout an articulation at a specific position in a stack.
fn layout_articulation_at(
    input: &ArticulationInput,
    context: &ArticulationContext,
    above: bool,
    stack_offset: f64,
    spatium: f64,
    config: &ArticulationConfig,
) -> ArticulationLayout {
    let art_type = input.articulation_type;

    // Get glyph and dimensions
    let glyph = if above {
        art_type.smufl_codepoint_above()
    } else {
        art_type.smufl_codepoint_below()
    };

    let (glyph_width, glyph_height) = art_type.glyph_dimensions();
    let width = glyph_width * spatium * config.scale;
    let height = glyph_height * spatium * config.scale;

    // Calculate X position based on alignment
    let x = match config.align {
        ArticulationAlign::Notehead => context.notehead_x,
        ArticulationAlign::Stem => context.stem_x.unwrap_or(context.notehead_x),
        ArticulationAlign::Average => {
            let stem_x = context.stem_x.unwrap_or(context.notehead_x);
            (context.notehead_x + stem_x) / 2.0
        }
    };

    // Calculate Y position
    let base_distance = if art_type.layout_close_to_note() {
        config.inside_distance
    } else {
        config.outside_distance
    };

    let y = if above {
        // Place above: start from chord top, move up
        let base_y = context.chord_top.min(context.staff_top);
        base_y - (config.min_distance + base_distance) * spatium - height / 2.0 - stack_offset
    } else {
        // Place below: start from chord bottom, move down
        let base_y = context.chord_bottom.max(context.staff_bottom);
        base_y + (config.min_distance + base_distance) * spatium + height / 2.0 + stack_offset
    };

    let position = Point::new(x, y);

    // Calculate bounding box
    let bbox = Rect::new(
        x - width / 2.0,
        y - height / 2.0,
        x + width / 2.0,
        y + height / 2.0,
    );

    // Create paint command
    let commands = vec![PaintCommand::glyph(
        glyph,
        position,
        spatium * config.scale,
        Color::BLACK,
    )];

    // Create scene node
    let mut scene = SceneNode::group(SemanticId::new(ElementType::Articulation, input.id));
    scene.commands = commands.clone();
    scene.bounds = bbox;

    ArticulationLayout {
        articulation_type: art_type,
        position,
        bbox,
        glyph,
        above,
        commands,
        scene,
    }
}

/// Layout multiple articulations on the same note, handling stacking.
///
/// Articulations are sorted by stacking priority and placed from
/// closest to the note (inside) to furthest (outside).
#[must_use]
pub fn layout_articulations(
    inputs: &[ArticulationInput],
    context: &ArticulationContext,
    spatium: f64,
    config: &ArticulationConfig,
) -> Vec<ArticulationLayout> {
    if inputs.is_empty() {
        return Vec::new();
    }

    // Single articulation - simple case
    if inputs.len() == 1 {
        return vec![layout_articulation(&inputs[0], context, spatium, config)];
    }

    // Sort inputs by stacking priority (closest to note first)
    let mut sorted: Vec<(usize, &ArticulationInput)> = inputs.iter().enumerate().collect();
    sorted.sort_by_key(|(_, input)| input.articulation_type.stacking_priority());

    // Separate into above and below groups
    let mut above_inputs: Vec<&ArticulationInput> = Vec::new();
    let mut below_inputs: Vec<&ArticulationInput> = Vec::new();

    for (_, input) in &sorted {
        let above = match input.anchor {
            ArticulationAnchor::Top => true,
            ArticulationAnchor::Bottom => false,
            ArticulationAnchor::Auto => !context.stem_up,
        };

        if above {
            above_inputs.push(input);
        } else {
            below_inputs.push(input);
        }
    }

    let mut results = Vec::with_capacity(inputs.len());

    // Layout articulations above
    let mut stack_offset = 0.0;
    for input in &above_inputs {
        let layout = layout_articulation_at(input, context, true, stack_offset, spatium, config);
        stack_offset += layout.bbox.height() + config.stacking_distance * spatium;
        results.push(layout);
    }

    // Layout articulations below
    stack_offset = 0.0;
    for input in &below_inputs {
        let layout = layout_articulation_at(input, context, false, stack_offset, spatium, config);
        stack_offset += layout.bbox.height() + config.stacking_distance * spatium;
        results.push(layout);
    }

    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_context() -> ArticulationContext {
        ArticulationContext::new(100.0, 0.0, -20.0, 20.0)
    }

    #[test]
    fn test_articulation_glyphs() {
        assert_eq!(
            ArticulationType::Staccato.smufl_codepoint_above(),
            '\u{E4A2}'
        );
        assert_eq!(
            ArticulationType::Staccato.smufl_codepoint_below(),
            '\u{E4A3}'
        );
        assert_eq!(ArticulationType::Accent.smufl_codepoint_above(), '\u{E4A0}');
    }

    #[test]
    fn test_stacking_priority() {
        // Inside articulations should have lower priority (closer to note)
        assert!(
            ArticulationType::Staccato.stacking_priority()
                < ArticulationType::Accent.stacking_priority()
        );
        assert!(
            ArticulationType::Tenuto.stacking_priority()
                < ArticulationType::Marcato.stacking_priority()
        );
    }

    #[test]
    fn test_layout_close_to_note() {
        assert!(ArticulationType::Staccato.layout_close_to_note());
        assert!(ArticulationType::Tenuto.layout_close_to_note());
        assert!(!ArticulationType::Accent.layout_close_to_note());
        assert!(!ArticulationType::Marcato.layout_close_to_note());
    }

    #[test]
    fn test_single_articulation_above() {
        let input =
            ArticulationInput::new(ArticulationType::Staccato).with_anchor(ArticulationAnchor::Top);
        let context = make_context();
        let config = ArticulationConfig::default();

        let layout = layout_articulation(&input, &context, 5.0, &config);

        assert!(layout.above);
        assert!(layout.position.y < context.chord_top);
        assert_eq!(
            layout.glyph,
            ArticulationType::Staccato.smufl_codepoint_above()
        );
    }

    #[test]
    fn test_single_articulation_below() {
        let input = ArticulationInput::new(ArticulationType::Accent)
            .with_anchor(ArticulationAnchor::Bottom);
        let context = make_context();
        let config = ArticulationConfig::default();

        let layout = layout_articulation(&input, &context, 5.0, &config);

        assert!(!layout.above);
        assert!(layout.position.y > context.chord_bottom);
        assert_eq!(
            layout.glyph,
            ArticulationType::Accent.smufl_codepoint_below()
        );
    }

    #[test]
    fn test_auto_anchor_stem_up() {
        let input = ArticulationInput::new(ArticulationType::Staccato);
        let context = make_context().with_stem(105.0, true);
        let config = ArticulationConfig::default();

        let layout = layout_articulation(&input, &context, 5.0, &config);

        // Auto with stem up should place below
        assert!(!layout.above);
    }

    #[test]
    fn test_auto_anchor_stem_down() {
        let input = ArticulationInput::new(ArticulationType::Staccato);
        let context = make_context().with_stem(95.0, false);
        let config = ArticulationConfig::default();

        let layout = layout_articulation(&input, &context, 5.0, &config);

        // Auto with stem down should place above
        assert!(layout.above);
    }

    #[test]
    fn test_stacking_multiple_articulations() {
        let inputs = vec![
            ArticulationInput::new(ArticulationType::Staccato)
                .with_id(1)
                .with_anchor(ArticulationAnchor::Top),
            ArticulationInput::new(ArticulationType::Accent)
                .with_id(2)
                .with_anchor(ArticulationAnchor::Top),
            ArticulationInput::new(ArticulationType::Marcato)
                .with_id(3)
                .with_anchor(ArticulationAnchor::Top),
        ];
        let context = make_context();
        let config = ArticulationConfig::default();

        let layouts = layout_articulations(&inputs, &context, 5.0, &config);

        assert_eq!(layouts.len(), 3);

        // All should be above
        for layout in &layouts {
            assert!(layout.above);
        }

        // Staccato (lowest priority) should be closest to note (highest Y)
        // Marcato (highest priority) should be furthest (lowest Y)
        let Some(staccato) = layouts
            .iter()
            .find(|l| l.articulation_type == ArticulationType::Staccato)
        else {
            panic!("Expected staccato articulation in results");
        };
        let Some(marcato) = layouts
            .iter()
            .find(|l| l.articulation_type == ArticulationType::Marcato)
        else {
            panic!("Expected marcato articulation in results");
        };
        assert!(staccato.position.y > marcato.position.y);
    }

    #[test]
    fn test_mixed_above_below_stacking() {
        let inputs = vec![
            ArticulationInput::new(ArticulationType::Staccato)
                .with_id(1)
                .with_anchor(ArticulationAnchor::Top),
            ArticulationInput::new(ArticulationType::Accent)
                .with_id(2)
                .with_anchor(ArticulationAnchor::Bottom),
        ];
        let context = make_context();
        let config = ArticulationConfig::default();

        let layouts = layout_articulations(&inputs, &context, 5.0, &config);

        assert_eq!(layouts.len(), 2);

        let Some(above_layout) = layouts.iter().find(|l| l.above) else {
            panic!("Expected an above articulation in results");
        };
        let Some(below_layout) = layouts.iter().find(|l| !l.above) else {
            panic!("Expected a below articulation in results");
        };

        assert_eq!(above_layout.articulation_type, ArticulationType::Staccato);
        assert_eq!(below_layout.articulation_type, ArticulationType::Accent);
        assert!(above_layout.position.y < below_layout.position.y);
    }

    #[test]
    fn test_alignment_modes() {
        let input =
            ArticulationInput::new(ArticulationType::Staccato).with_anchor(ArticulationAnchor::Top);
        let context = make_context().with_stem(110.0, true);
        let spatium = 5.0;

        // Notehead alignment
        let mut config = ArticulationConfig {
            align: ArticulationAlign::Notehead,
            ..Default::default()
        };
        let layout = layout_articulation(&input, &context, spatium, &config);
        assert!((layout.position.x - 100.0).abs() < 0.01);

        // Stem alignment
        config.align = ArticulationAlign::Stem;
        let layout = layout_articulation(&input, &context, spatium, &config);
        assert!((layout.position.x - 110.0).abs() < 0.01);

        // Average alignment
        config.align = ArticulationAlign::Average;
        let layout = layout_articulation(&input, &context, spatium, &config);
        assert!((layout.position.x - 105.0).abs() < 0.01);
    }

    #[test]
    fn test_scaling() {
        let input = ArticulationInput::new(ArticulationType::Accent);
        let context = make_context();

        let config_normal = ArticulationConfig::default();
        let config_scaled = ArticulationConfig {
            scale: 1.5,
            ..Default::default()
        };

        let layout_normal = layout_articulation(&input, &context, 5.0, &config_normal);
        let layout_scaled = layout_articulation(&input, &context, 5.0, &config_scaled);

        assert!(layout_scaled.bbox.width() > layout_normal.bbox.width());
    }

    #[test]
    fn test_all_articulation_types() {
        let context = make_context();
        let config = ArticulationConfig::default();
        let spatium = 5.0;

        for art_type in [
            ArticulationType::Staccato,
            ArticulationType::Staccatissimo,
            ArticulationType::Accent,
            ArticulationType::Marcato,
            ArticulationType::Tenuto,
            ArticulationType::TenutoStaccato,
            ArticulationType::AccentStaccato,
            ArticulationType::MarcatoStaccato,
            ArticulationType::MarcatoTenuto,
            ArticulationType::Stress,
            ArticulationType::Unstress,
            ArticulationType::SoftAccent,
            ArticulationType::LaissezVibrer,
        ] {
            let input = ArticulationInput::new(art_type).with_id(1);
            let layout = layout_articulation(&input, &context, spatium, &config);

            // Should produce valid output
            assert!(!layout.commands.is_empty());
            assert!(layout.bbox.width() > 0.0);
            assert!(layout.bbox.height() > 0.0);
        }
    }
}
