//! Shape system for collision detection.
//!
//! This module provides the `Shape` type for representing element boundaries
//! in music notation layout, based on MuseScore's Shape class.
//!
//! # Collision Detection Algorithm
//!
//! The shape system uses a "horizontal slice" approach for accurate collision
//! detection between complex shapes. Instead of just comparing bounding boxes,
//! we check each rectangle pair for vertical overlap before computing horizontal
//! distances. This allows shapes like:
//!
//! ```text
//!   ┌──┐
//!   │  │     ┌──┐
//!   │  │     │  │
//!   └──┘     │  │
//!            └──┘
//! ```
//!
//! to be recognized as non-colliding, even though their bounding boxes overlap.

use std::borrow::Cow;

use kurbo::{Point, Rect};

use crate::engraver::model::ElementId;

use super::kerning::KerningType;

/// Element in a shape (rectangle + optional element reference).
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeElement {
    /// Bounding rectangle (in points)
    pub rect: Rect,
    /// Optional reference to the element owning this shape
    pub element: Option<ElementId>,
    /// Whether to ignore this shape for layout calculations
    pub ignore_for_layout: bool,
}

impl ShapeElement {
    /// Create a new shape element.
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            element: None,
            ignore_for_layout: false,
        }
    }

    /// Create a shape element with an element reference.
    #[must_use]
    pub fn with_element(rect: Rect, element: ElementId) -> Self {
        Self {
            rect,
            element: Some(element),
            ignore_for_layout: false,
        }
    }
}

/// Shape for collision detection.
///
/// Represents the boundary of a music notation element using rectangles.
/// Based on MuseScore's Shape class, which uses horizontal slices for
/// efficient collision detection.
///
/// # Variants
///
/// - `Fixed`: Single bounding box (most common case - optimized)
/// - `Composite`: Multiple rectangles for complex shapes
///
/// # Example
///
/// ```ignore
/// // Simple rectangular shape
/// let shape = Shape::from_rect(Rect::new(0.0, 0.0, 10.0, 5.0));
///
/// // Complex shape with multiple rectangles
/// let elements = vec![
///     ShapeElement::new(Rect::new(0.0, 0.0, 5.0, 10.0)),
///     ShapeElement::new(Rect::new(10.0, 2.0, 15.0, 8.0)),
/// ];
/// let shape = Shape::from_elements(elements);
///
/// // Collision detection
/// let distance = shape1.min_horizontal_distance(&shape2, 1.0);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// Single bounding box (fast path for most elements)
    Fixed {
        /// Bounding rectangle
        bbox: Rect,
        /// Optional element reference
        element: Option<ElementId>,
    },

    /// Multiple rectangles (for complex shapes)
    Composite {
        /// Shape elements (uses Cow for zero-copy when possible)
        elements: Cow<'static, [ShapeElement]>,
        /// Cached bounding box (computed lazily)
        bbox_cache: Option<Rect>,
    },
}

impl Shape {
    /// Create an empty shape.
    #[must_use]
    pub fn empty() -> Self {
        Self::Fixed {
            bbox: Rect::ZERO,
            element: None,
        }
    }

    /// Create a shape from a single rectangle.
    #[must_use]
    pub fn from_rect(rect: Rect) -> Self {
        Self::Fixed {
            bbox: rect,
            element: None,
        }
    }

    /// Create a shape from a rectangle with an element reference.
    #[must_use]
    pub fn from_rect_with_element(rect: Rect, element: ElementId) -> Self {
        Self::Fixed {
            bbox: rect,
            element: Some(element),
        }
    }

    /// Create a shape from multiple elements.
    #[must_use]
    pub fn from_elements(elements: Vec<ShapeElement>) -> Self {
        if elements.is_empty() {
            return Self::empty();
        }

        if elements.len() == 1 {
            // Optimize single-element case
            return Self::Fixed {
                bbox: elements[0].rect,
                element: elements[0].element,
            };
        }

        Self::Composite {
            elements: Cow::Owned(elements),
            bbox_cache: None,
        }
    }

    /// Get the bounding box of this shape.
    #[must_use]
    pub fn bbox(&self) -> Rect {
        match self {
            Self::Fixed { bbox, .. } => *bbox,
            Self::Composite {
                elements,
                bbox_cache,
            } => {
                if let Some(bbox) = bbox_cache {
                    *bbox
                } else {
                    compute_bbox_from_elements(elements)
                }
            }
        }
    }

    /// Translate shape by an offset.
    #[must_use]
    pub fn translate(&self, offset: Point) -> Self {
        match self {
            Self::Fixed { bbox, element } => Self::Fixed {
                bbox: bbox.with_origin(Point::new(bbox.x0 + offset.x, bbox.y0 + offset.y)),
                element: *element,
            },
            Self::Composite { elements, .. } => {
                let translated: Vec<_> = elements
                    .iter()
                    .map(|e| ShapeElement {
                        rect: e
                            .rect
                            .with_origin(Point::new(e.rect.x0 + offset.x, e.rect.y0 + offset.y)),
                        element: e.element,
                        ignore_for_layout: e.ignore_for_layout,
                    })
                    .collect();
                Self::from_elements(translated)
            }
        }
    }

    /// Get the right-most edge of this shape.
    #[must_use]
    pub fn right(&self) -> f64 {
        self.bbox().x1
    }

    /// Get the left-most edge of this shape.
    #[must_use]
    pub fn left(&self) -> f64 {
        self.bbox().x0
    }

    /// Get the top edge of this shape.
    #[must_use]
    pub fn top(&self) -> f64 {
        self.bbox().y0
    }

    /// Get the bottom edge of this shape.
    #[must_use]
    pub fn bottom(&self) -> f64 {
        self.bbox().y1
    }

    /// Get the number of rectangles in this shape.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::Fixed { bbox, .. } => {
                if bbox.width() > 0.0 || bbox.height() > 0.0 {
                    1
                } else {
                    0
                }
            }
            Self::Composite { elements, .. } => elements.len(),
        }
    }

    /// Get the rightmost edge within a vertical range.
    ///
    /// Returns the rightmost X coordinate of any rectangle that intersects
    /// the Y range [y_above, y_below].
    #[must_use]
    pub fn right_most_edge_at_height(&self, y_above: f64, y_below: f64) -> f64 {
        let mut edge = f64::NEG_INFINITY;

        self.for_each_rect(|rect| {
            // Check if this rect intersects the Y range
            if rect.y1 > y_above && rect.y0 < y_below {
                edge = edge.max(rect.x1);
            }
        });

        edge
    }

    /// Get the leftmost edge within a vertical range.
    ///
    /// Returns the leftmost X coordinate of any rectangle that intersects
    /// the Y range [y_above, y_below].
    #[must_use]
    pub fn left_most_edge_at_height(&self, y_above: f64, y_below: f64) -> f64 {
        let mut edge = f64::INFINITY;

        self.for_each_rect(|rect| {
            // Check if this rect intersects the Y range
            if rect.y1 > y_above && rect.y0 < y_below {
                edge = edge.min(rect.x0);
            }
        });

        edge
    }

    /// Get the top (minimum Y) at a specific X coordinate.
    #[must_use]
    pub fn top_at_x(&self, x: f64) -> f64 {
        let mut local_top = f64::INFINITY;

        self.for_each_rect(|rect| {
            if rect.x0 < x && rect.x1 > x {
                local_top = local_top.min(rect.y0);
            }
        });

        if local_top.is_infinite() {
            self.top()
        } else {
            local_top
        }
    }

    /// Get the bottom (maximum Y) at a specific X coordinate.
    #[must_use]
    pub fn bottom_at_x(&self, x: f64) -> f64 {
        let mut local_bottom = f64::NEG_INFINITY;

        self.for_each_rect(|rect| {
            if rect.x0 < x && rect.x1 > x {
                local_bottom = local_bottom.max(rect.y1);
            }
        });

        if local_bottom.is_infinite() {
            self.bottom()
        } else {
            local_bottom
        }
    }

    /// Iterate over all rectangles in this shape.
    fn for_each_rect<F>(&self, mut f: F)
    where
        F: FnMut(&Rect),
    {
        match self {
            Self::Fixed { bbox, .. } => f(bbox),
            Self::Composite { elements, .. } => {
                for elem in elements.iter() {
                    if !elem.ignore_for_layout {
                        f(&elem.rect);
                    }
                }
            }
        }
    }

    /// Calculate minimum horizontal distance to avoid collision with another shape.
    ///
    /// This uses the "horizontal slice" algorithm: for each pair of rectangles
    /// that overlap vertically, compute the horizontal distance needed to avoid
    /// collision. Return the maximum of all such distances.
    ///
    /// # Arguments
    ///
    /// * `other` - The other shape to check against (positioned to the right)
    /// * `min_spacing` - Minimum spacing margin to add (in points)
    #[must_use]
    pub fn min_horizontal_distance(&self, other: &Self, min_spacing: f64) -> f64 {
        self.min_horizontal_distance_with_kerning(other, min_spacing, KerningType::Kerning)
    }

    /// Calculate minimum horizontal distance with kerning type support.
    ///
    /// This is the full horizontal slice algorithm that checks each rectangle
    /// pair for vertical overlap.
    ///
    /// # Arguments
    ///
    /// * `other` - The other shape (to the right of self)
    /// * `min_spacing` - Minimum spacing margin
    /// * `kerning` - Kerning type controlling overlap behavior
    #[must_use]
    pub fn min_horizontal_distance_with_kerning(
        &self,
        other: &Self,
        min_spacing: f64,
        kerning: KerningType,
    ) -> f64 {
        // Handle special kerning types
        if kerning.allows_collision() {
            return 0.0;
        }

        if self.is_empty() || other.is_empty() {
            return 0.0;
        }

        let mut max_distance: f64 = 0.0;
        let min_clearance = if kerning.allows_kerning() {
            0.0
        } else {
            min_spacing
        };

        // Check each pair of rectangles
        self.for_each_rect(|r1| {
            // Skip zero-height rects for vertical overlap check
            // but zero-height rects act as "walls" that collide with everything
            let r1_is_wall = r1.height() <= 0.0;

            other.for_each_rect(|r2| {
                let r2_is_wall = r2.height() <= 0.0;

                // Check vertical overlap
                // Walls (zero-height) collide with everything
                let vertical_overlap = if r1_is_wall || r2_is_wall {
                    true // Walls always "overlap" vertically
                } else {
                    ranges_intersect(r1.y0, r1.y1, r2.y0, r2.y1, min_clearance)
                };

                if vertical_overlap {
                    // Calculate horizontal distance: how far right must r2 move
                    // to not overlap with r1
                    let distance = r1.x1 - r2.x0 + min_spacing;
                    max_distance = max_distance.max(distance);
                }
            });
        });

        // Apply kerning limit if specified
        if let Some(fraction) = kerning.kern_limit_fraction() {
            let other_width = other.right() - other.left();
            let kern_limit = other_width * fraction;
            max_distance = max_distance.max(self.right() - other.left() + kern_limit + min_spacing);
        }

        max_distance.max(0.0)
    }

    /// Calculate minimum vertical distance to another shape below.
    ///
    /// Returns the distance from this shape's bottom to the other shape's top,
    /// only considering rectangles that overlap horizontally.
    ///
    /// # Arguments
    ///
    /// * `other` - The shape below this one
    /// * `min_horizontal_clearance` - Minimum X overlap to consider
    #[must_use]
    pub fn min_vertical_distance(&self, other: &Self, min_horizontal_clearance: f64) -> f64 {
        if self.is_empty() || other.is_empty() {
            return 0.0;
        }

        let mut dist = f64::NEG_INFINITY;

        self.for_each_rect(|r1| {
            if r1.height() <= 0.0 {
                return;
            }

            other.for_each_rect(|r2| {
                if r2.height() <= 0.0 {
                    return;
                }

                // Check horizontal overlap
                if ranges_intersect(r1.x0, r1.x1, r2.x0, r2.x1, min_horizontal_clearance) {
                    dist = dist.max(r1.y1 - r2.y0);
                }
            });
        });

        dist
    }

    /// Calculate vertical clearance to another shape below.
    ///
    /// Returns positive if there's space between shapes, negative if overlapping.
    #[must_use]
    pub fn vertical_clearance(&self, other: &Self, min_horizontal_distance: f64) -> f64 {
        if self.is_empty() || other.is_empty() {
            return 0.0;
        }

        let mut clearance = f64::INFINITY;

        self.for_each_rect(|r1| {
            if r1.height() <= 0.0 {
                return;
            }

            other.for_each_rect(|r2| {
                if r2.height() <= 0.0 {
                    return;
                }

                // Check horizontal overlap
                if ranges_intersect(r1.x0, r1.x1, r2.x0, r2.x1, min_horizontal_distance) {
                    clearance = clearance.min(r2.y0 - r1.y1);
                }
            });
        });

        clearance
    }

    /// Check if two shapes intersect.
    #[must_use]
    pub fn intersects(&self, other: &Self) -> bool {
        let mut result = false;

        self.for_each_rect(|r1| {
            if result {
                return;
            }
            other.for_each_rect(|r2| {
                if result {
                    return;
                }
                if rects_intersect(r1, r2) {
                    result = true;
                }
            });
        });

        result
    }

    /// Check if this shape contains a point.
    #[must_use]
    pub fn contains(&self, p: Point) -> bool {
        let mut result = false;

        self.for_each_rect(|rect| {
            if rect.contains(p) {
                result = true;
            }
        });

        result
    }

    /// Add a horizontal spacing "wall".
    ///
    /// Walls are zero-height rectangles that collide with everything vertically.
    /// Use this to create space that cannot tuck above/below other elements.
    pub fn add_horizontal_spacing(&mut self, left_edge: f64, right_edge: f64) {
        // Zero-width walls collide with everything, so add a tiny epsilon
        let right = if (left_edge - right_edge).abs() < f64::EPSILON {
            right_edge + 100.0 * f64::EPSILON
        } else {
            right_edge
        };

        self.add_rect(Rect::new(left_edge, 0.0, right, 0.0));
    }

    /// Scale this shape by a factor.
    #[must_use]
    pub fn scaled(&self, scale_x: f64, scale_y: f64) -> Self {
        match self {
            Self::Fixed { bbox, element } => Self::Fixed {
                bbox: Rect::new(
                    bbox.x0 * scale_x,
                    bbox.y0 * scale_y,
                    bbox.x1 * scale_x,
                    bbox.y1 * scale_y,
                ),
                element: *element,
            },
            Self::Composite { elements, .. } => {
                let scaled: Vec<_> = elements
                    .iter()
                    .map(|e| ShapeElement {
                        rect: Rect::new(
                            e.rect.x0 * scale_x,
                            e.rect.y0 * scale_y,
                            e.rect.x1 * scale_x,
                            e.rect.y1 * scale_y,
                        ),
                        element: e.element,
                        ignore_for_layout: e.ignore_for_layout,
                    })
                    .collect();
                Self::from_elements(scaled)
            }
        }
    }

    /// Pad this shape by adding margin on all sides.
    #[must_use]
    pub fn padded(&self, padding: f64) -> Self {
        match self {
            Self::Fixed { bbox, element } => Self::Fixed {
                bbox: Rect::new(
                    bbox.x0 - padding,
                    bbox.y0 - padding,
                    bbox.x1 + padding,
                    bbox.y1 + padding,
                ),
                element: *element,
            },
            Self::Composite { elements, .. } => {
                let padded: Vec<_> = elements
                    .iter()
                    .map(|e| ShapeElement {
                        rect: Rect::new(
                            e.rect.x0 - padding,
                            e.rect.y0 - padding,
                            e.rect.x1 + padding,
                            e.rect.y1 + padding,
                        ),
                        element: e.element,
                        ignore_for_layout: e.ignore_for_layout,
                    })
                    .collect();
                Self::from_elements(padded)
            }
        }
    }

    /// Check if this shape is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Fixed { bbox, .. } => bbox.width() == 0.0 && bbox.height() == 0.0,
            Self::Composite { elements, .. } => elements.is_empty(),
        }
    }

    /// Add a rectangle to this shape.
    pub fn add_rect(&mut self, rect: Rect) {
        match self {
            Self::Fixed { bbox, element } => {
                // Convert to composite if adding another rect
                if bbox.width() > 0.0 || bbox.height() > 0.0 {
                    let elements = vec![
                        ShapeElement {
                            rect: *bbox,
                            element: *element,
                            ignore_for_layout: false,
                        },
                        ShapeElement::new(rect),
                    ];
                    *self = Self::from_elements(elements);
                } else {
                    // First rect, just replace
                    *bbox = rect;
                }
            }
            Self::Composite { elements, .. } => {
                // Add to existing composite
                let mut new_elements = elements.clone().into_owned();
                new_elements.push(ShapeElement::new(rect));
                *self = Self::from_elements(new_elements);
            }
        }
    }
}

/// Check if two 1D ranges intersect.
///
/// Returns true if ranges [a, b] and [c, d] overlap, with optional clearance.
#[inline]
fn ranges_intersect(a: f64, b: f64, c: f64, d: f64, min_clearance: f64) -> bool {
    if a == b || c == d {
        return false;
    }
    (b + min_clearance > c) && (a < d + min_clearance)
}

/// Check if two rectangles intersect.
#[inline]
fn rects_intersect(r1: &Rect, r2: &Rect) -> bool {
    r1.x0 < r2.x1 && r1.x1 > r2.x0 && r1.y0 < r2.y1 && r1.y1 > r2.y0
}

/// Compute bounding box from a list of shape elements.
fn compute_bbox_from_elements(elements: &[ShapeElement]) -> Rect {
    if elements.is_empty() {
        return Rect::ZERO;
    }

    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    for elem in elements {
        if elem.ignore_for_layout {
            continue;
        }

        min_x = min_x.min(elem.rect.x0);
        min_y = min_y.min(elem.rect.y0);
        max_x = max_x.max(elem.rect.x1);
        max_y = max_y.max(elem.rect.y1);
    }

    if min_x.is_infinite() {
        Rect::ZERO
    } else {
        Rect::new(min_x, min_y, max_x, max_y)
    }
}

impl Default for Shape {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_shape_from_rect() {
        let rect = Rect::new(10.0, 20.0, 50.0, 40.0);
        let shape = Shape::from_rect(rect);

        assert_eq!(shape.bbox(), rect);
        assert_eq!(shape.left(), 10.0);
        assert_eq!(shape.right(), 50.0);
        assert_eq!(shape.top(), 20.0);
        assert_eq!(shape.bottom(), 40.0);
    }

    #[test]
    fn test_shape_translate() {
        let rect = Rect::new(10.0, 20.0, 50.0, 40.0);
        let shape = Shape::from_rect(rect);

        let offset = Point::new(5.0, 3.0);
        let translated = shape.translate(offset);

        let bbox = translated.bbox();
        assert_eq!(bbox.x0, 15.0);
        assert_eq!(bbox.y0, 23.0);
        assert_eq!(bbox.x1, 55.0);
        assert_eq!(bbox.y1, 43.0);
    }

    #[test]
    fn test_shape_collision_no_overlap() {
        let shape1 = Shape::from_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        let shape2 = Shape::from_rect(Rect::new(15.0, 0.0, 25.0, 10.0));

        let distance = shape1.min_horizontal_distance(&shape2, 0.5);
        // Shapes are 5 units apart, min_spacing is 0.5, so no adjustment needed
        assert_eq!(distance, 0.0);
    }

    #[test]
    fn test_shape_collision_overlap() {
        let shape1 = Shape::from_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        let shape2 = Shape::from_rect(Rect::new(8.0, 2.0, 18.0, 8.0));

        let distance = shape1.min_horizontal_distance(&shape2, 0.5);
        // Shapes overlap by 2 units, need to move 2 + 0.5 = 2.5 units
        assert_eq!(distance, 2.5);
    }

    #[test]
    fn test_shape_collision_no_vertical_overlap() {
        let shape1 = Shape::from_rect(Rect::new(0.0, 0.0, 10.0, 5.0));
        let shape2 = Shape::from_rect(Rect::new(5.0, 10.0, 15.0, 15.0));

        let distance = shape1.min_horizontal_distance(&shape2, 0.5);
        // No vertical overlap, no collision
        assert_eq!(distance, 0.0);
    }

    #[test]
    fn test_shape_composite() {
        let elements = vec![
            ShapeElement::new(Rect::new(0.0, 0.0, 5.0, 10.0)),
            ShapeElement::new(Rect::new(10.0, 2.0, 15.0, 8.0)),
        ];
        let shape = Shape::from_elements(elements);

        let bbox = shape.bbox();
        assert_eq!(bbox.x0, 0.0);
        assert_eq!(bbox.y0, 0.0);
        assert_eq!(bbox.x1, 15.0);
        assert_eq!(bbox.y1, 10.0);
    }

    #[test]
    fn test_shape_add_rect() {
        let mut shape = Shape::from_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        shape.add_rect(Rect::new(15.0, 5.0, 20.0, 15.0));

        let bbox = shape.bbox();
        assert_eq!(bbox.x0, 0.0);
        assert_eq!(bbox.y0, 0.0);
        assert_eq!(bbox.x1, 20.0);
        assert_eq!(bbox.y1, 15.0);
    }

    #[test]
    fn test_shape_empty() {
        let shape = Shape::empty();
        assert!(shape.is_empty());
        assert_eq!(shape.bbox(), Rect::ZERO);
    }

    #[test]
    fn test_right_most_edge_at_height() {
        // Create an L-shaped composite shape
        let elements = vec![
            ShapeElement::new(Rect::new(0.0, 0.0, 10.0, 20.0)), // Tall left part
            ShapeElement::new(Rect::new(10.0, 15.0, 30.0, 20.0)), // Short right part at bottom
        ];
        let shape = Shape::from_elements(elements);

        // At y=5 (top section), only left rect is present
        assert_eq!(shape.right_most_edge_at_height(0.0, 10.0), 10.0);

        // At y=17 (bottom section), both rects are present
        assert_eq!(shape.right_most_edge_at_height(15.0, 20.0), 30.0);
    }

    #[test]
    fn test_left_most_edge_at_height() {
        let elements = vec![
            ShapeElement::new(Rect::new(10.0, 0.0, 20.0, 10.0)), // Top rect
            ShapeElement::new(Rect::new(0.0, 10.0, 20.0, 20.0)), // Bottom rect (extends left)
        ];
        let shape = Shape::from_elements(elements);

        // At top, left edge is 10
        assert_eq!(shape.left_most_edge_at_height(0.0, 10.0), 10.0);

        // At bottom, left edge is 0
        assert_eq!(shape.left_most_edge_at_height(10.0, 20.0), 0.0);
    }

    #[test]
    fn test_sliced_collision_non_overlapping() {
        // Two shapes that overlap in bounding box but not actually
        //   ┌──┐
        //   │  │     ┌──┐
        //   │  │     │  │
        //   └──┘     │  │
        //            └──┘
        let shape1 = Shape::from_rect(Rect::new(0.0, 0.0, 10.0, 10.0));

        let elements = vec![ShapeElement::new(Rect::new(5.0, 12.0, 15.0, 22.0))];
        let shape2 = Shape::from_elements(elements);

        // These shapes don't overlap vertically at any horizontal position
        let distance = shape1.min_horizontal_distance(&shape2, 0.0);
        assert_eq!(distance, 0.0);
    }

    #[test]
    fn test_sliced_collision_overlapping() {
        // Two shapes that actually overlap
        let shape1 = Shape::from_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        let shape2 = Shape::from_rect(Rect::new(5.0, 5.0, 15.0, 15.0));

        let distance = shape1.min_horizontal_distance(&shape2, 1.0);
        // shape1 right edge (10) - shape2 left edge (5) + spacing (1) = 6
        assert_eq!(distance, 6.0);
    }

    #[test]
    fn test_horizontal_wall() {
        let mut shape = Shape::from_rect(Rect::new(0.0, 5.0, 10.0, 15.0));
        shape.add_horizontal_spacing(0.0, 10.0);

        // Wall should collide with everything
        let other = Shape::from_rect(Rect::new(8.0, 100.0, 18.0, 110.0));

        // Even though shapes don't overlap vertically, the wall forces collision
        let distance = shape.min_horizontal_distance(&other, 1.0);
        assert!(distance > 0.0);
    }

    #[test]
    fn test_vertical_clearance() {
        // Shape above
        let shape1 = Shape::from_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        // Shape below
        let shape2 = Shape::from_rect(Rect::new(0.0, 15.0, 10.0, 25.0));

        let clearance = shape1.vertical_clearance(&shape2, 0.0);
        // Distance from shape1 bottom (10) to shape2 top (15) = 5
        assert_eq!(clearance, 5.0);
    }

    #[test]
    fn test_vertical_clearance_overlap() {
        let shape1 = Shape::from_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        let shape2 = Shape::from_rect(Rect::new(0.0, 8.0, 10.0, 18.0));

        let clearance = shape1.vertical_clearance(&shape2, 0.0);
        // Shapes overlap, clearance is negative
        assert_eq!(clearance, -2.0);
    }

    #[test]
    fn test_shape_intersects() {
        let shape1 = Shape::from_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        let shape2 = Shape::from_rect(Rect::new(5.0, 5.0, 15.0, 15.0));
        let shape3 = Shape::from_rect(Rect::new(20.0, 20.0, 30.0, 30.0));

        assert!(shape1.intersects(&shape2));
        assert!(!shape1.intersects(&shape3));
    }

    #[test]
    fn test_shape_contains() {
        let shape = Shape::from_rect(Rect::new(0.0, 0.0, 10.0, 10.0));

        assert!(shape.contains(Point::new(5.0, 5.0)));
        assert!(!shape.contains(Point::new(15.0, 15.0)));
    }

    #[test]
    fn test_shape_scaled() {
        let shape = Shape::from_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        let scaled = shape.scaled(2.0, 0.5);

        let bbox = scaled.bbox();
        assert_eq!(bbox.x1, 20.0);
        assert_eq!(bbox.y1, 5.0);
    }

    #[test]
    fn test_shape_padded() {
        let shape = Shape::from_rect(Rect::new(10.0, 10.0, 20.0, 20.0));
        let padded = shape.padded(5.0);

        let bbox = padded.bbox();
        assert_eq!(bbox.x0, 5.0);
        assert_eq!(bbox.y0, 5.0);
        assert_eq!(bbox.x1, 25.0);
        assert_eq!(bbox.y1, 25.0);
    }

    #[test]
    fn test_top_at_x() {
        let elements = vec![
            ShapeElement::new(Rect::new(0.0, 10.0, 10.0, 20.0)),
            ShapeElement::new(Rect::new(5.0, 0.0, 15.0, 15.0)),
        ];
        let shape = Shape::from_elements(elements);

        // At x=2, only first rect, top is 10
        assert_eq!(shape.top_at_x(2.0), 10.0);

        // At x=7, both rects overlap, top is min(10, 0) = 0
        assert_eq!(shape.top_at_x(7.0), 0.0);

        // At x=12, only second rect, top is 0
        assert_eq!(shape.top_at_x(12.0), 0.0);
    }

    #[test]
    fn test_bottom_at_x() {
        let elements = vec![
            ShapeElement::new(Rect::new(0.0, 0.0, 10.0, 10.0)),
            ShapeElement::new(Rect::new(5.0, 5.0, 15.0, 20.0)),
        ];
        let shape = Shape::from_elements(elements);

        // At x=2, only first rect, bottom is 10
        assert_eq!(shape.bottom_at_x(2.0), 10.0);

        // At x=7, both rects overlap, bottom is max(10, 20) = 20
        assert_eq!(shape.bottom_at_x(7.0), 20.0);
    }

    #[test]
    fn test_kerning_allow_collision() {
        let shape1 = Shape::from_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        let shape2 = Shape::from_rect(Rect::new(5.0, 5.0, 15.0, 15.0));

        let distance =
            shape1.min_horizontal_distance_with_kerning(&shape2, 1.0, KerningType::AllowCollision);
        assert_eq!(distance, 0.0);
    }

    #[test]
    fn test_ranges_intersect() {
        assert!(ranges_intersect(0.0, 10.0, 5.0, 15.0, 0.0));
        assert!(!ranges_intersect(0.0, 10.0, 15.0, 25.0, 0.0));
        assert!(ranges_intersect(0.0, 10.0, 10.0, 20.0, 1.0)); // With clearance
        assert!(!ranges_intersect(0.0, 0.0, 0.0, 10.0, 0.0)); // Zero-length range
    }
}
