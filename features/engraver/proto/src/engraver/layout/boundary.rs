//! Measure boundary constraint utilities.
//!
//! This module provides utilities for enforcing measure boundaries on
//! music notation elements (tuplet brackets, rhythm slashes, beams, chord symbols).
//! The approach follows MuseScore's hierarchical constraint system where elements
//! are clamped to not extend past barlines.
//!
//! # MuseScore Reference
//!
//! MuseScore enforces measure boundaries through:
//! 1. **Segment Level**: `Segment::minRight()` and `Segment::minLeft()` using Shape-based bounding
//! 2. **Measure Level**: `getMeasureStartEndPos()` for explicit boundaries
//! 3. **Element Level**: Individual elements positioned relative to their segment's constrained width

use kurbo::Point;

use crate::engraver::layout::shape::Shape;

/// Padding from barlines for various element types (in spatiums).
///
/// These values are derived from MuseScore's style settings and provide
/// appropriate visual separation between notation elements and barlines.
pub mod padding {
    /// Tuplet bracket extension padding from barline (MuseScore `tupletBracketPadding`).
    pub const TUPLET_BRACKET: f64 = 0.6;

    /// Rhythm slash/notehead padding from barline.
    pub const RHYTHM_SLASH: f64 = 0.3;

    /// Harmony (chord symbol) text padding from barline.
    pub const CHORD_SYMBOL: f64 = 0.25;

    /// Beam endpoint padding from barline.
    pub const BEAM: f64 = 0.2;

    /// Absolute minimum spacing (used when squeezing measures tight).
    pub const MIN_SPACING: f64 = 0.1;
}

/// Clamp an X position to not exceed measure boundary.
///
/// This ensures elements like tuplet brackets don't extend past the barline.
///
/// # Arguments
/// * `x` - The original X position
/// * `boundary` - Optional measure right boundary
/// * `padding` - Padding to apply (in spatiums)
/// * `spatium` - Staff space size for converting padding
///
/// # Returns
/// The clamped X position (either original or reduced to fit within boundary)
#[inline]
#[must_use]
pub fn clamp_to_boundary(x: f64, boundary: Option<f64>, padding: f64, spatium: f64) -> f64 {
    match boundary {
        Some(b) => x.min(b - padding * spatium),
        None => x,
    }
}

/// Clamp a point's X coordinate to measure boundary.
///
/// # Arguments
/// * `point` - The original point
/// * `boundary` - Optional measure right boundary
/// * `padding` - Padding to apply (in spatiums)
/// * `spatium` - Staff space size for converting padding
///
/// # Returns
/// A new point with clamped X coordinate
#[inline]
#[must_use]
pub fn clamp_point_to_boundary(
    point: Point,
    boundary: Option<f64>,
    padding: f64,
    spatium: f64,
) -> Point {
    Point::new(
        clamp_to_boundary(point.x, boundary, padding, spatium),
        point.y,
    )
}

/// Calculate the overhang of a shape past a boundary.
///
/// # Arguments
/// * `shape` - The shape to check
/// * `boundary` - The right boundary
/// * `padding` - Padding to apply (already in points)
///
/// # Returns
/// The amount by which the shape extends past the boundary, or 0.0 if within bounds
#[inline]
#[must_use]
pub fn calculate_overhang(shape: &Shape, boundary: f64, padding: f64) -> f64 {
    let overhang = shape.right() - (boundary - padding);
    overhang.max(0.0)
}

/// Translate a shape left to fit within measure boundaries.
///
/// If the shape's right edge exceeds the boundary (minus padding),
/// the entire shape is translated left by the overhang amount.
///
/// # Arguments
/// * `shape` - The shape to constrain (mutated in place via return)
/// * `boundary` - The right boundary
/// * `padding` - Padding to apply (already in points)
///
/// # Returns
/// A new shape translated to fit within the boundary, or the original if no adjustment needed
#[must_use]
pub fn constrain_shape_to_boundary(shape: Shape, boundary: f64, padding: f64) -> Shape {
    let overhang = calculate_overhang(&shape, boundary, padding);
    if overhang > 0.0 {
        shape.translate(Point::new(-overhang, 0.0))
    } else {
        shape
    }
}

/// Context for applying boundary constraints to a measure's elements.
///
/// This struct holds the boundary parameters needed when laying out
/// elements within a measure, allowing consistent constraint application.
#[derive(Debug, Clone, Copy)]
pub struct BoundaryContext {
    /// Right boundary of the measure (in points)
    pub right_boundary: Option<f64>,
    /// Staff space size (in points)
    pub spatium: f64,
}

impl BoundaryContext {
    /// Create a new boundary context.
    #[must_use]
    pub const fn new(right_boundary: Option<f64>, spatium: f64) -> Self {
        Self {
            right_boundary,
            spatium,
        }
    }

    /// Clamp an X position using tuplet bracket padding.
    #[inline]
    #[must_use]
    pub fn clamp_tuplet(&self, x: f64) -> f64 {
        clamp_to_boundary(
            x,
            self.right_boundary,
            padding::TUPLET_BRACKET,
            self.spatium,
        )
    }

    /// Clamp an X position using rhythm slash padding.
    #[inline]
    #[must_use]
    pub fn clamp_rhythm(&self, x: f64) -> f64 {
        clamp_to_boundary(x, self.right_boundary, padding::RHYTHM_SLASH, self.spatium)
    }

    /// Clamp an X position using beam padding.
    #[inline]
    #[must_use]
    pub fn clamp_beam(&self, x: f64) -> f64 {
        clamp_to_boundary(x, self.right_boundary, padding::BEAM, self.spatium)
    }

    /// Clamp an X position using chord symbol padding.
    #[inline]
    #[must_use]
    pub fn clamp_chord_symbol(&self, x: f64) -> f64 {
        clamp_to_boundary(x, self.right_boundary, padding::CHORD_SYMBOL, self.spatium)
    }

    /// Check if a position exceeds the boundary.
    #[inline]
    #[must_use]
    pub fn exceeds_boundary(&self, x: f64, padding: f64) -> bool {
        match self.right_boundary {
            Some(b) => x > (b - padding * self.spatium),
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clamp_to_boundary_within() {
        // X position within boundary should not change
        let x = 50.0;
        let result = clamp_to_boundary(x, Some(100.0), padding::TUPLET_BRACKET, 5.0);
        assert_eq!(result, 50.0);
    }

    #[test]
    fn test_clamp_to_boundary_exceeds() {
        // X position exceeding boundary should be clamped
        let x = 98.0;
        let spatium = 5.0;
        let boundary = 100.0;
        let result = clamp_to_boundary(x, Some(boundary), padding::TUPLET_BRACKET, spatium);
        // Expected: 100.0 - (0.6 * 5.0) = 97.0
        assert_eq!(result, 97.0);
    }

    #[test]
    fn test_clamp_to_boundary_no_boundary() {
        // No boundary means no clamping
        let x = 150.0;
        let result = clamp_to_boundary(x, None, padding::TUPLET_BRACKET, 5.0);
        assert_eq!(result, 150.0);
    }

    #[test]
    fn test_boundary_context() {
        let ctx = BoundaryContext::new(Some(100.0), 5.0);

        // Within boundary
        assert_eq!(ctx.clamp_tuplet(50.0), 50.0);

        // Exceeds boundary
        assert_eq!(ctx.clamp_tuplet(98.0), 97.0); // 100 - 0.6*5 = 97

        // Different padding types
        assert_eq!(ctx.clamp_rhythm(99.0), 98.5); // 100 - 0.3*5 = 98.5
        assert_eq!(ctx.clamp_beam(99.5), 99.0); // 100 - 0.2*5 = 99
    }

    #[test]
    fn test_exceeds_boundary() {
        let ctx = BoundaryContext::new(Some(100.0), 5.0);

        assert!(!ctx.exceeds_boundary(50.0, padding::TUPLET_BRACKET));
        assert!(ctx.exceeds_boundary(98.0, padding::TUPLET_BRACKET)); // 98 > 100 - 3 = 97
        assert!(!ctx.exceeds_boundary(96.0, padding::TUPLET_BRACKET)); // 96 <= 97
    }
}
