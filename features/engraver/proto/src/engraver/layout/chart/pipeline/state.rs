//! Layout state for chart pipeline.
//!
//! Holds the mutable state that's shared and mutated during the layout process.
//! This is separated from configuration (which is immutable) and from the
//! adapter (which handles mode-specific concerns).

use crate::engraver::layout::chart::types::BeatPosition;
use crate::engraver::layout::orchestrator::{PageLayout, SystemLayout};

/// State accumulated during the layout process.
///
/// This struct holds all the mutable state that changes as we progress through
/// the layout. It's shared between the pipeline and the adapter.
#[derive(Debug, Clone)]
pub struct LayoutState {
    /// Current Y position in the layout coordinate space.
    pub current_y: f64,

    /// Current page number (1-indexed, for paginated mode).
    pub page_number: u32,

    /// Global system index (0-indexed, across all pages).
    pub global_system_index: usize,

    /// Global measure index (0-indexed, across all sections).
    pub global_measure_index: usize,

    /// Counter for generating unique IDs.
    pub id_counter: u64,

    /// Previous chord symbol for hiding duplicates.
    pub previous_chord_symbol: Option<String>,

    /// Whether count-in has been rendered.
    pub count_in_rendered: bool,

    /// Whether title header has been added.
    pub title_header_added: bool,

    /// Beat positions collected during layout (for cursor/highlight).
    pub beat_positions: Vec<BeatPosition>,

    /// Pages collected during layout (paginated mode only).
    pub pages: Vec<PageLayout>,

    /// Systems for the current page (paginated mode only).
    pub current_page_systems: Vec<SystemLayout>,

    /// Cumulative time in seconds (for beat positions).
    pub cumulative_time: f64,

    /// Cumulative ticks (for beat positions).
    pub cumulative_ticks: i64,

    /// Total height accumulated (continuous mode tracking).
    pub total_height: f64,

    /// Total width for the layout.
    pub total_width: f64,
}

impl LayoutState {
    /// Create a new layout state with default values.
    pub fn new() -> Self {
        Self {
            current_y: 0.0,
            page_number: 1,
            global_system_index: 0,
            global_measure_index: 0,
            id_counter: 100,
            previous_chord_symbol: None,
            count_in_rendered: false,
            title_header_added: false,
            beat_positions: Vec::new(),
            pages: Vec::new(),
            current_page_systems: Vec::new(),
            cumulative_time: 0.0,
            cumulative_ticks: 0,
            total_height: 0.0,
            total_width: 0.0,
        }
    }

    /// Create state for paginated mode with count-in offset.
    pub fn for_paginated(margin_top: f64, count_in_seconds: f64, count_in_ticks: i64) -> Self {
        Self {
            current_y: margin_top,
            cumulative_time: -count_in_seconds,
            cumulative_ticks: -count_in_ticks,
            ..Self::new()
        }
    }

    /// Create state for continuous mode.
    pub fn for_continuous(margin_top: f64) -> Self {
        Self {
            current_y: margin_top,
            total_height: margin_top,
            ..Self::new()
        }
    }

    /// Generate a new unique ID.
    pub fn next_id(&mut self) -> u64 {
        let id = self.id_counter;
        self.id_counter += 1;
        id
    }

    /// Reset chord tracking at section boundaries.
    pub fn reset_chord_tracking(&mut self) {
        self.previous_chord_symbol = None;
    }

    /// Check if a chord should be rendered (considering duplicate hiding).
    pub fn should_render_chord(&self, symbol: &str, hide_repeated: bool) -> bool {
        if !hide_repeated {
            return true;
        }
        match &self.previous_chord_symbol {
            Some(prev) => symbol != prev,
            None => true,
        }
    }

    /// Record a chord as rendered for duplicate tracking.
    pub fn record_chord(&mut self, symbol: &str) {
        self.previous_chord_symbol = Some(symbol.to_string());
    }

    /// Advance to the next system.
    pub fn advance_system(&mut self, system_height: f64) {
        self.current_y += system_height;
        self.global_system_index += 1;
    }

    /// Advance measures by count.
    pub fn advance_measures(&mut self, count: usize) {
        self.global_measure_index += count;
    }

    /// Start a new page (paginated mode).
    pub fn start_new_page(&mut self, margin_top: f64) {
        self.page_number += 1;
        self.current_y = margin_top;
        self.current_page_systems.clear();
    }
}

impl Default for LayoutState {
    fn default() -> Self {
        Self::new()
    }
}

/// State for building a single system.
///
/// Temporary state used while laying out one system (line of music).
#[derive(Debug, Clone)]
pub struct SystemState {
    /// Current X position within the system.
    pub current_x: f64,

    /// System width.
    pub width: f64,

    /// Staff Y position.
    pub staff_y: f64,

    /// Whether this is the first system (needs time signature).
    pub is_first_system: bool,

    /// Whether this is the first system of a section (needs rehearsal mark).
    pub is_section_start: bool,

    /// Measure indices being laid out in this system.
    pub measure_indices: Vec<usize>,
}

impl SystemState {
    /// Create a new system state.
    pub fn new(width: f64, staff_y: f64, measure_indices: Vec<usize>) -> Self {
        Self {
            current_x: 0.0,
            width,
            staff_y,
            is_first_system: false,
            is_section_start: false,
            measure_indices,
        }
    }

    /// Mark this as the first system in the layout.
    pub fn with_first_system(mut self, is_first: bool) -> Self {
        self.is_first_system = is_first;
        self
    }

    /// Mark this as the start of a new section.
    pub fn with_section_start(mut self, is_start: bool) -> Self {
        self.is_section_start = is_start;
        self
    }
}
