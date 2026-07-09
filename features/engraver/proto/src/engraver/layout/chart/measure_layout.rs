//! Measure layout utilities for chart rendering.
//!
//! This module provides functions for measure width distribution
//! and system grouping.

use super::spacing;

/// Distribute available width among measures using spring physics.
///
/// This implements MuseScore-style proportional spacing where measures with
/// more content receive proportionally more width, while respecting minimum
/// widths for each measure.
///
/// # Arguments
/// * `weights` - Content weights for each regular measure
/// * `count_in_measures` - Number of count-in measures (fixed width)
/// * `total_width` - Total available width for all measures
/// * `compact_scale` - Scale factor for count-in measures (typically 0.5)
/// * `base_measure_width` - Base width for a single measure
///
/// # Returns
/// Vector of widths for each measure (count-in measures first, then regular)
#[must_use]
pub fn distribute_measure_widths(
    weights: &[f64],
    count_in_measures: usize,
    total_width: f64,
    compact_scale: f64,
    base_measure_width: f64,
) -> Vec<f64> {
    // Delegate to the version with no minimum widths
    distribute_measure_widths_with_mins(
        weights,
        count_in_measures,
        total_width,
        compact_scale,
        base_measure_width,
        &[], // No minimum widths
    )
}

/// Distribute available width among measures with minimum width constraints.
///
/// This implements MuseScore-style proportional spacing where measures with
/// more content receive proportionally more width, while ensuring each measure
/// is at least as wide as its minimum.
///
/// # Arguments
/// * `weights` - Content weights for each regular measure
/// * `count_in_measures` - Number of count-in measures (fixed width)
/// * `total_width` - Total available width for all measures
/// * `compact_scale` - Scale factor for count-in measures (typically 0.5)
/// * `base_measure_width` - Base width for a single measure
/// * `min_widths` - Minimum widths for each regular measure (empty = no constraints)
///
/// # Returns
/// Vector of widths for each measure (count-in measures first, then regular)
#[must_use]
pub fn distribute_measure_widths_with_mins(
    weights: &[f64],
    count_in_measures: usize,
    total_width: f64,
    compact_scale: f64,
    base_measure_width: f64,
    min_widths: &[f64],
) -> Vec<f64> {
    if weights.is_empty() {
        return Vec::new();
    }

    // Calculate count-in width (fixed, compact)
    let count_in_width = base_measure_width * compact_scale;
    let count_in_total = count_in_measures as f64 * count_in_width;

    // Remaining width for regular measures
    let available_for_regular = total_width - count_in_total;

    // Sum of weights for spring calculation
    let weight_sum: f64 = weights.iter().sum();

    // First pass: calculate proportional widths
    let mut widths = Vec::with_capacity(count_in_measures + weights.len());

    // Add count-in widths
    for _ in 0..count_in_measures {
        widths.push(count_in_width);
    }

    // Calculate initial proportional widths for regular measures
    let mut regular_widths: Vec<f64> = if weight_sum > 0.0 {
        weights
            .iter()
            .map(|&weight| {
                let proportion = weight / weight_sum;
                available_for_regular * proportion
            })
            .collect()
    } else {
        // Fallback: equal distribution
        let equal_width = available_for_regular / weights.len() as f64;
        vec![equal_width; weights.len()]
    };

    // Second pass: enforce minimum widths and redistribute excess
    if !min_widths.is_empty() {
        // Track which measures are at their minimum (locked)
        let mut locked = vec![false; regular_widths.len()];
        let mut deficit = 0.0;

        // Find measures that need to be expanded to their minimum
        for (i, &min_w) in min_widths.iter().enumerate() {
            if i < regular_widths.len() && regular_widths[i] < min_w {
                deficit += min_w - regular_widths[i];
                regular_widths[i] = min_w;
                locked[i] = true;
            }
        }

        // If there's a deficit, take space from unlocked measures proportionally
        if deficit > 0.0 {
            // Calculate total width available from unlocked measures
            let unlocked_total: f64 = regular_widths
                .iter()
                .enumerate()
                .filter(|(i, _)| !locked[*i])
                .map(|(i, &w)| {
                    // Only take down to minimum
                    let min = min_widths.get(i).copied().unwrap_or(0.0);
                    (w - min).max(0.0)
                })
                .sum();

            if unlocked_total > 0.0 {
                // Distribute deficit proportionally among unlocked measures
                let compression_ratio = (unlocked_total - deficit).max(0.0) / unlocked_total;

                for (i, width) in regular_widths.iter_mut().enumerate() {
                    if !locked[i] {
                        let min = min_widths.get(i).copied().unwrap_or(0.0);
                        let compressible = (*width - min).max(0.0);
                        *width = min + compressible * compression_ratio;
                    }
                }
            }
            // If unlocked_total <= 0, we can't compress further - measures will overflow
        }
    }

    // Add regular widths to output
    widths.extend(regular_widths);

    widths
}

/// Distribute available width among measures using spring-based justification.
///
/// Each measure gets a natural width based on its total duration stretch
/// (sum of its beat segments' stretches). The spring model then distributes
/// extra space so that measures with longer average note values expand more.
///
/// # Arguments
/// * `stretches` - Total duration stretch for each regular measure
/// * `count_in_measures` - Number of count-in measures (fixed width)
/// * `total_width` - Total available width for all measures
/// * `compact_scale` - Scale factor for count-in measures
/// * `base_measure_width` - Base width per measure (for count-in sizing)
/// * `min_widths` - Minimum widths for each regular measure (from collision detection)
/// * `spatium` - Staff space in points
/// * `slope` - Duration stretch slope
/// * `density` - Spacing density
///
/// # Returns
/// Vector of widths for each measure (count-in measures first, then regular)
#[must_use]
pub fn distribute_measure_widths_spring(
    stretches: &[f64],
    count_in_measures: usize,
    total_width: f64,
    compact_scale: f64,
    base_measure_width: f64,
    min_widths: &[f64],
    spatium: f64,
    _slope: f64,
    density: f64,
) -> Vec<f64> {
    if stretches.is_empty() {
        return Vec::new();
    }

    // Count-in measures get fixed compact width
    let count_in_width = base_measure_width * compact_scale;
    let count_in_total = count_in_measures as f64 * count_in_width;
    let available_for_regular = total_width - count_in_total;

    let mut widths = Vec::with_capacity(count_in_measures + stretches.len());
    for _ in 0..count_in_measures {
        widths.push(count_in_width);
    }

    // Compute natural widths from stretches
    let base_quarter_width = spacing::DEFAULT_QUARTER_NOTE_SPACE_SPATIUMS * spatium;
    let mut regular_widths: Vec<f64> = stretches
        .iter()
        .enumerate()
        .map(|(i, &stretch)| {
            let natural = base_quarter_width * stretch / density.max(0.01);
            let min = min_widths.get(i).copied().unwrap_or(0.0);
            natural.max(min)
        })
        .collect();

    let total_natural: f64 = regular_widths.iter().sum();
    let extra = available_for_regular - total_natural;

    if extra > 1.0 {
        // Build springs for each measure
        let mut springs: Vec<spacing::Spring> = stretches
            .iter()
            .enumerate()
            .map(|(i, &stretch)| spacing::Spring::new(stretch.max(0.1), regular_widths[i], i))
            .collect();

        let results = spacing::stretch_segments_to_width(&mut springs, extra);
        for (idx, new_width) in results {
            let min = min_widths.get(idx).copied().unwrap_or(0.0);
            regular_widths[idx] = new_width.max(min);
        }
    } else if extra < -1.0 {
        // Need to compress — proportionally reduce widths above minimums
        let compressible_total: f64 = regular_widths
            .iter()
            .enumerate()
            .map(|(i, &w)| {
                let min = min_widths.get(i).copied().unwrap_or(0.0);
                (w - min).max(0.0)
            })
            .sum();

        if compressible_total > 0.0 {
            let ratio = (compressible_total + extra).max(0.0) / compressible_total;
            for (i, w) in regular_widths.iter_mut().enumerate() {
                let min = min_widths.get(i).copied().unwrap_or(0.0);
                let compressible = (*w - min).max(0.0);
                *w = min + compressible * ratio;
            }
        }
    }

    widths.extend(regular_widths);
    widths
}

/// Check whether a system should be justified (stretched to fill line width).
///
/// Returns false for sparse last systems (below fill limit) to prevent
/// ugly stretching of partially-filled final lines.
///
/// # Arguments
/// * `content_width` - Current total width of content on the system
/// * `target_width` - Target system width (full page width)
/// * `fill_limit` - Minimum fill ratio to justify (default 0.3)
/// * `is_last_system` - Whether this is the last system in a section
#[must_use]
pub fn should_justify_system(
    content_width: f64,
    target_width: f64,
    fill_limit: f64,
    is_last_system: bool,
) -> bool {
    if !is_last_system {
        return true; // Always justify non-last systems
    }
    if target_width <= 0.0 {
        return false;
    }
    content_width / target_width >= fill_limit
}

/// Compute the minimum measure width scaled by time signature ratio.
///
/// For partial measures (e.g., a 2/4 pickup in a 4/4 chart), the minimum
/// width is proportionally reduced: `min_width * (measure_ticks / timesig_ticks)`.
/// Full measures get the full minimum width.
///
/// After computing the scaled minimum, if the measure's content width (from
/// natural spacing) is less than the minimum, it will be stretched via the
/// spring model during justification.
///
/// # Arguments
/// * `min_measure_width` - Base minimum width for a full measure (from `LayoutParams`)
/// * `measure_ticks` - Actual ticks in this measure
/// * `timesig_ticks` - Ticks for the time signature's full bar
///
/// # Returns
/// Scaled minimum width for this measure
#[must_use]
pub fn scaled_min_measure_width(
    min_measure_width: f64,
    measure_ticks: f64,
    timesig_ticks: f64,
) -> f64 {
    if timesig_ticks <= 0.0 || measure_ticks <= 0.0 {
        return min_measure_width;
    }

    let ratio = (measure_ticks / timesig_ticks).min(1.0); // Cap at 1.0 for full measures
    min_measure_width * ratio
}

/// Enforce minimum measure widths on a set of measures.
///
/// For each measure, computes the duration-scaled minimum width and ensures
/// the measure is at least that wide. Measures below the minimum are expanded,
/// and the excess width is redistributed from other measures if possible.
///
/// # Arguments
/// * `widths` - Current measure widths (will be adjusted in place)
/// * `measure_ticks` - Duration of each measure in ticks
/// * `timesig_ticks` - Ticks per full bar (from time signature)
/// * `min_measure_width` - Base minimum width from layout params
///
/// # Returns
/// Adjusted measure widths
#[must_use]
pub fn enforce_min_widths(
    widths: &[f64],
    measure_ticks: &[f64],
    timesig_ticks: f64,
    min_measure_width: f64,
) -> Vec<f64> {
    let mut result: Vec<f64> = widths.to_vec();
    let mut deficit = 0.0;

    // First pass: expand measures below their scaled minimum
    for (i, w) in result.iter_mut().enumerate() {
        let ticks = measure_ticks.get(i).copied().unwrap_or(timesig_ticks);
        let min_w = scaled_min_measure_width(min_measure_width, ticks, timesig_ticks);
        if *w < min_w {
            deficit += min_w - *w;
            *w = min_w;
        }
    }

    // Second pass: if there's a deficit, proportionally shrink measures above min
    if deficit > 0.0 {
        let compressible: f64 = result
            .iter()
            .enumerate()
            .map(|(i, &w)| {
                let ticks = measure_ticks.get(i).copied().unwrap_or(timesig_ticks);
                let min_w = scaled_min_measure_width(min_measure_width, ticks, timesig_ticks);
                (w - min_w).max(0.0)
            })
            .sum();

        if compressible > deficit {
            let ratio = (compressible - deficit) / compressible;
            for (i, w) in result.iter_mut().enumerate() {
                let ticks = measure_ticks.get(i).copied().unwrap_or(timesig_ticks);
                let min_w = scaled_min_measure_width(min_measure_width, ticks, timesig_ticks);
                let excess = (*w - min_w).max(0.0);
                *w = min_w + excess * ratio;
            }
        }
    }

    result
}

/// Group measures into systems based on maximum measures per system.
///
/// # Arguments
/// * `measure_count` - Total number of measures to group
/// * `max_measures_per_system` - Maximum measures allowed per system
///
/// # Returns
/// Vector of systems, each containing measure indices
#[must_use]
pub fn group_measures_into_systems(
    measure_count: usize,
    max_measures_per_system: usize,
) -> Vec<Vec<usize>> {
    let mut systems = Vec::new();
    let mut current_system = Vec::new();

    for i in 0..measure_count {
        current_system.push(i);
        if current_system.len() >= max_measures_per_system {
            systems.push(std::mem::take(&mut current_system));
        }
    }

    if !current_system.is_empty() {
        systems.push(current_system);
    }

    systems
}

/// Group measures into systems based on available width and minimum widths.
///
/// This improves on count-based grouping by ensuring measures fit within
/// the available width. If a measure's minimum width would cause overflow,
/// it starts a new system.
///
/// # Arguments
/// * `min_widths` - Minimum width required for each measure
/// * `available_width` - Total width available for measures on a system
/// * `max_measures_per_system` - Maximum measures allowed per system (upper bound)
///
/// # Returns
/// Vector of systems, each containing measure indices
#[must_use]
pub fn group_measures_into_systems_by_width(
    min_widths: &[f64],
    available_width: f64,
    max_measures_per_system: usize,
) -> Vec<Vec<usize>> {
    if min_widths.is_empty() {
        return Vec::new();
    }

    let mut systems = Vec::new();
    let mut current_system = Vec::new();
    let mut current_width = 0.0;

    for (i, &min_w) in min_widths.iter().enumerate() {
        // Would adding this measure exceed available width?
        let would_overflow = current_width + min_w > available_width && !current_system.is_empty();

        // Would adding this measure exceed max measures?
        let would_exceed_max = current_system.len() >= max_measures_per_system;

        if would_overflow || would_exceed_max {
            // Start a new system
            systems.push(std::mem::take(&mut current_system));
            current_width = 0.0;
        }

        current_system.push(i);
        current_width += min_w;
    }

    if !current_system.is_empty() {
        systems.push(current_system);
    }

    systems
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_distribute_measure_widths_equal_weights() {
        let weights = vec![1.0, 1.0, 1.0, 1.0];
        let widths = distribute_measure_widths(&weights, 0, 400.0, 0.5, 100.0);

        assert_eq!(widths.len(), 4);
        // All measures should have equal width
        for &w in &widths {
            assert!((w - 100.0).abs() < 0.001);
        }
    }

    #[test]
    fn test_distribute_measure_widths_with_count_in() {
        let weights = vec![1.0, 1.0];
        let widths = distribute_measure_widths(&weights, 2, 400.0, 0.5, 100.0);

        // 2 count-in + 2 regular = 4 measures
        assert_eq!(widths.len(), 4);

        // Count-in measures should be 50.0 each (100 * 0.5)
        assert!((widths[0] - 50.0).abs() < 0.001);
        assert!((widths[1] - 50.0).abs() < 0.001);

        // Regular measures share remaining 300.0 equally
        assert!((widths[2] - 150.0).abs() < 0.001);
        assert!((widths[3] - 150.0).abs() < 0.001);
    }

    #[test]
    fn test_distribute_measure_widths_proportional() {
        // Different weights should give different widths
        let weights = vec![1.0, 2.0, 1.0];
        let widths = distribute_measure_widths(&weights, 0, 400.0, 0.5, 100.0);

        assert_eq!(widths.len(), 3);

        // Weight 1.0 should get 1/4 of width
        assert!((widths[0] - 100.0).abs() < 0.001);
        // Weight 2.0 should get 2/4 of width
        assert!((widths[1] - 200.0).abs() < 0.001);
        // Weight 1.0 should get 1/4 of width
        assert!((widths[2] - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_distribute_measure_widths_empty() {
        let weights: Vec<f64> = vec![];
        let widths = distribute_measure_widths(&weights, 0, 400.0, 0.5, 100.0);

        assert!(widths.is_empty());
    }

    #[test]
    fn test_group_measures_into_systems_basic() {
        let systems = group_measures_into_systems(8, 4);

        assert_eq!(systems.len(), 2);
        assert_eq!(systems[0], vec![0, 1, 2, 3]);
        assert_eq!(systems[1], vec![4, 5, 6, 7]);
    }

    #[test]
    fn test_group_measures_into_systems_partial_last() {
        let systems = group_measures_into_systems(10, 4);

        assert_eq!(systems.len(), 3);
        assert_eq!(systems[0], vec![0, 1, 2, 3]);
        assert_eq!(systems[1], vec![4, 5, 6, 7]);
        assert_eq!(systems[2], vec![8, 9]);
    }

    #[test]
    fn test_group_measures_into_systems_fewer_than_max() {
        let systems = group_measures_into_systems(3, 4);

        assert_eq!(systems.len(), 1);
        assert_eq!(systems[0], vec![0, 1, 2]);
    }

    #[test]
    fn test_group_measures_into_systems_empty() {
        let systems = group_measures_into_systems(0, 4);

        assert!(systems.is_empty());
    }

    #[test]
    fn test_group_measures_by_width_basic() {
        // 4 measures of 100pt each, 400pt available, max 4 per system
        let min_widths = vec![100.0, 100.0, 100.0, 100.0];
        let systems = group_measures_into_systems_by_width(&min_widths, 400.0, 4);

        // All 4 fit in one system
        assert_eq!(systems.len(), 1);
        assert_eq!(systems[0], vec![0, 1, 2, 3]);
    }

    #[test]
    fn test_group_measures_by_width_overflow() {
        // 4 measures, one is very wide (200pt), available width 300pt
        let min_widths = vec![80.0, 80.0, 200.0, 80.0];
        let systems = group_measures_into_systems_by_width(&min_widths, 300.0, 4);

        // Measures 0+1 = 160pt (fits in 300pt)
        // Adding measure 2 (200pt) would be 360pt > 300pt, so new system
        // Measure 2 alone = 200pt (fits)
        // Adding measure 3 (80pt) would be 280pt < 300pt (fits)
        assert_eq!(systems.len(), 2);
        assert_eq!(systems[0], vec![0, 1]);
        assert_eq!(systems[1], vec![2, 3]);
    }

    #[test]
    fn test_group_measures_by_width_respects_max() {
        // 8 measures of 50pt each, 400pt available, but max 4 per system
        let min_widths = vec![50.0; 8];
        let systems = group_measures_into_systems_by_width(&min_widths, 400.0, 4);

        // Should respect max_measures_per_system even though width allows more
        assert_eq!(systems.len(), 2);
        assert_eq!(systems[0], vec![0, 1, 2, 3]);
        assert_eq!(systems[1], vec![4, 5, 6, 7]);
    }

    #[test]
    fn test_group_measures_by_width_wide_measure_alone() {
        // One very wide measure that exceeds available width by itself
        // It should still be placed (can't split a measure)
        let min_widths = vec![100.0, 500.0, 100.0];
        let systems = group_measures_into_systems_by_width(&min_widths, 300.0, 4);

        // Measure 0 = 100pt (fits)
        // Adding measure 1 (500pt) would be 600pt > 300pt, so new system
        // Measure 1 alone = 500pt (exceeds 300pt, but must be placed)
        // Adding measure 2 (100pt) would be 600pt > 300pt, so new system
        assert_eq!(systems.len(), 3);
        assert_eq!(systems[0], vec![0]);
        assert_eq!(systems[1], vec![1]);
        assert_eq!(systems[2], vec![2]);
    }

    // --- Spring-based distribution ---

    #[test]
    fn test_spring_distribution_basic() {
        // 4 measures with equal stretch — should get roughly equal widths
        let stretches = vec![4.0, 4.0, 4.0, 4.0];
        let widths =
            distribute_measure_widths_spring(&stretches, 0, 400.0, 0.5, 100.0, &[], 5.0, 1.2, 1.0);

        assert_eq!(widths.len(), 4);
        let w0 = widths[0];
        for &w in &widths[1..] {
            assert!(
                (w - w0).abs() < 1.0,
                "Equal stretches should give equal widths: {w} vs {w0}"
            );
        }
    }

    #[test]
    fn test_spring_distribution_different_stretches() {
        // Measure with more stretch should get more width
        let stretches = vec![2.0, 6.0, 2.0, 6.0];
        let widths =
            distribute_measure_widths_spring(&stretches, 0, 400.0, 0.5, 100.0, &[], 5.0, 1.2, 1.0);

        assert!(
            widths[1] > widths[0],
            "Higher stretch measure should be wider: {} vs {}",
            widths[1],
            widths[0]
        );
        assert!(
            widths[3] > widths[2],
            "Higher stretch measure should be wider: {} vs {}",
            widths[3],
            widths[2]
        );
    }

    #[test]
    fn test_spring_distribution_respects_mins() {
        let stretches = vec![1.0, 1.0];
        let min_widths = vec![150.0, 50.0]; // First measure needs at least 150pt
        let widths = distribute_measure_widths_spring(
            &stretches,
            0,
            300.0,
            0.5,
            100.0,
            &min_widths,
            5.0,
            1.2,
            1.0,
        );

        assert!(
            widths[0] >= 150.0,
            "Should respect min_width: got {}",
            widths[0]
        );
    }

    #[test]
    fn test_spring_distribution_with_count_in() {
        let stretches = vec![4.0, 4.0];
        let widths =
            distribute_measure_widths_spring(&stretches, 1, 400.0, 0.5, 100.0, &[], 5.0, 1.2, 1.0);

        // 1 count-in + 2 regular = 3 total
        assert_eq!(widths.len(), 3);
        assert!((widths[0] - 50.0).abs() < 0.001, "Count-in should be 50pt");
    }

    #[test]
    fn test_should_justify_system() {
        // Non-last system always justifies
        assert!(should_justify_system(100.0, 400.0, 0.3, false));

        // Last system above fill limit — justify
        assert!(should_justify_system(200.0, 400.0, 0.3, true));

        // Last system below fill limit — don't justify
        assert!(!should_justify_system(100.0, 400.0, 0.3, true));

        // Last system at exactly fill limit — justify
        assert!(should_justify_system(120.0, 400.0, 0.3, true));
    }

    #[test]
    fn test_group_measures_by_width_empty() {
        let min_widths: Vec<f64> = vec![];
        let systems = group_measures_into_systems_by_width(&min_widths, 400.0, 4);

        assert!(systems.is_empty());
    }

    // --- Minimum measure width ---

    #[test]
    fn test_scaled_min_width_full_measure() {
        // 4/4 measure in 4/4 time → full min width
        let min_w = scaled_min_measure_width(100.0, 1920.0, 1920.0);
        assert!((min_w - 100.0).abs() < 1e-6);
    }

    #[test]
    fn test_scaled_min_width_half_measure() {
        // 2/4 pickup in 4/4 time → half min width
        let min_w = scaled_min_measure_width(100.0, 960.0, 1920.0);
        assert!((min_w - 50.0).abs() < 1e-6);
    }

    #[test]
    fn test_scaled_min_width_quarter_pickup() {
        // 1/4 pickup in 4/4 time → quarter min width
        let min_w = scaled_min_measure_width(100.0, 480.0, 1920.0);
        assert!((min_w - 25.0).abs() < 1e-6);
    }

    #[test]
    fn test_scaled_min_width_capped_at_full() {
        // Measure with more ticks than time sig → capped at full min width
        let min_w = scaled_min_measure_width(100.0, 3840.0, 1920.0);
        assert!((min_w - 100.0).abs() < 1e-6);
    }

    #[test]
    fn test_enforce_min_widths_no_change() {
        // All measures already above minimum
        let widths = vec![150.0, 150.0, 150.0, 150.0];
        let ticks = vec![1920.0; 4];
        let result = enforce_min_widths(&widths, &ticks, 1920.0, 100.0);

        for (i, &w) in result.iter().enumerate() {
            assert!((w - widths[i]).abs() < 1e-6, "Measure {i} shouldn't change");
        }
    }

    #[test]
    fn test_enforce_min_widths_expands_narrow() {
        // One narrow measure should be expanded, others compressed to compensate
        let widths = vec![50.0, 150.0, 150.0, 150.0];
        let ticks = vec![1920.0; 4]; // All full measures
        let result = enforce_min_widths(&widths, &ticks, 1920.0, 100.0);

        assert!(
            result[0] >= 100.0,
            "Narrow measure should be expanded to min: got {}",
            result[0]
        );
    }

    #[test]
    fn test_enforce_min_widths_pickup_measure() {
        // Pickup (2/4) gets half the min width
        let widths = vec![30.0, 150.0, 150.0, 150.0];
        let ticks = vec![960.0, 1920.0, 1920.0, 1920.0]; // First is pickup
        let result = enforce_min_widths(&widths, &ticks, 1920.0, 100.0);

        // Pickup min = 50.0, was 30.0 → should expand to 50.0
        assert!(
            result[0] >= 50.0,
            "Pickup should expand to scaled min: got {}",
            result[0]
        );
    }
}
