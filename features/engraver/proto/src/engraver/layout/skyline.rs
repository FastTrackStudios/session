//! Skyline algorithm for collision avoidance.
//!
//! A skyline represents the profile of elements above (north) or below (south)
//! a staff line. It's used by the AutoPlace algorithm to position elements
//! without overlapping.
//!
//! Based on MuseScore's Skyline implementation.

use kurbo::Rect;
use std::cmp::Ordering;

use crate::engraver::layout::shape::Shape;
use crate::engraver::scene::id::ElementType;

/// A single element in the skyline.
#[derive(Debug, Clone)]
pub struct SkylineElement {
    /// The bounding rectangle
    pub rect: Rect,
    /// The element type that created this skyline entry
    pub element_type: Option<ElementType>,
    /// Element ID for collision filtering
    pub element_id: Option<u64>,
}

impl SkylineElement {
    /// Create a new skyline element.
    #[must_use]
    pub fn new(rect: Rect) -> Self {
        Self {
            rect,
            element_type: None,
            element_id: None,
        }
    }

    /// Create with element info for filtering.
    #[must_use]
    pub fn with_element(rect: Rect, element_type: ElementType, element_id: u64) -> Self {
        Self {
            rect,
            element_type: Some(element_type),
            element_id: Some(element_id),
        }
    }

    /// Get the left x coordinate.
    #[must_use]
    pub fn left(&self) -> f64 {
        self.rect.x0
    }

    /// Get the right x coordinate.
    #[must_use]
    pub fn right(&self) -> f64 {
        self.rect.x1
    }

    /// Get the top y coordinate.
    #[must_use]
    pub fn top(&self) -> f64 {
        self.rect.y0
    }

    /// Get the bottom y coordinate.
    #[must_use]
    pub fn bottom(&self) -> f64 {
        self.rect.y1
    }
}

/// A skyline line - either north (above) or south (below) staff.
///
/// The skyline maintains a sorted list of rectangles representing
/// the highest (north) or lowest (south) points of elements at each
/// horizontal position.
#[derive(Debug, Clone, Default)]
pub struct SkylineLine {
    /// Elements in the skyline, sorted by x position
    elements: Vec<SkylineElement>,
    /// Whether this is a north (true) or south (false) skyline
    is_north: bool,
}

impl SkylineLine {
    /// Create a new skyline line.
    #[must_use]
    pub fn new(is_north: bool) -> Self {
        Self {
            elements: Vec::new(),
            is_north,
        }
    }

    /// Create a north (above staff) skyline.
    #[must_use]
    pub fn north() -> Self {
        Self::new(true)
    }

    /// Create a south (below staff) skyline.
    #[must_use]
    pub fn south() -> Self {
        Self::new(false)
    }

    /// Check if the skyline is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    /// Get the number of elements.
    #[must_use]
    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Get the elements.
    #[must_use]
    pub fn elements(&self) -> &[SkylineElement] {
        &self.elements
    }

    /// Add a rectangle to the skyline.
    pub fn add(&mut self, rect: Rect) {
        self.add_element(SkylineElement::new(rect));
    }

    /// Add a shape to the skyline.
    pub fn add_shape(&mut self, shape: &Shape) {
        let bbox = shape.bbox();
        if !bbox.is_zero_area() {
            self.add(bbox);
        }
    }

    /// Add a skyline element.
    pub fn add_element(&mut self, element: SkylineElement) {
        if element.rect.is_zero_area() {
            return;
        }

        // Find insertion point (maintain sorted by x)
        let pos = self
            .elements
            .binary_search_by(|e| {
                e.left()
                    .partial_cmp(&element.left())
                    .unwrap_or(Ordering::Equal)
            })
            .unwrap_or_else(|p| p);

        self.elements.insert(pos, element);
    }

    /// Clear the skyline.
    pub fn clear(&mut self) {
        self.elements.clear();
    }

    /// Get the maximum y value (for north skyline) at a given x position.
    /// Returns None if no element covers that position.
    #[must_use]
    pub fn max_at(&self, x: f64) -> Option<f64> {
        let mut result: Option<f64> = None;

        for el in &self.elements {
            if x >= el.left() && x <= el.right() {
                let y = if self.is_north { el.top() } else { el.bottom() };
                result = Some(match result {
                    None => y,
                    Some(current) => {
                        if self.is_north {
                            current.min(y) // For north, we want the minimum (highest on screen)
                        } else {
                            current.max(y) // For south, we want the maximum (lowest on screen)
                        }
                    }
                });
            }
        }

        result
    }

    /// Calculate the minimum distance needed to place a shape above this skyline.
    ///
    /// Returns the distance to move the shape upward (negative y) to clear the skyline.
    #[must_use]
    pub fn min_distance_to_shape_above(&self, shape: &Shape, horizontal_clearance: f64) -> f64 {
        if self.elements.is_empty() {
            return 0.0;
        }

        let shape_bbox = shape.bbox();
        let mut min_dist = f64::NEG_INFINITY;

        for el in &self.elements {
            // Check horizontal overlap with clearance
            let h_overlap = shape_bbox.x1 + horizontal_clearance > el.left()
                && shape_bbox.x0 - horizontal_clearance < el.right();

            if h_overlap {
                // Distance from shape bottom to skyline top
                let dist = el.top() - shape_bbox.y1;
                min_dist = min_dist.max(dist);
            }
        }

        if min_dist == f64::NEG_INFINITY {
            0.0
        } else {
            -min_dist // Return positive distance to move upward
        }
    }

    /// Calculate the minimum distance needed to place a shape below this skyline.
    ///
    /// Returns the distance to move the shape downward (positive y) to clear the skyline.
    #[must_use]
    pub fn min_distance_to_shape_below(&self, shape: &Shape, horizontal_clearance: f64) -> f64 {
        if self.elements.is_empty() {
            return 0.0;
        }

        let shape_bbox = shape.bbox();
        let mut min_dist = f64::NEG_INFINITY;

        for el in &self.elements {
            // Check horizontal overlap with clearance
            let h_overlap = shape_bbox.x1 + horizontal_clearance > el.left()
                && shape_bbox.x0 - horizontal_clearance < el.right();

            if h_overlap {
                // Distance from skyline bottom to shape top
                let dist = shape_bbox.y0 - el.bottom();
                min_dist = min_dist.max(dist);
            }
        }

        if min_dist == f64::NEG_INFINITY {
            0.0
        } else {
            -min_dist // Return positive distance to move downward
        }
    }

    /// Get a filtered copy excluding elements that match the predicate.
    #[must_use]
    pub fn filtered<F>(&self, should_exclude: F) -> Self
    where
        F: Fn(&SkylineElement) -> bool,
    {
        let elements = self
            .elements
            .iter()
            .filter(|e| !should_exclude(e))
            .cloned()
            .collect();

        Self {
            elements,
            is_north: self.is_north,
        }
    }

    /// Get the overall bounding box of all elements.
    #[must_use]
    pub fn bbox(&self) -> Option<Rect> {
        if self.elements.is_empty() {
            return None;
        }

        let mut x0 = f64::INFINITY;
        let mut y0 = f64::INFINITY;
        let mut x1 = f64::NEG_INFINITY;
        let mut y1 = f64::NEG_INFINITY;

        for el in &self.elements {
            x0 = x0.min(el.left());
            y0 = y0.min(el.top());
            x1 = x1.max(el.right());
            y1 = y1.max(el.bottom());
        }

        Some(Rect::new(x0, y0, x1, y1))
    }
}

/// A complete skyline for a staff, with both north and south profiles.
#[derive(Debug, Clone, Default)]
pub struct Skyline {
    /// Skyline above the staff
    north: SkylineLine,
    /// Skyline below the staff
    south: SkylineLine,
}

impl Skyline {
    /// Create a new empty skyline.
    #[must_use]
    pub fn new() -> Self {
        Self {
            north: SkylineLine::north(),
            south: SkylineLine::south(),
        }
    }

    /// Get the north (above) skyline.
    #[must_use]
    pub fn north(&self) -> &SkylineLine {
        &self.north
    }

    /// Get a mutable reference to the north skyline.
    pub fn north_mut(&mut self) -> &mut SkylineLine {
        &mut self.north
    }

    /// Get the south (below) skyline.
    #[must_use]
    pub fn south(&self) -> &SkylineLine {
        &self.south
    }

    /// Get a mutable reference to the south skyline.
    pub fn south_mut(&mut self) -> &mut SkylineLine {
        &mut self.south
    }

    /// Add a shape to the appropriate skyline based on placement.
    pub fn add(&mut self, shape: &Shape, above: bool) {
        if above {
            self.north.add_shape(shape);
        } else {
            self.south.add_shape(shape);
        }
    }

    /// Clear both skylines.
    pub fn clear(&mut self) {
        self.north.clear();
        self.south.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_skyline_line_creation() {
        let north = SkylineLine::north();
        assert!(north.is_empty());
        assert!(north.is_north);

        let south = SkylineLine::south();
        assert!(south.is_empty());
        assert!(!south.is_north);
    }

    #[test]
    fn test_skyline_add() {
        let mut skyline = SkylineLine::north();
        skyline.add(Rect::new(0.0, -10.0, 100.0, 0.0));

        assert_eq!(skyline.len(), 1);
        assert_eq!(skyline.elements()[0].left(), 0.0);
        assert_eq!(skyline.elements()[0].right(), 100.0);
    }

    #[test]
    fn test_skyline_sorted_insertion() {
        let mut skyline = SkylineLine::north();
        skyline.add(Rect::new(100.0, -10.0, 200.0, 0.0));
        skyline.add(Rect::new(0.0, -10.0, 50.0, 0.0));
        skyline.add(Rect::new(50.0, -10.0, 100.0, 0.0));

        assert_eq!(skyline.len(), 3);
        assert_eq!(skyline.elements()[0].left(), 0.0);
        assert_eq!(skyline.elements()[1].left(), 50.0);
        assert_eq!(skyline.elements()[2].left(), 100.0);
    }

    #[test]
    fn test_skyline_max_at() {
        let mut skyline = SkylineLine::north();
        skyline.add(Rect::new(0.0, -20.0, 100.0, 0.0));
        skyline.add(Rect::new(50.0, -30.0, 150.0, -10.0));

        // At x=25, only first rect covers it
        let max_25 = skyline.max_at(25.0);
        assert_eq!(max_25, Some(-20.0));

        // At x=75, both rects overlap, should get minimum (highest)
        let max_75 = skyline.max_at(75.0);
        assert_eq!(max_75, Some(-30.0));

        // At x=125, only second rect covers it
        let max_125 = skyline.max_at(125.0);
        assert_eq!(max_125, Some(-30.0));

        // At x=200, no rect covers it
        let max_200 = skyline.max_at(200.0);
        assert_eq!(max_200, None);
    }

    #[test]
    fn test_skyline_distance_above() {
        let mut skyline = SkylineLine::south();
        skyline.add(Rect::new(0.0, 0.0, 100.0, 20.0));

        let shape = Shape::from_rect(Rect::new(10.0, 10.0, 50.0, 30.0));
        let dist = skyline.min_distance_to_shape_above(&shape, 0.0);

        // Shape bottom (30) needs to clear skyline top (20)
        // Distance should be negative (shape overlaps)
        assert!(dist > 0.0);
    }

    #[test]
    fn test_skyline_distance_below() {
        let mut skyline = SkylineLine::north();
        skyline.add(Rect::new(0.0, -20.0, 100.0, 0.0));

        let shape = Shape::from_rect(Rect::new(10.0, -10.0, 50.0, 10.0));
        let dist = skyline.min_distance_to_shape_below(&shape, 0.0);

        // Shape top (-10) needs to clear skyline bottom (0)
        assert!(dist > 0.0);
    }

    #[test]
    fn test_skyline_filtered() {
        let mut skyline = SkylineLine::north();
        skyline.add_element(SkylineElement::with_element(
            Rect::new(0.0, -10.0, 50.0, 0.0),
            ElementType::Note,
            1,
        ));
        skyline.add_element(SkylineElement::with_element(
            Rect::new(50.0, -10.0, 100.0, 0.0),
            ElementType::Rest,
            2,
        ));

        let filtered = skyline.filtered(|e| e.element_type == Some(ElementType::Note));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered.elements()[0].element_type, Some(ElementType::Rest));
    }

    #[test]
    fn test_full_skyline() {
        let mut skyline = Skyline::new();

        let shape_above = Shape::from_rect(Rect::new(0.0, -20.0, 50.0, -10.0));
        let shape_below = Shape::from_rect(Rect::new(0.0, 10.0, 50.0, 20.0));

        skyline.add(&shape_above, true);
        skyline.add(&shape_below, false);

        assert_eq!(skyline.north().len(), 1);
        assert_eq!(skyline.south().len(), 1);
    }

    #[test]
    fn test_skyline_bbox() {
        let mut skyline = SkylineLine::north();
        skyline.add(Rect::new(10.0, -30.0, 50.0, -10.0));
        skyline.add(Rect::new(40.0, -20.0, 100.0, 0.0));

        let bbox = skyline.bbox().unwrap();
        assert_eq!(bbox.x0, 10.0);
        assert_eq!(bbox.y0, -30.0);
        assert_eq!(bbox.x1, 100.0);
        assert_eq!(bbox.y1, 0.0);
    }

    #[test]
    fn test_empty_skyline_bbox() {
        let skyline = SkylineLine::north();
        assert!(skyline.bbox().is_none());
    }
}
