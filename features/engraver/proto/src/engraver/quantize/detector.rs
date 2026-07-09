//! Duration detection and quantization from MIDI ticks.

use crate::engraver::model::DurationKind;
use crate::engraver::notation::{Duration, TupletRatio};

use super::config::QuantizeConfig;

/// Types of tuplets that can be detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TupletType {
    /// 3 notes in the space of 2 (triplet)
    Triplet,
    /// 5 notes in the space of 4 (quintuplet)
    Quintuplet,
    /// 6 notes in the space of 4 (sextuplet)
    Sextuplet,
    /// 7 notes in the space of 8 (septuplet) - matches MuseScore
    Septuplet,
}

impl TupletType {
    /// Convert to TupletRatio.
    #[must_use]
    pub fn to_ratio(self) -> TupletRatio {
        match self {
            Self::Triplet => TupletRatio::triplet(),
            Self::Quintuplet => TupletRatio::quintuplet(),
            Self::Sextuplet => TupletRatio::sextuplet(),
            Self::Septuplet => TupletRatio::septuplet(),
        }
    }

    /// Get the expected note count for a complete tuplet group.
    #[must_use]
    pub fn expected_count(self) -> usize {
        match self {
            Self::Triplet => 3,
            Self::Quintuplet => 5,
            Self::Sextuplet => 6,
            Self::Septuplet => 7,
        }
    }
}

/// Result of quantizing a single duration.
#[derive(Debug, Clone)]
pub struct QuantizedDuration {
    /// The quantized notation duration
    pub duration: Duration,

    /// The tick value at the target PPQ (480)
    pub ticks: i32,

    /// Confidence score (0.0 - 1.0)
    /// 1.0 = exact match, lower values indicate rounding was applied
    pub confidence: f64,

    /// Whether this is part of a tuplet
    pub is_tuplet: bool,

    /// The tuplet type if applicable
    pub tuplet_type: Option<TupletType>,

    /// Original tick value (at source PPQ)
    pub original_ticks: i32,

    /// Deviation from the ideal duration in target PPQ ticks
    pub deviation_ticks: i32,
}

impl QuantizedDuration {
    /// Convert to a notation Duration.
    #[must_use]
    pub fn to_duration(&self) -> Duration {
        self.duration
    }
}

/// A candidate duration for matching.
#[derive(Debug, Clone)]
struct DurationCandidate {
    duration: Duration,
    ticks: i32,
    tuplet_type: Option<TupletType>,
}

/// Quantize a single MIDI tick duration to notation.
///
/// # Arguments
/// * `ticks` - Duration in source PPQ ticks
/// * `config` - Quantization configuration
///
/// # Returns
/// A `QuantizedDuration` with the best-matching notation duration.
#[must_use]
pub fn quantize_duration(ticks: i32, config: &QuantizeConfig) -> QuantizedDuration {
    // Scale to target PPQ if needed
    let scaled_ticks = config.scale_ticks(ticks);

    // Generate all candidate durations
    let candidates = generate_duration_candidates(config);

    // Find best match
    let (best_match, deviation) = find_best_match(scaled_ticks, &candidates, config);

    // Calculate confidence based on deviation
    let confidence = calculate_confidence(deviation, scaled_ticks, config);

    QuantizedDuration {
        duration: best_match.duration,
        ticks: best_match.ticks,
        confidence,
        is_tuplet: best_match.tuplet_type.is_some(),
        tuplet_type: best_match.tuplet_type,
        original_ticks: ticks,
        deviation_ticks: deviation,
    }
}

/// Batch quantize multiple durations with context awareness.
///
/// Uses surrounding notes to improve tuplet detection accuracy.
/// For example, if two notes look like triplets and a third is ambiguous,
/// it will be classified as a triplet to complete the group.
///
/// # Arguments
/// * `tick_durations` - Durations in source PPQ ticks
/// * `start_positions` - Start positions of each note (for beat alignment analysis)
/// * `config` - Quantization configuration
#[must_use]
pub fn quantize_duration_batch(
    tick_durations: &[i32],
    start_positions: &[i32],
    config: &QuantizeConfig,
) -> Vec<QuantizedDuration> {
    // First pass: individual quantization
    let mut results: Vec<QuantizedDuration> = tick_durations
        .iter()
        .map(|&t| quantize_duration(t, config))
        .collect();

    // Second pass: context-aware refinement
    refine_with_context(&mut results, start_positions, config);

    results
}

/// Generate all candidate durations for matching.
fn generate_duration_candidates(config: &QuantizeConfig) -> Vec<DurationCandidate> {
    let ppq = config.target_ppq;
    let mut candidates = Vec::new();

    // Standard durations at target PPQ
    let standard_durations = [
        (DurationKind::Whole, ppq * 4),        // 1920 at 480 PPQ
        (DurationKind::Half, ppq * 2),         // 960
        (DurationKind::Quarter, ppq),          // 480
        (DurationKind::Eighth, ppq / 2),       // 240
        (DurationKind::Sixteenth, ppq / 4),    // 120
        (DurationKind::ThirtySecond, ppq / 8), // 60
        (DurationKind::SixtyFourth, ppq / 16), // 30
    ];

    // Add standard durations (no dots, no tuplet)
    for (kind, ticks) in &standard_durations {
        candidates.push(DurationCandidate {
            duration: Duration::new(*kind),
            ticks: *ticks,
            tuplet_type: None,
        });

        // Dotted versions (1.5x)
        candidates.push(DurationCandidate {
            duration: Duration::dotted(*kind),
            ticks: ticks * 3 / 2,
            tuplet_type: None,
        });

        // Double-dotted versions (1.75x)
        candidates.push(DurationCandidate {
            duration: Duration::double_dotted(*kind),
            ticks: ticks * 7 / 4,
            tuplet_type: None,
        });
    }

    // Triplet durations (2/3 of normal) - only if not compound meter
    if config.detect_triplets && !config.compound_meter {
        for (kind, base_ticks) in &standard_durations {
            // Skip very small triplets (64th note triplets are rarely used)
            if *base_ticks < ppq / 8 {
                continue;
            }
            let triplet_ticks = base_ticks * 2 / 3;
            candidates.push(DurationCandidate {
                duration: Duration::triplet(*kind),
                ticks: triplet_ticks,
                tuplet_type: Some(TupletType::Triplet),
            });
        }
    }

    // Quintuplet durations (4/5 of normal)
    if config.detect_quintuplets {
        for (kind, base_ticks) in &standard_durations {
            if *base_ticks < ppq / 4 {
                continue;
            }
            let quintuplet_ticks = base_ticks * 4 / 5;
            candidates.push(DurationCandidate {
                duration: Duration::quintuplet(*kind),
                ticks: quintuplet_ticks,
                tuplet_type: Some(TupletType::Quintuplet),
            });
        }
    }

    // Sextuplet durations (4/6 = 2/3 of normal, same as triplet but grouped differently)
    if config.detect_sextuplets {
        for (kind, base_ticks) in &standard_durations {
            if *base_ticks < ppq / 4 {
                continue;
            }
            let sextuplet_ticks = base_ticks * 4 / 6;
            candidates.push(DurationCandidate {
                duration: Duration::sextuplet(*kind),
                ticks: sextuplet_ticks,
                tuplet_type: Some(TupletType::Sextuplet),
            });
        }
    }

    // Septuplet durations (8/7 of normal) - 7 notes in the space of 8 (matches MuseScore)
    if config.detect_septuplets {
        for (kind, base_ticks) in &standard_durations {
            if *base_ticks < ppq / 4 {
                continue;
            }
            let septuplet_ticks = base_ticks * 8 / 7;
            candidates.push(DurationCandidate {
                duration: Duration::septuplet(*kind),
                ticks: septuplet_ticks,
                tuplet_type: Some(TupletType::Septuplet),
            });
        }
    }

    candidates
}

/// Find the best matching candidate for a given tick duration.
fn find_best_match(
    ticks: i32,
    candidates: &[DurationCandidate],
    config: &QuantizeConfig,
) -> (DurationCandidate, i32) {
    let tolerance = config.scaled_tolerance();

    // Find candidates within tolerance
    let within_tolerance: Vec<_> = candidates
        .iter()
        .map(|c| {
            let deviation = (ticks - c.ticks).abs();
            (c, deviation)
        })
        .filter(|(_, dev)| *dev <= tolerance)
        .collect();

    if !within_tolerance.is_empty() {
        // Prefer non-tuplet if both are within tolerance and similar deviation
        let best = within_tolerance
            .iter()
            .min_by(|(a, dev_a), (b, dev_b)| {
                // If deviations are similar, prefer standard durations
                if (*dev_a - *dev_b).abs() <= tolerance / 4 {
                    match (a.tuplet_type.is_some(), b.tuplet_type.is_some()) {
                        (false, true) => std::cmp::Ordering::Less,
                        (true, false) => std::cmp::Ordering::Greater,
                        _ => dev_a.cmp(dev_b),
                    }
                } else {
                    dev_a.cmp(dev_b)
                }
            })
            .unwrap();

        return (best.0.clone(), best.1);
    }

    // Fallback: find absolute closest
    candidates
        .iter()
        .map(|c| (c.clone(), (ticks - c.ticks).abs()))
        .min_by_key(|(_, dev)| *dev)
        .unwrap_or_else(|| {
            // Ultimate fallback to quarter note
            (
                DurationCandidate {
                    duration: Duration::Quarter,
                    ticks: config.target_ppq,
                    tuplet_type: None,
                },
                (ticks - config.target_ppq).abs(),
            )
        })
}

/// Calculate confidence based on deviation.
fn calculate_confidence(deviation: i32, original_ticks: i32, config: &QuantizeConfig) -> f64 {
    if deviation == 0 {
        return 1.0;
    }

    let tolerance = config.scaled_tolerance();
    let relative_deviation = deviation as f64 / original_ticks.max(1) as f64;

    if deviation <= tolerance {
        // Within tolerance: high confidence
        1.0 - (relative_deviation * 0.5)
    } else {
        // Outside tolerance: lower confidence
        (0.5 - relative_deviation.min(0.5)).max(0.1)
    }
}

/// Refine quantization results using surrounding context.
fn refine_with_context(
    results: &mut [QuantizedDuration],
    _start_positions: &[i32],
    config: &QuantizeConfig,
) {
    // Look for tuplet patterns and strengthen their classification
    for tuplet_type in [
        TupletType::Triplet,
        TupletType::Quintuplet,
        TupletType::Sextuplet,
        TupletType::Septuplet,
    ] {
        let window_size = tuplet_type.expected_count();

        if results.len() < window_size {
            continue;
        }

        for i in 0..=results.len() - window_size {
            let window = &results[i..i + window_size];

            // Check if this window looks like a complete tuplet group
            if looks_like_tuplet_group(window, tuplet_type, config) {
                // Strengthen tuplet classification for all notes in the group
                for item in results.iter_mut().skip(i).take(window_size) {
                    if item.tuplet_type == Some(tuplet_type) {
                        item.confidence = (item.confidence + 0.15).min(1.0);
                    }
                }
            }
        }
    }
}

/// Heuristic: does this group look like a complete tuplet pattern?
fn looks_like_tuplet_group(
    durations: &[QuantizedDuration],
    tuplet_type: TupletType,
    config: &QuantizeConfig,
) -> bool {
    // All notes should be of the same tuplet type
    let all_same_type = durations.iter().all(|d| d.tuplet_type == Some(tuplet_type));

    if !all_same_type {
        return false;
    }

    // Sum should be close to a beat boundary
    let total_ticks: i32 = durations.iter().map(|d| d.ticks).sum();
    let beat_ticks = config.target_ppq; // One quarter note beat

    // Check if total is close to a beat (or multiple beats)
    let remainder = total_ticks % beat_ticks;
    let tolerance = config.scaled_tolerance();

    remainder <= tolerance || (beat_ticks - remainder) <= tolerance
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quantize_quarter_note() {
        let config = QuantizeConfig::default();
        let result = quantize_duration(480, &config);

        assert_eq!(result.ticks, 480);
        assert_eq!(result.duration, Duration::Quarter);
        assert!(!result.is_tuplet);
        assert_eq!(result.confidence, 1.0);
    }

    #[test]
    fn test_quantize_eighth_note() {
        let config = QuantizeConfig::default();
        let result = quantize_duration(240, &config);

        assert_eq!(result.ticks, 240);
        assert_eq!(result.duration, Duration::Eighth);
        assert!(!result.is_tuplet);
    }

    #[test]
    fn test_quantize_triplet_eighth() {
        let config = QuantizeConfig::default();
        // Triplet eighth = 480 * 2 / 3 = 320 at the quarter level
        // But triplet eighth is 240 * 2 / 3 = 160
        let result = quantize_duration(160, &config);

        assert_eq!(result.ticks, 160);
        assert!(result.is_tuplet);
        assert_eq!(result.tuplet_type, Some(TupletType::Triplet));
    }

    #[test]
    fn test_quantize_triplet_quarter() {
        let config = QuantizeConfig::default();
        // Triplet quarter = 480 * 2 / 3 = 320
        let result = quantize_duration(320, &config);

        assert_eq!(result.ticks, 320);
        assert!(result.is_tuplet);
        assert_eq!(result.tuplet_type, Some(TupletType::Triplet));
    }

    #[test]
    fn test_quantize_dotted_quarter() {
        let config = QuantizeConfig::default();
        // Dotted quarter = 480 * 1.5 = 720
        let result = quantize_duration(720, &config);

        assert_eq!(result.ticks, 720);
        assert_eq!(result.duration, Duration::DottedQuarter);
        assert!(!result.is_tuplet);
    }

    #[test]
    fn test_quantize_with_tolerance() {
        let config = QuantizeConfig::default();
        // Slightly off from 480 (within tolerance)
        let result = quantize_duration(485, &config);

        assert_eq!(result.ticks, 480);
        assert_eq!(result.duration, Duration::Quarter);
        assert!(result.confidence > 0.9);
    }

    #[test]
    fn test_quantize_reaper_ppq() {
        let config = QuantizeConfig::reaper();
        // Quarter note at 960 PPQ
        let result = quantize_duration(960, &config);

        assert_eq!(result.ticks, 480); // Scaled to 480 PPQ
        assert_eq!(result.duration, Duration::Quarter);
    }

    #[test]
    fn test_quantize_quintuplet() {
        let config = QuantizeConfig::default().with_all_tuplets();
        // Quintuplet eighth = 240 * 4 / 5 = 192
        let result = quantize_duration(192, &config);

        assert_eq!(result.ticks, 192);
        assert!(result.is_tuplet);
        assert_eq!(result.tuplet_type, Some(TupletType::Quintuplet));
    }

    #[test]
    fn test_batch_quantize() {
        let config = QuantizeConfig::default();
        let durations = vec![160, 160, 160]; // Three triplet eighths
        let positions = vec![0, 160, 320];

        let results = quantize_duration_batch(&durations, &positions, &config);

        assert_eq!(results.len(), 3);
        for result in &results {
            assert!(result.is_tuplet);
            assert_eq!(result.tuplet_type, Some(TupletType::Triplet));
        }
    }

    #[test]
    fn test_quantize_triplet_sixteenth_reaper_ppq() {
        // REAPER uses 960 PPQ, engraver uses 480 PPQ
        // 160 ticks at 960 PPQ = 80 ticks at 480 PPQ = triplet sixteenth
        // (Sixteenth = 120 ticks, triplet sixteenth = 120 * 2/3 = 80 ticks)
        let config = QuantizeConfig::reaper();
        let result = quantize_duration(160, &config);

        // Should scale to 80 ticks (triplet sixteenth at 480 PPQ)
        assert_eq!(result.ticks, 80);
        assert!(result.is_tuplet);
        assert_eq!(result.tuplet_type, Some(TupletType::Triplet));
    }

    #[test]
    fn test_quantize_thriller_durations() {
        // Test actual durations from "Thriller - Dirty Loops" MIDI file
        // 960 PPQ, notes at 160 ticks duration = triplet sixteenths
        let config = QuantizeConfig::reaper();

        // Multiple notes at the same duration (chord voicings)
        let durations = vec![160, 160, 160, 160, 160]; // 5-note chord
        let positions = vec![11840, 11840, 11840, 11840, 11840];

        let results = quantize_duration_batch(&durations, &positions, &config);

        for result in &results {
            assert_eq!(result.ticks, 80);
            assert!(result.is_tuplet);
            assert_eq!(result.tuplet_type, Some(TupletType::Triplet));
        }
    }
}
