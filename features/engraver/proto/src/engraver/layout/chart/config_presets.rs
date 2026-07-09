//! Preset constructors + small builder methods for [`ChartLayoutConfig`].
//!
//! These live outside `mod.rs` purely to keep that file readable; nothing
//! here changes behaviour. The struct definition still lives next to the
//! engine in `mod.rs` so the field list and its callers stay close.

use crate::engraver::layout::orchestrator::PageMargins;
use crate::engraver::layout::tlayout::HarmonyStyle;

use super::spacing;
use super::{BeamGroupingMode, Breakpoint, ChartLayoutConfig, DEFAULT_MIN_CHORD_SYMBOL_GAP};

impl Default for ChartLayoutConfig {
    fn default() -> Self {
        // Default is the master rhythm preset
        Self::master_rhythm()
    }
}

impl ChartLayoutConfig {
    /// Master Rhythm Chart preset.
    ///
    /// Optimized for professional master rhythm charts with:
    /// - A4-friendly margins (extra left margin for section labels)
    /// - Compact system spacing (~10 systems per page)
    /// - Stemless quarter slashes for sustained chords (auto_rhythm_slashes)
    /// - MuseJazz harmony style
    /// - 4 measures per system
    ///
    /// This is the standard format for rhythm section charts used in
    /// professional music production and live performance.
    #[must_use]
    pub fn master_rhythm() -> Self {
        Self {
            margins: PageMargins {
                top: 36.0,    // Reduced top for header closer to top
                right: 50.0,  // Standard right margin
                bottom: 50.0, // Standard bottom margin
                left: 72.0,   // Extra left margin for section labels
            },
            spatium: 5.0,
            // System spacing: 40pt (8 spatiums). Doubled from 20pt to leave a
            // minimum vertical gap between systems for text annotations
            // (staff text, figures, dynamics) without them crowding the
            // staff above/below.
            system_spacing: 40.0,
            max_measures_per_system: 4,
            min_measure_width: 100.0,
            harmony_style: HarmonyStyle::musejazz(),
            hide_repeated_chords: true,
            use_stems: true,
            auto_rhythm_slashes: true,
            show_measure_numbers: true,
            measure_number_offset: 0,
            count_in_measures: 0,
            count_in_uses_negative_numbers: false,
            snippet_mode: false,
            min_chord_symbol_gap: DEFAULT_MIN_CHORD_SYMBOL_GAP,
            push_alters_rhythm: true,
            use_page_offsets: true,
            spacing_slope: spacing::DEFAULT_SPACING_SLOPE,
            spacing_density: spacing::DEFAULT_SPACING_DENSITY,
            last_system_fill_limit: spacing::DEFAULT_LAST_SYSTEM_FILL_LIMIT,
            draw_melody_barline_ties: true,
            beam_grouping: BeamGroupingMode::Standard,
        }
    }

    /// Snippet preset for small embedded chart excerpts.
    ///
    /// Optimized for displaying chart snippets in documentation, tutorials,
    /// or inline displays with minimal margins.
    #[must_use]
    pub fn snippet() -> Self {
        Self {
            margins: PageMargins {
                top: 20.0,
                bottom: 20.0,
                left: 20.0,
                right: 20.0,
            },
            spatium: 5.0,
            system_spacing: 30.0,
            max_measures_per_system: 4,
            min_measure_width: 80.0,
            harmony_style: HarmonyStyle::musejazz(),
            hide_repeated_chords: false,
            use_stems: true,
            auto_rhythm_slashes: true,
            show_measure_numbers: true,
            measure_number_offset: 0,
            count_in_measures: 0,
            count_in_uses_negative_numbers: false,
            snippet_mode: true,
            min_chord_symbol_gap: DEFAULT_MIN_CHORD_SYMBOL_GAP,
            push_alters_rhythm: true,
            use_page_offsets: true,
            spacing_slope: spacing::DEFAULT_SPACING_SLOPE,
            spacing_density: spacing::DEFAULT_SPACING_DENSITY,
            last_system_fill_limit: spacing::DEFAULT_LAST_SYSTEM_FILL_LIMIT,
            draw_melody_barline_ties: true,
            beam_grouping: BeamGroupingMode::Standard,
        }
    }

    /// iReal Pro-inspired preset for a specific viewport size class.
    ///
    /// Varies staff size and layout density per device class — a phone gets
    /// a few large measures rather than many tiny ones. All breakpoints share
    /// iReal Pro's reading style: no stems, no auto rhythm slashes, explicit
    /// repeated chords, no melody barline ties, jazz half-bar beam grouping.
    #[must_use]
    pub fn responsive_for(breakpoint: Breakpoint) -> Self {
        // root_size derived from spatium so chord symbols and staff scale
        // together. Desktop ratio (24/7 ≈ 3.43) is the iReal Pro baseline.
        //
        // left_margin > base_margin because section-label capsules (VS 1,
        // CH 1, etc.) render in the left margin. Without enough room they
        // get cut off. Right margin stays compact so the chart fills the
        // viewport.
        //
        // top_margin must clear the first row's chord symbols, which sit
        // above the staff: root_size + breathing room. Otherwise the
        // chords for measure 1 render above y=0 and get clipped.
        let (spatium, margin, left_margin, system_spacing, min_measure_width, min_gap, root_size) =
            match breakpoint {
                Breakpoint::Phone => (12.0, 12.0, 56.0, 64.0, 110.0, 12.0, 40.0),
                Breakpoint::Tablet => (9.0, 16.0, 60.0, 48.0, 88.0, 10.0, 30.0),
                Breakpoint::Desktop => (7.0, 18.0, 64.0, 38.0, 72.0, 8.0, 24.0),
            };

        let top_margin: f64 = (root_size + 14.0_f64).max(margin);

        Self {
            margins: PageMargins {
                top: top_margin,
                bottom: margin * 1.3,
                left: left_margin,
                right: margin,
            },
            spatium,
            system_spacing,
            max_measures_per_system: breakpoint.measures_per_system(),
            min_measure_width,
            harmony_style: HarmonyStyle::ireal_pro_screen().with_root_size(root_size),
            hide_repeated_chords: false,
            use_stems: false,
            auto_rhythm_slashes: false,
            show_measure_numbers: false,
            measure_number_offset: 0,
            count_in_measures: 0,
            count_in_uses_negative_numbers: false,
            snippet_mode: true,
            min_chord_symbol_gap: min_gap,
            push_alters_rhythm: false,
            use_page_offsets: false,
            spacing_slope: 1.0,
            spacing_density: 1.35,
            last_system_fill_limit: 0.85,
            draw_melody_barline_ties: false,
            beam_grouping: BeamGroupingMode::JazzHalfBar,
        }
    }

    /// iReal Pro-inspired responsive screen preset with fixed sizing.
    ///
    /// For phones/tablets and rehearsal screens: no paper shadow, larger
    /// chord symbols, compact margins, four measures per system, repeated
    /// chord names shown explicitly for fast reading.
    ///
    /// For viewport-aware sizing, prefer [`Self::responsive_for`].
    #[must_use]
    pub fn ireal_pro_responsive() -> Self {
        Self {
            margins: PageMargins {
                top: 18.0,
                bottom: 24.0,
                left: 18.0,
                right: 18.0,
            },
            spatium: 7.0,
            system_spacing: 34.0,
            max_measures_per_system: 4,
            min_measure_width: 72.0,
            harmony_style: HarmonyStyle::ireal_pro_screen(),
            hide_repeated_chords: false,
            use_stems: false,
            auto_rhythm_slashes: false,
            show_measure_numbers: false,
            measure_number_offset: 0,
            count_in_measures: 0,
            count_in_uses_negative_numbers: false,
            snippet_mode: true,
            min_chord_symbol_gap: 8.0,
            push_alters_rhythm: false,
            use_page_offsets: false,
            spacing_slope: 1.0,
            spacing_density: 1.35,
            last_system_fill_limit: 0.85,
            draw_melody_barline_ties: false,
            beam_grouping: BeamGroupingMode::JazzHalfBar,
        }
    }

    /// Apply settings from a parsed chart (`/AUTO_RHYTHM_SLASHES=…`,
    /// `/PUSH_ALTERS_RHYTHM=…`, …).
    #[must_use]
    pub fn with_chart_settings(mut self, settings: &crate::chart::ChartSettings) -> Self {
        self.auto_rhythm_slashes = settings.auto_rhythm_slashes();
        self.push_alters_rhythm = settings.push_alters_rhythm();
        self
    }

    /// Disable page offsets for single-page export.
    ///
    /// When disabled, the page renders at (0, 0) instead of (20, 20),
    /// suitable for PDF export or edge-to-edge bitmap output.
    #[must_use]
    pub fn for_export(mut self) -> Self {
        self.use_page_offsets = false;
        self
    }

    /// Set whether to use page offsets for multi-page viewing.
    #[must_use]
    pub fn with_page_offsets(mut self, use_offsets: bool) -> Self {
        self.use_page_offsets = use_offsets;
        self
    }

    /// Apply a uniform scale factor to spatium, spacing, gaps, and margins.
    ///
    /// `factor` is clamped to `[0.1, 10.0]` and ignored if non-finite. Use
    /// for DPI scaling or user-controlled zoom — every geometric field
    /// scales together so notes / spacing / margins stay coherent.
    #[must_use]
    pub fn with_scale(mut self, factor: f64) -> Self {
        if !factor.is_finite() {
            return self;
        }
        let f = factor.clamp(0.1, 10.0);
        self.spatium *= f;
        self.system_spacing *= f;
        self.min_measure_width *= f;
        self.min_chord_symbol_gap *= f;
        self.margins.top *= f;
        self.margins.bottom *= f;
        self.margins.left *= f;
        self.margins.right *= f;
        self
    }
}
