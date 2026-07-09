//! Horizontal spacing orchestrator.
//!
//! This module implements MuseScore's horizontal spacing algorithm, which:
//! 1. Computes minimum widths based on collision detection
//! 2. Builds spring rows from segments
//! 3. Stretches springs to fit available width
//! 4. Applies positions back to segments
//!
//! The algorithm uses a spring-based system where longer durations get more
//! space than shorter durations, while respecting minimum distances from
//! collision detection.
//!
//! ## Squeeze-to-Fit Algorithm
//!
//! When content doesn't fit, the algorithm progressively:
//! 1. Reduces stretch (duration-based spacing): 1.0 → 0.67 → 0.33 → 0.0
//! 2. Reduces squeeze (collision padding): 1.0 → 0.8 → 0.6 → 0.4 → 0.2 → 0.0
//!
//! This is based on MuseScore's `squeezeSystemToFit()` function.

use super::kerning::{KerningType, SpacingPadding};
use super::segment::Segment;
use super::segment_list::SegmentList;
use super::shape::Shape;
use super::springs::{HorizontalSpacingContext, SpacingConfig, Spring, SpringRow};

/// Step size for stretch reduction during squeeze-to-fit.
/// MuseScore uses 0.33, reducing stretch from 1.0 → 0.67 → 0.33 → 0.0
const STRETCH_REDUCTION_STEP: f64 = 0.33;

/// Step size for squeeze factor during squeeze-to-fit.
/// MuseScore uses 0.2, reducing squeeze from 1.0 → 0.8 → 0.6 → 0.4 → 0.2 → 0.0
const SQUEEZE_STEP: f64 = 0.2;

/// Smaller step for final collision-allowing squeeze.
const FINAL_SQUEEZE_STEP: f64 = 0.05;

/// Iterator for progressive squeeze-to-fit attempts.
///
/// Each iteration reduces both stretch and squeeze factors,
/// trying progressively tighter layouts until content fits.
#[derive(Debug, Clone)]
pub struct SqueezeIterator {
    stretch_reduction: f64,
    squeeze_factor: f64,
    phase: SqueezePhase,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SqueezePhase {
    /// Normal squeeze phase: reduce both stretch and squeeze
    Normal,
    /// Final phase: allow collisions by further reducing width
    AllowCollisions,
    /// Done: no more squeeze steps available
    Done,
}

impl SqueezeIterator {
    /// Create a new squeeze iterator starting at normal spacing.
    #[must_use]
    pub fn new() -> Self {
        Self {
            stretch_reduction: 1.0,
            squeeze_factor: 1.0 - SQUEEZE_STEP, // Start with first squeeze step
            phase: SqueezePhase::Normal,
        }
    }

    /// Get the current stretch reduction factor.
    #[must_use]
    pub fn stretch_reduction(&self) -> f64 {
        self.stretch_reduction
    }

    /// Get the current squeeze factor.
    #[must_use]
    pub fn squeeze_factor(&self) -> f64 {
        self.squeeze_factor
    }

    /// Check if we're in the collision-allowing phase.
    #[must_use]
    pub fn allows_collisions(&self) -> bool {
        self.phase == SqueezePhase::AllowCollisions
    }

    /// Check if we've exhausted all squeeze options.
    #[must_use]
    pub fn is_done(&self) -> bool {
        self.phase == SqueezePhase::Done
    }

    /// Create a context for the current squeeze level.
    #[must_use]
    pub fn context(&self) -> HorizontalSpacingContext {
        HorizontalSpacingContext::for_squeeze(self.stretch_reduction, self.squeeze_factor)
    }
}

impl Default for SqueezeIterator {
    fn default() -> Self {
        Self::new()
    }
}

impl Iterator for SqueezeIterator {
    type Item = (f64, f64); // (stretch_reduction, squeeze_factor)

    fn next(&mut self) -> Option<Self::Item> {
        match self.phase {
            SqueezePhase::Normal => {
                // Clamp to non-negative values
                let result = (
                    self.stretch_reduction.max(0.0),
                    self.squeeze_factor.max(0.0),
                );

                // Advance to next squeeze level
                self.squeeze_factor -= SQUEEZE_STEP;
                self.stretch_reduction -= STRETCH_REDUCTION_STEP;

                // Check if we've exhausted normal squeeze
                if self.squeeze_factor < 0.0 {
                    self.phase = SqueezePhase::AllowCollisions;
                    self.squeeze_factor = 1.0 - FINAL_SQUEEZE_STEP;
                }

                Some(result)
            }
            SqueezePhase::AllowCollisions => {
                let result = (0.0, self.squeeze_factor.max(0.0)); // Stretch is 0 in this phase

                // Continue reducing until we can't anymore
                self.squeeze_factor -= FINAL_SQUEEZE_STEP;

                if self.squeeze_factor < 0.0 {
                    self.phase = SqueezePhase::Done;
                }

                Some(result)
            }
            SqueezePhase::Done => None,
        }
    }
}

/// Result of horizontal spacing computation.
#[derive(Debug, Clone)]
pub struct SpacingResult {
    /// Total width after spacing
    pub total_width: f64,
    /// Whether spacing was compressed (squeezed)
    pub is_compressed: bool,
    /// Compression ratio if squeezed (1.0 = no compression)
    pub compression_ratio: f64,
}

/// Horizontal spacing engine.
///
/// Orchestrates the spring-based spacing algorithm, computing optimal
/// horizontal positions for all segments in a measure or system.
#[derive(Debug, Clone)]
pub struct HorizontalSpacing {
    /// Spacing configuration
    pub config: SpacingConfig,
    /// Padding values between elements
    pub padding: SpacingPadding,
    /// Minimum distance between noteheads
    pub min_note_distance: f64,
}

impl Default for HorizontalSpacing {
    fn default() -> Self {
        Self {
            config: SpacingConfig::default(),
            padding: SpacingPadding::default(),
            min_note_distance: 0.35, // In spatiums
        }
    }
}

impl HorizontalSpacing {
    /// Create a new horizontal spacing engine with the given spatium.
    #[must_use]
    pub fn new(spatium: f64) -> Self {
        Self {
            config: SpacingConfig::new(spatium),
            ..Default::default()
        }
    }

    /// Create with custom configuration.
    #[must_use]
    pub fn with_config(config: SpacingConfig, padding: SpacingPadding) -> Self {
        Self {
            config,
            padding,
            min_note_distance: 0.35,
        }
    }

    /// Compute spacing for segments to fit within available width.
    ///
    /// This is the main entry point for horizontal spacing. It:
    /// 1. Computes minimum widths from collision detection
    /// 2. Calculates natural widths from duration stretch
    /// 3. Builds spring row and stretches to fit
    /// 4. Updates segment positions
    ///
    /// # Arguments
    /// * `segments` - The segments to space (modified in place)
    /// * `available_width` - Target width to fill
    /// * `justify` - Whether to stretch to fill the width
    ///
    /// # Returns
    /// Spacing result with total width and compression info
    pub fn compute_spacing(
        &self,
        segments: &mut SegmentList,
        available_width: f64,
        justify: bool,
    ) -> SpacingResult {
        if segments.is_empty() {
            return SpacingResult {
                total_width: 0.0,
                is_compressed: false,
                compression_ratio: 1.0,
            };
        }

        // Phase 1: Compute minimum widths from collision detection
        self.compute_minimum_widths(segments);

        // Phase 2: Calculate natural widths from duration
        self.compute_natural_widths(segments);

        // Phase 3: Ensure minimum widths are respected
        self.apply_minimum_widths(segments);

        // Get total natural width
        let natural_width = segments.total_width();

        // Phase 4: Adjust to fit available width
        let (final_width, compression_ratio) = if justify && available_width > natural_width {
            // Stretch to fill
            let extra = available_width - natural_width;
            self.stretch_segments(segments, extra);
            (available_width, 1.0)
        } else if natural_width > available_width {
            // Need to compress
            let ratio = available_width / natural_width;
            self.compress_segments(segments, ratio);
            (available_width, ratio)
        } else {
            (natural_width, 1.0)
        };

        // Phase 5: Compute final x positions
        segments.compute_x_positions();

        SpacingResult {
            total_width: final_width,
            is_compressed: compression_ratio < 1.0,
            compression_ratio,
        }
    }

    /// Compute minimum widths based on collision detection between segments.
    fn compute_minimum_widths(&self, segments: &mut SegmentList) {
        let padding_px = self.padding.to_pixels(self.config.spatium);
        let len = segments.len();

        for i in 0..len.saturating_sub(1) {
            // Get shapes from current and next segment
            let current_shape = segments.get(i).map(|s| s.combined_shape());
            let next_shape = segments.get(i + 1).map(|s| s.combined_shape());

            if let (Some(curr), Some(next)) = (current_shape, next_shape) {
                // Get kerning type (default to standard kerning)
                let kerning = segments
                    .get(i)
                    .map(|s| s.kerning)
                    .unwrap_or(KerningType::Kerning);

                // Calculate minimum distance
                let min_spacing = padding_px.for_kerning_type(kerning);
                let min_distance = curr.min_horizontal_distance(&next, min_spacing);

                // Store as minimum width
                if let Some(seg) = segments.get_mut(i) {
                    seg.min_width = min_distance.max(seg.min_width);
                }
            }
        }
    }

    /// Compute natural widths from duration stretch.
    fn compute_natural_widths(&self, segments: &mut SegmentList) {
        for segment in segments.iter_mut() {
            if segment.ticks > 0 {
                // Duration-based width
                let stretch = self.config.duration_stretch(segment.ticks);
                segment.stretch = stretch;
                segment.width = self.config.natural_width(stretch, segment.user_stretch);
            } else {
                // Non-timed elements get minimum width
                segment.stretch = 0.0;
                segment.width = segment.min_width.max(self.config.spatium * 0.5);
            }
        }
    }

    /// Apply minimum widths to ensure no collisions.
    fn apply_minimum_widths(&self, segments: &mut SegmentList) {
        for segment in segments.iter_mut() {
            if segment.width < segment.min_width {
                segment.width = segment.min_width;
            }
        }
    }

    /// Stretch segments to add extra width using spring system.
    fn stretch_segments(&self, segments: &mut SegmentList, extra_width: f64) {
        if extra_width <= 0.0 {
            return;
        }

        // Build spring row from segments
        let mut spring_row = SpringRow::with_capacity(segments.len());

        for (i, segment) in segments.iter().enumerate() {
            // Only add springs for stretchable segments (those with duration)
            if segment.stretch > 0.0 {
                spring_row.push(Spring::from_segment(segment, i));
            }
        }

        if spring_row.is_empty() {
            // No stretchable segments - distribute evenly
            let per_segment = extra_width / segments.len() as f64;
            for segment in segments.iter_mut() {
                segment.width += per_segment;
            }
            return;
        }

        // Solve spring system
        let stretched = spring_row.stretch(extra_width);

        // Apply stretched widths
        for (index, new_width) in stretched {
            if let Some(segment) = segments.get_mut(index) {
                segment.width = new_width + segment.width_offset;
            }
        }
    }

    /// Compress segments to fit within available width.
    fn compress_segments(&self, segments: &mut SegmentList, ratio: f64) {
        let ratio = ratio.max(0.5); // Don't compress below 50%

        for segment in segments.iter_mut() {
            // Compress proportionally, but respect minimum widths
            let compressed = segment.width * ratio;
            segment.width = compressed.max(segment.min_width);
        }
    }

    /// Compute spacing for a single measure.
    ///
    /// Convenience method that computes spacing without justification,
    /// returning the natural width needed for the measure.
    pub fn compute_measure_width(&self, segments: &mut SegmentList) -> f64 {
        let result = self.compute_spacing(segments, f64::MAX, false);
        result.total_width
    }

    /// Compute justified spacing for a system line.
    ///
    /// Stretches segments to fill the available width exactly.
    pub fn justify_system(&self, segments: &mut SegmentList, system_width: f64) {
        self.compute_spacing(segments, system_width, true);
    }

    /// Squeeze segments to fit within target width.
    ///
    /// This implements MuseScore's `squeezeSystemToFit()` algorithm:
    /// 1. Progressively reduce stretch (duration spacing): 1.0 → 0.67 → 0.33 → 0.0
    /// 2. Progressively reduce squeeze (collision padding): 1.0 → 0.8 → 0.6 → ...
    /// 3. If still too wide, allow collisions and continue reducing
    ///
    /// # Arguments
    /// * `segments` - The segments to space
    /// * `target_width` - The width to fit within
    ///
    /// # Returns
    /// The final width achieved (may still be > target if content can't fit)
    pub fn squeeze_to_fit(&self, segments: &mut SegmentList, target_width: f64) -> f64 {
        // First try normal spacing
        let result = self.compute_spacing(segments, target_width, false);
        if result.total_width <= target_width {
            return result.total_width;
        }

        // Progressive squeeze
        for (stretch_reduction, squeeze_factor) in SqueezeIterator::new() {
            let ctx = HorizontalSpacingContext::for_squeeze(stretch_reduction, squeeze_factor);
            let width = self.compute_spacing_with_context(segments, target_width, &ctx);

            if width <= target_width {
                return width;
            }
        }

        // Still too wide - return best effort
        segments.total_width()
    }

    /// Compute spacing with a specific context.
    ///
    /// This allows fine-grained control over stretch and squeeze factors.
    pub fn compute_spacing_with_context(
        &self,
        segments: &mut SegmentList,
        _available_width: f64,
        ctx: &HorizontalSpacingContext,
    ) -> f64 {
        if segments.is_empty() {
            return 0.0;
        }

        // Phase 1: Compute minimum widths from collision detection
        self.compute_minimum_widths_with_squeeze(segments, ctx.squeeze_factor);

        // Phase 2: Calculate natural widths from duration with stretch reduction
        self.compute_natural_widths_with_context(segments, ctx);

        // Phase 3: Ensure minimum widths are respected
        self.apply_minimum_widths(segments);

        // Phase 4: Compute final x positions
        segments.compute_x_positions();

        segments.total_width()
    }

    /// Compute minimum widths with squeeze factor applied to padding.
    fn compute_minimum_widths_with_squeeze(&self, segments: &mut SegmentList, squeeze_factor: f64) {
        let base_padding_px = self.padding.to_pixels(self.config.spatium);

        let len = segments.len();

        for i in 0..len.saturating_sub(1) {
            let current_shape = segments.get(i).map(|s| s.combined_shape());
            let next_shape = segments.get(i + 1).map(|s| s.combined_shape());

            if let (Some(curr), Some(next)) = (current_shape, next_shape) {
                let kerning = segments
                    .get(i)
                    .map(|s| s.kerning)
                    .unwrap_or(KerningType::Kerning);

                // Get base padding and apply squeeze factor
                let base_spacing = base_padding_px.for_kerning_type(kerning);
                let min_spacing = base_spacing * squeeze_factor;
                let min_distance = curr.min_horizontal_distance(&next, min_spacing);

                if let Some(seg) = segments.get_mut(i) {
                    seg.min_width = min_distance.max(seg.min_width);
                }
            }
        }
    }

    /// Compute natural widths with context-aware stretch and density.
    fn compute_natural_widths_with_context(
        &self,
        segments: &mut SegmentList,
        ctx: &HorizontalSpacingContext,
    ) {
        for segment in segments.iter_mut() {
            if segment.ticks > 0 {
                // Duration-based width with context adjustments
                let base_stretch = self.config.duration_stretch(segment.ticks);
                let total_stretch = base_stretch * segment.user_stretch.clamp(0.1, 10.0);

                segment.stretch = total_stretch;

                // Apply stretch reduction and density from context
                let natural_width = self.config.spatium
                    * self.config.quarter_note_space
                    * total_stretch
                    * ctx.stretch_reduction
                    / ctx.spacing_density.max(0.1);

                segment.width = natural_width;
            } else {
                // Non-timed elements get minimum width
                segment.stretch = 0.0;
                segment.width = segment.min_width.max(self.config.spatium * 0.5);
            }
        }
    }

    /// Distribute extra space backward from a collision point.
    ///
    /// When segment N collides with segment N-1, this distributes the
    /// required extra space backward across all previous ChordRest segments.
    ///
    /// This is based on MuseScore's `spaceAgainstPreviousSegments()`.
    pub fn distribute_backward(
        &self,
        segments: &mut SegmentList,
        collision_index: usize,
        extra_space: f64,
    ) {
        if extra_space <= 0.0 || collision_index == 0 {
            return;
        }

        // Count ChordRest segments before collision point
        let mut cr_count = 0;
        for i in 0..collision_index {
            if let Some(seg) = segments.get(i)
                && seg.ticks > 0
            {
                cr_count += 1;
            }
        }

        if cr_count == 0 {
            return;
        }

        // Distribute evenly across ChordRest segments
        let per_segment = extra_space / cr_count as f64;

        for i in 0..collision_index {
            if let Some(seg) = segments.get_mut(i)
                && seg.ticks > 0
            {
                seg.width += per_segment;
            }
        }
    }
}

/// Minimum distance calculator for segment pairs.
///
/// Computes the minimum horizontal distance between two segments
/// based on their shapes and kerning rules.
#[derive(Debug)]
pub struct MinimumDistance {
    /// Left segment shape
    pub left_shape: Shape,
    /// Right segment shape
    pub right_shape: Shape,
    /// Kerning type
    pub kerning: KerningType,
    /// Base padding in pixels
    pub padding: f64,
}

impl MinimumDistance {
    /// Create a new minimum distance calculator.
    #[must_use]
    pub fn new(left: &Segment, right: &Segment, padding: f64) -> Self {
        Self {
            left_shape: left.combined_shape(),
            right_shape: right.combined_shape(),
            kerning: left.kerning,
            padding,
        }
    }

    /// Calculate the minimum distance.
    #[must_use]
    pub fn calculate(&self) -> f64 {
        match self.kerning {
            KerningType::AllowCollision => 0.0,
            KerningType::NonKerning => {
                // Full separation - no overlap allowed
                let left_width = self.left_shape.right() - self.left_shape.left();
                left_width + self.padding
            }
            _ => {
                // Standard kerning - use collision detection
                self.left_shape
                    .min_horizontal_distance(&self.right_shape, self.padding)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_segment(tick: i32, ticks: i32, width: f64) -> Segment {
        let mut seg = Segment::chord_rest(tick, ticks);
        seg.width = width;
        seg
    }

    #[test]
    fn test_horizontal_spacing_new() {
        let spacing = HorizontalSpacing::new(10.0);
        assert_eq!(spacing.config.spatium, 10.0);
    }

    #[test]
    fn test_compute_spacing_empty() {
        let spacing = HorizontalSpacing::new(10.0);
        let mut segments = SegmentList::new();

        let result = spacing.compute_spacing(&mut segments, 100.0, true);

        assert_eq!(result.total_width, 0.0);
        assert!(!result.is_compressed);
    }

    #[test]
    fn test_compute_natural_widths() {
        let spacing = HorizontalSpacing::new(10.0);
        let mut segments = SegmentList::new();

        // Quarter note (480 ticks)
        segments.push(create_test_segment(0, 480, 0.0));
        // Half note (960 ticks)
        segments.push(create_test_segment(480, 960, 0.0));

        spacing.compute_natural_widths(&mut segments);

        // Half note should be wider than quarter note
        assert!(segments[1].width > segments[0].width);
    }

    #[test]
    fn test_stretch_segments() {
        let spacing = HorizontalSpacing::new(10.0);
        let mut segments = SegmentList::new();

        // Two quarter notes
        let mut seg1 = create_test_segment(0, 480, 35.0);
        seg1.stretch = 1.0;
        let mut seg2 = create_test_segment(480, 480, 35.0);
        seg2.stretch = 1.0;

        segments.push(seg1);
        segments.push(seg2);

        let initial_width = segments.total_width();
        spacing.stretch_segments(&mut segments, 30.0);
        let final_width = segments.total_width();

        // Width should have increased
        assert!(final_width > initial_width);
    }

    #[test]
    fn test_compress_segments() {
        let spacing = HorizontalSpacing::new(10.0);
        let mut segments = SegmentList::new();

        let mut seg = create_test_segment(0, 480, 100.0);
        seg.min_width = 50.0;
        segments.push(seg);

        spacing.compress_segments(&mut segments, 0.6);

        // Should be compressed to 60% but not below min_width
        assert!(segments[0].width >= 50.0);
        assert!(segments[0].width <= 100.0);
    }

    #[test]
    fn test_justify_system() {
        let spacing = HorizontalSpacing::new(10.0);
        let mut segments = SegmentList::new();

        let mut seg1 = create_test_segment(0, 480, 35.0);
        seg1.stretch = 1.0;
        let mut seg2 = create_test_segment(480, 480, 35.0);
        seg2.stretch = 1.0;

        segments.push(seg1);
        segments.push(seg2);

        spacing.justify_system(&mut segments, 200.0);

        // Total width should be close to target (200.0)
        let total = segments.total_width();
        assert!((total - 200.0).abs() < 1.0);
    }

    #[test]
    fn test_minimum_distance() {
        let seg1 = create_test_segment(0, 480, 50.0);
        let seg2 = create_test_segment(480, 480, 50.0);

        let min_dist = MinimumDistance::new(&seg1, &seg2, 5.0);
        let distance = min_dist.calculate();

        // Should return a reasonable distance
        assert!(distance >= 0.0);
    }

    #[test]
    fn test_minimum_distance_allow_collision() {
        let mut seg1 = create_test_segment(0, 480, 50.0);
        seg1.kerning = KerningType::AllowCollision;
        let seg2 = create_test_segment(480, 480, 50.0);

        let min_dist = MinimumDistance::new(&seg1, &seg2, 5.0);
        let distance = min_dist.calculate();

        assert_eq!(distance, 0.0);
    }

    #[test]
    fn test_x_positions_computed() {
        let spacing = HorizontalSpacing::new(10.0);
        let mut segments = SegmentList::new();

        let mut seg1 = create_test_segment(0, 480, 50.0);
        seg1.stretch = 1.0;
        let mut seg2 = create_test_segment(480, 480, 60.0);
        seg2.stretch = 1.0;

        segments.push(seg1);
        segments.push(seg2);

        spacing.compute_spacing(&mut segments, 200.0, false);

        // First segment should be at x=0
        assert_eq!(segments[0].x, 0.0);
        // Second segment should be after first
        assert!(segments[1].x > 0.0);
    }

    #[test]
    fn test_squeeze_iterator_initial() {
        let iter = SqueezeIterator::new();
        assert!(!iter.is_done());
        assert!(!iter.allows_collisions());
        assert!((iter.stretch_reduction() - 1.0).abs() < 1e-10);
        assert!((iter.squeeze_factor() - 0.8).abs() < 1e-10); // 1.0 - SQUEEZE_STEP
    }

    #[test]
    fn test_squeeze_iterator_progression() {
        let mut iter = SqueezeIterator::new();
        let mut count = 0;

        // Collect all squeeze levels
        for (stretch, squeeze) in iter.by_ref() {
            count += 1;
            // Stretch should decrease
            assert!(stretch >= 0.0);
            assert!(stretch <= 1.0);
            // Squeeze should be between 0 and 1
            assert!(squeeze >= -0.1); // Allow small negative due to floating point
            assert!(squeeze <= 1.0);

            if count > 50 {
                // Safety: prevent infinite loop
                break;
            }
        }

        // Should have multiple squeeze levels
        assert!(count > 3);
        // Iterator should be done
        assert!(iter.is_done());
    }

    #[test]
    fn test_squeeze_iterator_enters_collision_phase() {
        let mut iter = SqueezeIterator::new();

        // Consume normal phase
        while !iter.allows_collisions() && iter.next().is_some() {}

        // Should enter collision phase eventually
        assert!(iter.allows_collisions() || iter.is_done());
    }

    #[test]
    fn test_squeeze_to_fit_normal() {
        let spacing = HorizontalSpacing::new(10.0);
        let mut segments = SegmentList::new();

        // Create segments that easily fit
        let mut seg1 = create_test_segment(0, 480, 30.0);
        seg1.stretch = 1.0;
        let mut seg2 = create_test_segment(480, 480, 30.0);
        seg2.stretch = 1.0;

        segments.push(seg1);
        segments.push(seg2);

        // Should fit without squeezing
        let width = spacing.squeeze_to_fit(&mut segments, 500.0);
        assert!(width <= 500.0);
    }

    #[test]
    fn test_squeeze_to_fit_requires_squeeze() {
        let spacing = HorizontalSpacing::new(10.0);
        let mut segments = SegmentList::new();

        // Create segments that need squeezing
        // Quarter note at spatium=10 with quarter_note_space=3.5 = 35px natural width
        segments.push(create_test_segment(0, 480, 0.0));
        segments.push(create_test_segment(480, 480, 0.0));
        segments.push(create_test_segment(960, 480, 0.0));
        segments.push(create_test_segment(1440, 480, 0.0));

        // Very tight target - will need squeezing
        let target = 80.0;
        let width = spacing.squeeze_to_fit(&mut segments, target);

        // Should get close to target (may not be exact if can't squeeze enough)
        // But should be less than unsqueezed width
        let unsqueezed = spacing.compute_measure_width(&mut segments);
        assert!(width < unsqueezed || width <= target);
    }

    #[test]
    fn test_spacing_context_apply_stretch() {
        let ctx = HorizontalSpacingContext::for_squeeze(0.67, 0.8);

        let width = ctx.apply_stretch(100.0);
        // 100 * 0.67 / 1.0 = 67
        assert!((width - 67.0).abs() < 1e-10);
    }

    #[test]
    fn test_spacing_context_apply_squeeze() {
        let ctx = HorizontalSpacingContext::for_squeeze(1.0, 0.5);

        let padding = ctx.apply_squeeze(10.0);
        assert!((padding - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_distribute_backward() {
        let spacing = HorizontalSpacing::new(10.0);
        let mut segments = SegmentList::new();

        // 3 ChordRest segments
        let mut seg0 = create_test_segment(0, 480, 35.0);
        seg0.stretch = 1.0;
        let mut seg1 = create_test_segment(480, 480, 35.0);
        seg1.stretch = 1.0;
        let mut seg2 = create_test_segment(960, 480, 35.0);
        seg2.stretch = 1.0;

        segments.push(seg0);
        segments.push(seg1);
        segments.push(seg2);

        let initial_width: f64 = segments.iter().map(|s| s.width).sum();

        // Distribute 30px backward from segment 2
        spacing.distribute_backward(&mut segments, 2, 30.0);

        let final_width: f64 = segments.iter().map(|s| s.width).sum();

        // Total width should increase by 30
        assert!((final_width - initial_width - 30.0).abs() < 1e-10);

        // First two segments should each get 15 (30 / 2 CR segments before index 2)
        assert!((segments[0].width - 50.0).abs() < 1e-10);
        assert!((segments[1].width - 50.0).abs() < 1e-10);
        // Last segment unchanged
        assert!((segments[2].width - 35.0).abs() < 1e-10);
    }
}
