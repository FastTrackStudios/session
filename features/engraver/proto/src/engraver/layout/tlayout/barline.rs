//! Barline layout implementation.
//!
//! Handles layout of barlines including single, double, repeat, and end barlines.

use kurbo::{Point, Rect};
use peniko::Color;

use crate::engraver::layout::context::LayoutContext;
use crate::engraver::layout::shape::Shape;
use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::PaintCommand;

use super::LayoutData;

/// SMuFL codepoints for barline-related glyphs.
pub mod glyphs {
    /// Repeat dots
    pub const REPEAT_DOT: char = '\u{E044}';
    /// Caesura
    pub const CAESURA: char = '\u{E4D1}';
    /// Short caesura
    pub const CAESURA_SHORT: char = '\u{E4D2}';
    /// Breath mark (comma)
    pub const BREATH_MARK_COMMA: char = '\u{E4CE}';
    /// Breath mark (tick)
    pub const BREATH_MARK_TICK: char = '\u{E4CF}';
}

/// Barline type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BarlineType {
    /// Single thin line
    Single,
    /// Double thin lines
    Double,
    /// Start repeat (thick-thin with dots)
    StartRepeat,
    /// End repeat (thin-thick with dots)
    EndRepeat,
    /// End barline (thin-thick)
    End,
    /// Dashed barline
    Dashed,
    /// Dotted barline
    Dotted,
    /// Short barline (doesn't extend full staff height)
    Short,
    /// Tick barline (very short, above staff)
    Tick,
}

impl BarlineType {
    /// Get the width of this barline type in spatiums.
    #[must_use]
    pub const fn width(&self) -> f64 {
        match self {
            Self::Single => 0.16,
            Self::Double => 0.7,
            Self::StartRepeat | Self::EndRepeat => 1.6,
            Self::End => 0.7,
            Self::Dashed | Self::Dotted => 0.16,
            Self::Short | Self::Tick => 0.16,
        }
    }

    /// Whether this barline has repeat dots.
    #[must_use]
    pub const fn has_dots(&self) -> bool {
        matches!(self, Self::StartRepeat | Self::EndRepeat)
    }

    /// Whether this barline has a thick line.
    #[must_use]
    pub const fn has_thick(&self) -> bool {
        matches!(self, Self::StartRepeat | Self::EndRepeat | Self::End)
    }
}

/// Barline layout parameters.
#[derive(Debug, Clone)]
pub struct BarlineParams {
    /// Unique identifier
    pub id: u64,
    /// Barline type
    pub barline_type: BarlineType,
    /// Staff height in spatiums (default 4 for 5-line staff)
    pub staff_height: f64,
    /// Whether this is a span barline (connects multiple staves)
    pub span_staves: bool,
}

impl Default for BarlineParams {
    fn default() -> Self {
        Self {
            id: 0,
            barline_type: BarlineType::Single,
            staff_height: 4.0,
            span_staves: false,
        }
    }
}

/// Layout a barline.
#[must_use]
pub fn layout_barline(params: &BarlineParams, ctx: &LayoutContext) -> (LayoutData, SceneNode) {
    let spatium = ctx.spatium();
    let thin_width = spatium * 0.16;
    let thick_width = spatium * 0.5;
    let gap = spatium * 0.4;

    let half_height = params.staff_height * spatium / 2.0;
    let y_top = -half_height;
    let y_bottom = half_height;

    let mut commands = Vec::new();
    let mut x = 0.0;

    let total_width = match params.barline_type {
        BarlineType::Single => {
            commands.push(draw_thin_line(x, y_top, y_bottom, thin_width));
            thin_width
        }

        BarlineType::Double => {
            commands.push(draw_thin_line(x, y_top, y_bottom, thin_width));
            x += thin_width + gap;
            commands.push(draw_thin_line(x, y_top, y_bottom, thin_width));
            x + thin_width
        }

        BarlineType::End => {
            commands.push(draw_thin_line(x, y_top, y_bottom, thin_width));
            x += thin_width + gap;
            commands.push(draw_thick_line(x, y_top, y_bottom, thick_width));
            x + thick_width
        }

        BarlineType::StartRepeat => {
            // Thick line first
            commands.push(draw_thick_line(x, y_top, y_bottom, thick_width));
            x += thick_width + gap;
            // Thin line
            commands.push(draw_thin_line(x, y_top, y_bottom, thin_width));
            x += thin_width + gap;
            // Repeat dots
            let dot_commands = draw_repeat_dots(x, spatium);
            commands.extend(dot_commands);
            x + spatium * 0.5
        }

        BarlineType::EndRepeat => {
            // Repeat dots first
            let dot_commands = draw_repeat_dots(x, spatium);
            commands.extend(dot_commands);
            x += spatium * 0.5 + gap;
            // Thin line
            commands.push(draw_thin_line(x, y_top, y_bottom, thin_width));
            x += thin_width + gap;
            // Thick line
            commands.push(draw_thick_line(x, y_top, y_bottom, thick_width));
            x + thick_width
        }

        BarlineType::Dashed => {
            // Draw dashed line as series of short segments
            let dash_length = spatium * 0.5;
            let dash_gap = spatium * 0.3;
            let mut y = y_top;
            while y < y_bottom {
                let y_end = (y + dash_length).min(y_bottom);
                commands.push(PaintCommand::line(
                    Point::new(x + thin_width / 2.0, y),
                    Point::new(x + thin_width / 2.0, y_end),
                    Color::BLACK,
                    thin_width,
                ));
                y += dash_length + dash_gap;
            }
            thin_width
        }

        BarlineType::Dotted => {
            // Draw dotted line as series of dots
            let dot_spacing = spatium * 0.4;
            let dot_radius = thin_width;
            let mut y = y_top;
            while y <= y_bottom {
                commands.push(PaintCommand::filled_rect(
                    Rect::new(
                        x,
                        y - dot_radius / 2.0,
                        x + dot_radius,
                        y + dot_radius / 2.0,
                    ),
                    Color::BLACK,
                ));
                y += dot_spacing;
            }
            dot_radius
        }

        BarlineType::Short => {
            // Only middle two spaces
            let short_top = -spatium;
            let short_bottom = spatium;
            commands.push(draw_thin_line(x, short_top, short_bottom, thin_width));
            thin_width
        }

        BarlineType::Tick => {
            // Short tick above staff
            let tick_top = y_top - spatium;
            let tick_bottom = y_top;
            commands.push(draw_thin_line(x, tick_top, tick_bottom, thin_width));
            thin_width
        }
    };

    let bbox = Rect::new(0.0, y_top, total_width, y_bottom);
    let shape = Shape::from_rect(bbox);
    let layout = LayoutData::new(Point::ZERO, bbox, shape);

    let semantic_id = SemanticId::new(ElementType::Barline, params.id);
    let node = SceneNode::leaf(semantic_id, commands)
        .with_metadata("barline-type", format!("{:?}", params.barline_type));

    (layout, node)
}

/// Draw a thin barline.
fn draw_thin_line(x: f64, y_top: f64, y_bottom: f64, width: f64) -> PaintCommand {
    PaintCommand::line(
        Point::new(x + width / 2.0, y_top),
        Point::new(x + width / 2.0, y_bottom),
        Color::BLACK,
        width,
    )
}

/// Draw a thick barline.
fn draw_thick_line(x: f64, y_top: f64, y_bottom: f64, width: f64) -> PaintCommand {
    PaintCommand::filled_rect(Rect::new(x, y_top, x + width, y_bottom), Color::BLACK)
}

/// Draw repeat dots (two dots in spaces 2 and 3).
fn draw_repeat_dots(x: f64, spatium: f64) -> Vec<PaintCommand> {
    vec![
        PaintCommand::glyph(
            glyphs::REPEAT_DOT,
            Point::new(x, -spatium * 0.5), // Space 3
            spatium,
            Color::BLACK,
        ),
        PaintCommand::glyph(
            glyphs::REPEAT_DOT,
            Point::new(x, spatium * 0.5), // Space 2
            spatium,
            Color::BLACK,
        ),
    ]
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
    fn test_layout_single_barline() {
        let ctx = test_ctx();
        let params = BarlineParams {
            id: 1,
            barline_type: BarlineType::Single,
            ..Default::default()
        };

        let (layout, node) = layout_barline(&params, &ctx);

        assert!(!layout.bbox.is_zero_area());
        assert!(node.id.is_some());
        assert_eq!(node.commands.len(), 1);
    }

    #[test]
    fn test_layout_double_barline() {
        let ctx = test_ctx();
        let params = BarlineParams {
            id: 2,
            barline_type: BarlineType::Double,
            ..Default::default()
        };

        let (_layout, node) = layout_barline(&params, &ctx);

        // Double barline has 2 thin lines
        assert_eq!(node.commands.len(), 2);
    }

    #[test]
    fn test_layout_end_barline() {
        let ctx = test_ctx();
        let params = BarlineParams {
            id: 3,
            barline_type: BarlineType::End,
            ..Default::default()
        };

        let (_layout, node) = layout_barline(&params, &ctx);

        // End barline has thin + thick
        assert_eq!(node.commands.len(), 2);
    }

    #[test]
    fn test_layout_start_repeat() {
        let ctx = test_ctx();
        let params = BarlineParams {
            id: 4,
            barline_type: BarlineType::StartRepeat,
            ..Default::default()
        };

        let (_layout, node) = layout_barline(&params, &ctx);

        // Start repeat: thick + thin + 2 dots = 4 commands
        assert_eq!(node.commands.len(), 4);
    }

    #[test]
    fn test_layout_end_repeat() {
        let ctx = test_ctx();
        let params = BarlineParams {
            id: 5,
            barline_type: BarlineType::EndRepeat,
            ..Default::default()
        };

        let (_layout, node) = layout_barline(&params, &ctx);

        // End repeat: 2 dots + thin + thick = 4 commands
        assert_eq!(node.commands.len(), 4);
    }

    #[test]
    fn test_barline_widths() {
        assert!(BarlineType::Single.width() < BarlineType::Double.width());
        assert!(BarlineType::Double.width() < BarlineType::StartRepeat.width());
    }

    #[test]
    fn test_barline_has_dots() {
        assert!(BarlineType::StartRepeat.has_dots());
        assert!(BarlineType::EndRepeat.has_dots());
        assert!(!BarlineType::Single.has_dots());
        assert!(!BarlineType::End.has_dots());
    }
}
