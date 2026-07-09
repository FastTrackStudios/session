//! Rendering options for chart display.
//!
//! Controls how the chart is visually rendered, including chord symbol style,
//! measure numbers, and count-in display.

use crate::engraver::layout::tlayout::HarmonyStyle;

/// Rendering options for chart display.
///
/// Controls visual aspects of the chart that don't affect layout geometry
/// but do affect how elements are drawn.
#[derive(Debug, Clone)]
pub struct RenderOptions {
    /// Harmony style for chord symbols.
    ///
    /// Controls font, sizing, and formatting of chord symbol text.
    pub harmony_style: HarmonyStyle,

    /// Show measure numbers above the first measure of each system.
    ///
    /// When true, displays measure numbers (accounting for offset)
    /// above bars for navigation.
    pub show_measure_numbers: bool,

    /// Offset for measure numbering.
    ///
    /// From REAPER's `projmeasoffs` or song start.
    /// If the project starts at measure 5, set this to 4 so measure 1
    /// displays as "5".
    pub measure_number_offset: i32,

    /// Number of count-in measures (0, 1, or 2).
    ///
    /// Determined by the distance between Count-In marker and SONGSTART marker.
    /// Count-in measures are the N measures immediately before measure 1.
    pub count_in_measures: u8,

    /// How count-in measures participate in measure numbering.
    ///
    /// - `false` (default): count-in measures get sequential numbers (1, 2)
    ///   and the first non-count-in measure is `count_in_measures + 1`.
    ///   This matches imported charts (Finale / Sibelius / MuseXML) where
    ///   the click-count bars *are* part of the printed measure count, so a
    ///   chart with two count-in bars puts the first real chord on m. 3.
    /// - `true`: count-in measures number as `-(count_in_measures - 1) .. 0`
    ///   so the first real measure is 1 regardless of count-in length.
    ///   This is the historical keyflow / DAW workflow.
    pub count_in_uses_negative_numbers: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            harmony_style: HarmonyStyle::musejazz(),
            show_measure_numbers: true,
            measure_number_offset: 0,
            count_in_measures: 0,
            count_in_uses_negative_numbers: false,
        }
    }
}

impl RenderOptions {
    /// Create render options with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create render options for minimal display (no numbers, no count-in).
    #[must_use]
    pub fn minimal() -> Self {
        Self {
            harmony_style: HarmonyStyle::musejazz(),
            show_measure_numbers: false,
            measure_number_offset: 0,
            count_in_measures: 0,
            count_in_uses_negative_numbers: false,
        }
    }

    // =========================================================================
    // Builder-style setters
    // =========================================================================

    /// Set the harmony style.
    #[must_use]
    pub fn with_harmony_style(mut self, style: HarmonyStyle) -> Self {
        self.harmony_style = style;
        self
    }

    /// Set whether to show measure numbers.
    #[must_use]
    pub fn with_show_measure_numbers(mut self, show: bool) -> Self {
        self.show_measure_numbers = show;
        self
    }

    /// Set the measure number offset.
    #[must_use]
    pub fn with_measure_number_offset(mut self, offset: i32) -> Self {
        self.measure_number_offset = offset;
        self
    }

    /// Set the number of count-in measures.
    #[must_use]
    pub fn with_count_in_measures(mut self, count: u8) -> Self {
        self.count_in_measures = count;
        self
    }

    // =========================================================================
    // Computed values
    // =========================================================================

    /// Get the display measure number for a given actual measure index.
    ///
    /// Accounts for [`measure_number_offset`] and the
    /// [`count_in_uses_negative_numbers`] setting. When negative-numbering
    /// is on, the first `count_in_measures` measures count down to 0 and
    /// the first non-count-in measure is `1`.
    #[must_use]
    pub fn display_measure_number(&self, actual_index: usize) -> i32 {
        let raw = actual_index as i32 + 1;
        let shifted = if self.count_in_uses_negative_numbers && self.count_in_measures > 0 {
            raw - self.count_in_measures as i32
        } else {
            raw
        };
        shifted + self.measure_number_offset
    }

    /// Check if count-in should be rendered.
    #[must_use]
    pub fn has_count_in(&self) -> bool {
        self.count_in_measures > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_options() {
        let options = RenderOptions::default();

        assert!(options.show_measure_numbers);
        assert_eq!(options.measure_number_offset, 0);
        assert_eq!(options.count_in_measures, 0);
    }

    #[test]
    fn test_minimal_options() {
        let options = RenderOptions::minimal();

        assert!(!options.show_measure_numbers);
        assert!(!options.has_count_in());
    }

    #[test]
    fn test_display_measure_number() {
        let options = RenderOptions::default().with_measure_number_offset(4);

        // First measure (index 0) should display as 5
        assert_eq!(options.display_measure_number(0), 5);
        // Tenth measure (index 9) should display as 14
        assert_eq!(options.display_measure_number(9), 14);
    }

    #[test]
    fn test_builder() {
        let options = RenderOptions::new()
            .with_show_measure_numbers(false)
            .with_count_in_measures(2);

        assert!(!options.show_measure_numbers);
        assert!(options.has_count_in());
        assert_eq!(options.count_in_measures, 2);
    }
}
