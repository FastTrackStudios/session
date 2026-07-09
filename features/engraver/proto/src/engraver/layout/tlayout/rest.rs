//! Rest layout implementation.
//!
//! Handles layout of rest symbols for different durations.

use kurbo::{Point, Rect};
use peniko::Color;

use crate::engraver::layout::context::LayoutContext;
use crate::engraver::layout::shape::Shape;
use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::PaintCommand;

use super::LayoutData;

/// SMuFL codepoints for rests.
pub mod glyphs {
    /// Double whole rest (breve)
    pub const REST_DOUBLE_WHOLE: char = '\u{E4E2}';
    /// Whole rest (semibreve)
    pub const REST_WHOLE: char = '\u{E4E3}';
    /// Half rest (minim)
    pub const REST_HALF: char = '\u{E4E4}';
    /// Quarter rest (crotchet)
    pub const REST_QUARTER: char = '\u{E4E5}';
    /// Eighth rest (quaver)
    pub const REST_EIGHTH: char = '\u{E4E6}';
    /// Sixteenth rest (semiquaver)
    pub const REST_SIXTEENTH: char = '\u{E4E7}';
    /// Thirty-second rest (demisemiquaver)
    pub const REST_THIRTY_SECOND: char = '\u{E4E8}';
    /// Sixty-fourth rest (hemidemisemiquaver)
    pub const REST_SIXTY_FOURTH: char = '\u{E4E9}';
    /// Multi-measure rest
    pub const REST_H_BAR: char = '\u{E4EE}';
}

/// Rest duration type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestDuration {
    DoubleWhole,
    Whole,
    Half,
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
    SixtyFourth,
}

impl RestDuration {
    /// Get the SMuFL glyph for this rest.
    #[must_use]
    pub const fn glyph(&self) -> char {
        match self {
            Self::DoubleWhole => glyphs::REST_DOUBLE_WHOLE,
            Self::Whole => glyphs::REST_WHOLE,
            Self::Half => glyphs::REST_HALF,
            Self::Quarter => glyphs::REST_QUARTER,
            Self::Eighth => glyphs::REST_EIGHTH,
            Self::Sixteenth => glyphs::REST_SIXTEENTH,
            Self::ThirtySecond => glyphs::REST_THIRTY_SECOND,
            Self::SixtyFourth => glyphs::REST_SIXTY_FOURTH,
        }
    }

    /// Get the width of this rest in spatiums.
    #[must_use]
    pub const fn width(&self) -> f64 {
        match self {
            Self::DoubleWhole => 1.5,
            Self::Whole => 1.0,
            Self::Half => 1.0,
            Self::Quarter => 0.9,
            Self::Eighth => 0.8,
            Self::Sixteenth => 1.0,
            Self::ThirtySecond => 1.2,
            Self::SixtyFourth => 1.4,
        }
    }

    /// Get the height of this rest in spatiums.
    #[must_use]
    pub const fn height(&self) -> f64 {
        match self {
            Self::DoubleWhole => 1.0,
            Self::Whole => 0.5,
            Self::Half => 0.5,
            Self::Quarter => 3.0,
            Self::Eighth => 1.5,
            Self::Sixteenth => 2.5,
            Self::ThirtySecond => 3.5,
            Self::SixtyFourth => 4.5,
        }
    }

    /// Get the Y offset from staff center in spatiums.
    /// Whole/half rests hang from lines, others are centered.
    #[must_use]
    pub const fn y_offset(&self) -> f64 {
        match self {
            Self::Whole => -0.5, // Hangs below 4th line
            Self::Half => 0.0,   // Sits on 3rd line
            Self::DoubleWhole => 0.0,
            _ => 0.0, // Centered
        }
    }
}

/// Rest layout parameters.
#[derive(Debug, Clone)]
pub struct RestParams {
    /// Unique identifier
    pub id: u64,
    /// Duration type
    pub duration: RestDuration,
    /// Number of augmentation dots
    pub dots: u8,
    /// Staff line position (usually 0 for center)
    pub line: i32,
}

impl Default for RestParams {
    fn default() -> Self {
        Self {
            id: 0,
            duration: RestDuration::Quarter,
            dots: 0,
            line: 0,
        }
    }
}

/// Layout a rest symbol.
#[must_use]
pub fn layout_rest(params: &RestParams, ctx: &LayoutContext) -> (LayoutData, SceneNode) {
    let spatium = ctx.spatium();

    // Calculate position
    let y = -params.line as f64 * spatium / 2.0 + params.duration.y_offset() * spatium;
    let x = 0.0;

    let mut commands = Vec::new();
    let rest_width = params.duration.width() * spatium;
    let rest_height = params.duration.height() * spatium;

    // Draw rest glyph
    commands.push(PaintCommand::glyph(
        params.duration.glyph(),
        Point::new(x, y),
        spatium,
        Color::BLACK,
    ));

    let mut total_width = rest_width;

    // Draw augmentation dots
    // MuseScore uses dotNoteDistance = 0.5 spatiums (default)
    const DOT_NOTE_DISTANCE: f64 = 0.5;
    const DOT_DOT_DISTANCE: f64 = 0.5; // between adjacent dots
    const DOT_GLYPH_WIDTH: f64 = 0.35; // approximate width of the dot glyph

    if params.dots > 0 {
        let dot_x = x + rest_width + spatium * DOT_NOTE_DISTANCE;
        let dot_y = y - spatium * 0.25; // Slightly above center

        for i in 0..params.dots {
            commands.push(PaintCommand::glyph(
                super::note::glyphs::AUGMENTATION_DOT,
                Point::new(dot_x + i as f64 * spatium * DOT_DOT_DISTANCE, dot_y),
                spatium,
                Color::BLACK,
            ));
        }

        // Width calculation:
        // - DOT_NOTE_DISTANCE: gap from rest to first dot
        // - (params.dots - 1) * DOT_DOT_DISTANCE: gaps between adjacent dots
        // - DOT_GLYPH_WIDTH: width of the last dot's glyph
        let dots_spacing = if params.dots > 1 {
            (params.dots - 1) as f64 * DOT_DOT_DISTANCE
        } else {
            0.0
        };
        total_width += spatium * (DOT_NOTE_DISTANCE + dots_spacing + DOT_GLYPH_WIDTH);
    }

    // Calculate bounding box
    let bbox = Rect::new(
        0.0,
        y - rest_height / 2.0,
        total_width,
        y + rest_height / 2.0,
    );

    let shape = Shape::from_rect(bbox);
    let layout = LayoutData::new(Point::ZERO, bbox, shape);

    let semantic_id = SemanticId::new(ElementType::Rest, params.id);
    let node = SceneNode::leaf(semantic_id, commands)
        .with_metadata("duration", format!("{:?}", params.duration));

    (layout, node)
}

/// Layout a multi-measure rest.
#[must_use]
pub fn layout_multi_measure_rest(
    id: u64,
    measure_count: u32,
    width: f64,
    ctx: &LayoutContext,
) -> (LayoutData, SceneNode) {
    let spatium = ctx.spatium();
    let mut commands = Vec::new();

    // Draw horizontal bar
    let bar_height = spatium * 0.5;
    let bar_y = 0.0;

    commands.push(PaintCommand::filled_rect(
        Rect::new(
            0.0,
            bar_y - bar_height / 2.0,
            width,
            bar_y + bar_height / 2.0,
        ),
        Color::BLACK,
    ));

    // Draw vertical end caps
    let cap_height = spatium * 2.0;
    let cap_width = spatium * 0.2;

    commands.push(PaintCommand::filled_rect(
        Rect::new(0.0, -cap_height / 2.0, cap_width, cap_height / 2.0),
        Color::BLACK,
    ));

    commands.push(PaintCommand::filled_rect(
        Rect::new(
            width - cap_width,
            -cap_height / 2.0,
            width,
            cap_height / 2.0,
        ),
        Color::BLACK,
    ));

    // Draw measure count above
    let count_text = measure_count.to_string();
    commands.push(PaintCommand::text(
        count_text,
        "Times New Roman",
        spatium * 2.0,
        Point::new(width / 2.0, -spatium * 2.0),
        Color::BLACK,
    ));

    let bbox = Rect::new(0.0, -cap_height / 2.0, width, spatium * 2.5);
    let shape = Shape::from_rect(bbox);
    let layout = LayoutData::new(Point::ZERO, bbox, shape);

    let semantic_id = SemanticId::new(ElementType::Rest, id);
    let node = SceneNode::leaf(semantic_id, commands)
        .with_metadata("multi-measure", measure_count.to_string());

    (layout, node)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engraver::layout::context::LayoutConfiguration;
    use crate::engraver::style::MStyle;

    fn test_ctx() -> LayoutContext<'static> {
        let config = LayoutConfiguration::default();
        let style = Box::leak(Box::new(MStyle::default()));
        LayoutContext::new_for_test(config, style)
    }

    #[test]
    fn test_layout_quarter_rest() {
        let ctx = test_ctx();
        let params = RestParams {
            id: 1,
            duration: RestDuration::Quarter,
            ..Default::default()
        };

        let (layout, node) = layout_rest(&params, &ctx);

        assert!(!layout.bbox.is_zero_area());
        assert!(node.id.is_some());
        assert!(!node.commands.is_empty());
    }

    #[test]
    fn test_layout_whole_rest() {
        let ctx = test_ctx();
        let params = RestParams {
            id: 2,
            duration: RestDuration::Whole,
            ..Default::default()
        };

        let (layout, _node) = layout_rest(&params, &ctx);

        assert!(!layout.bbox.is_zero_area());
    }

    #[test]
    fn test_layout_dotted_rest() {
        let ctx = test_ctx();
        let params = RestParams {
            id: 3,
            duration: RestDuration::Half,
            dots: 1,
            ..Default::default()
        };

        let (_layout, node) = layout_rest(&params, &ctx);

        // Should have rest + dot = 2 commands
        assert!(node.commands.len() >= 2);
    }

    #[test]
    fn test_multi_measure_rest() {
        let ctx = test_ctx();
        let (layout, node) = layout_multi_measure_rest(1, 8, 200.0, &ctx);

        assert!(!layout.bbox.is_zero_area());
        // Should have bar, 2 caps, and text = 4 commands
        assert_eq!(node.commands.len(), 4);
    }

    #[test]
    fn test_rest_glyphs() {
        assert_eq!(RestDuration::Quarter.glyph(), glyphs::REST_QUARTER);
        assert_eq!(RestDuration::Eighth.glyph(), glyphs::REST_EIGHTH);
        assert_eq!(RestDuration::Whole.glyph(), glyphs::REST_WHOLE);
    }
}
