//! Shape-based collision detection for chord symbols.
//!
//! This module provides collision detection using the Shape system from
//! `engraver::layout::shape`. It replaces the heuristic-based collision
//! detection that was previously done in the paint pass.
//!
//! # Architecture
//!
//! The collision detection follows MuseScore's pattern:
//! 1. **Measure pass**: Pre-compute chord layouts with shapes
//! 2. **Layout pass**: Use shapes to compute collision-free positions
//! 3. **Paint pass**: Render at pre-computed positions (no collision fixing)
//!
//! # Horizontal Slice Algorithm
//!
//! The Shape system uses a "horizontal slice" approach for accurate collision
//! detection. Instead of just comparing bounding boxes, we check each rectangle
//! pair for vertical overlap before computing horizontal distances.

use kurbo::Point;

use super::measure_pass::ChordLayoutData;
use crate::engraver::layout::shape::Shape;

/// Context for chord collision detection in a measure.
///
/// This holds the chord layout data and provides methods to:
/// - Check if two chords would collide at their proposed positions
/// - Compute the minimum offset needed to avoid collision
/// - Find collision-free positions for all chords in a measure
#[derive(Debug)]
pub struct ChordCollisionContext<'a> {
    /// Chord layouts from the measure pass.
    chord_layouts: &'a [ChordLayoutData],
    /// Minimum gap between chord symbols (in points).
    min_gap: f64,
}

impl<'a> ChordCollisionContext<'a> {
    /// Create a new collision context.
    ///
    /// # Arguments
    /// * `chord_layouts` - Pre-computed chord layouts from measure pass
    /// * `min_gap` - Minimum gap between chord symbols (in points)
    #[must_use]
    pub fn new(chord_layouts: &'a [ChordLayoutData], min_gap: f64) -> Self {
        Self {
            chord_layouts,
            min_gap,
        }
    }

    /// Check if two chord shapes would collide when positioned.
    ///
    /// # Arguments
    /// * `shape1` - First chord's collision shape (at its position)
    /// * `shape2` - Second chord's collision shape (at its position)
    ///
    /// # Returns
    /// `true` if the shapes would collide (including min_gap)
    #[must_use]
    pub fn shapes_collide(&self, shape1: &Shape, shape2: &Shape) -> bool {
        // If either shape is empty, no collision
        if shape1.is_empty() || shape2.is_empty() {
            return false;
        }

        // Check if minimum horizontal distance exceeds 0 (meaning they overlap)
        let distance = shape1.min_horizontal_distance(shape2, self.min_gap);
        distance > 0.0
    }

    /// Compute the minimum offset needed to avoid collision between two positioned shapes.
    ///
    /// Returns the horizontal distance that `shape2` needs to move right to avoid collision.
    /// Returns 0.0 if no collision would occur.
    ///
    /// # Arguments
    /// * `shape1` - First chord's collision shape (at its position)
    /// * `shape2` - Second chord's collision shape (at its position)
    #[must_use]
    pub fn collision_offset(&self, shape1: &Shape, shape2: &Shape) -> f64 {
        shape1
            .min_horizontal_distance(shape2, self.min_gap)
            .max(0.0)
    }

    /// Compute collision-free X positions for all chords in the measure.
    ///
    /// This is the main entry point for collision resolution. Given the initial
    /// X positions from segment layout, it adjusts positions to avoid collisions.
    ///
    /// # Arguments
    /// * `segment_positions` - X positions of segments (where chords would be placed)
    /// * `measure_x` - Left edge of measure content
    /// * `measure_width` - Width of measure content (for clamping)
    ///
    /// # Returns
    /// Vector of adjusted X positions for each chord layout
    #[must_use]
    pub fn resolve_positions(
        &self,
        segment_positions: &[f64],
        measure_x: f64,
        measure_width: f64,
    ) -> Vec<f64> {
        if self.chord_layouts.is_empty() {
            return Vec::new();
        }

        let mut positions: Vec<f64> = self
            .chord_layouts
            .iter()
            .map(|layout| {
                // Get segment position for this chord
                let seg_x = segment_positions
                    .get(layout.segment_index)
                    .copied()
                    .unwrap_or(0.0);
                measure_x + seg_x
            })
            .collect();

        // Check each pair of adjacent visible chords for collision
        for i in 0..self.chord_layouts.len().saturating_sub(1) {
            let layout1 = &self.chord_layouts[i];
            let layout2 = &self.chord_layouts[i + 1];

            // Skip if either is not visible
            if !layout1.visible || !layout2.visible {
                continue;
            }

            // Position shapes at their current positions
            let shape1 = layout1.shape.translate(Point::new(positions[i], 0.0));
            let shape2 = layout2.shape.translate(Point::new(positions[i + 1], 0.0));

            // Check for collision and compute offset
            let offset = self.collision_offset(&shape1, &shape2);
            if offset > 0.0 {
                // Strategy: prefer shifting the first chord left rather than stretching.
                // The first chord of a measure can overhang into the clef/prefix area,
                // which looks more natural than stretching the whole measure.

                // For the first chord (i == 0), allow more aggressive left shift.
                // It can overhang up to half its width past the measure start.
                let max_left_overhang = if i == 0 {
                    layout1.text_width * 0.5 // Allow 50% overhang for first chord
                } else {
                    0.0 // Other chords can't go past their segment start
                };

                // How much can we shift left?
                let available_left = (positions[i] - measure_x) + max_left_overhang;

                // Prefer shifting left more aggressively (up to 70% of offset)
                let preferred_left_ratio = 0.7;
                let left_shift = (offset * preferred_left_ratio).min(available_left);
                positions[i] -= left_shift;

                // Move the second chord right by the remaining offset
                let right_shift = offset - left_shift;
                positions[i + 1] += right_shift;

                // Clamp to measure bounds
                positions[i + 1] =
                    positions[i + 1].min(measure_x + measure_width - layout2.text_width);
            }
        }

        positions
    }

    /// Get the number of chord layouts in this context.
    #[must_use]
    pub fn len(&self) -> usize {
        self.chord_layouts.len()
    }

    /// Check if there are no chord layouts.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.chord_layouts.is_empty()
    }
}

/// Compute collision-free positions for chords using cached layout data.
///
/// This is a convenience function that creates a `ChordCollisionContext` and
/// resolves positions in one call.
///
/// # Arguments
/// * `chord_layouts` - Pre-computed chord layouts from measure pass
/// * `segment_positions` - X positions of segments (where chords would be placed)
/// * `measure_x` - Left edge of measure content
/// * `measure_width` - Width of measure content (for clamping)
/// * `min_gap` - Minimum gap between chord symbols (in points)
///
/// # Returns
/// Vector of adjusted X positions for each chord layout
#[must_use]
pub fn resolve_chord_positions(
    chord_layouts: &[ChordLayoutData],
    segment_positions: &[f64],
    measure_x: f64,
    measure_width: f64,
    min_gap: f64,
) -> Vec<f64> {
    let ctx = ChordCollisionContext::new(chord_layouts, min_gap);
    ctx.resolve_positions(segment_positions, measure_x, measure_width)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Rect;

    fn make_chord_layout(segment_index: usize, width: f64, x_offset: f64) -> ChordLayoutData {
        let bbox = Rect::new(x_offset, -12.0, x_offset + width, 2.0);
        ChordLayoutData {
            position: Point::ZERO,
            bbox,
            shape: Shape::from_rect(bbox),
            text_width: width,
            segment_index,
            visible: true,
            chord_index: segment_index,
        }
    }

    #[test]
    fn test_no_collision_with_single_chord() {
        let layouts = vec![make_chord_layout(0, 30.0, 0.0)];
        let segment_positions = vec![0.0, 25.0, 50.0, 75.0];

        let positions = resolve_chord_positions(&layouts, &segment_positions, 100.0, 200.0, 2.0);

        assert_eq!(positions.len(), 1);
        assert!((positions[0] - 100.0).abs() < 0.01); // measure_x + segment 0
    }

    #[test]
    fn test_no_collision_with_spaced_chords() {
        let layouts = vec![
            make_chord_layout(0, 30.0, 0.0),
            make_chord_layout(2, 30.0, 0.0),
        ];
        // Wide spacing between segments
        let segment_positions = vec![0.0, 50.0, 100.0, 150.0];

        let positions = resolve_chord_positions(&layouts, &segment_positions, 0.0, 300.0, 2.0);

        assert_eq!(positions.len(), 2);
        // With wide spacing, chords should remain at original positions
        assert!((positions[0] - 0.0).abs() < 0.01);
        assert!((positions[1] - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_collision_resolved() {
        // Two chords that would overlap without collision detection
        let layouts = vec![
            make_chord_layout(0, 40.0, 0.0), // 40pt wide chord
            make_chord_layout(1, 30.0, 0.0), // 30pt wide chord right after
        ];
        // Tight spacing: segment 1 starts at 25pt, but chord 0 is 40pt wide
        let segment_positions = vec![0.0, 25.0, 50.0, 75.0];

        let positions = resolve_chord_positions(&layouts, &segment_positions, 0.0, 200.0, 2.0);

        assert_eq!(positions.len(), 2);
        // First chord should be moved left, second chord moved right
        // Collision: chord1 at 0 is 40pt wide, chord2 at 25pt would overlap by 15pt + 2pt gap
        // Total overlap = 40 - 25 + 2 = 17pt
        let gap = positions[1] - (positions[0] + 40.0);
        assert!(gap >= 1.9, "Gap should be at least min_gap (2pt): {gap}");
    }

    #[test]
    fn test_collision_context_shapes_collide() {
        let layouts = vec![make_chord_layout(0, 30.0, 0.0)];
        let ctx = ChordCollisionContext::new(&layouts, 2.0);

        let shape1 = Shape::from_rect(Rect::new(0.0, 0.0, 30.0, 10.0));
        let shape2 = Shape::from_rect(Rect::new(25.0, 0.0, 55.0, 10.0));
        let shape3 = Shape::from_rect(Rect::new(100.0, 0.0, 130.0, 10.0));

        // Overlapping shapes should collide
        assert!(ctx.shapes_collide(&shape1, &shape2));

        // Well-separated shapes should not collide
        assert!(!ctx.shapes_collide(&shape1, &shape3));
    }

    #[test]
    fn test_collision_context_no_vertical_overlap() {
        let layouts = vec![make_chord_layout(0, 30.0, 0.0)];
        let ctx = ChordCollisionContext::new(&layouts, 2.0);

        // Two shapes that overlap horizontally but NOT vertically
        let shape1 = Shape::from_rect(Rect::new(0.0, 0.0, 30.0, 10.0));
        let shape2 = Shape::from_rect(Rect::new(25.0, 20.0, 55.0, 30.0)); // Different Y range

        // Should NOT collide because no vertical overlap
        assert!(!ctx.shapes_collide(&shape1, &shape2));
    }

    #[test]
    fn test_empty_context() {
        let layouts: Vec<ChordLayoutData> = vec![];
        let ctx = ChordCollisionContext::new(&layouts, 2.0);

        assert!(ctx.is_empty());
        assert_eq!(ctx.len(), 0);

        let positions = ctx.resolve_positions(&[0.0, 25.0], 0.0, 100.0);
        assert!(positions.is_empty());
    }
}
