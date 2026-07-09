//! Layout engine for music engraving.
//!
//! This module implements the core engraving stages following MuseScore's
//! proven architecture:
//!
//! 1. **Segment Creation** - Group elements by time position
//! 2. **Horizontal Spacing** - Spring-based note spacing with collision detection
//! 3. **Vertical Positioning** - Skyline-based system spacing
//! 4. **Collision Avoidance** - Autoplace for overlapping elements
//! 5. **Element Layout** - Position-specific layout (chords, text, etc.)
//!
//! # Architecture
//!
//! - `LayoutContext` - Central orchestrator with configuration and state
//! - `Shape` - Collision detection geometry
//! - `Layout` trait - Element-specific layout implementations
//! - Spring system - Proportional space distribution (Phase 1)
//! - Segment system - Time-based element grouping (Phase 1)
//!
//! # Example
//!
//! ```ignore
//! use engraver::layout::{LayoutContext, LayoutConfiguration};
//!
//! let config = LayoutConfiguration::default();
//! let ctx = LayoutContext::new(config, &score, &style, &font);
//!
//! // Future: layout_score() will orchestrate full layout
//! // let layout_data = layout_score(&ctx);
//! ```

// region:    --- Modules

pub mod autoplace;
pub mod boundary;
#[cfg(feature = "svg")]
pub mod chart;
pub mod context;
pub mod kerning;
pub mod orchestrator;
pub mod segment;
pub mod segment_list;
pub mod shape;
pub mod skyline;
pub mod spacing;
pub mod springs;
pub mod text_metrics;
pub mod tlayout;

// endregion: --- Modules

// region:    --- Re-exports

pub use autoplace::{Autoplace, AutoplaceConfig, AutoplaceResult, AutoplaceState};
pub use boundary::{
    BoundaryContext, calculate_overhang, clamp_point_to_boundary, clamp_to_boundary,
    constrain_shape_to_boundary, padding as boundary_padding,
};
pub use context::{
    LayoutConfiguration, LayoutContext, LayoutContextOwned, LayoutMode, LayoutState,
};
pub use kerning::{KerningType, SpacingPadding, SpacingPaddingPixels};
pub use orchestrator::{PageLayout, PageMargins, SystemLayout};
pub use segment::{ElementId, Segment, SegmentType, VOICES};
pub use segment_list::SegmentList;
pub use shape::{Shape, ShapeElement};
pub use skyline::{Skyline, SkylineElement, SkylineLine};
pub use spacing::{HorizontalSpacing, MinimumDistance, SpacingResult, SqueezeIterator};
pub use springs::{HorizontalSpacingContext, SpacingConfig, Spring, SpringRow};
pub use text_metrics::TextFontMetrics;
pub use tlayout::{Layout, LayoutData};

#[cfg(feature = "svg")]
pub use chart::{
    ChartCursor, ChartLayoutEngine, ChartLayoutResult, CursorConfig, CursorRgba, CursorState,
    CursorStyle, HighlightCommand, LayoutMode as ChartLayoutMode, MeasureMelodyData,
    MelodyNoteSegment, PageLayoutMetrics, expand_melodies_across_measures,
};

// endregion: --- Re-exports

// region:    --- Unit Types

/// Unit conversions and newtype wrappers for dimensional safety.
///
/// These newtypes prevent common unit confusion bugs at compile time.
/// Staff spaces (fundamental unit in music notation).
///
/// One spatium = 1/4 of staff height = distance between two staff lines.
/// Typically 1.75mm (5 points) in standard engraving.
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub struct Spatium(pub f64);

impl Spatium {
    /// Convert spatium to points.
    ///
    /// # Arguments
    ///
    /// * `base_spatium` - The base spatium value from MStyle (in points)
    #[must_use]
    pub fn to_points(self, base_spatium: f64) -> Points {
        Points(self.0 * base_spatium)
    }

    /// Convert spatium to pixels.
    #[must_use]
    pub fn to_pixels(self, base_spatium: f64, dpi: f64) -> Pixels {
        self.to_points(base_spatium).to_pixels(dpi)
    }
}

impl From<f64> for Spatium {
    fn from(value: f64) -> Self {
        Self(value)
    }
}

/// Points (1/72 inch - PostScript/PDF standard).
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub struct Points(pub f64);

impl Points {
    /// Convert points to pixels at given DPI.
    #[must_use]
    pub fn to_pixels(self, dpi: f64) -> Pixels {
        Pixels(self.0 * dpi / 72.0)
    }

    /// Convert points to spatiums.
    #[must_use]
    pub fn to_spatium(self, base_spatium: f64) -> Spatium {
        Spatium(self.0 / base_spatium)
    }
}

impl From<f64> for Points {
    fn from(value: f64) -> Self {
        Self(value)
    }
}

/// Pixels (device-dependent units).
#[derive(Debug, Copy, Clone, PartialEq, PartialOrd)]
pub struct Pixels(pub f64);

impl Pixels {
    /// Convert pixels to points at given DPI.
    #[must_use]
    pub fn to_points(self, dpi: f64) -> Points {
        Points(self.0 * 72.0 / dpi)
    }
}

impl From<f64> for Pixels {
    fn from(value: f64) -> Self {
        Self(value)
    }
}

// Main layout entry point is now in orchestrator module.
// Use layout_score() or LayoutEngine for full score layout.

// endregion: --- Unit Types

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spatium_to_points() {
        let sp = Spatium(2.0);
        let base_spatium = 5.0; // 5 points per spatium
        let pts = sp.to_points(base_spatium);
        assert_eq!(pts.0, 10.0);
    }

    #[test]
    fn test_points_to_pixels() {
        let pts = Points(72.0);
        let dpi = 96.0;
        let pixels = pts.to_pixels(dpi);
        assert_eq!(pixels.0, 96.0); // 72 points at 96 DPI = 96 pixels
    }

    #[test]
    fn test_spatium_to_pixels() {
        let sp = Spatium(1.0);
        let base_spatium = 5.0;
        let dpi = 96.0;
        let pixels = sp.to_pixels(base_spatium, dpi);
        // 1 spatium = 5 points, 5 points at 96 DPI = 5 * 96/72 = 6.667 pixels
        assert!((pixels.0 - 6.666666).abs() < 0.001);
    }

    #[test]
    fn test_points_to_spatium() {
        let pts = Points(10.0);
        let base_spatium = 5.0;
        let sp = pts.to_spatium(base_spatium);
        assert_eq!(sp.0, 2.0);
    }
}

// endregion: --- Tests
