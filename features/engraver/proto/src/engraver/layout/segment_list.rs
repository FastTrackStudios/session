//! Container for segments within a measure.
//!
//! The SegmentList maintains segments in sorted order by tick and type,
//! providing efficient access and iteration for layout algorithms.

use super::segment::{Segment, SegmentType};
use crate::engraver::layout::chart::spacing;

/// A container for segments within a measure.
///
/// Segments are maintained in sorted order (by tick, then by type).
/// This enables efficient iteration and lookup for layout algorithms.
#[derive(Debug, Clone, Default)]
pub struct SegmentList {
    segments: Vec<Segment>,
}

impl SegmentList {
    /// Create a new empty segment list.
    #[must_use]
    pub fn new() -> Self {
        Self {
            segments: Vec::new(),
        }
    }

    /// Create a segment list with pre-allocated capacity.
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            segments: Vec::with_capacity(capacity),
        }
    }

    /// Create a segment list from a pre-sorted vector.
    ///
    /// The caller is responsible for ensuring the segments are sorted
    /// by tick and then by type. No validation is performed in release builds.
    #[must_use]
    pub fn from_sorted(segments: Vec<Segment>) -> Self {
        debug_assert!(
            segments.windows(2).all(|w| w[0] <= w[1]),
            "Segments must be sorted"
        );
        Self { segments }
    }

    /// Clear all segments from the list.
    pub fn clear(&mut self) {
        self.segments.clear();
    }

    /// Get the number of segments.
    #[must_use]
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// Check if the list is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// Get a segment by index.
    #[must_use]
    pub fn get(&self, index: usize) -> Option<&Segment> {
        self.segments.get(index)
    }

    /// Get a mutable segment by index.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut Segment> {
        self.segments.get_mut(index)
    }

    /// Get the first segment.
    #[must_use]
    pub fn first(&self) -> Option<&Segment> {
        self.segments.first()
    }

    /// Get the first mutable segment.
    pub fn first_mut(&mut self) -> Option<&mut Segment> {
        self.segments.first_mut()
    }

    /// Get the last segment.
    #[must_use]
    pub fn last(&self) -> Option<&Segment> {
        self.segments.last()
    }

    /// Get the last mutable segment.
    pub fn last_mut(&mut self) -> Option<&mut Segment> {
        self.segments.last_mut()
    }

    /// Get the first active segment (enabled and visible, not a time tick).
    #[must_use]
    pub fn first_active(&self) -> Option<&Segment> {
        self.segments.iter().find(|s| s.is_active())
    }

    /// Get the first segment of a specific type.
    #[must_use]
    pub fn first_of_type(&self, seg_type: SegmentType) -> Option<&Segment> {
        self.segments
            .iter()
            .find(|s| s.seg_type.intersects(seg_type))
    }

    /// Get the last segment of a specific type.
    #[must_use]
    pub fn last_of_type(&self, seg_type: SegmentType) -> Option<&Segment> {
        self.segments
            .iter()
            .rev()
            .find(|s| s.seg_type.intersects(seg_type))
    }

    /// Get the first ChordRest segment.
    #[must_use]
    pub fn first_chord_rest(&self) -> Option<&Segment> {
        self.first_of_type(SegmentType::CHORD_REST)
    }

    /// Insert a segment in sorted order.
    /// Returns the index where the segment was inserted.
    pub fn insert(&mut self, segment: Segment) -> usize {
        // Find the insertion point (binary search for efficiency)
        let pos = self
            .segments
            .binary_search_by(|s| s.cmp_order(&segment))
            .unwrap_or_else(|pos| pos);

        self.segments.insert(pos, segment);
        pos
    }

    /// Push a segment to the end (assumes it's the latest).
    /// Use `insert` if the segment might not be the latest.
    pub fn push(&mut self, segment: Segment) {
        // Verify ordering in debug builds
        debug_assert!(
            self.segments.last().is_none_or(|last| last <= &segment),
            "Segment pushed out of order"
        );
        self.segments.push(segment);
    }

    /// Remove a segment at the given index.
    pub fn remove(&mut self, index: usize) -> Option<Segment> {
        if index < self.segments.len() {
            Some(self.segments.remove(index))
        } else {
            None
        }
    }

    /// Remove and return the first segment matching a predicate.
    pub fn remove_first<F>(&mut self, predicate: F) -> Option<Segment>
    where
        F: Fn(&Segment) -> bool,
    {
        if let Some(pos) = self.segments.iter().position(predicate) {
            Some(self.segments.remove(pos))
        } else {
            None
        }
    }

    /// Find the index of a segment at a given tick and type.
    #[must_use]
    pub fn find_index(&self, tick: i32, seg_type: SegmentType) -> Option<usize> {
        self.segments
            .iter()
            .position(|s| s.tick == tick && s.seg_type == seg_type)
    }

    /// Find a segment at a given tick and type.
    #[must_use]
    pub fn find(&self, tick: i32, seg_type: SegmentType) -> Option<&Segment> {
        self.find_index(tick, seg_type)
            .and_then(|i| self.segments.get(i))
    }

    /// Find a mutable segment at a given tick and type.
    pub fn find_mut(&mut self, tick: i32, seg_type: SegmentType) -> Option<&mut Segment> {
        self.find_index(tick, seg_type)
            .and_then(|i| self.segments.get_mut(i))
    }

    /// Find or create a segment at the given tick and type.
    /// Returns a mutable reference to the segment.
    pub fn find_or_create(&mut self, tick: i32, seg_type: SegmentType) -> &mut Segment {
        if let Some(idx) = self.find_index(tick, seg_type) {
            &mut self.segments[idx]
        } else {
            let mut segment = Segment::new(seg_type);
            segment.tick = tick;
            let idx = self.insert(segment);
            &mut self.segments[idx]
        }
    }

    /// Get all segments at a given tick.
    #[must_use]
    pub fn at_tick(&self, tick: i32) -> Vec<&Segment> {
        self.segments.iter().filter(|s| s.tick == tick).collect()
    }

    /// Get all segments of a given type.
    #[must_use]
    pub fn of_type(&self, seg_type: SegmentType) -> Vec<&Segment> {
        self.segments
            .iter()
            .filter(|s| s.seg_type.intersects(seg_type))
            .collect()
    }

    /// Iterate over all segments.
    pub fn iter(&self) -> impl Iterator<Item = &Segment> {
        self.segments.iter()
    }

    /// Iterate mutably over all segments.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Segment> {
        self.segments.iter_mut()
    }

    /// Iterate over active segments only.
    pub fn iter_active(&self) -> impl Iterator<Item = &Segment> {
        self.segments.iter().filter(|s| s.is_active())
    }

    /// Iterate over segments of a specific type.
    pub fn iter_type(&self, seg_type: SegmentType) -> impl Iterator<Item = &Segment> {
        self.segments
            .iter()
            .filter(move |s| s.seg_type.intersects(seg_type))
    }

    /// Get the tick range covered by this segment list.
    #[must_use]
    pub fn tick_range(&self) -> Option<(i32, i32)> {
        let first = self.segments.first()?;
        let last = self.segments.last()?;
        Some((first.tick, last.tick + last.ticks))
    }

    /// Get the total width of all segments including leading space.
    #[must_use]
    pub fn total_width(&self) -> f64 {
        self.segments
            .iter()
            .map(|s| s.width + s.extra_leading_space + s.spacing + s.width_offset)
            .sum()
    }

    /// Compute x positions for all segments based on their widths.
    /// Starts from x = 0, accounting for extra_leading_space on each segment.
    pub fn compute_x_positions(&mut self) {
        let mut x = 0.0;
        for segment in &mut self.segments {
            // Add extra leading space before this segment
            x += segment.extra_leading_space;
            segment.x = x;
            x += segment.width + segment.spacing + segment.width_offset;
        }
    }

    /// Compute x positions starting from a given offset.
    pub fn compute_x_positions_from(&mut self, start_x: f64) {
        let mut x = start_x;
        for segment in &mut self.segments {
            // Add extra leading space before this segment
            x += segment.extra_leading_space;
            segment.x = x;
            x += segment.width + segment.spacing + segment.width_offset;
        }
    }

    /// Compute x positions using duration-proportional natural widths and spring justification.
    ///
    /// For each ChordRest segment, computes a natural width based on its tick duration
    /// using the power-law formula from `spacing::natural_width`. If the total natural
    /// width is less than `available_width`, the excess is distributed via spring-based
    /// justification so that longer-duration segments expand more.
    ///
    /// Non-ChordRest segments keep their existing widths.
    ///
    /// # Arguments
    /// * `available_width` - Total width available for the measure's content
    /// * `spatium` - Staff space in points
    /// * `slope` - Duration stretch slope (e.g., 1.2)
    /// * `density` - Spacing density (1.0 = normal)
    /// * `stretch_reduction` - Squeeze multiplier (1.0 = normal)
    pub fn compute_x_positions_proportional(
        &mut self,
        available_width: f64,
        spatium: f64,
        slope: f64,
        density: f64,
        stretch_reduction: f64,
    ) {
        if self.segments.is_empty() {
            return;
        }

        // Phase 1: Compute natural widths for ChordRest segments
        let mut total_natural = 0.0;
        let mut total_non_cr = 0.0;

        for segment in self.segments.iter_mut() {
            if segment.seg_type.is_chord_rest() && segment.ticks > 0 {
                let nat_w = spacing::natural_width(
                    segment.ticks as f64,
                    spatium,
                    slope,
                    density,
                    stretch_reduction,
                );
                // Respect minimum width from collision detection
                let w = nat_w.max(segment.min_width);
                segment.width = w;
                let stretch = spacing::duration_stretch(
                    segment.ticks as f64,
                    crate::engraver::layout::chart::constants::TICKS_PER_QUARTER,
                    slope,
                );
                segment.stretch = stretch;
                total_natural +=
                    w + segment.extra_leading_space + segment.spacing + segment.width_offset;
            } else {
                total_non_cr += segment.width
                    + segment.extra_leading_space
                    + segment.spacing
                    + segment.width_offset;
            }
        }

        // Phase 2: Spring justification if there's extra space
        let cr_available = available_width - total_non_cr;
        let extra = cr_available - total_natural;

        if extra > 1.0 {
            // Build springs from ChordRest segments
            let mut springs: Vec<spacing::Spring> = Vec::new();
            for (i, segment) in self.segments.iter().enumerate() {
                if segment.seg_type.is_chord_rest() && segment.ticks > 0 {
                    springs.push(spacing::Spring::new(segment.stretch, segment.width, i));
                }
            }

            let results = spacing::stretch_segments_to_width(&mut springs, extra);
            for (idx, new_width) in results {
                self.segments[idx].width = new_width;
            }
        }

        // Phase 3: Compute x positions from the final widths
        self.compute_x_positions();
    }

    /// Re-sort the segment list.
    /// Call this after modifying segment ticks or types.
    pub fn sort(&mut self) {
        self.segments.sort();
    }

    /// Validate that the list is properly sorted.
    #[must_use]
    pub fn is_sorted(&self) -> bool {
        self.segments.windows(2).all(|w| w[0] <= w[1])
    }
}

impl IntoIterator for SegmentList {
    type Item = Segment;
    type IntoIter = std::vec::IntoIter<Segment>;

    fn into_iter(self) -> Self::IntoIter {
        self.segments.into_iter()
    }
}

impl<'a> IntoIterator for &'a SegmentList {
    type Item = &'a Segment;
    type IntoIter = std::slice::Iter<'a, Segment>;

    fn into_iter(self) -> Self::IntoIter {
        self.segments.iter()
    }
}

impl<'a> IntoIterator for &'a mut SegmentList {
    type Item = &'a mut Segment;
    type IntoIter = std::slice::IterMut<'a, Segment>;

    fn into_iter(self) -> Self::IntoIter {
        self.segments.iter_mut()
    }
}

impl std::ops::Index<usize> for SegmentList {
    type Output = Segment;

    fn index(&self, index: usize) -> &Self::Output {
        &self.segments[index]
    }
}

impl std::ops::IndexMut<usize> for SegmentList {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.segments[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_segment_list() {
        let list = SegmentList::new();
        assert!(list.is_empty());
        assert_eq!(list.len(), 0);
    }

    #[test]
    fn test_push_segment() {
        let mut list = SegmentList::new();
        list.push(Segment::chord_rest(0, 480));
        list.push(Segment::chord_rest(480, 480));

        assert_eq!(list.len(), 2);
        assert_eq!(list[0].tick, 0);
        assert_eq!(list[1].tick, 480);
    }

    #[test]
    fn test_insert_sorted() {
        let mut list = SegmentList::new();
        list.insert(Segment::chord_rest(480, 480));
        list.insert(Segment::chord_rest(0, 480));
        list.insert(Segment::chord_rest(960, 480));

        assert!(list.is_sorted());
        assert_eq!(list[0].tick, 0);
        assert_eq!(list[1].tick, 480);
        assert_eq!(list[2].tick, 960);
    }

    #[test]
    fn test_insert_multiple_at_same_tick() {
        let mut list = SegmentList::new();
        list.insert(Segment::chord_rest(0, 480));
        list.insert(Segment::clef(0));
        list.insert(Segment::key_sig(0));

        assert!(list.is_sorted());
        // Order: KEY_SIG < CLEF < CHORD_REST (by bit position)
        assert!(list[0].seg_type == SegmentType::KEY_SIG);
        assert!(list[1].seg_type == SegmentType::CLEF);
        assert!(list[2].seg_type == SegmentType::CHORD_REST);
    }

    #[test]
    fn test_find_segment() {
        let mut list = SegmentList::new();
        list.insert(Segment::chord_rest(0, 480));
        list.insert(Segment::clef(0));
        list.insert(Segment::chord_rest(480, 480));

        let found = list.find(0, SegmentType::CLEF);
        assert!(found.is_some());
        assert!(found.unwrap().seg_type == SegmentType::CLEF);

        let not_found = list.find(0, SegmentType::TIME_SIG);
        assert!(not_found.is_none());
    }

    #[test]
    fn test_find_or_create() {
        let mut list = SegmentList::new();

        // Create new segment
        let seg = list.find_or_create(0, SegmentType::CHORD_REST);
        seg.ticks = 480;
        assert_eq!(list.len(), 1);

        // Find existing segment
        let seg = list.find_or_create(0, SegmentType::CHORD_REST);
        assert_eq!(seg.ticks, 480);
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn test_first_of_type() {
        let mut list = SegmentList::new();
        list.insert(Segment::chord_rest(0, 480));
        list.insert(Segment::barline(480));
        list.insert(Segment::chord_rest(480, 480));

        let first_cr = list.first_chord_rest();
        assert!(first_cr.is_some());
        assert_eq!(first_cr.unwrap().tick, 0);

        let first_bar = list.first_of_type(SegmentType::BAR_LINE);
        assert!(first_bar.is_some());
        assert_eq!(first_bar.unwrap().tick, 480);
    }

    #[test]
    fn test_tick_range() {
        let mut list = SegmentList::new();
        assert!(list.tick_range().is_none());

        list.push(Segment::chord_rest(0, 480));
        list.push(Segment::chord_rest(480, 480));
        list.push(Segment::chord_rest(960, 480));

        let range = list.tick_range();
        assert!(range.is_some());
        assert_eq!(range.unwrap(), (0, 1440));
    }

    #[test]
    fn test_compute_x_positions() {
        let mut list = SegmentList::new();

        let mut seg1 = Segment::chord_rest(0, 480);
        seg1.width = 50.0;

        let mut seg2 = Segment::chord_rest(480, 480);
        seg2.width = 60.0;

        let mut seg3 = Segment::chord_rest(960, 480);
        seg3.width = 40.0;

        list.push(seg1);
        list.push(seg2);
        list.push(seg3);

        list.compute_x_positions();

        assert!((list[0].x - 0.0).abs() < 1e-10);
        assert!((list[1].x - 50.0).abs() < 1e-10);
        assert!((list[2].x - 110.0).abs() < 1e-10);
    }

    #[test]
    fn test_compute_x_positions_proportional_basic() {
        let mut list = SegmentList::new();

        // Quarter note (480 ticks) + half note (960 ticks) in 4/4
        let mut seg1 = Segment::chord_rest(0, 480);
        seg1.width = 10.0; // will be overwritten
        let mut seg2 = Segment::chord_rest(480, 960);
        seg2.width = 10.0; // will be overwritten

        list.push(seg1);
        list.push(seg2);

        list.compute_x_positions_proportional(100.0, 5.0, 1.2, 1.0, 1.0);

        // Half note should be wider than quarter
        assert!(
            list[1].width > list[0].width,
            "Half note ({}) should be wider than quarter ({})",
            list[1].width,
            list[0].width
        );

        // Total should approximately fill available width
        let total: f64 = list.iter().map(|s| s.width).sum();
        assert!(
            (total - 100.0).abs() < 1.0,
            "Total width ({total}) should be close to available (100.0)"
        );
    }

    #[test]
    fn test_compute_x_positions_proportional_four_quarters() {
        let mut list = SegmentList::new();

        // 4 quarter notes — should get roughly equal width
        for i in 0..4 {
            let mut seg = Segment::chord_rest(i * 480, 480);
            seg.width = 10.0;
            list.push(seg);
        }

        list.compute_x_positions_proportional(100.0, 5.0, 1.2, 1.0, 1.0);

        // All should have same width (equal durations)
        let w0 = list[0].width;
        for i in 1..4 {
            assert!(
                (list[i].width - w0).abs() < 0.01,
                "All quarter notes should have same width: {} vs {}",
                list[i].width,
                w0
            );
        }
    }

    #[test]
    fn test_compute_x_positions_proportional_respects_min_width() {
        let mut list = SegmentList::new();

        // Eighth note with a large min_width from chord collision
        let mut seg1 = Segment::chord_rest(0, 240);
        seg1.min_width = 50.0; // Chord symbol is wide
        let mut seg2 = Segment::chord_rest(240, 240);
        seg2.min_width = 0.0;

        list.push(seg1);
        list.push(seg2);

        list.compute_x_positions_proportional(100.0, 5.0, 1.2, 1.0, 1.0);

        assert!(
            list[0].width >= 50.0,
            "Should respect min_width: got {}",
            list[0].width
        );
    }

    #[test]
    fn test_compute_x_positions_proportional_mixed_durations() {
        let mut list = SegmentList::new();

        // Dm7_8 C_8 Dm7(quarter) pattern — two eighths + one quarter
        list.push(Segment::chord_rest(0, 240)); // eighth
        list.push(Segment::chord_rest(240, 240)); // eighth
        list.push(Segment::chord_rest(480, 480)); // quarter

        list.compute_x_positions_proportional(100.0, 5.0, 1.2, 1.0, 1.0);

        // Quarter should be wider than either eighth
        assert!(
            list[2].width > list[0].width,
            "Quarter ({}) should be wider than eighth ({})",
            list[2].width,
            list[0].width
        );

        // Both eighths should be approximately equal
        assert!(
            (list[0].width - list[1].width).abs() < 0.01,
            "Eighths should be equal: {} vs {}",
            list[0].width,
            list[1].width
        );
    }

    #[test]
    fn test_iter_active() {
        let mut list = SegmentList::new();

        let mut seg1 = Segment::chord_rest(0, 480);
        seg1.enabled = true;

        let mut seg2 = Segment::chord_rest(480, 480);
        seg2.enabled = false;

        let mut seg3 = Segment::new(SegmentType::TIME_TICK); // Not active by type
        seg3.tick = 960; // Set tick to be after seg2

        list.push(seg1);
        list.push(seg2);
        list.push(seg3);

        let active: Vec<_> = list.iter_active().collect();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].tick, 0);
    }

    #[test]
    fn test_remove() {
        let mut list = SegmentList::new();
        list.push(Segment::chord_rest(0, 480));
        list.push(Segment::chord_rest(480, 480));
        list.push(Segment::chord_rest(960, 480));

        let removed = list.remove(1);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().tick, 480);
        assert_eq!(list.len(), 2);
        assert_eq!(list[1].tick, 960);
    }
}
