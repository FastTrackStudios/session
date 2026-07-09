//! AutoPlace algorithm for automatic collision avoidance.
//!
//! This module implements the autoplace algorithm that automatically
//! positions elements to avoid collisions with other elements.
//!
//! Based on MuseScore's autoplace.cpp implementation.

use kurbo::Point;

use crate::engraver::layout::shape::Shape;
use crate::engraver::layout::skyline::{Skyline, SkylineElement, SkylineLine};
use crate::engraver::scene::id::ElementType;

/// Configuration for autoplace behavior.
#[derive(Debug, Clone)]
pub struct AutoplaceConfig {
    /// Minimum distance from staff in spatiums
    pub min_distance: f64,
    /// Minimum horizontal clearance in spatiums
    pub horizontal_clearance: f64,
    /// Whether autoplace is enabled
    pub enabled: bool,
}

impl Default for AutoplaceConfig {
    fn default() -> Self {
        Self {
            min_distance: 0.5,
            horizontal_clearance: 0.2,
            enabled: true,
        }
    }
}

/// Result of an autoplace operation.
#[derive(Debug, Clone)]
pub struct AutoplaceResult {
    /// The vertical offset to apply
    pub offset_y: f64,
    /// Whether the element was moved
    pub moved: bool,
    /// Whether the element should be added to the skyline
    pub add_to_skyline: bool,
}

impl Default for AutoplaceResult {
    fn default() -> Self {
        Self {
            offset_y: 0.0,
            moved: false,
            add_to_skyline: true,
        }
    }
}

/// AutoPlace engine for positioning elements.
pub struct Autoplace {
    config: AutoplaceConfig,
}

impl Autoplace {
    /// Create a new AutoPlace engine with default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: AutoplaceConfig::default(),
        }
    }

    /// Create with custom configuration.
    #[must_use]
    pub fn with_config(config: AutoplaceConfig) -> Self {
        Self { config }
    }

    /// Autoplace an element relative to a skyline.
    ///
    /// # Arguments
    /// * `shape` - The shape of the element to place
    /// * `skyline` - The skyline to avoid
    /// * `above` - Whether to place above (true) or below (false) the staff
    /// * `spatium` - Staff space size in pixels
    ///
    /// # Returns
    /// The result with offset to apply.
    pub fn autoplace_element(
        &self,
        shape: &Shape,
        skyline: &SkylineLine,
        above: bool,
        spatium: f64,
    ) -> AutoplaceResult {
        if !self.config.enabled || skyline.is_empty() {
            return AutoplaceResult::default();
        }

        let min_distance = self.config.min_distance * spatium;
        let horizontal_clearance = self.config.horizontal_clearance * spatium;

        let distance = if above {
            skyline.min_distance_to_shape_above(shape, horizontal_clearance)
        } else {
            skyline.min_distance_to_shape_below(shape, horizontal_clearance)
        };

        if distance > -min_distance {
            let offset_y = if above {
                -(distance + min_distance)
            } else {
                distance + min_distance
            };

            AutoplaceResult {
                offset_y,
                moved: true,
                add_to_skyline: true,
            }
        } else {
            AutoplaceResult::default()
        }
    }

    /// Autoplace a segment element (attached to a segment, like dynamics).
    ///
    /// This is the main autoplace method for most notation elements.
    #[allow(clippy::too_many_arguments)]
    pub fn autoplace_segment_element(
        &self,
        shape: &Shape,
        position: Point,
        skyline: &mut Skyline,
        above: bool,
        spatium: f64,
        element_type: ElementType,
        element_id: u64,
    ) -> AutoplaceResult {
        if !self.config.enabled {
            return AutoplaceResult::default();
        }

        let staff_skyline = if above {
            skyline.north()
        } else {
            skyline.south()
        };

        // Filter skyline for elements that should ignore each other
        let filtered_skyline = self.filter_skyline(staff_skyline, element_type, element_id);

        if filtered_skyline.is_empty() {
            // No obstacles, just add to skyline
            let translated_shape = shape.translate(Point::new(position.x, position.y));
            if above {
                skyline.north_mut().add_shape(&translated_shape);
            } else {
                skyline.south_mut().add_shape(&translated_shape);
            }
            return AutoplaceResult::default();
        }

        let min_distance = self.config.min_distance * spatium;
        let horizontal_clearance = self.config.horizontal_clearance * spatium;

        let translated_shape = shape.translate(Point::new(position.x, position.y));
        let distance = if above {
            filtered_skyline.min_distance_to_shape_above(&translated_shape, horizontal_clearance)
        } else {
            filtered_skyline.min_distance_to_shape_below(&translated_shape, horizontal_clearance)
        };

        let mut result = AutoplaceResult::default();

        if distance > -min_distance {
            let offset_y = if above {
                -(distance + min_distance)
            } else {
                distance + min_distance
            };

            result.offset_y = offset_y;
            result.moved = true;

            // Update shape position and add to skyline
            let final_shape = translated_shape.translate(Point::new(0.0, offset_y));
            if above {
                skyline.north_mut().add_shape(&final_shape);
            } else {
                skyline.south_mut().add_shape(&final_shape);
            }
        } else {
            // No movement needed, add current shape to skyline
            if above {
                skyline.north_mut().add_shape(&translated_shape);
            } else {
                skyline.south_mut().add_shape(&translated_shape);
            }
        }

        result
    }

    /// Filter skyline elements that should be ignored for collision detection.
    fn filter_skyline(
        &self,
        skyline: &SkylineLine,
        element_type: ElementType,
        element_id: u64,
    ) -> SkylineLine {
        skyline.filtered(|el| self.should_ignore(element_type, element_id, el))
    }

    /// Determine if two elements should ignore each other for autoplace.
    ///
    /// Based on MuseScore's `itemsShouldIgnoreEachOther` function.
    fn should_ignore(
        &self,
        element_type: ElementType,
        element_id: u64,
        skyline_element: &SkylineElement,
    ) -> bool {
        // Same element always ignores itself
        if skyline_element.element_id == Some(element_id) {
            return true;
        }

        let Some(other_type) = skyline_element.element_type else {
            return false;
        };

        // Same type generally ignores each other
        if element_type == other_type {
            return matches!(
                element_type,
                ElementType::Note
                    | ElementType::Rest
                    | ElementType::Chord
                    | ElementType::Beam
                    | ElementType::Stem
            );
        }

        // Specific type combinations that should ignore each other
        match (element_type, other_type) {
            // Time signatures ignore everything except key signatures
            (ElementType::TimeSignature, ElementType::KeySignature) => false,
            (ElementType::TimeSignature, _) => true,

            // Dynamics and hairpins ignore each other
            (ElementType::Dynamic, ElementType::Dynamic) => true,

            // Tuplets and staff lines can ignore each other
            (ElementType::StaffLines, _) | (_, ElementType::StaffLines) => true,

            _ => false,
        }
    }

    /// Calculate vertical clearance between two shapes.
    #[must_use]
    pub fn vertical_clearance(shape1: &Shape, shape2: &Shape) -> f64 {
        let bbox1 = shape1.bbox();
        let bbox2 = shape2.bbox();

        // Check horizontal overlap
        if bbox1.x1 <= bbox2.x0 || bbox2.x1 <= bbox1.x0 {
            return f64::INFINITY; // No horizontal overlap
        }

        // Calculate vertical distance
        if bbox1.y1 <= bbox2.y0 {
            // shape1 is above shape2
            bbox2.y0 - bbox1.y1
        } else if bbox2.y1 <= bbox1.y0 {
            // shape2 is above shape1
            bbox1.y0 - bbox2.y1
        } else {
            // Shapes overlap vertically
            0.0
        }
    }
}

impl Default for Autoplace {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper struct for managing autoplace state during layout.
#[derive(Debug, Default)]
pub struct AutoplaceState {
    /// Skylines per staff
    pub staff_skylines: Vec<Skyline>,
}

impl AutoplaceState {
    /// Create state for a given number of staves.
    #[must_use]
    pub fn new(staff_count: usize) -> Self {
        Self {
            staff_skylines: vec![Skyline::new(); staff_count],
        }
    }

    /// Get skyline for a staff.
    pub fn skyline(&self, staff_idx: usize) -> Option<&Skyline> {
        self.staff_skylines.get(staff_idx)
    }

    /// Get mutable skyline for a staff.
    pub fn skyline_mut(&mut self, staff_idx: usize) -> Option<&mut Skyline> {
        self.staff_skylines.get_mut(staff_idx)
    }

    /// Clear all skylines.
    pub fn clear(&mut self) {
        for skyline in &mut self.staff_skylines {
            skyline.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Rect;

    #[test]
    fn test_autoplace_config_default() {
        let config = AutoplaceConfig::default();
        assert!(config.enabled);
        assert!(config.min_distance > 0.0);
    }

    #[test]
    fn test_autoplace_no_collision() {
        let autoplace = Autoplace::new();
        let shape = Shape::from_rect(Rect::new(0.0, -10.0, 50.0, 0.0));
        let skyline = SkylineLine::north();

        let result = autoplace.autoplace_element(&shape, &skyline, true, 10.0);

        assert!(!result.moved);
        assert_eq!(result.offset_y, 0.0);
    }

    #[test]
    fn test_autoplace_with_collision_above() {
        let autoplace = Autoplace::new();
        let shape = Shape::from_rect(Rect::new(10.0, -15.0, 40.0, -5.0));

        let mut skyline = SkylineLine::north();
        skyline.add(Rect::new(0.0, -20.0, 50.0, -10.0));

        let result = autoplace.autoplace_element(&shape, &skyline, true, 10.0);

        // Shape should move up to avoid skyline
        assert!(result.moved);
        assert!(result.offset_y < 0.0);
    }

    #[test]
    fn test_autoplace_with_collision_below() {
        let autoplace = Autoplace::new();
        let shape = Shape::from_rect(Rect::new(10.0, 5.0, 40.0, 15.0));

        let mut skyline = SkylineLine::south();
        skyline.add(Rect::new(0.0, 10.0, 50.0, 30.0));

        let result = autoplace.autoplace_element(&shape, &skyline, false, 10.0);

        // Shape should move down to avoid skyline
        assert!(result.moved);
        assert!(result.offset_y > 0.0);
    }

    #[test]
    fn test_autoplace_disabled() {
        let config = AutoplaceConfig {
            enabled: false,
            ..Default::default()
        };
        let autoplace = Autoplace::with_config(config);

        let shape = Shape::from_rect(Rect::new(10.0, -15.0, 40.0, -5.0));
        let mut skyline = SkylineLine::north();
        skyline.add(Rect::new(0.0, -20.0, 50.0, -10.0));

        let result = autoplace.autoplace_element(&shape, &skyline, true, 10.0);

        assert!(!result.moved);
    }

    #[test]
    fn test_should_ignore_same_element() {
        let autoplace = Autoplace::new();
        let el =
            SkylineElement::with_element(Rect::new(0.0, 0.0, 10.0, 10.0), ElementType::Note, 42);

        assert!(autoplace.should_ignore(ElementType::Note, 42, &el));
    }

    #[test]
    fn test_should_ignore_same_type() {
        let autoplace = Autoplace::new();
        let el =
            SkylineElement::with_element(Rect::new(0.0, 0.0, 10.0, 10.0), ElementType::Note, 1);

        // Same type notes should ignore each other
        assert!(autoplace.should_ignore(ElementType::Note, 2, &el));
    }

    #[test]
    fn test_autoplace_state() {
        let mut state = AutoplaceState::new(2);

        assert_eq!(state.staff_skylines.len(), 2);

        let skyline = state.skyline_mut(0).unwrap();
        skyline.north_mut().add(Rect::new(0.0, -10.0, 100.0, 0.0));

        assert_eq!(state.skyline(0).unwrap().north().len(), 1);
        assert_eq!(state.skyline(1).unwrap().north().len(), 0);
    }

    #[test]
    fn test_vertical_clearance() {
        let shape1 = Shape::from_rect(Rect::new(0.0, 0.0, 100.0, 10.0));
        let shape2 = Shape::from_rect(Rect::new(0.0, 20.0, 100.0, 30.0));

        let clearance = Autoplace::vertical_clearance(&shape1, &shape2);
        assert_eq!(clearance, 10.0); // Gap between y1=10 and y0=20
    }

    #[test]
    fn test_vertical_clearance_no_horizontal_overlap() {
        let shape1 = Shape::from_rect(Rect::new(0.0, 0.0, 10.0, 10.0));
        let shape2 = Shape::from_rect(Rect::new(20.0, 0.0, 30.0, 10.0));

        let clearance = Autoplace::vertical_clearance(&shape1, &shape2);
        assert_eq!(clearance, f64::INFINITY);
    }

    #[test]
    fn test_vertical_clearance_overlap() {
        let shape1 = Shape::from_rect(Rect::new(0.0, 0.0, 100.0, 20.0));
        let shape2 = Shape::from_rect(Rect::new(0.0, 10.0, 100.0, 30.0));

        let clearance = Autoplace::vertical_clearance(&shape1, &shape2);
        assert_eq!(clearance, 0.0);
    }
}
