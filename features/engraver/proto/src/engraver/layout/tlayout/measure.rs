//! Measure layout implementation.
//!
//! Orchestrates layout of all elements within a measure.

use kurbo::{Point, Rect};
use peniko::Color;

use crate::engraver::layout::context::LayoutContext;
use crate::engraver::layout::segment::{Segment, SegmentType};
use crate::engraver::layout::shape::Shape;
use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::PaintCommand;

use super::LayoutData;
use super::barline::{BarlineParams, BarlineType, layout_barline};

/// Measure layout parameters.
#[derive(Debug, Clone)]
pub struct MeasureParams {
    /// Unique identifier
    pub id: u64,
    /// Measure number
    pub number: u32,
    /// Measure width (after spacing calculation)
    pub width: f64,
    /// Staff height in spatiums
    pub staff_height: f64,
    /// Number of staff lines
    pub staff_lines: u8,
    /// Segments in this measure
    pub segments: Vec<Segment>,
    /// Left barline type
    pub left_barline: Option<BarlineType>,
    /// Right barline type
    pub right_barline: BarlineType,
    /// Whether this is the first measure in a system
    pub first_in_system: bool,
}

impl Default for MeasureParams {
    fn default() -> Self {
        Self {
            id: 0,
            number: 1,
            width: 200.0,
            staff_height: 4.0,
            staff_lines: 5,
            segments: Vec::new(),
            left_barline: None,
            right_barline: BarlineType::Single,
            first_in_system: false,
        }
    }
}

/// Layout a complete measure.
#[must_use]
pub fn layout_measure(params: &MeasureParams, ctx: &LayoutContext) -> (LayoutData, SceneNode) {
    let spatium = ctx.spatium();
    let half_height = params.staff_height * spatium / 2.0;

    // Create measure group node
    let mut measure_node = SceneNode::group(SemanticId::measure(params.id));
    measure_node = measure_node.with_metadata("measure-number", params.number.to_string());

    // Draw staff lines
    let staff_commands = draw_staff_lines(params.width, params.staff_lines, spatium);
    let staff_node = SceneNode::anonymous_leaf(staff_commands);
    measure_node.add_child(staff_node);

    let mut current_x = 0.0;

    // Left barline (usually only for repeats or first measure)
    if let Some(barline_type) = params.left_barline {
        let barline_params = BarlineParams {
            id: params.id * 1000,
            barline_type,
            staff_height: params.staff_height,
            span_staves: false,
        };
        let (barline_layout, barline_node) = layout_barline(&barline_params, ctx);
        let mut positioned_node = barline_node;
        positioned_node.transform = kurbo::Affine::translate((current_x, 0.0));
        measure_node.add_child(positioned_node);
        current_x += barline_layout.bbox.width() + spatium * 0.5;
    }

    // Layout segments
    for segment in &params.segments {
        if let Some(segment_node) = layout_segment(segment, ctx) {
            let mut positioned_node = segment_node;
            positioned_node.transform = kurbo::Affine::translate((current_x + segment.x, 0.0));
            measure_node.add_child(positioned_node);
        }
    }

    // Right barline
    let right_barline_params = BarlineParams {
        id: params.id * 1000 + 1,
        barline_type: params.right_barline,
        staff_height: params.staff_height,
        span_staves: false,
    };
    let (right_barline_layout, right_barline_node) = layout_barline(&right_barline_params, ctx);
    let mut positioned_right = right_barline_node;
    positioned_right.transform =
        kurbo::Affine::translate((params.width - right_barline_layout.bbox.width(), 0.0));
    measure_node.add_child(positioned_right);

    // Calculate total bounding box
    let bbox = Rect::new(0.0, -half_height, params.width, half_height);
    let shape = Shape::from_rect(bbox);
    let layout = LayoutData::new(Point::ZERO, bbox, shape);

    (layout, measure_node)
}

/// Draw staff lines.
fn draw_staff_lines(width: f64, num_lines: u8, spatium: f64) -> Vec<PaintCommand> {
    let mut commands = Vec::new();
    let line_width = spatium * 0.1;
    let half_staff = (num_lines as f64 - 1.0) / 2.0;

    for i in 0..num_lines {
        let y = (i as f64 - half_staff) * spatium;
        commands.push(PaintCommand::line(
            Point::new(0.0, y),
            Point::new(width, y),
            Color::BLACK,
            line_width,
        ));
    }

    commands
}

/// Layout a single segment (placeholder - returns node with segment type info).
fn layout_segment(segment: &Segment, _ctx: &LayoutContext) -> Option<SceneNode> {
    // This is a placeholder that would dispatch to appropriate element layouts
    // based on segment type and contents. In a full implementation, segments
    // would contain element references that get laid out here.

    let seg_type = segment.seg_type;

    if seg_type.intersects(SegmentType::CLEF_TYPE) {
        // Would call layout_clef for each element
        None
    } else if seg_type.intersects(SegmentType::KEY_SIG_TYPE) {
        // Would call layout_keysig for each element
        None
    } else if seg_type.intersects(SegmentType::TIME_SIG_TYPE) {
        // Would call layout_timesig for each element
        None
    } else if seg_type.is_chord_rest() {
        // Would call layout_chord or layout_rest for each element
        None
    } else if seg_type.is_barline() {
        // Would call layout_barline for each element
        None
    } else {
        None
    }
}

/// Layout result for a system (multiple measures on one line).
#[derive(Debug)]
pub struct SystemLayout {
    /// Measures in this system
    pub measures: Vec<(LayoutData, SceneNode)>,
    /// Total width
    pub width: f64,
    /// Total height
    pub height: f64,
}

/// Layout multiple measures into a system.
#[must_use]
pub fn layout_system(
    measures: &[MeasureParams],
    _system_width: f64,
    ctx: &LayoutContext,
) -> (LayoutData, SceneNode) {
    let _spatium = ctx.spatium();

    // Create system group
    let mut system_node = SceneNode::group(SemanticId::new(ElementType::System, 0));

    let mut current_x = 0.0;
    let mut max_height = 0.0_f64;

    for (i, measure_params) in measures.iter().enumerate() {
        let mut params = measure_params.clone();
        params.first_in_system = i == 0;

        let (measure_layout, measure_node) = layout_measure(&params, ctx);

        let mut positioned_node = measure_node;
        positioned_node.transform = kurbo::Affine::translate((current_x, 0.0));
        system_node.add_child(positioned_node);

        current_x += measure_layout.bbox.width();
        max_height = max_height.max(measure_layout.bbox.height());
    }

    let bbox = Rect::new(0.0, -max_height / 2.0, current_x, max_height / 2.0);
    let shape = Shape::from_rect(bbox);
    let layout = LayoutData::new(Point::ZERO, bbox, shape);

    (layout, system_node)
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
    fn test_layout_empty_measure() {
        let ctx = test_ctx();
        let params = MeasureParams {
            id: 1,
            number: 1,
            width: 200.0,
            ..Default::default()
        };

        let (layout, node) = layout_measure(&params, &ctx);

        assert!(!layout.bbox.is_zero_area());
        assert!(node.id.is_some());
        // Should have staff lines + right barline
        assert!(!node.children.is_empty());
    }

    #[test]
    fn test_layout_measure_with_repeat() {
        let ctx = test_ctx();
        let params = MeasureParams {
            id: 2,
            number: 1,
            width: 200.0,
            left_barline: Some(BarlineType::StartRepeat),
            right_barline: BarlineType::EndRepeat,
            ..Default::default()
        };

        let (_layout, node) = layout_measure(&params, &ctx);

        // Should have staff lines + left barline + right barline
        assert!(node.children.len() >= 3);
    }

    #[test]
    fn test_draw_staff_lines() {
        let spatium = 8.0;
        let commands = draw_staff_lines(200.0, 5, spatium);

        // 5 staff lines
        assert_eq!(commands.len(), 5);
    }

    #[test]
    fn test_layout_system() {
        let ctx = test_ctx();
        let measures = vec![
            MeasureParams {
                id: 1,
                number: 1,
                width: 200.0,
                ..Default::default()
            },
            MeasureParams {
                id: 2,
                number: 2,
                width: 200.0,
                ..Default::default()
            },
        ];

        let (layout, node) = layout_system(&measures, 400.0, &ctx);

        assert!(!layout.bbox.is_zero_area());
        // Should have 2 measure children
        assert_eq!(node.children.len(), 2);
    }

    #[test]
    fn test_measure_barline_types() {
        let ctx = test_ctx();

        for barline_type in [BarlineType::Single, BarlineType::Double, BarlineType::End] {
            let params = MeasureParams {
                id: 1,
                right_barline: barline_type,
                ..Default::default()
            };

            let (layout, _node) = layout_measure(&params, &ctx);
            assert!(!layout.bbox.is_zero_area());
        }
    }
}
