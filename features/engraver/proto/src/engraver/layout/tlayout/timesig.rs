//! Time signature layout implementation.
//!
//! Handles layout of time signatures including common time, cut time, and numeric.

use kurbo::{Point, Rect};
use peniko::Color;

use crate::engraver::layout::context::LayoutContext;
use crate::engraver::layout::shape::Shape;
use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::PaintCommand;

use super::LayoutData;

/// SMuFL codepoints for time signature glyphs.
pub mod glyphs {
    /// Time signature 0
    pub const TIMESIG_0: char = '\u{E080}';
    /// Time signature 1
    pub const TIMESIG_1: char = '\u{E081}';
    /// Time signature 2
    pub const TIMESIG_2: char = '\u{E082}';
    /// Time signature 3
    pub const TIMESIG_3: char = '\u{E083}';
    /// Time signature 4
    pub const TIMESIG_4: char = '\u{E084}';
    /// Time signature 5
    pub const TIMESIG_5: char = '\u{E085}';
    /// Time signature 6
    pub const TIMESIG_6: char = '\u{E086}';
    /// Time signature 7
    pub const TIMESIG_7: char = '\u{E087}';
    /// Time signature 8
    pub const TIMESIG_8: char = '\u{E088}';
    /// Time signature 9
    pub const TIMESIG_9: char = '\u{E089}';
    /// Common time (C)
    pub const TIMESIG_COMMON: char = '\u{E08A}';
    /// Cut time (C with slash)
    pub const TIMESIG_CUT: char = '\u{E08B}';
    /// Plus sign for additive meters
    pub const TIMESIG_PLUS: char = '\u{E08C}';
    /// Fraction slash
    pub const TIMESIG_FRACTION_SLASH: char = '\u{E08E}';
}

/// Time signature type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeSigType {
    /// Common time (4/4)
    Common,
    /// Cut time (2/2)
    Cut,
    /// Numeric time signature
    Numeric { numerator: u8, denominator: u8 },
    /// Single number (no denominator)
    SingleNumber(u8),
    /// Additive meter (e.g., 3+2+2/8)
    Additive { groups: Vec<u8>, denominator: u8 },
}

impl Default for TimeSigType {
    fn default() -> Self {
        Self::Numeric {
            numerator: 4,
            denominator: 4,
        }
    }
}

/// Time signature layout parameters.
#[derive(Debug, Clone, Default)]
pub struct TimeSigParams {
    /// Unique identifier
    pub id: u64,
    /// Time signature type
    pub sig_type: TimeSigType,
    /// Large time signature (spans entire staff height)
    pub large: bool,
    /// Optional glyph color override. `None` renders the default black; mid-chart
    /// meter changes pass a red so they stand out from the prevailing prefix.
    pub color: Option<Color>,
}

/// Get the SMuFL glyph for a digit.
const fn digit_glyph(digit: u8) -> char {
    match digit {
        0 => glyphs::TIMESIG_0,
        1 => glyphs::TIMESIG_1,
        2 => glyphs::TIMESIG_2,
        3 => glyphs::TIMESIG_3,
        4 => glyphs::TIMESIG_4,
        5 => glyphs::TIMESIG_5,
        6 => glyphs::TIMESIG_6,
        7 => glyphs::TIMESIG_7,
        8 => glyphs::TIMESIG_8,
        9 => glyphs::TIMESIG_9,
        _ => glyphs::TIMESIG_0,
    }
}

/// Layout a time signature.
#[must_use]
pub fn layout_timesig(params: &TimeSigParams, ctx: &LayoutContext) -> (LayoutData, SceneNode) {
    let spatium = ctx.spatium();
    let scale = 1.0;
    let color = params.color.unwrap_or(Color::BLACK);

    let mut commands = Vec::new();

    match &params.sig_type {
        TimeSigType::Common => {
            commands.push(PaintCommand::glyph(
                glyphs::TIMESIG_COMMON,
                Point::new(0.0, 0.0),
                spatium * scale,
                color,
            ));

            let width = spatium * 1.8;
            let height = spatium * 4.0;
            let bbox = Rect::new(0.0, -height / 2.0, width, height / 2.0);
            let shape = Shape::from_rect(bbox);
            let layout = LayoutData::new(Point::ZERO, bbox, shape);

            let semantic_id = SemanticId::new(ElementType::TimeSignature, params.id);
            let node = SceneNode::leaf(semantic_id, commands).with_metadata("timesig", "C");

            (layout, node)
        }

        TimeSigType::Cut => {
            commands.push(PaintCommand::glyph(
                glyphs::TIMESIG_CUT,
                Point::new(0.0, 0.0),
                spatium * scale,
                color,
            ));

            let width = spatium * 1.8;
            let height = spatium * 4.0;
            let bbox = Rect::new(0.0, -height / 2.0, width, height / 2.0);
            let shape = Shape::from_rect(bbox);
            let layout = LayoutData::new(Point::ZERO, bbox, shape);

            let semantic_id = SemanticId::new(ElementType::TimeSignature, params.id);
            let node = SceneNode::leaf(semantic_id, commands).with_metadata("timesig", "C|");

            (layout, node)
        }

        TimeSigType::Numeric {
            numerator,
            denominator,
        } => {
            let (num_commands, num_width) =
                layout_number(*numerator, 0.0, -spatium, spatium * scale, color);
            let (denom_commands, denom_width) =
                layout_number(*denominator, 0.0, spatium, spatium * scale, color);

            commands.extend(num_commands);
            commands.extend(denom_commands);

            let width = num_width.max(denom_width);
            let height = spatium * 4.0;
            let bbox = Rect::new(0.0, -height / 2.0, width, height / 2.0);
            let shape = Shape::from_rect(bbox);
            let layout = LayoutData::new(Point::ZERO, bbox, shape);

            let semantic_id = SemanticId::new(ElementType::TimeSignature, params.id);
            let node = SceneNode::leaf(semantic_id, commands)
                .with_metadata("timesig", format!("{}/{}", numerator, denominator));

            (layout, node)
        }

        TimeSigType::SingleNumber(n) => {
            let (num_commands, num_width) = layout_number(*n, 0.0, 0.0, spatium * scale, color);
            commands.extend(num_commands);

            let height = spatium * 2.0;
            let bbox = Rect::new(0.0, -height / 2.0, num_width, height / 2.0);
            let shape = Shape::from_rect(bbox);
            let layout = LayoutData::new(Point::ZERO, bbox, shape);

            let semantic_id = SemanticId::new(ElementType::TimeSignature, params.id);
            let node =
                SceneNode::leaf(semantic_id, commands).with_metadata("timesig", n.to_string());

            (layout, node)
        }

        TimeSigType::Additive {
            groups,
            denominator,
        } => {
            // Layout numerator with plus signs: 3+2+2
            let mut x = 0.0;
            let y_num = -spatium;
            let digit_width = spatium;

            for (i, group) in groups.iter().enumerate() {
                if i > 0 {
                    // Add plus sign
                    commands.push(PaintCommand::glyph(
                        glyphs::TIMESIG_PLUS,
                        Point::new(x, y_num),
                        spatium * scale,
                        color,
                    ));
                    x += spatium * 0.8;
                }

                commands.push(PaintCommand::glyph(
                    digit_glyph(*group),
                    Point::new(x, y_num),
                    spatium * scale,
                    color,
                ));
                x += digit_width;
            }

            let num_width = x;

            // Layout denominator centered below
            let (denom_commands, denom_width) =
                layout_number(*denominator, 0.0, spatium, spatium * scale, color);

            // Center the denominator under the numerator
            let x_offset = (num_width - denom_width) / 2.0;
            for cmd in denom_commands {
                if let PaintCommand::Glyph {
                    codepoint,
                    position,
                    size,
                    color,
                } = cmd
                {
                    commands.push(PaintCommand::glyph(
                        codepoint,
                        Point::new(position.x + x_offset, position.y),
                        size,
                        color,
                    ));
                }
            }

            let width = num_width.max(denom_width);
            let height = spatium * 4.0;
            let bbox = Rect::new(0.0, -height / 2.0, width, height / 2.0);
            let shape = Shape::from_rect(bbox);
            let layout = LayoutData::new(Point::ZERO, bbox, shape);

            let groups_str: Vec<String> = groups.iter().map(|g| g.to_string()).collect();
            let semantic_id = SemanticId::new(ElementType::TimeSignature, params.id);
            let node = SceneNode::leaf(semantic_id, commands).with_metadata(
                "timesig",
                format!("{}+/{}", groups_str.join("+"), denominator),
            );

            (layout, node)
        }
    }
}

/// Layout a number as time signature digits.
fn layout_number(n: u8, x: f64, y: f64, size: f64, color: Color) -> (Vec<PaintCommand>, f64) {
    let mut commands = Vec::new();
    let digit_width = size;

    if n < 10 {
        commands.push(PaintCommand::glyph(
            digit_glyph(n),
            Point::new(x, y),
            size,
            color,
        ));
        (commands, digit_width)
    } else {
        // Two digit number
        let tens = n / 10;
        let ones = n % 10;

        commands.push(PaintCommand::glyph(
            digit_glyph(tens),
            Point::new(x, y),
            size,
            color,
        ));
        commands.push(PaintCommand::glyph(
            digit_glyph(ones),
            Point::new(x + digit_width, y),
            size,
            color,
        ));

        (commands, digit_width * 2.0)
    }
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
    fn test_layout_common_time() {
        let ctx = test_ctx();
        let params = TimeSigParams {
            id: 1,
            sig_type: TimeSigType::Common,
            ..Default::default()
        };

        let (layout, node) = layout_timesig(&params, &ctx);

        assert!(!layout.bbox.is_zero_area());
        assert!(node.id.is_some());
        assert_eq!(node.commands.len(), 1);
    }

    #[test]
    fn test_layout_cut_time() {
        let ctx = test_ctx();
        let params = TimeSigParams {
            id: 2,
            sig_type: TimeSigType::Cut,
            ..Default::default()
        };

        let (_layout, node) = layout_timesig(&params, &ctx);

        assert_eq!(node.commands.len(), 1);
    }

    #[test]
    fn test_layout_numeric_4_4() {
        let ctx = test_ctx();
        let params = TimeSigParams {
            id: 3,
            sig_type: TimeSigType::Numeric {
                numerator: 4,
                denominator: 4,
            },
            ..Default::default()
        };

        let (_layout, node) = layout_timesig(&params, &ctx);

        // Single digit numerator + single digit denominator = 2 commands
        assert_eq!(node.commands.len(), 2);
    }

    #[test]
    fn test_layout_numeric_6_8() {
        let ctx = test_ctx();
        let params = TimeSigParams {
            id: 4,
            sig_type: TimeSigType::Numeric {
                numerator: 6,
                denominator: 8,
            },
            ..Default::default()
        };

        let (_layout, node) = layout_timesig(&params, &ctx);

        assert_eq!(node.commands.len(), 2);
    }

    #[test]
    fn test_layout_numeric_12_8() {
        let ctx = test_ctx();
        let params = TimeSigParams {
            id: 5,
            sig_type: TimeSigType::Numeric {
                numerator: 12,
                denominator: 8,
            },
            ..Default::default()
        };

        let (_layout, node) = layout_timesig(&params, &ctx);

        // Two digit numerator (2 glyphs) + single digit denominator = 3 commands
        assert_eq!(node.commands.len(), 3);
    }

    #[test]
    fn test_layout_additive_meter() {
        let ctx = test_ctx();
        let params = TimeSigParams {
            id: 6,
            sig_type: TimeSigType::Additive {
                groups: vec![3, 2, 2],
                denominator: 8,
            },
            ..Default::default()
        };

        let (_layout, node) = layout_timesig(&params, &ctx);

        // 3 digits + 2 plus signs in numerator + 1 digit denominator = 6 commands
        assert_eq!(node.commands.len(), 6);
    }

    #[test]
    fn test_digit_glyphs() {
        assert_eq!(digit_glyph(0), glyphs::TIMESIG_0);
        assert_eq!(digit_glyph(4), glyphs::TIMESIG_4);
        assert_eq!(digit_glyph(9), glyphs::TIMESIG_9);
    }
}
