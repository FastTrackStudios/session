//! Spring-based spacing system for music notation.
//!
//! This module implements MuseScore's spring-based horizontal spacing algorithm.
//! Each segment acts as a spring that can stretch or compress based on:
//! - Duration-based natural width
//! - Spring constant (inversely proportional to stretch factor)
//! - Pre-tension (minimum force needed to start stretching)
//!
//! The algorithm distributes extra space proportionally to spring constants,
//! ensuring longer notes get more space than shorter notes.

use super::segment::Segment;

/// A spring representing a segment's spacing properties.
///
/// Springs are used for justification - distributing extra space across
/// a line of music proportionally to duration.
///
/// # Spring Physics
///
/// The spring model uses Hooke's Law: F = k * x
/// - `spring_const` (k): Stiffness, inversely proportional to stretch factor
/// - `width` (x): Current extension
/// - `pre_tension`: Force threshold before spring starts to extend
///
/// Segments with higher stretch factors have lower spring constants,
/// meaning they extend more under the same force.
#[derive(Debug, Clone)]
pub struct Spring {
    /// Spring constant (stiffness). Lower = more stretchable.
    /// Typically 1 / segment.stretch
    pub spring_const: f64,

    /// Current width of the spring (segment width minus offset)
    pub width: f64,

    /// Pre-tension: minimum force before spring starts stretching.
    /// Equals width * spring_const
    pub pre_tension: f64,

    /// Index of the segment this spring represents
    pub segment_index: usize,
}

impl Spring {
    /// Create a new spring from segment properties.
    ///
    /// # Arguments
    /// * `segment_index` - Index of the segment in the segment list
    /// * `width` - Natural width of the segment (minus width offset)
    /// * `stretch` - Stretch factor of the segment (from duration)
    #[must_use]
    pub fn new(segment_index: usize, width: f64, stretch: f64) -> Self {
        let spring_const = if stretch > 0.0 { 1.0 / stretch } else { 1.0 };
        let pre_tension = width * spring_const;

        Self {
            spring_const,
            width,
            pre_tension,
            segment_index,
        }
    }

    /// Create a spring from a segment.
    #[must_use]
    pub fn from_segment(segment: &Segment, index: usize) -> Self {
        let width = segment.width - segment.width_offset;
        Self::new(index, width, segment.stretch)
    }
}

/// A collection of springs for justification.
///
/// The spring row solves for the force needed to achieve a target width,
/// then applies that force to stretch each spring proportionally.
#[derive(Debug, Clone, Default)]
pub struct SpringRow {
    springs: Vec<Spring>,
}

impl SpringRow {
    /// Create a new empty spring row.
    #[must_use]
    pub fn new() -> Self {
        Self {
            springs: Vec::new(),
        }
    }

    /// Create a spring row with pre-allocated capacity.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            springs: Vec::with_capacity(capacity),
        }
    }

    /// Add a spring to the row.
    pub fn push(&mut self, spring: Spring) {
        self.springs.push(spring);
    }

    /// Get the number of springs.
    #[must_use]
    pub fn len(&self) -> usize {
        self.springs.len()
    }

    /// Check if the row is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.springs.is_empty()
    }

    /// Get the total natural width of all springs.
    #[must_use]
    pub fn total_width(&self) -> f64 {
        self.springs.iter().map(|s| s.width).sum()
    }

    /// Stretch springs to add extra width.
    ///
    /// This implements MuseScore's spring stretching algorithm:
    /// 1. Sort springs by pre-tension (ascending)
    /// 2. Progressively add springs until force exceeds next spring's pre-tension
    /// 3. Calculate final force and distribute to all participating springs
    ///
    /// # Arguments
    /// * `extra_width` - Additional width to distribute across springs
    ///
    /// # Returns
    /// A vector of (segment_index, new_width) pairs
    #[must_use]
    pub fn stretch(&mut self, extra_width: f64) -> Vec<(usize, f64)> {
        if self.springs.is_empty() || extra_width <= 0.0 {
            return Vec::new();
        }

        // Sort by pre-tension (springs with lower pre-tension stretch first)
        self.springs
            .sort_by(|a, b| a.pre_tension.total_cmp(&b.pre_tension));

        let mut inverse_spring_const = 0.0;
        let mut accumulated_width = extra_width;
        let mut force = 0.0;

        // Find the equilibrium force
        // Springs engage progressively as force increases
        let mut engaged_count = 0;
        for spring in &self.springs {
            inverse_spring_const += 1.0 / spring.spring_const;
            accumulated_width += spring.width;
            force = accumulated_width / inverse_spring_const;
            engaged_count += 1;

            // Stop when force is less than next spring's pre-tension
            // (next spring won't engage at this force level)
            if engaged_count < self.springs.len() {
                let next_pre_tension = self.springs[engaged_count].pre_tension;
                if force < next_pre_tension {
                    break;
                }
            }
        }

        // Apply force to calculate new widths
        let mut results = Vec::with_capacity(self.springs.len());
        for spring in &self.springs {
            if force > spring.pre_tension {
                let new_width = force / spring.spring_const;
                results.push((spring.segment_index, new_width));
            }
        }

        results
    }

    /// Clear all springs.
    pub fn clear(&mut self) {
        self.springs.clear();
    }

    /// Iterate over springs.
    pub fn iter(&self) -> impl Iterator<Item = &Spring> {
        self.springs.iter()
    }
}

/// Configuration for spacing calculations.
#[derive(Debug, Clone)]
pub struct SpacingConfig {
    /// Spatium (staff space) in pixels
    pub spatium: f64,

    /// Spacing density multiplier (higher = tighter spacing)
    pub spacing_density: f64,

    /// Stretch reduction factor (for squeezing)
    pub stretch_reduction: f64,

    /// Squeeze factor for collision padding reduction
    pub squeeze_factor: f64,

    /// Base width for a quarter note (in spatiums)
    pub quarter_note_space: f64,

    /// Measure spacing slope (exponent for duration ratio)
    /// Default is around 1.3-1.5
    pub measure_spacing: f64,
}

/// Context for horizontal spacing computation.
///
/// This tracks state during the spacing algorithm, including:
/// - Current X position as we place segments
/// - Left barrier for lyrics/margin enforcement
/// - Progressive squeeze factors when fitting content
///
/// Based on MuseScore's HorizontalSpacingContext.
#[derive(Debug, Clone)]
pub struct HorizontalSpacingContext {
    /// Current X position in the system
    pub x_cur: f64,

    /// Left margin barrier (for lyrics, etc.)
    pub x_left_barrier: f64,

    /// Stretch reduction factor (1.0 = normal, 0.67 = squeezed)
    /// Reduces duration-based spacing
    pub stretch_reduction: f64,

    /// Squeeze factor (1.0 = normal, 0.0 = maximum squeeze)
    /// Reduces collision padding
    pub squeeze_factor: f64,

    /// Spacing density from style (higher = tighter)
    pub spacing_density: f64,

    /// Whether to override minimum measure width
    pub override_min_measure_width: bool,

    /// Whether the system is full (affects margin checks)
    pub system_is_full: bool,
}

impl Default for HorizontalSpacingContext {
    fn default() -> Self {
        Self {
            x_cur: 0.0,
            x_left_barrier: 0.0,
            stretch_reduction: 1.0,
            squeeze_factor: 1.0,
            spacing_density: 1.0,
            override_min_measure_width: false,
            system_is_full: false,
        }
    }
}

impl HorizontalSpacingContext {
    /// Create a new context with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a context for squeeze-to-fit layout.
    #[must_use]
    pub fn for_squeeze(stretch_reduction: f64, squeeze_factor: f64) -> Self {
        Self {
            stretch_reduction,
            squeeze_factor,
            system_is_full: true,
            override_min_measure_width: true,
            ..Default::default()
        }
    }

    /// Apply stretch reduction to a duration-based width.
    #[must_use]
    pub fn apply_stretch(&self, width: f64) -> f64 {
        width * self.stretch_reduction / self.spacing_density
    }

    /// Apply squeeze factor to padding.
    #[must_use]
    pub fn apply_squeeze(&self, padding: f64) -> f64 {
        padding * self.squeeze_factor
    }

    /// Create a spacing config from this context.
    #[must_use]
    pub fn to_spacing_config(&self, spatium: f64, measure_spacing: f64) -> SpacingConfig {
        SpacingConfig {
            spatium,
            spacing_density: self.spacing_density,
            stretch_reduction: self.stretch_reduction,
            squeeze_factor: self.squeeze_factor,
            quarter_note_space: 3.5,
            measure_spacing,
        }
    }
}

impl Default for SpacingConfig {
    fn default() -> Self {
        Self {
            spatium: 10.0,
            spacing_density: 1.0,
            stretch_reduction: 1.0,
            squeeze_factor: 1.0,
            quarter_note_space: 3.5,
            measure_spacing: 1.3,
        }
    }
}

impl SpacingConfig {
    /// Create a new spacing config with the given spatium.
    #[must_use]
    pub fn new(spatium: f64) -> Self {
        Self {
            spatium,
            ..Default::default()
        }
    }

    /// Calculate the duration stretch for a given duration.
    ///
    /// The formula is: slope^(log2(ticks / quarter_note_ticks))
    ///
    /// This gives:
    /// - Quarter note: stretch = 1.0
    /// - Half note: stretch = slope (about 1.3-1.5)
    /// - Whole note: stretch = slope^2
    /// - Eighth note: stretch = 1/slope
    ///
    /// # Arguments
    /// * `ticks` - Duration in ticks (480 = quarter note)
    #[must_use]
    pub fn duration_stretch(&self, ticks: i32) -> f64 {
        const QUARTER_NOTE_TICKS: f64 = 480.0;

        if ticks <= 0 {
            return 1.0;
        }

        let duration_ratio = f64::from(ticks) / QUARTER_NOTE_TICKS;
        self.measure_spacing.powf(duration_ratio.log2())
    }

    /// Calculate the natural width for a chord/rest segment.
    ///
    /// # Arguments
    /// * `duration_stretch` - The duration-based stretch factor
    /// * `user_stretch` - User-specified stretch multiplier
    #[must_use]
    pub fn natural_width(&self, duration_stretch: f64, user_stretch: f64) -> f64 {
        let total_stretch = duration_stretch * user_stretch.clamp(0.1, 10.0);
        self.spatium * self.quarter_note_space * total_stretch * self.stretch_reduction
            / self.spacing_density
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spring_new() {
        let spring = Spring::new(0, 50.0, 2.0);
        assert_eq!(spring.segment_index, 0);
        assert!((spring.width - 50.0).abs() < 1e-10);
        assert!((spring.spring_const - 0.5).abs() < 1e-10);
        assert!((spring.pre_tension - 25.0).abs() < 1e-10);
    }

    #[test]
    fn test_spring_row_total_width() {
        let mut row = SpringRow::new();
        row.push(Spring::new(0, 50.0, 1.0));
        row.push(Spring::new(1, 60.0, 1.0));
        row.push(Spring::new(2, 40.0, 1.0));

        assert!((row.total_width() - 150.0).abs() < 1e-10);
    }

    #[test]
    fn test_spring_row_stretch_equal() {
        // Three springs with equal stretch factors
        let mut row = SpringRow::new();
        row.push(Spring::new(0, 50.0, 1.0));
        row.push(Spring::new(1, 50.0, 1.0));
        row.push(Spring::new(2, 50.0, 1.0));

        let results = row.stretch(30.0);

        // Each spring should get 10.0 extra width
        assert_eq!(results.len(), 3);
        for (_, width) in &results {
            assert!((width - 60.0).abs() < 1e-10);
        }
    }

    #[test]
    fn test_spring_row_stretch_unequal() {
        // Springs with different stretch factors
        let mut row = SpringRow::new();
        row.push(Spring::new(0, 50.0, 1.0)); // spring_const = 1.0, pre_tension = 50
        row.push(Spring::new(1, 50.0, 2.0)); // spring_const = 0.5, pre_tension = 25

        // Need enough extra width to engage both springs
        // After sorting by pre_tension: Spring 1 (25), Spring 0 (50)
        // Force = (extra + width) / inverse_k
        // Need force >= 50 to engage Spring 0: (extra + 50) / 2 >= 50 → extra >= 50
        let results = row.stretch(60.0);

        // Both springs should be engaged
        assert_eq!(results.len(), 2);

        let Some((_, width_0)) = results.iter().find(|(i, _)| *i == 0) else {
            panic!("Expected to find spring 0 in results");
        };
        let Some((_, width_1)) = results.iter().find(|(i, _)| *i == 1) else {
            panic!("Expected to find spring 1 in results");
        };

        // Higher stretch = lower spring constant = more extension
        // Spring 1 (stretch=2, k=0.5) should be wider than Spring 0 (stretch=1, k=1.0)
        assert!(width_1 > width_0);
    }

    #[test]
    fn test_spring_row_no_stretch_for_zero() {
        let mut row = SpringRow::new();
        row.push(Spring::new(0, 50.0, 1.0));

        let results = row.stretch(0.0);
        assert!(results.is_empty());
    }

    #[test]
    fn test_duration_stretch() {
        let config = SpacingConfig::default();

        // Quarter note (480 ticks) should have stretch = 1.0
        let quarter_stretch = config.duration_stretch(480);
        assert!((quarter_stretch - 1.0).abs() < 1e-10);

        // Half note (960 ticks) should have stretch > 1.0
        let half_stretch = config.duration_stretch(960);
        assert!(half_stretch > 1.0);
        assert!((half_stretch - config.measure_spacing).abs() < 1e-10);

        // Eighth note (240 ticks) should have stretch < 1.0
        let eighth_stretch = config.duration_stretch(240);
        assert!(eighth_stretch < 1.0);
    }

    #[test]
    fn test_natural_width() {
        let config = SpacingConfig::new(10.0);

        // Quarter note
        let width = config.natural_width(1.0, 1.0);
        let expected = 10.0 * 3.5 * 1.0 * 1.0 / 1.0;
        assert!((width - expected).abs() < 1e-10);

        // Half note (stretch = slope)
        let half_stretch = config.measure_spacing;
        let half_width = config.natural_width(half_stretch, 1.0);
        assert!(half_width > width);
    }
}
