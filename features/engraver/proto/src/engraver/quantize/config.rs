//! Configuration for rhythm quantization.

use crate::engraver::notation::TimeSignature;

/// Configuration for MIDI-to-notation rhythm quantization.
#[derive(Debug, Clone)]
pub struct QuantizeConfig {
    /// PPQ resolution of the source MIDI (REAPER uses 960, standard is 480)
    pub source_ppq: i32,

    /// Target PPQ for output (engraver uses 480)
    pub target_ppq: i32,

    /// Time signature for context-aware quantization
    pub time_signature: TimeSignature,

    /// Whether to detect triplet rhythms (3:2)
    pub detect_triplets: bool,

    /// Whether to detect quintuplet rhythms (5:4)
    pub detect_quintuplets: bool,

    /// Whether to detect sextuplet rhythms (6:4)
    pub detect_sextuplets: bool,

    /// Whether to detect septuplet rhythms (7:4)
    pub detect_septuplets: bool,

    /// Tolerance in ticks (at source PPQ) for matching durations.
    /// A duration within this tolerance of a candidate will match.
    /// Default: ~5% of the smallest detectable duration.
    pub tolerance_ticks: i32,

    /// Minimum duration to consider (in source PPQ ticks).
    /// Durations shorter than this are treated as grace notes.
    pub min_duration_ticks: i32,

    /// Whether this is a compound meter (6/8, 9/8, 12/8).
    /// In compound meters, the natural subdivision is triplet-based,
    /// so "duplets" become the exception rather than triplets.
    pub compound_meter: bool,
}

impl Default for QuantizeConfig {
    fn default() -> Self {
        Self {
            source_ppq: 480,
            target_ppq: 480,
            time_signature: TimeSignature::COMMON,
            detect_triplets: true,
            detect_quintuplets: true,
            detect_sextuplets: true,
            detect_septuplets: true,
            tolerance_ticks: 12,    // ~2.5% of a quarter note at 480 PPQ
            min_duration_ticks: 30, // 64th note at 480 PPQ
            compound_meter: false,
        }
    }
}

impl QuantizeConfig {
    /// Create a new config with default settings.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create config for REAPER (960 PPQ source).
    #[must_use]
    pub fn reaper() -> Self {
        Self {
            source_ppq: 960,
            tolerance_ticks: 24,    // Scaled for 960 PPQ
            min_duration_ticks: 60, // 64th note at 960 PPQ
            ..Default::default()
        }
    }

    /// Create config for a compound meter (6/8, 9/8, 12/8).
    /// In compound meters, triplets are the natural subdivision.
    #[must_use]
    pub fn compound(numerator: u8, denominator: u8) -> Self {
        Self {
            time_signature: TimeSignature::new(numerator, denominator),
            compound_meter: true,
            // In compound meters, "triplets" are the norm, not exception
            detect_triplets: false,
            ..Default::default()
        }
    }

    /// Set the source PPQ resolution.
    #[must_use]
    pub fn with_source_ppq(mut self, ppq: i32) -> Self {
        self.source_ppq = ppq;
        // Scale tolerance proportionally
        self.tolerance_ticks = ppq * 12 / 480;
        self.min_duration_ticks = ppq * 30 / 480;
        self
    }

    /// Set the time signature.
    #[must_use]
    pub fn with_time_signature(mut self, ts: TimeSignature) -> Self {
        self.time_signature = ts;
        self.compound_meter = Self::is_compound_time_signature(&ts);
        self
    }

    /// Set the tolerance in ticks.
    #[must_use]
    pub fn with_tolerance(mut self, ticks: i32) -> Self {
        self.tolerance_ticks = ticks;
        self
    }

    /// Enable or disable triplet detection.
    #[must_use]
    pub fn with_triplet_detection(mut self, enabled: bool) -> Self {
        self.detect_triplets = enabled;
        self
    }

    /// Enable all tuplet types.
    #[must_use]
    pub fn with_all_tuplets(mut self) -> Self {
        self.detect_triplets = true;
        self.detect_quintuplets = true;
        self.detect_sextuplets = true;
        self.detect_septuplets = true;
        self
    }

    /// Disable all tuplet detection (only standard durations).
    #[must_use]
    pub fn without_tuplets(mut self) -> Self {
        self.detect_triplets = false;
        self.detect_quintuplets = false;
        self.detect_sextuplets = false;
        self.detect_septuplets = false;
        self
    }

    /// Check if a time signature is compound (natural triplet subdivision).
    #[must_use]
    pub fn is_compound_time_signature(ts: &TimeSignature) -> bool {
        // Compound meters: 6/8, 9/8, 12/8, 6/4, etc.
        // The numerator is divisible by 3 and greater than 3
        ts.numerator.is_multiple_of(3) && ts.numerator > 3
    }

    /// Scale a tick value from source PPQ to target PPQ.
    #[must_use]
    pub fn scale_ticks(&self, ticks: i32) -> i32 {
        if self.source_ppq == self.target_ppq {
            ticks
        } else {
            (ticks as i64 * self.target_ppq as i64 / self.source_ppq as i64) as i32
        }
    }

    /// Get the tolerance scaled to target PPQ.
    #[must_use]
    pub fn scaled_tolerance(&self) -> i32 {
        self.scale_ticks(self.tolerance_ticks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = QuantizeConfig::default();
        assert_eq!(config.source_ppq, 480);
        assert_eq!(config.target_ppq, 480);
        assert!(config.detect_triplets);
        assert!(!config.compound_meter);
    }

    #[test]
    fn test_reaper_config() {
        let config = QuantizeConfig::reaper();
        assert_eq!(config.source_ppq, 960);
        assert_eq!(config.tolerance_ticks, 24);
    }

    #[test]
    fn test_compound_meter_detection() {
        assert!(QuantizeConfig::is_compound_time_signature(
            &TimeSignature::new(6, 8)
        ));
        assert!(QuantizeConfig::is_compound_time_signature(
            &TimeSignature::new(9, 8)
        ));
        assert!(QuantizeConfig::is_compound_time_signature(
            &TimeSignature::new(12, 8)
        ));
        assert!(!QuantizeConfig::is_compound_time_signature(
            &TimeSignature::new(4, 4)
        ));
        assert!(!QuantizeConfig::is_compound_time_signature(
            &TimeSignature::new(3, 4)
        ));
    }

    #[test]
    fn test_scale_ticks() {
        let config = QuantizeConfig::reaper();
        // 960 PPQ -> 480 PPQ
        assert_eq!(config.scale_ticks(960), 480);
        assert_eq!(config.scale_ticks(480), 240);
    }
}
