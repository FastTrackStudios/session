//! Count-in pattern logic — determines which beats get a count number.
//!
//! Ported from the legacy FTS Guide plugin's `count_in/pattern.rs` and
//! `count_in/calculator.rs`.

/// Count-in pattern calculator.
pub struct CountInPattern;

impl CountInPattern {
    /// Calculate the total number of count-in measures from a quarter-note distance.
    ///
    /// Rounds to the nearest whole measure, clamped to 1–8.
    /// Returns 1 for degenerate inputs (zero/negative distance or measure length).
    pub fn calculate_measures(
        count_in_start_quarters: f64,
        section_start_quarters: f64,
        quarters_per_measure: f64,
    ) -> usize {
        if quarters_per_measure <= 0.0 {
            return 1;
        }
        let distance = section_start_quarters - count_in_start_quarters;
        if distance <= 0.0 {
            return 1;
        }
        let measures = (distance / quarters_per_measure).round() as i32;
        measures.max(1).min(8) as usize
    }

    /// Determine if a beat should produce a count, and which number (1–8).
    ///
    /// # Arguments
    /// * `measure_index` — 0-based measure index within the count-in region
    /// * `total_measures` — total number of count-in measures
    /// * `beat_index` — 1-based beat within the measure
    /// * `beats_per_measure` — time signature numerator
    /// * `time_sig_den` — time signature denominator
    /// * `offset_by_one` — if true, subtract one from countdown numbers
    /// * `full_count_odd_time` — use full counting for odd time signatures
    ///
    /// # Returns
    /// `Some(count_number)` (1–8) if this beat should be counted, `None` otherwise.
    #[allow(clippy::too_many_arguments)]
    pub fn should_count(
        measure_index: usize,
        beat_index: usize,
        total_measures: usize,
        beats_per_measure: u32,
        time_sig_den: u32,
        offset_by_one: bool,
        full_count_odd_time: bool,
    ) -> Option<u8> {
        let measure_index = measure_index as i32;
        let beat_index = beat_index as i32;
        let total_measures = total_measures as i32;
        let beats_per_measure = beats_per_measure as i32;
        let time_sig_den = time_sig_den as i32;

        // Special handling for /16 time signatures
        if time_sig_den == 16 {
            if total_measures <= 1 {
                // For 1-measure count-ins in /16: no counts, just guide voice
                return None;
            }
            // Multi-measure: count once per measure at beat 1
            if beat_index == 1 {
                let count_number = if offset_by_one {
                    total_measures - measure_index - 1
                } else {
                    total_measures - measure_index
                };
                if (1..=8).contains(&count_number) {
                    return Some(count_number as u8);
                }
            }
            return None;
        }

        // Standard time signatures
        if total_measures <= 1 {
            // Check if odd-time splitting applies to this time signature
            let is_odd_time = {
                let is_odd_with_16 = beats_per_measure % 2 == 1 && time_sig_den == 16;
                let is_numerator_gt_6 = beats_per_measure > 6;
                is_odd_with_16 || is_numerator_gt_6
            };

            if is_odd_time {
                // Odd time: use splitting logic exclusively
                return calculate_odd_time_count(
                    beat_index,
                    beats_per_measure,
                    time_sig_den,
                    full_count_odd_time,
                )
                .map(|n| n as u8);
            }

            // Standard 1-measure count: count all beats
            if beat_index >= 1 && beat_index <= beats_per_measure {
                return Some(beat_index as u8);
            }
            return None;
        }

        let measure_1 = measure_index + 1; // 1-indexed

        // Adaptive half-measure: count on beat 1 and the midpoint beat.
        // For 4/4 → beats 1 and 3; for 2/4 → beats 1 and 2; for 6/8 → beats 1 and 4.
        let half_measure_beat = beats_per_measure / 2 + 1;

        if total_measures == 2 {
            if measure_1 == 1 {
                return half_measure_count(beat_index, half_measure_beat);
            }
            // Last measure: full count with odd-time handling
            return full_count_beat(
                beat_index,
                beats_per_measure,
                time_sig_den,
                full_count_odd_time,
            );
        }

        // 3+ measures
        if measure_1 <= total_measures - 2 {
            // Early measures: count only on beat 1 (measure number)
            if beat_index == 1 {
                return Some(measure_1 as u8);
            }
            return None;
        }

        if measure_1 == total_measures - 1 {
            // Penultimate measure: half measure
            return half_measure_count(beat_index, half_measure_beat);
        }

        // Last measure: full count
        full_count_beat(
            beat_index,
            beats_per_measure,
            time_sig_den,
            full_count_odd_time,
        )
    }
}

/// Half-measure count: count "1" on beat 1, count "2" on the midpoint beat.
fn half_measure_count(beat_index: i32, half_measure_beat: i32) -> Option<u8> {
    if beat_index == 1 {
        Some(1)
    } else if beat_index == half_measure_beat {
        Some(2)
    } else {
        None
    }
}

/// Full count on all beats, with odd-time splitting if applicable.
fn full_count_beat(
    beat_index: i32,
    beats_per_measure: i32,
    time_sig_den: i32,
    full_count_odd_time: bool,
) -> Option<u8> {
    if let Some(n) = calculate_odd_time_count(
        beat_index,
        beats_per_measure,
        time_sig_den,
        full_count_odd_time,
    ) {
        return Some(n as u8);
    }
    if beat_index >= 1 && beat_index <= beats_per_measure {
        Some(beat_index as u8)
    } else {
        None
    }
}

/// Calculate count number for odd time signatures split into groups ending with 4.
///
/// Only applies when numerator is odd AND denominator is 16, OR numerator > 6.
/// Returns `None` for regular time signatures (delegates to normal counting).
fn calculate_odd_time_count(
    beat_in_measure: i32,
    time_sig_num: i32,
    time_sig_den: i32,
    full_count: bool,
) -> Option<i32> {
    let is_odd_with_16 = time_sig_num % 2 == 1 && time_sig_den == 16;
    let is_numerator_gt_6 = time_sig_num > 6;
    let should_split = is_odd_with_16 || is_numerator_gt_6;

    if !should_split {
        return None;
    }

    // Last group is always 4 beats (starts at num - 3)
    let last_group_start = time_sig_num - 3;

    if beat_in_measure >= last_group_start && beat_in_measure <= time_sig_num {
        let beat_in_group = beat_in_measure - last_group_start + 1;
        return Some(beat_in_group);
    }

    if !full_count {
        return None;
    }

    // Earlier groups (up to 8 beats each)
    let first_group_end = last_group_start - 1;
    let mut current_start = 1;

    while current_start <= first_group_end {
        let remaining_before_last = last_group_start - current_start;
        let group_size = if remaining_before_last > 8 {
            8
        } else {
            remaining_before_last
        };
        let current_end = current_start + group_size - 1;

        if beat_in_measure >= current_start && beat_in_measure <= current_end {
            let beat_in_group = beat_in_measure - current_start + 1;
            return Some(beat_in_group);
        }

        current_start = current_end + 1;
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: collect all count numbers for a single-measure count-in.
    fn counts_for_1_measure(beats: u32, den: u32) -> Vec<Option<u8>> {
        (1..=beats as usize)
            .map(|b| CountInPattern::should_count(0, b, 1, beats, den, false, true))
            .collect()
    }

    /// Helper: collect counts for multi-measure patterns (standard 4/4).
    fn count_grid_4_4(total_measures: usize) -> Vec<Vec<Option<u8>>> {
        (0..total_measures)
            .map(|m| {
                (1..=4)
                    .map(|b| CountInPattern::should_count(m, b, total_measures, 4, 4, false, true))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn test_1_measure_4_4() {
        let counts = counts_for_1_measure(4, 4);
        assert_eq!(counts, vec![Some(1), Some(2), Some(3), Some(4)]);
    }

    #[test]
    fn test_2_measures_4_4() {
        let grid = count_grid_4_4(2);
        // Measure 1: half measure (1, _, 2, _)
        assert_eq!(grid[0], vec![Some(1), None, Some(2), None]);
        // Measure 2: full count
        assert_eq!(grid[1], vec![Some(1), Some(2), Some(3), Some(4)]);
    }

    #[test]
    fn test_4_measures_4_4() {
        let grid = count_grid_4_4(4);
        // Measures 1-2: beat 1 only (measure number)
        assert_eq!(grid[0], vec![Some(1), None, None, None]);
        assert_eq!(grid[1], vec![Some(2), None, None, None]);
        // Measure 3: half measure
        assert_eq!(grid[2], vec![Some(1), None, Some(2), None]);
        // Measure 4: full count
        assert_eq!(grid[3], vec![Some(1), Some(2), Some(3), Some(4)]);
    }

    #[test]
    fn test_3_measures_4_4() {
        let grid = count_grid_4_4(3);
        // Measure 1: beat 1 only
        assert_eq!(grid[0], vec![Some(1), None, None, None]);
        // Measure 2: half measure
        assert_eq!(grid[1], vec![Some(1), None, Some(2), None]);
        // Measure 3: full count
        assert_eq!(grid[2], vec![Some(1), Some(2), Some(3), Some(4)]);
    }

    #[test]
    fn test_7_8_odd_time_1_measure() {
        // 7/8: numerator > 6, so odd-time splitting applies
        // Last group starts at 7 - 3 = 4, so last group = beats 4,5,6,7 -> count 1,2,3,4
        // With full_count=true, earlier group = beats 1,2,3 -> count 1,2,3
        let counts: Vec<Option<u8>> = (1..=7)
            .map(|b| CountInPattern::should_count(0, b, 1, 7, 8, false, true))
            .collect();
        assert_eq!(
            counts,
            vec![
                Some(1),
                Some(2),
                Some(3),
                Some(1),
                Some(2),
                Some(3),
                Some(4)
            ]
        );
    }

    #[test]
    fn test_7_8_odd_time_no_full_count() {
        // With full_count=false, only the last group of 4 is counted
        let counts: Vec<Option<u8>> = (1..=7)
            .map(|b| CountInPattern::should_count(0, b, 1, 7, 8, false, false))
            .collect();
        assert_eq!(
            counts,
            vec![None, None, None, Some(1), Some(2), Some(3), Some(4)]
        );
    }

    #[test]
    fn test_6_8_not_odd_time() {
        // 6/8: numerator is 6 (not > 6, and 6 is even), so normal counting
        let counts = counts_for_1_measure(6, 8);
        assert_eq!(
            counts,
            vec![Some(1), Some(2), Some(3), Some(4), Some(5), Some(6)]
        );
    }

    #[test]
    fn test_16th_time_sig_1_measure() {
        // /16 with 1 measure: no counts
        let counts: Vec<Option<u8>> = (1..=7)
            .map(|b| CountInPattern::should_count(0, b, 1, 7, 16, false, true))
            .collect();
        assert!(counts.iter().all(|c| c.is_none()));
    }

    #[test]
    fn test_16th_time_sig_multi_measure() {
        // /16 with 3 measures: count at beat 1 each measure (countdown)
        let counts: Vec<Option<u8>> = (0..3)
            .map(|m| CountInPattern::should_count(m, 1, 3, 7, 16, false, true))
            .collect();
        assert_eq!(counts, vec![Some(3), Some(2), Some(1)]);
    }

    #[test]
    fn test_calculate_measures() {
        // 4 quarter notes per measure in 4/4
        assert_eq!(CountInPattern::calculate_measures(0.0, 8.0, 4.0), 2);
        assert_eq!(CountInPattern::calculate_measures(0.0, 4.0, 4.0), 1);
        assert_eq!(CountInPattern::calculate_measures(0.0, 32.0, 4.0), 8); // clamped to 8
        assert_eq!(CountInPattern::calculate_measures(0.0, 100.0, 4.0), 8); // clamped to 8
    }

    // ── Edge case tests ─────────────────────────────────────────────────────

    #[test]
    fn test_calculate_measures_zero_length() {
        // Zero-length measure (degenerate) → returns 1
        assert_eq!(CountInPattern::calculate_measures(0.0, 4.0, 0.0), 1);
    }

    #[test]
    fn test_calculate_measures_negative_distance() {
        // Section starts before count-in (degenerate) → returns 1
        assert_eq!(CountInPattern::calculate_measures(8.0, 4.0, 4.0), 1);
    }

    #[test]
    fn test_calculate_measures_negative_measure_length() {
        // Negative measure length (degenerate) → returns 1
        assert_eq!(CountInPattern::calculate_measures(0.0, 8.0, -4.0), 1);
    }

    #[test]
    fn test_2_4_time_1_measure() {
        // 2/4: count all 2 beats
        let counts = counts_for_1_measure(2, 4);
        assert_eq!(counts, vec![Some(1), Some(2)]);
    }

    #[test]
    fn test_2_4_time_2_measures() {
        // 2/4 with 2 measures: half measure should count beats 1 and 2
        // (adaptive: midpoint for 2 beats is beat 2)
        let grid: Vec<Vec<Option<u8>>> = (0..2)
            .map(|m| {
                (1..=2)
                    .map(|b| CountInPattern::should_count(m, b, 2, 2, 4, false, true))
                    .collect()
            })
            .collect();
        // Measure 1: half measure → beat 1 = count 1, beat 2 = count 2
        assert_eq!(grid[0], vec![Some(1), Some(2)]);
        // Measure 2: full count
        assert_eq!(grid[1], vec![Some(1), Some(2)]);
    }

    #[test]
    fn test_1_4_time_1_measure() {
        // 1/4: single beat
        let counts = counts_for_1_measure(1, 4);
        assert_eq!(counts, vec![Some(1)]);
    }

    #[test]
    fn test_3_4_time_2_measures() {
        // 3/4 with 2 measures: half measure at midpoint (beat 2)
        let grid: Vec<Vec<Option<u8>>> = (0..2)
            .map(|m| {
                (1..=3)
                    .map(|b| CountInPattern::should_count(m, b, 2, 3, 4, false, true))
                    .collect()
            })
            .collect();
        // Measure 1: half measure → beat 1 = count 1, beat 2 = count 2
        assert_eq!(grid[0], vec![Some(1), Some(2), None]);
        // Measure 2: full count
        assert_eq!(grid[1], vec![Some(1), Some(2), Some(3)]);
    }

    #[test]
    fn test_9_8_odd_time_1_measure() {
        // 9/8: numerator > 6, splits into [5 beats] + [4 beats]
        // Last group starts at 9 - 3 = 6
        // With full_count=true: beats 1-5 → group 1 (count 1-5), beats 6-9 → group 2 (count 1-4)
        let counts: Vec<Option<u8>> = (1..=9)
            .map(|b| CountInPattern::should_count(0, b, 1, 9, 8, false, true))
            .collect();
        assert_eq!(
            counts,
            vec![
                Some(1),
                Some(2),
                Some(3),
                Some(4),
                Some(5),
                Some(1),
                Some(2),
                Some(3),
                Some(4)
            ]
        );
    }

    #[test]
    fn test_15_16_odd_time_1_measure() {
        // 15/16: odd numerator with /16 denominator → falls into /16 branch
        // 1-measure /16: no counts at all
        let counts: Vec<Option<u8>> = (1..=15)
            .map(|b| CountInPattern::should_count(0, b, 1, 15, 16, false, true))
            .collect();
        assert!(counts.iter().all(|c| c.is_none()));
    }

    #[test]
    fn test_beat_index_out_of_range() {
        // Beat index beyond beats_per_measure should return None
        assert_eq!(
            CountInPattern::should_count(0, 5, 1, 4, 4, false, true),
            None
        );
        assert_eq!(
            CountInPattern::should_count(0, 0, 1, 4, 4, false, true),
            None
        );
    }
}
