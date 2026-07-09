//! Layout state management for chart rendering.
//!
//! This module provides a centralized state container for tracking
//! layout progress across pages and systems.

use super::types::BeatPosition;

/// State container for chart layout operations.
///
/// This encapsulates the 15+ mutable variables that were previously
/// scattered throughout the layout functions, making the code cleaner
/// and easier to reason about.
#[derive(Debug)]
pub struct LayoutState {
    // ========== ID Generation ==========
    /// Counter for generating unique element IDs.
    pub id_counter: u64,

    // ========== Position Tracking ==========
    /// Current page number (1-indexed).
    pub page_number: u32,
    /// Current Y position on the page (from top).
    pub page_y: f64,
    /// Global system index across all pages (0-indexed).
    pub global_system_index: usize,
    /// Global measure index across all systems (0-indexed).
    pub global_measure_index: usize,

    // ========== Time Tracking ==========
    /// Cumulative time in seconds from song start.
    pub cumulative_time: f64,
    /// Cumulative time in ticks from song start.
    pub cumulative_ticks: i64,

    // ========== Chord State ==========
    /// Previous chord symbol for duplicate detection.
    pub previous_chord_symbol: Option<String>,

    // ========== Output Collectors ==========
    /// Collected beat positions for playback cursor mapping.
    pub beat_positions: Vec<BeatPosition>,
}

impl LayoutState {
    /// Create a new layout state with initial values.
    #[must_use]
    pub fn new() -> Self {
        Self {
            id_counter: 100,
            page_number: 1,
            page_y: 0.0,
            global_system_index: 0,
            global_measure_index: 0,
            cumulative_time: 0.0,
            cumulative_ticks: 0,
            previous_chord_symbol: None,
            beat_positions: Vec::new(),
        }
    }

    /// Create state with count-in offset (negative initial time/ticks).
    #[must_use]
    pub fn with_count_in(mut self, count_in_seconds: f64, count_in_ticks: i64) -> Self {
        self.cumulative_time = -count_in_seconds;
        self.cumulative_ticks = -count_in_ticks;
        self
    }

    /// Set the initial page Y position (typically top margin).
    #[must_use]
    pub fn with_initial_y(mut self, y: f64) -> Self {
        self.page_y = y;
        self
    }

    /// Generate and return the next unique ID.
    pub fn next_id(&mut self) -> u64 {
        let id = self.id_counter;
        self.id_counter += 1;
        id
    }

    /// Advance to the next page.
    pub fn start_new_page(&mut self, initial_y: f64) {
        self.page_number += 1;
        self.page_y = initial_y;
    }

    /// Advance to the next system.
    pub fn advance_system(&mut self, system_height: f64, system_spacing: f64) {
        self.page_y += system_height + system_spacing;
        self.global_system_index += 1;
    }

    /// Advance to the next measure.
    pub fn advance_measure(&mut self, duration_seconds: f64, duration_ticks: i64) {
        self.cumulative_time += duration_seconds;
        self.cumulative_ticks += duration_ticks;
        self.global_measure_index += 1;
    }

    /// Record a beat position for playback cursor mapping.
    pub fn record_beat_position(&mut self, beat: BeatPosition) {
        self.beat_positions.push(beat);
    }

    /// Update the previous chord symbol tracker.
    pub fn update_previous_chord(&mut self, symbol: Option<String>) {
        self.previous_chord_symbol = symbol;
    }

    /// Check if we should start a new page based on available height.
    #[must_use]
    pub fn should_start_new_page(
        &self,
        system_height: f64,
        available_height: f64,
        margin_bottom: f64,
    ) -> bool {
        self.page_y + system_height > available_height - margin_bottom
    }
}

impl Default for LayoutState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layout_state_new() {
        let state = LayoutState::new();
        assert_eq!(state.id_counter, 100);
        assert_eq!(state.page_number, 1);
        assert_eq!(state.global_system_index, 0);
        assert!(state.beat_positions.is_empty());
    }

    #[test]
    fn test_next_id() {
        let mut state = LayoutState::new();
        assert_eq!(state.next_id(), 100);
        assert_eq!(state.next_id(), 101);
        assert_eq!(state.next_id(), 102);
        assert_eq!(state.id_counter, 103);
    }

    #[test]
    fn test_with_count_in() {
        let state = LayoutState::new().with_count_in(2.0, 1920);
        assert_eq!(state.cumulative_time, -2.0);
        assert_eq!(state.cumulative_ticks, -1920);
    }

    #[test]
    fn test_start_new_page() {
        let mut state = LayoutState::new();
        state.page_y = 500.0;
        state.start_new_page(36.0);
        assert_eq!(state.page_number, 2);
        assert_eq!(state.page_y, 36.0);
    }

    #[test]
    fn test_advance_system() {
        let mut state = LayoutState::new();
        state.page_y = 100.0;
        state.advance_system(50.0, 20.0);
        assert_eq!(state.page_y, 170.0);
        assert_eq!(state.global_system_index, 1);
    }

    #[test]
    fn test_advance_measure() {
        let mut state = LayoutState::new();
        state.advance_measure(2.0, 1920);
        assert_eq!(state.cumulative_time, 2.0);
        assert_eq!(state.cumulative_ticks, 1920);
        assert_eq!(state.global_measure_index, 1);
    }

    #[test]
    fn test_should_start_new_page() {
        let mut state = LayoutState::new();
        state.page_y = 700.0;

        // Page height 792, margin 50, system height 70
        // 700 + 70 = 770 > 792 - 50 = 742 → true
        assert!(state.should_start_new_page(70.0, 792.0, 50.0));

        state.page_y = 600.0;
        // 600 + 70 = 670 < 742 → false
        assert!(!state.should_start_new_page(70.0, 792.0, 50.0));
    }
}
