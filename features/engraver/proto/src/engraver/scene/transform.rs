//! Affine transform helpers for scene graph rendering.
//!
//! Provides utilities for converting transforms to SVG format
//! and managing transform stacks during traversal.

use kurbo::{Affine, Point, Vec2};

/// Convert an Affine transform to SVG transform attribute format.
///
/// The SVG transform is generated using the matrix form:
/// `matrix(a, b, c, d, e, f)`
///
/// # Arguments
/// * `transform` - The affine transform
/// * `precision` - Number of decimal places for coordinates
#[must_use]
pub fn affine_to_svg_transform(transform: &Affine, precision: u8) -> String {
    let coeffs = transform.as_coeffs();
    let p = precision as usize;

    // Check for identity transform
    if is_identity(transform) {
        return String::new();
    }

    // Check for simple translation
    if is_translation(transform) {
        let tx = coeffs[4];
        let ty = coeffs[5];
        return format!("translate({tx:.p$} {ty:.p$})");
    }

    // Check for simple scale
    if is_scale(transform) {
        let sx = coeffs[0];
        let sy = coeffs[3];
        let tx = coeffs[4];
        let ty = coeffs[5];

        if tx.abs() < 1e-10 && ty.abs() < 1e-10 {
            if (sx - sy).abs() < 1e-10 {
                return format!("scale({sx:.p$})");
            }
            return format!("scale({sx:.p$} {sy:.p$})");
        }
    }

    // Check for simple rotation around origin
    if let Some(angle) = rotation_angle(transform) {
        let tx = coeffs[4];
        let ty = coeffs[5];

        if tx.abs() < 1e-10 && ty.abs() < 1e-10 {
            return format!("rotate({:.p$})", angle.to_degrees());
        }
    }

    // General matrix form
    format!(
        "matrix({:.p$} {:.p$} {:.p$} {:.p$} {:.p$} {:.p$})",
        coeffs[0], coeffs[1], coeffs[2], coeffs[3], coeffs[4], coeffs[5]
    )
}

/// Check if a transform is the identity transform.
#[must_use]
pub fn is_identity(transform: &Affine) -> bool {
    let coeffs = transform.as_coeffs();
    (coeffs[0] - 1.0).abs() < 1e-10
        && coeffs[1].abs() < 1e-10
        && coeffs[2].abs() < 1e-10
        && (coeffs[3] - 1.0).abs() < 1e-10
        && coeffs[4].abs() < 1e-10
        && coeffs[5].abs() < 1e-10
}

/// Check if a transform is a pure translation (no rotation or scale).
#[must_use]
pub fn is_translation(transform: &Affine) -> bool {
    let coeffs = transform.as_coeffs();
    (coeffs[0] - 1.0).abs() < 1e-10
        && coeffs[1].abs() < 1e-10
        && coeffs[2].abs() < 1e-10
        && (coeffs[3] - 1.0).abs() < 1e-10
}

/// Check if a transform is a pure scale (no rotation).
#[must_use]
pub fn is_scale(transform: &Affine) -> bool {
    let coeffs = transform.as_coeffs();
    coeffs[1].abs() < 1e-10 && coeffs[2].abs() < 1e-10
}

/// Extract rotation angle from a transform (if it's a pure rotation).
/// Returns the angle in radians, or None if not a pure rotation.
#[must_use]
pub fn rotation_angle(transform: &Affine) -> Option<f64> {
    let coeffs = transform.as_coeffs();

    // For a rotation matrix: [cos(θ), sin(θ), -sin(θ), cos(θ), 0, 0]
    let cos_theta = coeffs[0];
    let sin_theta = coeffs[1];

    // Verify it's a valid rotation matrix
    let is_rotation = (cos_theta - coeffs[3]).abs() < 1e-10
        && (sin_theta + coeffs[2]).abs() < 1e-10
        && (cos_theta * cos_theta + sin_theta * sin_theta - 1.0).abs() < 1e-10;

    if is_rotation {
        Some(sin_theta.atan2(cos_theta))
    } else {
        None
    }
}

/// Get the translation component of a transform.
#[must_use]
pub fn get_translation(transform: &Affine) -> Vec2 {
    let coeffs = transform.as_coeffs();
    Vec2::new(coeffs[4], coeffs[5])
}

/// Get the scale components of a transform (x and y scale factors).
/// Note: This assumes the transform has no rotation component.
#[must_use]
pub fn get_scale(transform: &Affine) -> (f64, f64) {
    let coeffs = transform.as_coeffs();
    let sx = (coeffs[0] * coeffs[0] + coeffs[1] * coeffs[1]).sqrt();
    let sy = (coeffs[2] * coeffs[2] + coeffs[3] * coeffs[3]).sqrt();
    (sx, sy)
}

/// Create a transform that positions an element at a point.
#[must_use]
pub fn position_at(point: Point) -> Affine {
    Affine::translate((point.x, point.y))
}

/// Create a transform for rotation around a specific point.
#[must_use]
pub fn rotate_around(angle_degrees: f64, center: Point) -> Affine {
    let angle_radians = angle_degrees.to_radians();
    Affine::translate((center.x, center.y))
        * Affine::rotate(angle_radians)
        * Affine::translate((-center.x, -center.y))
}

/// Create a transform for scaling around a specific point.
#[must_use]
pub fn scale_around(scale_x: f64, scale_y: f64, center: Point) -> Affine {
    Affine::translate((center.x, center.y))
        * Affine::scale_non_uniform(scale_x, scale_y)
        * Affine::translate((-center.x, -center.y))
}

/// A stack for managing transforms during scene graph traversal.
#[derive(Debug, Clone)]
pub struct TransformStack {
    stack: Vec<Affine>,
}

impl Default for TransformStack {
    fn default() -> Self {
        Self::new()
    }
}

impl TransformStack {
    /// Create a new transform stack with identity at the base.
    #[must_use]
    pub fn new() -> Self {
        Self {
            stack: vec![Affine::IDENTITY],
        }
    }

    /// Push a local transform onto the stack.
    /// The new current transform is the composition of all transforms.
    pub fn push(&mut self, local_transform: Affine) {
        let current = self.current();
        self.stack.push(current * local_transform);
    }

    /// Pop the top transform from the stack.
    /// Returns the popped transform, or `None` if only the base transform remains.
    pub fn pop(&mut self) -> Option<Affine> {
        if self.stack.len() > 1 {
            self.stack.pop()
        } else {
            None
        }
    }

    /// Get the current (composed) transform.
    /// Returns identity if the stack is somehow empty (should never happen).
    #[must_use]
    pub fn current(&self) -> Affine {
        self.stack.last().copied().unwrap_or(Affine::IDENTITY)
    }

    /// Get the depth of the stack (number of pushed transforms).
    #[must_use]
    pub fn depth(&self) -> usize {
        self.stack.len() - 1
    }

    /// Transform a point by the current transform.
    #[must_use]
    pub fn transform_point(&self, point: Point) -> Point {
        self.current() * point
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn test_identity_detection() {
        assert!(is_identity(&Affine::IDENTITY));
        assert!(!is_identity(&Affine::translate((1.0, 0.0))));
    }

    #[test]
    fn test_translation_detection() {
        assert!(is_translation(&Affine::translate((10.0, 20.0))));
        assert!(!is_translation(&Affine::scale(2.0)));
        assert!(!is_translation(&Affine::rotate(0.1)));
    }

    #[test]
    fn test_scale_detection() {
        assert!(is_scale(&Affine::scale(2.0)));
        assert!(is_scale(&Affine::scale_non_uniform(2.0, 3.0)));
        assert!(!is_scale(&Affine::rotate(0.1)));
    }

    #[test]
    fn test_rotation_angle() {
        let rot90 = Affine::rotate(PI / 2.0);
        let angle = rotation_angle(&rot90);
        let Some(angle_value) = angle else {
            panic!("Expected rotation angle to be Some");
        };
        assert!((angle_value - PI / 2.0).abs() < 1e-10);
    }

    #[test]
    fn test_svg_transform_identity() {
        let svg = affine_to_svg_transform(&Affine::IDENTITY, 2);
        assert!(svg.is_empty());
    }

    #[test]
    fn test_svg_transform_translate() {
        let svg = affine_to_svg_transform(&Affine::translate((10.0, 20.0)), 2);
        assert_eq!(svg, "translate(10.00 20.00)");
    }

    #[test]
    fn test_svg_transform_scale() {
        let svg = affine_to_svg_transform(&Affine::scale(2.0), 2);
        assert_eq!(svg, "scale(2.00)");
    }

    #[test]
    fn test_svg_transform_non_uniform_scale() {
        let svg = affine_to_svg_transform(&Affine::scale_non_uniform(2.0, 3.0), 2);
        assert_eq!(svg, "scale(2.00 3.00)");
    }

    #[test]
    fn test_transform_stack() {
        let mut stack = TransformStack::new();
        assert_eq!(stack.depth(), 0);

        stack.push(Affine::translate((10.0, 0.0)));
        assert_eq!(stack.depth(), 1);

        stack.push(Affine::translate((5.0, 0.0)));
        assert_eq!(stack.depth(), 2);

        // Current should be the composed transform
        let pt = stack.transform_point(Point::new(0.0, 0.0));
        assert!((pt.x - 15.0).abs() < 1e-10);

        assert!(stack.pop().is_some());
        assert_eq!(stack.depth(), 1);

        let pt = stack.transform_point(Point::new(0.0, 0.0));
        assert!((pt.x - 10.0).abs() < 1e-10);

        // Pop should return None when only base remains
        assert!(stack.pop().is_some());
        assert!(stack.pop().is_none());
        assert_eq!(stack.depth(), 0);
    }

    #[test]
    fn test_rotate_around() {
        let center = Point::new(50.0, 50.0);
        let transform = rotate_around(90.0, center);

        // Point at (100, 50) should rotate to (50, 100)
        let pt = transform * Point::new(100.0, 50.0);
        assert!((pt.x - 50.0).abs() < 1e-10);
        assert!((pt.y - 100.0).abs() < 1e-10);
    }

    #[test]
    fn test_get_translation() {
        let t = Affine::translate((10.0, 20.0));
        let translation = get_translation(&t);
        assert!((translation.x - 10.0).abs() < 1e-10);
        assert!((translation.y - 20.0).abs() < 1e-10);
    }
}
