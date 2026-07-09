//! Layout parameters for chart rendering.
//!
//! Pure layout parameters that control spacing, sizing, and margins.
//! These parameters don't affect rendering style or behavior.

use crate::engraver::layout::chart::constants;
use crate::engraver::layout::chart::spacing;
use crate::engraver::layout::orchestrator::PageMargins;

/// Layout parameters for chart rendering.
///
/// Controls the physical layout of the chart: margins, spacing, and sizing.
/// These are "pure" layout parameters that don't affect rendering style
/// or behavioral aspects.
#[derive(Debug, Clone)]
pub struct LayoutParams {
    /// Page margins (top, right, bottom, left).
    pub margins: PageMargins,

    /// Staff space (spatium) in points.
    ///
    /// This is the fundamental unit for music notation spacing.
    /// All other spacing values are typically expressed as multiples of spatium.
    pub spatium: f64,

    /// Spacing between systems (in points).
    ///
    /// The vertical gap between consecutive systems on a page.
    pub system_spacing: f64,

    /// Maximum number of measures per system.
    ///
    /// Systems will break before this limit if the content requires it,
    /// but will never exceed this count.
    pub max_measures_per_system: usize,

    /// Minimum measure width (in points).
    ///
    /// Measures will not be narrower than this, even if content would allow.
    pub min_measure_width: f64,

    /// Slope for the duration-to-space power law.
    ///
    /// Controls how aggressively longer notes get more horizontal space.
    /// At slope=1.2: half note gets 1.2×, whole gets 1.44×, eighth gets 0.83×.
    /// At slope=1.0: all durations get equal space (uniform spacing).
    pub spacing_slope: f64,

    /// Spacing density (divides natural width; higher = tighter).
    ///
    /// At 1.0 (default): standard spacing.
    /// At 2.0: half the natural width (very tight).
    pub spacing_density: f64,

    /// Fill limit for last-system justification.
    ///
    /// Systems with fill ratio below this threshold are left ragged
    /// (not stretched to fill the line). Prevents sparse final lines
    /// from being stretched across the full width.
    pub last_system_fill_limit: f64,
}

impl Default for LayoutParams {
    fn default() -> Self {
        Self {
            margins: PageMargins {
                top: constants::DEFAULT_MARGIN_TOP,
                right: constants::DEFAULT_MARGIN_RIGHT,
                bottom: constants::DEFAULT_MARGIN_BOTTOM,
                left: constants::DEFAULT_MARGIN_LEFT,
            },
            spatium: constants::DEFAULT_SPATIUM,
            system_spacing: constants::DEFAULT_SYSTEM_SPACING,
            max_measures_per_system: constants::DEFAULT_MAX_MEASURES_PER_SYSTEM,
            min_measure_width: constants::DEFAULT_MIN_MEASURE_WIDTH,
            spacing_slope: spacing::DEFAULT_SPACING_SLOPE,
            spacing_density: spacing::DEFAULT_SPACING_DENSITY,
            last_system_fill_limit: spacing::DEFAULT_LAST_SYSTEM_FILL_LIMIT,
        }
    }
}

impl LayoutParams {
    /// Create layout params with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Calculate the staff height based on spatium.
    #[must_use]
    pub fn staff_height(&self) -> f64 {
        constants::staff_height(self.spatium)
    }

    /// Calculate the system height (staff + chord space).
    #[must_use]
    pub fn system_height(&self) -> f64 {
        constants::system_height(self.spatium)
    }

    /// Calculate available content width given page width.
    #[must_use]
    pub fn content_width(&self, page_width: f64) -> f64 {
        page_width - self.margins.left - self.margins.right
    }

    /// Calculate available content height given page height.
    #[must_use]
    pub fn content_height(&self, page_height: f64) -> f64 {
        page_height - self.margins.top - self.margins.bottom
    }

    // =========================================================================
    // Builder-style setters
    // =========================================================================

    /// Set page margins.
    #[must_use]
    pub fn with_margins(mut self, margins: PageMargins) -> Self {
        self.margins = margins;
        self
    }

    /// Set the spatium (staff space).
    #[must_use]
    pub fn with_spatium(mut self, spatium: f64) -> Self {
        self.spatium = spatium;
        self
    }

    /// Set system spacing.
    #[must_use]
    pub fn with_system_spacing(mut self, spacing: f64) -> Self {
        self.system_spacing = spacing;
        self
    }

    /// Set maximum measures per system.
    #[must_use]
    pub fn with_max_measures_per_system(mut self, max: usize) -> Self {
        self.max_measures_per_system = max;
        self
    }

    /// Set minimum measure width.
    #[must_use]
    pub fn with_min_measure_width(mut self, min: f64) -> Self {
        self.min_measure_width = min;
        self
    }

    /// Set the duration-to-space slope.
    #[must_use]
    pub fn with_spacing_slope(mut self, slope: f64) -> Self {
        self.spacing_slope = slope;
        self
    }

    /// Set the spacing density.
    #[must_use]
    pub fn with_spacing_density(mut self, density: f64) -> Self {
        self.spacing_density = density;
        self
    }

    /// Set the last-system fill limit.
    #[must_use]
    pub fn with_last_system_fill_limit(mut self, limit: f64) -> Self {
        self.last_system_fill_limit = limit;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_params() {
        let params = LayoutParams::default();

        assert_eq!(params.spatium, constants::DEFAULT_SPATIUM);
        assert_eq!(params.margins.top, constants::DEFAULT_MARGIN_TOP);
        assert_eq!(params.margins.left, constants::DEFAULT_MARGIN_LEFT);
    }

    #[test]
    fn test_staff_height() {
        let params = LayoutParams::default();
        let height = params.staff_height();

        // 5.0 spatium * 4.0 = 20.0
        assert!((height - 20.0).abs() < 0.001);
    }

    #[test]
    fn test_content_dimensions() {
        let params = LayoutParams::default();

        let content_width = params.content_width(612.0);
        let expected_width =
            612.0 - constants::DEFAULT_MARGIN_LEFT - constants::DEFAULT_MARGIN_RIGHT;
        assert!((content_width - expected_width).abs() < 0.001);

        let content_height = params.content_height(792.0);
        let expected_height =
            792.0 - constants::DEFAULT_MARGIN_TOP - constants::DEFAULT_MARGIN_BOTTOM;
        assert!((content_height - expected_height).abs() < 0.001);
    }

    #[test]
    fn test_builder() {
        let params = LayoutParams::new()
            .with_spatium(6.0)
            .with_max_measures_per_system(6);

        assert_eq!(params.spatium, 6.0);
        assert_eq!(params.max_measures_per_system, 6);
    }
}
