//! Layout constants for chart rendering.
//!
//! This module centralizes all magic numbers used in chart layout,
//! making them easier to tune and understand.

// ============================================================================
// Page Layout
// ============================================================================

/// Default page margin at the top (in points).
/// Reduced from 50 to bring header closer to top.
pub const DEFAULT_MARGIN_TOP: f64 = 36.0;

/// Default page margin on the right (in points).
pub const DEFAULT_MARGIN_RIGHT: f64 = 50.0;

/// Default page margin at the bottom (in points).
pub const DEFAULT_MARGIN_BOTTOM: f64 = 50.0;

/// Default page margin on the left (in points).
/// Extra margin for section labels.
pub const DEFAULT_MARGIN_LEFT: f64 = 72.0;

/// Default staff space (spatium) in points.
/// The fundamental unit for music notation spacing.
pub const DEFAULT_SPATIUM: f64 = 5.0;

/// Default spacing between systems (in points).
/// 4 spatiums = 20pt allows 10 systems per page.
pub const DEFAULT_SYSTEM_SPACING: f64 = 20.0;

/// Default minimum measure width (in points).
pub const DEFAULT_MIN_MEASURE_WIDTH: f64 = 100.0;

/// Default maximum measures per system.
pub const DEFAULT_MAX_MEASURES_PER_SYSTEM: usize = 4;

// ============================================================================
// Page Rendering
// ============================================================================

/// Horizontal offset for page positioning in multi-page layouts.
pub const PAGE_OFFSET_X: f64 = 20.0;

/// Vertical offset for page positioning in multi-page layouts.
pub const PAGE_OFFSET_Y: f64 = 20.0;

/// Gap between pages in multi-page layouts.
pub const PAGE_GAP: f64 = 40.0;

// ============================================================================
// Staff Layout
// ============================================================================

/// Staff height as a multiple of spatium.
/// Standard 5-line staff has 4 spaces between lines.
pub const STAFF_HEIGHT_SPATIUMS: f64 = 4.0;

/// Space above staff for chord symbols (in points).
pub const CHORD_SPACE_ABOVE_STAFF: f64 = 30.0;

/// Y offset for chord symbols above the staff (in points).
/// Negative means above the staff line.
pub const CHORD_Y_OFFSET: f64 = -6.0;

/// Middle of staff position (in spatiums from top line).
pub const STAFF_MIDDLE_Y_SPATIUMS: f64 = 2.0;

// ============================================================================
// Articulation Layout (following MuseScore defaults)
// ============================================================================

/// Minimum distance from notehead to articulation (in spatiums).
/// From MuseScore: articulationMinDistance = 0.4sp
pub const ARTICULATION_MIN_DISTANCE_SPATIUMS: f64 = 0.4;

/// Articulation scale factor (1.0 = normal size).
/// From MuseScore: articulationMag = 1.0
pub const ARTICULATION_MAG: f64 = 1.0;

/// Accent glyph width in spatiums (from Bravura metadata).
/// The bounding box width of articAccentAbove is 1.356 spatiums.
pub const ACCENT_WIDTH_SPATIUMS: f64 = 1.356;

/// Accent glyph height in spatiums (from Bravura metadata).
/// The bounding box height of articAccentAbove is approximately 0.5 spatiums.
pub const ACCENT_HEIGHT_SPATIUMS: f64 = 0.5;

// ============================================================================
// System Prefix (Clef and Time Signature)
// ============================================================================

/// Spacing after clef (as multiple of spatium).
pub const CLEF_SPACING_SPATIUMS: f64 = 0.5;

/// Spacing after time signature (as multiple of spatium).
pub const TIME_SIG_SPACING_SPATIUMS: f64 = 0.8;

/// Approximate time signature width (as multiple of spatium).
pub const TIME_SIG_WIDTH_SPATIUMS: f64 = 2.0;

// ============================================================================
// Count-in and Compact Measures
// ============================================================================

/// Scale factor for count-in measures (45% of normal width - slightly relaxed for better spacing).
pub const COUNT_IN_COMPACT_SCALE: f64 = 0.45;

// ============================================================================
// Tempo and Timing
// ============================================================================

/// Default tempo in BPM when not specified.
pub const DEFAULT_TEMPO_BPM: f64 = 120.0;

/// Ticks per quarter note (standard MIDI resolution).
pub const TICKS_PER_QUARTER: f64 = 480.0;

/// Seconds per minute.
pub const SECONDS_PER_MINUTE: f64 = 60.0;

// ============================================================================
// Measure Numbers
// ============================================================================

/// Y offset for measure numbers above staff (as multiple of spatium, negative = above).
pub const MEASURE_NUMBER_Y_OFFSET_SPATIUMS: f64 = -2.0;

/// X offset for measure numbers from barline (as multiple of spatium).
pub const MEASURE_NUMBER_X_OFFSET_SPATIUMS: f64 = 0.25;

/// Font size for measure numbers (as multiple of spatium).
pub const MEASURE_NUMBER_FONT_SIZE_SPATIUMS: f64 = 1.6;

// ============================================================================
// Section Labels (Rehearsal Marks)
// ============================================================================

/// Y offset for section labels below staff (as multiple of spatium).
pub const SECTION_LABEL_Y_OFFSET_SPATIUMS: f64 = 2.0;

/// Font size for section labels (as multiple of spatium).
pub const SECTION_LABEL_FONT_SIZE_SPATIUMS: f64 = 2.0;

// ============================================================================
// Ties and Slurs
// ============================================================================

/// X offset for tie endpoints from measure boundary (as multiple of spatium).
pub const TIE_ENDPOINT_X_OFFSET_SPATIUMS: f64 = 0.5;

// ============================================================================
// Measure Layout Weights
// ============================================================================

/// Base weight for measure width calculation.
pub const MEASURE_WEIGHT_BASE: f64 = 1.0;

/// Additional weight per extra chord in a measure.
pub const MEASURE_WEIGHT_PER_CHORD: f64 = 0.3;

/// Minimum weight for any measure.
pub const MEASURE_WEIGHT_MIN: f64 = 0.5;

/// Maximum weight for any measure.
pub const MEASURE_WEIGHT_MAX: f64 = 4.0;

/// Additional weight for measures with harmonics.
pub const MEASURE_WEIGHT_HARMONICS: f64 = 0.2;

/// Additional weight per extra melody note (beyond 4).
pub const MEASURE_WEIGHT_PER_MELODY_NOTE: f64 = 0.1;

/// Threshold melody note count for extra weight.
pub const MEASURE_WEIGHT_MELODY_THRESHOLD: usize = 4;

// ============================================================================
// Chord Symbol Layout
// ============================================================================

/// Minimum chord width as multiple of base font size.
pub const CHORD_MIN_WIDTH_FACTOR: f64 = 1.5;

/// Minimum chord width as multiple of base font size for short chords.
pub const CHORD_MIN_WIDTH_SHORT_FACTOR: f64 = 2.0;

/// Typical chord width as multiple of root font size.
pub const TYPICAL_CHORD_WIDTH_FACTOR: f64 = 2.5;

/// Chord padding as multiple of base font size.
pub const CHORD_PADDING_FACTOR: f64 = 0.5;

/// Minimum spacing between chords as multiple of base font size.
pub const CHORD_MIN_SPACING_FACTOR: f64 = 0.3;

// ============================================================================
// Chord Scaling
// ============================================================================

/// Minimum scale factor for chord symbols when reducing to fit.
pub const CHORD_SCALE_MIN: f64 = 0.5;

/// Maximum scale factor for chord symbols.
pub const CHORD_SCALE_MAX: f64 = 1.0;

// ============================================================================
// Layout Validation Tolerances
// ============================================================================

/// Tolerance for width comparisons in layout tests (in points).
pub const WIDTH_COMPARISON_TOLERANCE: f64 = 0.1;

/// Tolerance for position comparisons in layout tests (in points).
pub const POSITION_TOLERANCE: f64 = 1.0;

/// Maximum bottom space as fraction of available height.
pub const MAX_BOTTOM_SPACE_FRACTION: f64 = 0.35;

/// Minimum staff line width for detection (in points).
pub const MIN_STAFF_LINE_WIDTH: f64 = 100.0;

/// Height tolerance for horizontal line detection (in points).
pub const HORIZONTAL_LINE_TOLERANCE: f64 = 0.1;

/// Unique width tolerance for deduplication (in points).
pub const UNIQUE_WIDTH_TOLERANCE: f64 = 1.0;

/// Maximum width ratio for "short" system detection.
pub const SHORT_SYSTEM_WIDTH_RATIO: f64 = 0.75;

// ============================================================================
// Helper Functions
// ============================================================================

/// Calculate staff height from spatium.
#[inline]
#[must_use]
pub const fn staff_height(spatium: f64) -> f64 {
    spatium * STAFF_HEIGHT_SPATIUMS
}

/// Calculate system height (staff + chord space).
#[inline]
#[must_use]
pub const fn system_height(spatium: f64) -> f64 {
    staff_height(spatium) + CHORD_SPACE_ABOVE_STAFF
}

/// Calculate clef width with spacing.
#[inline]
#[must_use]
pub const fn clef_width_with_spacing(clef_base_width: f64, spatium: f64) -> f64 {
    clef_base_width + spatium * CLEF_SPACING_SPATIUMS
}

/// Calculate time signature width with spacing.
#[inline]
#[must_use]
pub const fn time_sig_width_with_spacing(spatium: f64) -> f64 {
    spatium * TIME_SIG_WIDTH_SPATIUMS + spatium * TIME_SIG_SPACING_SPATIUMS
}

/// Calculate seconds per tick from tempo.
#[inline]
#[must_use]
pub const fn seconds_per_tick(tempo_bpm: f64) -> f64 {
    (SECONDS_PER_MINUTE / tempo_bpm) / TICKS_PER_QUARTER
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_staff_height() {
        assert!((staff_height(5.0) - 20.0).abs() < 0.001);
        assert!((staff_height(4.0) - 16.0).abs() < 0.001);
    }

    #[test]
    fn test_system_height() {
        assert!((system_height(5.0) - 50.0).abs() < 0.001); // 20 + 30
    }

    #[test]
    fn test_seconds_per_tick() {
        // At 120 BPM: 0.5 seconds per quarter, 480 ticks per quarter
        // So seconds per tick = 0.5 / 480 ≈ 0.001041667
        let spt = seconds_per_tick(120.0);
        assert!((spt - 0.001041667).abs() < 0.0001);
    }

    #[test]
    fn test_time_sig_width() {
        let width = time_sig_width_with_spacing(5.0);
        // 2.0 * 5 + 0.8 * 5 = 10 + 4 = 14
        assert!((width - 14.0).abs() < 0.001);
    }
}
