//! Chart layout configuration.
//!
//! Configuration is split into three concerns:
//!
//! - [`LayoutParams`] - Pure layout parameters (margins, spacing, sizing)
//! - [`RenderOptions`] - Rendering style options (harmony style, measure numbers)
//! - [`BehavioralFlags`] - Behavioral toggles (hide repeated chords, use stems)
//!
//! These can be composed into [`ChartLayoutConfig`] for backward compatibility.

pub mod behavioral;
mod layout_params;
mod render_options;

pub use behavioral::{BehavioralFlags, DEFAULT_MIN_CHORD_SYMBOL_GAP};
pub use layout_params::LayoutParams;
pub use render_options::RenderOptions;

use crate::engraver::layout::orchestrator::PageMargins;
use crate::engraver::layout::tlayout::HarmonyStyle;

/// Complete chart layout configuration.
///
/// This struct combines all configuration concerns into a single type
/// for backward compatibility. New code should prefer using the
/// individual config types directly.
#[derive(Debug, Clone, Default)]
pub struct ChartLayoutConfig {
    /// Layout parameters (margins, spacing, sizing).
    pub layout: LayoutParams,
    /// Rendering options (harmony style, measure numbers).
    pub render: RenderOptions,
    /// Behavioral flags (hide duplicates, use stems).
    pub behavior: BehavioralFlags,
}

impl ChartLayoutConfig {
    /// Create a new configuration with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a configuration from individual components.
    #[must_use]
    pub fn from_parts(
        layout: LayoutParams,
        render: RenderOptions,
        behavior: BehavioralFlags,
    ) -> Self {
        Self {
            layout,
            render,
            behavior,
        }
    }

    // =========================================================================
    // Delegation to LayoutParams
    // =========================================================================

    /// Page margins.
    #[must_use]
    pub fn margins(&self) -> &PageMargins {
        &self.layout.margins
    }

    /// Staff space (spatium) in points.
    #[must_use]
    pub fn spatium(&self) -> f64 {
        self.layout.spatium
    }

    /// Spacing between systems.
    #[must_use]
    pub fn system_spacing(&self) -> f64 {
        self.layout.system_spacing
    }

    /// Maximum measures per system.
    #[must_use]
    pub fn max_measures_per_system(&self) -> usize {
        self.layout.max_measures_per_system
    }

    /// Minimum measure width.
    #[must_use]
    pub fn min_measure_width(&self) -> f64 {
        self.layout.min_measure_width
    }

    // =========================================================================
    // Delegation to RenderOptions
    // =========================================================================

    /// Harmony style for chord symbols.
    #[must_use]
    pub fn harmony_style(&self) -> &HarmonyStyle {
        &self.render.harmony_style
    }

    /// Mutable harmony style.
    #[must_use]
    pub fn harmony_style_mut(&mut self) -> &mut HarmonyStyle {
        &mut self.render.harmony_style
    }

    /// Show measure numbers above the first measure of each system.
    #[must_use]
    pub fn show_measure_numbers(&self) -> bool {
        self.render.show_measure_numbers
    }

    /// Offset for measure numbering.
    #[must_use]
    pub fn measure_number_offset(&self) -> i32 {
        self.render.measure_number_offset
    }

    /// Number of count-in measures.
    #[must_use]
    pub fn count_in_measures(&self) -> u8 {
        self.render.count_in_measures
    }

    // =========================================================================
    // Delegation to BehavioralFlags
    // =========================================================================

    /// Hide repeated consecutive chord symbols.
    #[must_use]
    pub fn hide_repeated_chords(&self) -> bool {
        self.behavior.hide_repeated_chords
    }

    /// Use stemmed rhythm notation.
    #[must_use]
    pub fn use_stems(&self) -> bool {
        self.behavior.use_stems
    }

    /// Minimum horizontal gap between adjacent chord symbols (in points).
    #[must_use]
    pub fn min_chord_symbol_gap(&self) -> f64 {
        self.behavior.min_chord_symbol_gap
    }

    // =========================================================================
    // Builder-style setters
    // =========================================================================

    /// Set the harmony style.
    #[must_use]
    pub fn with_harmony_style(mut self, style: HarmonyStyle) -> Self {
        self.render.harmony_style = style;
        self
    }

    /// Set whether to hide repeated chords.
    #[must_use]
    pub fn with_hide_repeated_chords(mut self, hide: bool) -> Self {
        self.behavior.hide_repeated_chords = hide;
        self
    }

    /// Set whether to use stemmed notation.
    #[must_use]
    pub fn with_use_stems(mut self, use_stems: bool) -> Self {
        self.behavior.use_stems = use_stems;
        self
    }

    /// Set the minimum horizontal gap between adjacent chord symbols (in points).
    #[must_use]
    pub fn with_min_chord_symbol_gap(mut self, gap: f64) -> Self {
        self.behavior.min_chord_symbol_gap = gap;
        self
    }

    /// Set the measure number offset.
    #[must_use]
    pub fn with_measure_number_offset(mut self, offset: i32) -> Self {
        self.render.measure_number_offset = offset;
        self
    }

    /// Set the count-in measures.
    #[must_use]
    pub fn with_count_in_measures(mut self, count: u8) -> Self {
        self.render.count_in_measures = count;
        self
    }
}

// ============================================================================
// Backward compatibility: Direct field access via flattened struct
// ============================================================================

/// Flattened view of [`ChartLayoutConfig`] for backward compatibility.
///
/// This allows code that expects the old flat struct to continue working.
/// New code should use the grouped accessors instead.
impl ChartLayoutConfig {
    /// Convert to a flat struct for legacy code.
    #[must_use]
    pub fn to_flat(&self) -> FlatChartLayoutConfig {
        FlatChartLayoutConfig {
            margins: self.layout.margins,
            spatium: self.layout.spatium,
            system_spacing: self.layout.system_spacing,
            max_measures_per_system: self.layout.max_measures_per_system,
            min_measure_width: self.layout.min_measure_width,
            harmony_style: self.render.harmony_style.clone(),
            hide_repeated_chords: self.behavior.hide_repeated_chords,
            use_stems: self.behavior.use_stems,
            show_measure_numbers: self.render.show_measure_numbers,
            measure_number_offset: self.render.measure_number_offset,
            count_in_measures: self.render.count_in_measures,
            count_in_uses_negative_numbers: self.render.count_in_uses_negative_numbers,
        }
    }

    /// Create from a flat struct.
    #[must_use]
    pub fn from_flat(flat: FlatChartLayoutConfig) -> Self {
        Self {
            layout: LayoutParams {
                margins: flat.margins,
                spatium: flat.spatium,
                system_spacing: flat.system_spacing,
                max_measures_per_system: flat.max_measures_per_system,
                min_measure_width: flat.min_measure_width,
                ..LayoutParams::default()
            },
            render: RenderOptions {
                harmony_style: flat.harmony_style,
                show_measure_numbers: flat.show_measure_numbers,
                measure_number_offset: flat.measure_number_offset,
                count_in_measures: flat.count_in_measures,
                count_in_uses_negative_numbers: flat.count_in_uses_negative_numbers,
            },
            behavior: BehavioralFlags {
                hide_repeated_chords: flat.hide_repeated_chords,
                use_stems: flat.use_stems,
                auto_rhythm_slashes: true, // Default to true for legacy conversion
                min_chord_symbol_gap: behavioral::DEFAULT_MIN_CHORD_SYMBOL_GAP,
                push_alters_rhythm: true, // Default to true for legacy conversion
            },
        }
    }
}

/// Flat chart layout configuration (legacy format).
///
/// This is the original struct layout before the refactoring.
/// Use [`ChartLayoutConfig`] for new code.
#[derive(Debug, Clone)]
pub struct FlatChartLayoutConfig {
    /// Page margins.
    pub margins: PageMargins,
    /// Staff space (spatium) in points.
    pub spatium: f64,
    /// Spacing between systems.
    pub system_spacing: f64,
    /// Maximum measures per system.
    pub max_measures_per_system: usize,
    /// Minimum measure width.
    pub min_measure_width: f64,
    /// Harmony style for chord symbols.
    pub harmony_style: HarmonyStyle,
    /// Hide repeated consecutive chord symbols.
    pub hide_repeated_chords: bool,
    /// Use stemmed rhythm notation.
    pub use_stems: bool,
    /// Show measure numbers above the first measure of each system.
    pub show_measure_numbers: bool,
    /// Offset for measure numbering.
    pub measure_number_offset: i32,
    /// Number of count-in measures.
    pub count_in_measures: u8,
    /// Whether count-in measures number as -(N-1)..0 (true) or sequentially
    /// (false). See [`RenderOptions::count_in_uses_negative_numbers`].
    pub count_in_uses_negative_numbers: bool,
}

impl Default for FlatChartLayoutConfig {
    fn default() -> Self {
        ChartLayoutConfig::default().to_flat()
    }
}

#[cfg(test)]
mod tests {
    use super::super::constants;
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ChartLayoutConfig::default();

        assert_eq!(config.spatium(), constants::DEFAULT_SPATIUM);
        assert_eq!(config.system_spacing(), constants::DEFAULT_SYSTEM_SPACING);
        assert_eq!(
            config.max_measures_per_system(),
            constants::DEFAULT_MAX_MEASURES_PER_SYSTEM
        );
    }

    #[test]
    fn test_flat_roundtrip() {
        let config = ChartLayoutConfig::default()
            .with_hide_repeated_chords(false)
            .with_use_stems(true);

        let flat = config.to_flat();
        let restored = ChartLayoutConfig::from_flat(flat);

        assert!(!restored.hide_repeated_chords());
        assert!(restored.use_stems());
    }

    #[test]
    fn test_builder_methods() {
        let config = ChartLayoutConfig::default()
            .with_measure_number_offset(4)
            .with_count_in_measures(2);

        assert_eq!(config.measure_number_offset(), 4);
        assert_eq!(config.count_in_measures(), 2);
    }
}
