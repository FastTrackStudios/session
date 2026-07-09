//! Automatic detection and grouping of tuplets.

use crate::engraver::model::DurationKind;
use crate::engraver::notation::TupletRatio;

use super::config::QuantizeConfig;
use super::detector::{QuantizedDuration, TupletType};

/// A detected tuplet group.
#[derive(Debug, Clone)]
pub struct TupletGroup {
    /// Start index in the note array (inclusive)
    pub start_idx: usize,
    /// End index (exclusive)
    pub end_idx: usize,
    /// The tuplet ratio (e.g., 3:2 for triplet)
    pub ratio: TupletRatio,
    /// The base duration level (e.g., Eighth for eighth-note triplets)
    pub base_duration: DurationKind,
    /// Confidence score for this grouping (0.0 - 1.0)
    pub confidence: f64,
}

impl TupletGroup {
    /// Get the number of notes in this group.
    #[must_use]
    pub fn len(&self) -> usize {
        self.end_idx - self.start_idx
    }

    /// Check if the group is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.end_idx <= self.start_idx
    }
}

/// Detect tuplet groups from quantized durations.
///
/// This function analyzes a sequence of quantized durations and identifies
/// complete tuplet groups (e.g., three consecutive triplet eighths).
///
/// # Arguments
/// * `quantized` - Quantized durations from `quantize_duration_batch`
/// * `start_positions` - Start positions of each note in ticks (for beat alignment)
/// * `config` - Quantization configuration
///
/// # Returns
/// A vector of detected tuplet groups with their indices and ratios.
#[must_use]
pub fn detect_tuplet_groups(
    quantized: &[QuantizedDuration],
    start_positions: &[i32],
    config: &QuantizeConfig,
) -> Vec<TupletGroup> {
    let mut groups = Vec::new();
    let mut i = 0;

    while i < quantized.len() {
        if let Some(tuplet_type) = quantized[i].tuplet_type {
            // Found a potential tuplet start
            if let Some((group, consumed)) =
                try_form_tuplet_group(quantized, start_positions, tuplet_type, i, config)
            {
                groups.push(group);
                i += consumed;
                continue;
            }
        }
        i += 1;
    }

    groups
}

/// Try to form a complete tuplet group starting at the given index.
fn try_form_tuplet_group(
    notes: &[QuantizedDuration],
    positions: &[i32],
    tuplet_type: TupletType,
    start_idx: usize,
    config: &QuantizeConfig,
) -> Option<(TupletGroup, usize)> {
    let expected_count = tuplet_type.expected_count();
    let remaining = &notes[start_idx..];

    if remaining.len() < expected_count {
        return None;
    }

    // Count consecutive notes of the same tuplet type
    let mut count = 0;

    for note in remaining {
        if note.tuplet_type == Some(tuplet_type) {
            count += 1;
        } else {
            break;
        }
    }

    // Need at least the expected count to form a complete group
    if count < expected_count {
        return None;
    }

    // Form groups of expected_count notes
    // (If we have 6 triplets, that's 2 triplet groups)
    let groups_possible = count / expected_count;
    if groups_possible == 0 {
        return None;
    }

    // For now, just form the first complete group
    let group_ticks: i32 = notes[start_idx..start_idx + expected_count]
        .iter()
        .map(|n| n.ticks)
        .sum();

    // Determine base duration from the total ticks of the group
    let base_duration = ticks_to_duration_kind(group_ticks, config.target_ppq);

    // Calculate confidence based on timing alignment and individual note confidence
    let confidence = calculate_group_confidence(
        &notes[start_idx..start_idx + expected_count],
        &positions[start_idx..start_idx + expected_count],
        config,
    );

    let group = TupletGroup {
        start_idx,
        end_idx: start_idx + expected_count,
        ratio: tuplet_type.to_ratio(),
        base_duration,
        confidence,
    };

    Some((group, expected_count))
}

/// Convert total ticks to the nearest duration kind.
fn ticks_to_duration_kind(ticks: i32, ppq: i32) -> DurationKind {
    let beat_ticks = ppq;

    if ticks >= beat_ticks * 4 {
        DurationKind::Whole
    } else if ticks >= beat_ticks * 2 {
        DurationKind::Half
    } else if ticks >= beat_ticks {
        DurationKind::Quarter
    } else if ticks >= beat_ticks / 2 {
        DurationKind::Eighth
    } else if ticks >= beat_ticks / 4 {
        DurationKind::Sixteenth
    } else if ticks >= beat_ticks / 8 {
        DurationKind::ThirtySecond
    } else {
        DurationKind::SixtyFourth
    }
}

/// Calculate confidence for a tuplet group.
fn calculate_group_confidence(
    notes: &[QuantizedDuration],
    positions: &[i32],
    config: &QuantizeConfig,
) -> f64 {
    if notes.is_empty() {
        return 0.0;
    }

    // Average confidence of constituent notes
    let avg_confidence: f64 = notes.iter().map(|n| n.confidence).sum::<f64>() / notes.len() as f64;

    // Bonus for consistent durations (all notes same length)
    let duration_consistency = if notes.iter().all(|n| n.ticks == notes[0].ticks) {
        0.1
    } else {
        0.0
    };

    // Bonus for beat alignment (group starts on a beat)
    let beat_alignment = if !positions.is_empty() {
        let start_pos = positions[0];
        let beat_ticks = config.target_ppq;
        if start_pos % beat_ticks == 0 {
            0.05
        } else {
            0.0
        }
    } else {
        0.0
    };

    (avg_confidence + duration_consistency + beat_alignment).min(1.0)
}

/// Merge adjacent tuplet groups of the same type if they form a larger pattern.
///
/// For example, two adjacent triplet groups might represent a 6-note pattern
/// that should be displayed with a single "6" bracket.
#[must_use]
pub fn merge_adjacent_groups(groups: Vec<TupletGroup>) -> Vec<TupletGroup> {
    if groups.len() < 2 {
        return groups;
    }

    let mut merged = Vec::new();
    let mut current: Option<TupletGroup> = None;

    for group in groups {
        match &mut current {
            None => {
                current = Some(group);
            }
            Some(curr) => {
                // Check if groups are adjacent and compatible
                if curr.end_idx == group.start_idx
                    && curr.ratio == group.ratio
                    && curr.base_duration == group.base_duration
                {
                    // Merge: extend current group
                    curr.end_idx = group.end_idx;
                    curr.confidence = (curr.confidence + group.confidence) / 2.0;
                } else {
                    // Not mergeable: push current and start new
                    merged.push(current.take().unwrap());
                    current = Some(group);
                }
            }
        }
    }

    if let Some(curr) = current {
        merged.push(curr);
    }

    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::engraver::quantize::detector::quantize_duration_batch;

    fn make_triplet_eighths(config: &QuantizeConfig) -> Vec<QuantizedDuration> {
        let durations = vec![160, 160, 160]; // Three triplet eighths
        let positions = vec![0, 160, 320];
        quantize_duration_batch(&durations, &positions, config)
    }

    #[test]
    fn test_detect_triplet_group() {
        let config = QuantizeConfig::default();
        let quantized = make_triplet_eighths(&config);
        let positions = vec![0, 160, 320];

        let groups = detect_tuplet_groups(&quantized, &positions, &config);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].start_idx, 0);
        assert_eq!(groups[0].end_idx, 3);
        assert_eq!(groups[0].ratio, TupletRatio::triplet());
    }

    #[test]
    fn test_no_groups_for_standard_notes() {
        let config = QuantizeConfig::default();
        let durations = vec![240, 240, 240, 240]; // Four regular eighths
        let positions = vec![0, 240, 480, 720];

        let quantized = quantize_duration_batch(&durations, &positions, &config);
        let groups = detect_tuplet_groups(&quantized, &positions, &config);

        assert!(groups.is_empty());
    }

    #[test]
    fn test_partial_triplet_not_grouped() {
        let config = QuantizeConfig::default();
        let durations = vec![160, 160]; // Only two triplet eighths (incomplete)
        let positions = vec![0, 160];

        let quantized = quantize_duration_batch(&durations, &positions, &config);
        let groups = detect_tuplet_groups(&quantized, &positions, &config);

        // Should not form a group with only 2 notes
        assert!(groups.is_empty());
    }

    #[test]
    fn test_multiple_triplet_groups() {
        let config = QuantizeConfig::default();
        // Two complete triplet groups
        let durations = vec![160, 160, 160, 160, 160, 160];
        let positions = vec![0, 160, 320, 480, 640, 800];

        let quantized = quantize_duration_batch(&durations, &positions, &config);
        let groups = detect_tuplet_groups(&quantized, &positions, &config);

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].start_idx, 0);
        assert_eq!(groups[0].end_idx, 3);
        assert_eq!(groups[1].start_idx, 3);
        assert_eq!(groups[1].end_idx, 6);
    }

    #[test]
    fn test_merge_adjacent_groups() {
        let groups = vec![
            TupletGroup {
                start_idx: 0,
                end_idx: 3,
                ratio: TupletRatio::triplet(),
                base_duration: DurationKind::Eighth,
                confidence: 0.9,
            },
            TupletGroup {
                start_idx: 3,
                end_idx: 6,
                ratio: TupletRatio::triplet(),
                base_duration: DurationKind::Eighth,
                confidence: 0.9,
            },
        ];

        let merged = merge_adjacent_groups(groups);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].start_idx, 0);
        assert_eq!(merged[0].end_idx, 6);
    }

    #[test]
    fn test_ticks_to_duration_kind() {
        let ppq = 480;

        assert_eq!(ticks_to_duration_kind(1920, ppq), DurationKind::Whole);
        assert_eq!(ticks_to_duration_kind(960, ppq), DurationKind::Half);
        assert_eq!(ticks_to_duration_kind(480, ppq), DurationKind::Quarter);
        assert_eq!(ticks_to_duration_kind(240, ppq), DurationKind::Eighth);
        assert_eq!(ticks_to_duration_kind(120, ppq), DurationKind::Sixteenth);
        assert_eq!(ticks_to_duration_kind(60, ppq), DurationKind::ThirtySecond);
    }
}
