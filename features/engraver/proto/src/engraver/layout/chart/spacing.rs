//! Horizontal spacing engine inspired by MuseScore 4.
//!
//! Provides duration-proportional spacing formulas and spring-based justification
//! for distributing horizontal space within measures and across systems.
//!
//! # Key Concepts
//!
//! - **Duration stretch**: Maps note duration to horizontal space using a power law.
//!   Longer notes get more space, but sub-linearly (a whole note doesn't get 4× the
//!   space of a quarter note).
//!
//! - **Spring justification**: Distributes extra space proportionally using a spring
//!   model where shorter-duration segments are "stiffer" (expand less).
//!
//! - **Squeeze**: When content exceeds available width, iteratively reduces padding
//!   and stretch to fit without clipping.
//!
//! Reference: `memory/MuseScoreSpacingReport.md`

use super::constants::TICKS_PER_QUARTER;

// ============================================================================
// Constants
// ============================================================================

/// Default base width of a quarter note in spatiums.
/// From MuseScore: `DEFAULT_QUARTER_NOTE_SPACE = 3.5_sp`.
pub const DEFAULT_QUARTER_NOTE_SPACE_SPATIUMS: f64 = 3.5;

/// Default slope for the duration stretch power law.
/// Controls how aggressively longer notes get more space.
/// At slope=1.2: half=1.2×, whole=1.44×, eighth=0.833×.
pub const DEFAULT_SPACING_SLOPE: f64 = 1.2;

/// Default spacing density (divides natural width; higher = tighter).
pub const DEFAULT_SPACING_DENSITY: f64 = 1.0;

/// Default fill limit for last system justification.
/// Systems below this fill ratio are left ragged (not justified).
pub const DEFAULT_LAST_SYSTEM_FILL_LIMIT: f64 = 0.3;

/// Step size for reducing stretch during squeeze Phase 1.
pub const STRETCH_REDUCTION_STEP: f64 = 0.33;

/// Step size for reducing squeeze factor during squeeze Phase 1.
pub const SQUEEZE_STEP: f64 = 0.2;

/// Step size for brute-force width reduction during squeeze Phase 2.
pub const WIDTH_REDUCTION_STEP: f64 = 0.05;

// ============================================================================
// Duration Stretch
// ============================================================================

/// Compute the duration stretch factor for a given tick duration.
///
/// Uses a power-law formula: `slope ^ log2(ticks / quarter_ticks)`.
/// A quarter note returns 1.0. Each doubling of duration multiplies by `slope`.
///
/// # Arguments
/// * `ticks` - Duration in ticks
/// * `quarter_ticks` - Ticks per quarter note (typically 480)
/// * `slope` - Power law slope (default 1.2)
///
/// # Returns
/// Stretch factor (1.0 for a quarter note)
#[must_use]
pub fn duration_stretch(ticks: f64, quarter_ticks: f64, slope: f64) -> f64 {
    if ticks <= 0.0 || quarter_ticks <= 0.0 || slope <= 0.0 {
        return 1.0;
    }
    let ratio = ticks / quarter_ticks;
    slope.powf(ratio.log2())
}

/// Compute the natural width of a segment based on its duration.
///
/// Formula: `base_quarter_width * duration_stretch * stretch_reduction / density`
///
/// # Arguments
/// * `ticks` - Duration in ticks
/// * `spatium` - Staff space in points
/// * `slope` - Duration stretch slope
/// * `density` - Spacing density (higher = tighter)
/// * `stretch_reduction` - Multiplier for squeeze (1.0 = normal)
///
/// # Returns
/// Natural width in points
#[must_use]
pub fn natural_width(
    ticks: f64,
    spatium: f64,
    slope: f64,
    density: f64,
    stretch_reduction: f64,
) -> f64 {
    let base_quarter_width = DEFAULT_QUARTER_NOTE_SPACE_SPATIUMS * spatium;
    let stretch = duration_stretch(ticks, TICKS_PER_QUARTER, slope);
    base_quarter_width * stretch * stretch_reduction / density.max(0.01)
}

/// Convenience: compute natural width using default slope and density.
#[must_use]
pub fn natural_width_default(ticks: f64, spatium: f64) -> f64 {
    natural_width(
        ticks,
        spatium,
        DEFAULT_SPACING_SLOPE,
        DEFAULT_SPACING_DENSITY,
        1.0,
    )
}

// ============================================================================
// Spring Model
// ============================================================================

/// A spring representing a segment's resistance to stretching.
///
/// Shorter-duration segments have higher spring constants (stiffer),
/// so they expand less during justification.
#[derive(Debug, Clone)]
pub struct Spring {
    /// Spring constant: `1.0 / duration_stretch` (stiffer for short notes)
    pub spring_const: f64,
    /// Current width of the segment
    pub width: f64,
    /// Pre-tension: `width * spring_const` (force at current width)
    pub pre_tension: f64,
    /// Index into the original segment/measure array
    pub index: usize,
}

impl Spring {
    /// Create a spring for a segment.
    ///
    /// # Arguments
    /// * `stretch` - Duration stretch factor (from `duration_stretch()`)
    /// * `width` - Current segment width
    /// * `index` - Index for mapping back to the original array
    #[must_use]
    pub fn new(stretch: f64, width: f64, index: usize) -> Self {
        let spring_const = 1.0 / stretch.max(0.001);
        let pre_tension = width * spring_const;
        Self {
            spring_const,
            width,
            pre_tension,
            index,
        }
    }
}

/// Distribute extra width across springs using MuseScore's equilibrium-force algorithm.
///
/// Springs are sorted by pre-tension. The algorithm finds the force level where
/// all activated springs expand proportionally. Short-duration segments (high spring
/// constant) expand less than long-duration segments (low spring constant).
///
/// # Arguments
/// * `springs` - Mutable slice of springs (will be sorted by pre_tension)
/// * `extra_width` - Extra width to distribute (must be positive)
///
/// # Returns
/// Vector of `(index, new_width)` pairs for segments that changed
pub fn stretch_segments_to_width(springs: &mut [Spring], extra_width: f64) -> Vec<(usize, f64)> {
    if springs.is_empty() || extra_width <= 0.0 {
        return Vec::new();
    }

    // Sort by pre-tension (ascending) — lowest tension springs activate first
    springs.sort_by(|a, b| {
        a.pre_tension
            .partial_cmp(&b.pre_tension)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut inverse_spring_const = 0.0;
    let mut accumulated_width = extra_width; // Start with extra width to distribute
    let mut force = 0.0;

    // Find equilibrium force by iteratively adding springs
    let mut activated_count = 0;
    for (i, spring) in springs.iter().enumerate() {
        inverse_spring_const += 1.0 / spring.spring_const;
        accumulated_width += spring.width;
        force = accumulated_width / inverse_spring_const;
        activated_count = i + 1;

        // Stop when force can't activate the next spring
        if i + 1 < springs.len() && force < springs[i + 1].pre_tension {
            break;
        }
    }

    // Apply new widths to activated springs
    let mut results = Vec::with_capacity(activated_count);
    for spring in springs.iter() {
        if force > spring.pre_tension {
            let new_width = force / spring.spring_const;
            results.push((spring.index, new_width));
        }
    }

    results
}

// ============================================================================
// Squeeze Algorithm
// ============================================================================

/// Parameters for the squeeze algorithm.
#[derive(Debug, Clone, Copy)]
pub struct SqueezeParams {
    /// Multiplier for padding values (1.0 = normal, < 1.0 = compressed)
    pub squeeze_factor: f64,
    /// Multiplier for duration-based natural width (1.0 = normal)
    pub stretch_reduction: f64,
}

impl Default for SqueezeParams {
    fn default() -> Self {
        Self {
            squeeze_factor: 1.0,
            stretch_reduction: 1.0,
        }
    }
}

impl SqueezeParams {
    /// Create default (no squeeze) parameters.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Step the squeeze parameters toward tighter spacing.
    /// Returns `None` if we've exhausted Phase 1 squeeze.
    #[must_use]
    pub fn step(&self) -> Option<Self> {
        let new_squeeze = self.squeeze_factor - SQUEEZE_STEP;
        let new_stretch = self.stretch_reduction - STRETCH_REDUCTION_STEP;

        if new_squeeze <= 0.0 {
            None // Phase 1 exhausted
        } else {
            Some(Self {
                squeeze_factor: new_squeeze.max(0.0),
                stretch_reduction: new_stretch.max(0.0),
            })
        }
    }
}

/// Result of a squeeze operation.
#[derive(Debug, Clone)]
pub struct SqueezeResult {
    /// Adjusted segment widths after squeeze
    pub widths: Vec<f64>,
    /// The squeeze params that achieved the fit (or the last tried)
    pub params: SqueezeParams,
    /// Whether the content fits within the target width
    pub fits: bool,
}

/// Compute segment widths for a measure given tick durations and squeeze parameters.
///
/// This is the core width computation that gets called repeatedly during squeeze.
///
/// # Arguments
/// * `segment_ticks` - Duration of each segment in ticks
/// * `min_widths` - Minimum width per segment (e.g. from chord collision prevention)
/// * `spatium` - Staff space in points
/// * `slope` - Duration stretch slope
/// * `density` - Spacing density
/// * `params` - Current squeeze parameters
///
/// # Returns
/// Width for each segment
#[must_use]
pub fn compute_segment_widths(
    segment_ticks: &[f64],
    min_widths: &[f64],
    spatium: f64,
    slope: f64,
    density: f64,
    params: &SqueezeParams,
) -> Vec<f64> {
    segment_ticks
        .iter()
        .enumerate()
        .map(|(i, &ticks)| {
            let w = natural_width(ticks, spatium, slope, density, params.stretch_reduction);
            let squeezed = w * params.squeeze_factor;
            let min = min_widths.get(i).copied().unwrap_or(0.0);
            squeezed.max(min)
        })
        .collect()
}

/// Squeeze a measure's segments to fit within a target width.
///
/// Uses two phases:
/// - **Phase 1**: Iteratively reduce `squeeze_factor` and `stretch_reduction`,
///   recomputing widths each step. Maintains proportional spacing.
/// - **Phase 2**: Brute-force scale all widths uniformly until they fit.
///   This may cause collisions as a last resort.
///
/// # Arguments
/// * `segment_ticks` - Duration of each segment in ticks
/// * `min_widths` - Minimum width per segment (from collision prevention)
/// * `target_width` - Available width for this measure
/// * `spatium` - Staff space in points
/// * `slope` - Duration stretch slope
/// * `density` - Spacing density
///
/// # Returns
/// `SqueezeResult` with adjusted widths and whether it fit
pub fn squeeze_to_fit(
    segment_ticks: &[f64],
    min_widths: &[f64],
    target_width: f64,
    spatium: f64,
    slope: f64,
    density: f64,
) -> SqueezeResult {
    // Start with no squeeze
    let mut params = SqueezeParams::default();
    let mut widths =
        compute_segment_widths(segment_ticks, min_widths, spatium, slope, density, &params);
    let mut total: f64 = widths.iter().sum();

    // Phase 1: iterative squeeze_factor + stretch_reduction reduction
    while total > target_width {
        match params.step() {
            Some(next_params) => {
                params = next_params;
                widths = compute_segment_widths(
                    segment_ticks,
                    min_widths,
                    spatium,
                    slope,
                    density,
                    &params,
                );
                total = widths.iter().sum();
            }
            None => break, // Phase 1 exhausted
        }
    }

    if total <= target_width {
        return SqueezeResult {
            widths,
            params,
            fits: true,
        };
    }

    // Phase 2: brute-force uniform scaling
    // Scale all widths proportionally, ignoring min_widths as last resort
    let mut scale = 1.0;
    while total > target_width && scale > WIDTH_REDUCTION_STEP {
        scale -= WIDTH_REDUCTION_STEP;
        widths =
            compute_segment_widths(segment_ticks, min_widths, spatium, slope, density, &params)
                .iter()
                .map(|&w| w * scale)
                .collect();
        total = widths.iter().sum();
    }

    // Final fallback: force exact fit by uniform scaling
    if total > target_width && total > 0.0 {
        let final_scale = target_width / total;
        widths.iter_mut().for_each(|w| *w *= final_scale);
        total = target_width;
    }

    SqueezeResult {
        widths,
        params,
        fits: total <= target_width,
    }
}

/// Squeeze an entire system (multiple measures) to fit within a target width.
///
/// When all measures on a line exceed the target width, applies uniform squeeze
/// across all measures proportionally.
///
/// # Arguments
/// * `measure_widths` - Current width of each measure
/// * `target_width` - Available system width
///
/// # Returns
/// Adjusted measure widths, or `None` if no squeeze was needed
#[must_use]
pub fn squeeze_system(measure_widths: &[f64], target_width: f64) -> Option<Vec<f64>> {
    let total: f64 = measure_widths.iter().sum();
    if total <= target_width || total <= 0.0 {
        return None; // No squeeze needed
    }

    let scale = target_width / total;
    Some(measure_widths.iter().map(|&w| w * scale).collect())
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const QUARTER: f64 = 480.0;
    const HALF: f64 = 960.0;
    const WHOLE: f64 = 1920.0;
    const EIGHTH: f64 = 240.0;
    const SIXTEENTH: f64 = 120.0;

    // --- Duration Stretch ---

    #[test]
    fn test_quarter_note_stretch_is_one() {
        let stretch = duration_stretch(QUARTER, QUARTER, 1.2);
        assert!(
            (stretch - 1.0).abs() < 1e-10,
            "Quarter note stretch should be 1.0, got {stretch}"
        );
    }

    #[test]
    fn test_half_note_stretch() {
        let stretch = duration_stretch(HALF, QUARTER, 1.2);
        assert!(
            (stretch - 1.2).abs() < 1e-10,
            "Half note stretch should be 1.2, got {stretch}"
        );
    }

    #[test]
    fn test_whole_note_stretch() {
        let stretch = duration_stretch(WHOLE, QUARTER, 1.2);
        let expected = 1.2_f64.powi(2); // 1.44
        assert!(
            (stretch - expected).abs() < 1e-10,
            "Whole note stretch should be {expected}, got {stretch}"
        );
    }

    #[test]
    fn test_eighth_note_stretch() {
        let stretch = duration_stretch(EIGHTH, QUARTER, 1.2);
        let expected = 1.0 / 1.2; // ~0.833
        assert!(
            (stretch - expected).abs() < 1e-10,
            "Eighth note stretch should be {expected}, got {stretch}"
        );
    }

    #[test]
    fn test_sixteenth_note_stretch() {
        let stretch = duration_stretch(SIXTEENTH, QUARTER, 1.2);
        let expected = 1.2_f64.powf(-2.0); // ~0.694
        assert!(
            (stretch - expected).abs() < 1e-6,
            "16th note stretch should be ~{expected}, got {stretch}"
        );
    }

    #[test]
    fn test_stretch_monotonic() {
        // Longer durations should always get more stretch
        let s16 = duration_stretch(SIXTEENTH, QUARTER, 1.2);
        let s8 = duration_stretch(EIGHTH, QUARTER, 1.2);
        let sq = duration_stretch(QUARTER, QUARTER, 1.2);
        let sh = duration_stretch(HALF, QUARTER, 1.2);
        let sw = duration_stretch(WHOLE, QUARTER, 1.2);

        assert!(s16 < s8);
        assert!(s8 < sq);
        assert!(sq < sh);
        assert!(sh < sw);
    }

    #[test]
    fn test_stretch_edge_cases() {
        // Zero/negative ticks should return 1.0
        assert_eq!(duration_stretch(0.0, QUARTER, 1.2), 1.0);
        assert_eq!(duration_stretch(-100.0, QUARTER, 1.2), 1.0);
        assert_eq!(duration_stretch(QUARTER, 0.0, 1.2), 1.0);
        assert_eq!(duration_stretch(QUARTER, QUARTER, 0.0), 1.0);
    }

    #[test]
    fn test_slope_1_gives_uniform() {
        // With slope=1.0, all durations should give stretch=1.0
        assert!((duration_stretch(EIGHTH, QUARTER, 1.0) - 1.0).abs() < 1e-10);
        assert!((duration_stretch(WHOLE, QUARTER, 1.0) - 1.0).abs() < 1e-10);
    }

    // --- Natural Width ---

    #[test]
    fn test_natural_width_quarter() {
        let spatium = 5.0;
        let width = natural_width(QUARTER, spatium, 1.2, 1.0, 1.0);
        let expected = 3.5 * spatium; // 17.5
        assert!(
            (width - expected).abs() < 1e-10,
            "Quarter natural width should be {expected}, got {width}"
        );
    }

    #[test]
    fn test_natural_width_half() {
        let spatium = 5.0;
        let width = natural_width(HALF, spatium, 1.2, 1.0, 1.0);
        let expected = 3.5 * spatium * 1.2; // 21.0
        assert!(
            (width - expected).abs() < 1e-10,
            "Half natural width should be {expected}, got {width}"
        );
    }

    #[test]
    fn test_natural_width_density() {
        let spatium = 5.0;
        let normal = natural_width(QUARTER, spatium, 1.2, 1.0, 1.0);
        let dense = natural_width(QUARTER, spatium, 1.2, 2.0, 1.0);
        assert!(
            (dense - normal / 2.0).abs() < 1e-10,
            "Density 2.0 should halve width"
        );
    }

    #[test]
    fn test_natural_width_stretch_reduction() {
        let spatium = 5.0;
        let normal = natural_width(QUARTER, spatium, 1.2, 1.0, 1.0);
        let squeezed = natural_width(QUARTER, spatium, 1.2, 1.0, 0.5);
        assert!(
            (squeezed - normal * 0.5).abs() < 1e-10,
            "stretch_reduction 0.5 should halve width"
        );
    }

    // --- Spring Model ---

    #[test]
    fn test_spring_creation() {
        let spring = Spring::new(1.2, 21.0, 0);
        assert!((spring.spring_const - 1.0 / 1.2).abs() < 1e-10);
        assert!((spring.pre_tension - 21.0 / 1.2).abs() < 1e-10);
    }

    #[test]
    fn test_stretch_empty_springs() {
        let mut springs: Vec<Spring> = Vec::new();
        let results = stretch_segments_to_width(&mut springs, 100.0);
        assert!(results.is_empty());
    }

    #[test]
    fn test_stretch_zero_extra() {
        let mut springs = vec![Spring::new(1.0, 50.0, 0)];
        let results = stretch_segments_to_width(&mut springs, 0.0);
        assert!(results.is_empty());
    }

    #[test]
    fn test_stretch_single_spring() {
        let mut springs = vec![Spring::new(1.0, 50.0, 0)];
        let results = stretch_segments_to_width(&mut springs, 30.0);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 0);
        assert!(
            (results[0].1 - 80.0).abs() < 1e-10,
            "Single spring should absorb all extra: got {}",
            results[0].1
        );
    }

    #[test]
    fn test_stretch_proportional() {
        // Two springs: one stiff (short note), one soft (long note)
        // Short note (eighth): stretch=0.833, k=1.2
        // Long note (half): stretch=1.2, k=0.833
        let eighth_stretch = duration_stretch(EIGHTH, QUARTER, 1.2);
        let half_stretch = duration_stretch(HALF, QUARTER, 1.2);

        let mut springs = vec![
            Spring::new(eighth_stretch, 14.58, 0), // eighth note width
            Spring::new(half_stretch, 21.0, 1),    // half note width
        ];

        let results = stretch_segments_to_width(&mut springs, 20.0);

        // Both should expand, but the half note (lower k) should expand more
        let eighth_new = results.iter().find(|(i, _)| *i == 0).map(|(_, w)| *w);
        let half_new = results.iter().find(|(i, _)| *i == 1).map(|(_, w)| *w);

        if let (Some(e), Some(h)) = (eighth_new, half_new) {
            let eighth_increase = e - 14.58;
            let half_increase = h - 21.0;
            assert!(
                half_increase > eighth_increase,
                "Half note should expand more: eighth +{eighth_increase:.2}, half +{half_increase:.2}"
            );
        }
    }

    #[test]
    fn test_stretch_preserves_total() {
        let mut springs = vec![
            Spring::new(1.0, 40.0, 0),
            Spring::new(1.5, 30.0, 1),
            Spring::new(0.8, 50.0, 2),
        ];

        let extra = 30.0;
        let original_total: f64 = springs.iter().map(|s| s.width).sum();
        let results = stretch_segments_to_width(&mut springs, extra);

        let new_total: f64 = results.iter().map(|(_, w)| w).sum();
        // Not all springs may activate, but the total of activated springs
        // should be at most original_total + extra
        assert!(
            new_total <= original_total + extra + 1e-6,
            "Total should not exceed original + extra"
        );
    }

    // --- Squeeze ---

    #[test]
    fn test_squeeze_params_step() {
        let params = SqueezeParams::default();
        assert_eq!(params.squeeze_factor, 1.0);
        assert_eq!(params.stretch_reduction, 1.0);

        let step1 = params.step().unwrap();
        assert!((step1.squeeze_factor - 0.8).abs() < 1e-10);
        assert!((step1.stretch_reduction - 0.67).abs() < 1e-2);

        let step2 = step1.step().unwrap();
        assert!((step2.squeeze_factor - 0.6).abs() < 1e-10);

        // Eventually exhausts
        let mut p = params;
        let mut count = 0;
        while let Some(next) = p.step() {
            p = next;
            count += 1;
            if count > 10 {
                panic!("Squeeze should exhaust");
            }
        }
        assert!(count >= 3, "Should have at least 3 steps");
    }

    // --- Squeeze Pipeline ---

    #[test]
    fn test_compute_segment_widths_basic() {
        let ticks = vec![QUARTER, QUARTER, QUARTER, QUARTER];
        let mins = vec![0.0; 4];
        let widths =
            compute_segment_widths(&ticks, &mins, 5.0, 1.2, 1.0, &SqueezeParams::default());

        // All quarters, so all should be equal
        let expected = natural_width(QUARTER, 5.0, 1.2, 1.0, 1.0);
        for w in &widths {
            assert!((*w - expected).abs() < 1e-10);
        }
    }

    #[test]
    fn test_compute_segment_widths_respects_min() {
        let ticks = vec![EIGHTH, EIGHTH];
        let mins = vec![30.0, 30.0]; // High mins
        let widths =
            compute_segment_widths(&ticks, &mins, 5.0, 1.2, 1.0, &SqueezeParams::default());

        // Natural width of eighth at spatium=5 is about 14.58, but min is 30
        for w in &widths {
            assert!(*w >= 30.0, "Width {w} should respect min 30.0");
        }
    }

    #[test]
    fn test_compute_segment_widths_with_squeeze() {
        let ticks = vec![QUARTER];
        let mins = vec![0.0];
        let normal =
            compute_segment_widths(&ticks, &mins, 5.0, 1.2, 1.0, &SqueezeParams::default());
        let squeezed = compute_segment_widths(
            &ticks,
            &mins,
            5.0,
            1.2,
            1.0,
            &SqueezeParams {
                squeeze_factor: 0.5,
                stretch_reduction: 0.5,
            },
        );

        assert!(squeezed[0] < normal[0], "Squeezed should be smaller");
    }

    #[test]
    fn test_squeeze_to_fit_no_squeeze_needed() {
        // 4 quarters at spatium=5 → ~70pt total, target 200pt → no squeeze
        let ticks = vec![QUARTER, QUARTER, QUARTER, QUARTER];
        let mins = vec![0.0; 4];
        let result = squeeze_to_fit(&ticks, &mins, 200.0, 5.0, 1.2, 1.0);

        assert!(result.fits);
        assert_eq!(result.params.squeeze_factor, 1.0);
        assert_eq!(result.params.stretch_reduction, 1.0);
    }

    #[test]
    fn test_squeeze_to_fit_phase1() {
        // 4 quarters at spatium=5 → ~70pt total, target 50pt → needs squeeze
        let ticks = vec![QUARTER, QUARTER, QUARTER, QUARTER];
        let mins = vec![0.0; 4];
        let result = squeeze_to_fit(&ticks, &mins, 50.0, 5.0, 1.2, 1.0);

        assert!(result.fits);
        assert!(result.params.squeeze_factor < 1.0, "Should have squeezed");
        let total: f64 = result.widths.iter().sum();
        assert!(total <= 50.0 + 1e-6, "Total {total} should fit in 50pt");
    }

    #[test]
    fn test_squeeze_to_fit_phase2() {
        // Very tight target — needs brute-force phase
        let ticks = vec![QUARTER, QUARTER, QUARTER, QUARTER];
        let mins = vec![0.0; 4];
        let result = squeeze_to_fit(&ticks, &mins, 5.0, 5.0, 1.2, 1.0);

        assert!(result.fits);
        let total: f64 = result.widths.iter().sum();
        assert!(total <= 5.0 + 1e-6, "Total {total} should fit in 5pt");
    }

    #[test]
    fn test_squeeze_system_no_squeeze() {
        let widths = vec![100.0, 120.0, 130.0];
        let result = squeeze_system(&widths, 400.0);
        assert!(result.is_none(), "No squeeze needed");
    }

    #[test]
    fn test_squeeze_system_proportional() {
        let widths = vec![100.0, 200.0];
        let result = squeeze_system(&widths, 150.0).unwrap();

        let total: f64 = result.iter().sum();
        assert!((total - 150.0).abs() < 1e-6, "Should sum to target");
        // Proportional: 100/300*150=50, 200/300*150=100
        assert!((result[0] - 50.0).abs() < 1e-6);
        assert!((result[1] - 100.0).abs() < 1e-6);
    }
}
