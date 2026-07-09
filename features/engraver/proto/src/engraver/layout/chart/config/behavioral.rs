//! Behavioral flags for chart layout.
//!
//! Controls behavioral aspects of the layout engine, such as whether to
//! hide repeated chords or use stemmed notation.

/// Behavioral flags for chart layout.
///
/// These flags control the behavior of the layout engine rather than
/// visual styling or physical layout dimensions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BehavioralFlags {
    /// Hide repeated consecutive chord symbols.
    ///
    /// When true, if a chord is the same as the previous chord, it won't
    /// be displayed. This reduces visual clutter in charts with many
    /// repeated chords (e.g., jazz charts with long vamps).
    pub hide_repeated_chords: bool,

    /// Use stemmed rhythm notation.
    ///
    /// When false (default), uses stemless slash notation for charts.
    /// When true, uses stemmed rhythmic notation with beams and
    /// triplet brackets. This is useful for charts with complex
    /// push/pull timing that needs to be precisely notated.
    pub use_stems: bool,

    /// Automatically fill whole/half notes with quarter note slashes.
    ///
    /// When true (default), chords with whole note or half note durations
    /// are automatically expanded to quarter note slashes:
    /// - A whole note chord (4 beats) becomes 4 quarter slashes
    /// - A half note chord (2 beats) becomes 2 quarter slashes
    ///
    /// This is the standard notation for master rhythm charts, making them
    /// easier to read. Can be disabled via `/AUTO_RHYTHM_SLASHES=false` for
    /// specific sections where sustained chords (diamonds) are desired.
    pub auto_rhythm_slashes: bool,

    /// Minimum horizontal gap between adjacent chord symbols (in points).
    ///
    /// When chord symbols would overlap or be closer than this gap,
    /// they are automatically pushed apart:
    /// - The first chord shifts slightly left
    /// - The second chord shifts slightly right
    ///
    /// Default is 4.0 points (approximately 0.5-1.0 spatiums), which provides
    /// enough separation for readability without excessive spacing.
    pub min_chord_symbol_gap: f64,

    /// Whether push/pull notation alters the rhythm display.
    ///
    /// When true (default), pushed/pulled chords are shown with triplet or
    /// syncopated rhythm notation, accurately reflecting when the chord is
    /// played relative to the beat.
    ///
    /// When false, pushed/pulled chords are displayed on the beat with an
    /// apostrophe marker before/after the chord symbol to indicate timing:
    /// - `'C` = pushed (anticipate, play earlier)
    /// - `C'` = pulled (delayed, play later)
    ///
    /// The apostrophe is rendered in a contrasting color (red) for visibility.
    /// This mode is simpler to read but less rhythmically precise.
    ///
    /// Can be configured via `/PUSH_ALTERS_RHYTHM=false` in the chart.
    pub push_alters_rhythm: bool,
}

/// Default minimum gap between chord symbols (in points).
/// A small positive value (3.0) enables post-render collision detection,
/// which pushes overlapping chord symbols apart by the minimum needed amount.
/// This prevents chord symbol text from overlapping while minimizing
/// chord/notation misalignment.
pub const DEFAULT_MIN_CHORD_SYMBOL_GAP: f64 = 3.0;

impl Default for BehavioralFlags {
    fn default() -> Self {
        Self {
            hide_repeated_chords: true,
            use_stems: false,
            auto_rhythm_slashes: true, // ON by default for master rhythm charts
            min_chord_symbol_gap: DEFAULT_MIN_CHORD_SYMBOL_GAP,
            push_alters_rhythm: true, // ON by default for accurate rhythm notation
        }
    }
}

impl BehavioralFlags {
    /// Create behavioral flags with default values.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create flags for simple chart rendering.
    ///
    /// Hides repeated chords and uses stemless notation.
    #[must_use]
    pub fn simple() -> Self {
        Self {
            hide_repeated_chords: true,
            use_stems: false,
            auto_rhythm_slashes: true,
            min_chord_symbol_gap: DEFAULT_MIN_CHORD_SYMBOL_GAP,
            push_alters_rhythm: true,
        }
    }

    /// Create flags for detailed chart rendering.
    ///
    /// Shows all chords and uses stemmed notation.
    #[must_use]
    pub fn detailed() -> Self {
        Self {
            hide_repeated_chords: false,
            use_stems: true,
            auto_rhythm_slashes: true,
            min_chord_symbol_gap: DEFAULT_MIN_CHORD_SYMBOL_GAP,
            push_alters_rhythm: true,
        }
    }

    /// Create flags for verbose display (show everything).
    #[must_use]
    pub fn verbose() -> Self {
        Self {
            hide_repeated_chords: false,
            use_stems: false,
            auto_rhythm_slashes: true,
            min_chord_symbol_gap: DEFAULT_MIN_CHORD_SYMBOL_GAP,
            push_alters_rhythm: true,
        }
    }

    // =========================================================================
    // Builder-style setters
    // =========================================================================

    /// Set whether to hide repeated chords.
    #[must_use]
    pub const fn with_hide_repeated_chords(mut self, hide: bool) -> Self {
        self.hide_repeated_chords = hide;
        self
    }

    /// Set whether to use stemmed notation.
    #[must_use]
    pub const fn with_use_stems(mut self, use_stems: bool) -> Self {
        self.use_stems = use_stems;
        self
    }

    /// Set whether to automatically fill whole/half notes with quarter slashes.
    #[must_use]
    pub const fn with_auto_rhythm_slashes(mut self, auto_slashes: bool) -> Self {
        self.auto_rhythm_slashes = auto_slashes;
        self
    }

    /// Set the minimum horizontal gap between adjacent chord symbols (in points).
    #[must_use]
    pub fn with_min_chord_symbol_gap(mut self, gap: f64) -> Self {
        self.min_chord_symbol_gap = gap;
        self
    }

    /// Set whether push/pull notation alters rhythm display.
    ///
    /// When true (default), pushes create triplet/syncopated notation.
    /// When false, pushes show apostrophe markers on chord symbols.
    #[must_use]
    pub const fn with_push_alters_rhythm(mut self, alters: bool) -> Self {
        self.push_alters_rhythm = alters;
        self
    }

    // =========================================================================
    // Queries
    // =========================================================================

    /// Check if a chord should be rendered given the previous chord.
    ///
    /// Returns `true` if the chord should be displayed, `false` if it
    /// should be hidden due to being a repeat.
    #[must_use]
    pub fn should_render_chord(&self, current: &str, previous: Option<&str>) -> bool {
        if !self.hide_repeated_chords {
            return true;
        }

        match previous {
            Some(prev) => current != prev,
            None => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_flags() {
        let flags = BehavioralFlags::default();

        assert!(flags.hide_repeated_chords);
        assert!(!flags.use_stems);
    }

    #[test]
    fn test_simple_preset() {
        let flags = BehavioralFlags::simple();

        assert!(flags.hide_repeated_chords);
        assert!(!flags.use_stems);
    }

    #[test]
    fn test_detailed_preset() {
        let flags = BehavioralFlags::detailed();

        assert!(!flags.hide_repeated_chords);
        assert!(flags.use_stems);
    }

    #[test]
    fn test_should_render_chord() {
        let flags = BehavioralFlags::default();

        // First chord should always render
        assert!(flags.should_render_chord("Cmaj7", None));

        // Different chord should render
        assert!(flags.should_render_chord("Dm7", Some("Cmaj7")));

        // Same chord should not render (hidden)
        assert!(!flags.should_render_chord("Cmaj7", Some("Cmaj7")));

        // With hide_repeated_chords = false, same chord should render
        let verbose = BehavioralFlags::verbose();
        assert!(verbose.should_render_chord("Cmaj7", Some("Cmaj7")));
    }

    #[test]
    fn test_builder() {
        let flags = BehavioralFlags::new()
            .with_hide_repeated_chords(false)
            .with_use_stems(true);

        assert!(!flags.hide_repeated_chords);
        assert!(flags.use_stems);
    }
}
