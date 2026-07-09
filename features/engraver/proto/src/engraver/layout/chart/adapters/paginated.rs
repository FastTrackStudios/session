//! Paginated layout adapter.
//!
//! Implements `LayoutAdapter` for page-based rendering with automatic
//! page breaks when content exceeds page height.

use crate::engraver::layout::chart::constants;
use crate::engraver::layout::chart::page_rendering;
use crate::engraver::layout::chart::pipeline::{LayoutAdapter, LayoutState};
use crate::engraver::layout::chart::types::ChartLayoutResult;
use crate::engraver::layout::orchestrator::{PageLayout, PageMargins};
use crate::engraver::scene::node::SceneNode;

/// Adapter for paginated (page-based) chart layout.
///
/// This adapter handles page breaks and creates multiple pages as needed.
/// It's suitable for printing or MuseScore-style page view.
#[derive(Debug, Clone)]
pub struct PaginatedAdapter {
    /// Page dimensions.
    page_width: f64,
    page_height: f64,

    /// Page margins.
    margins: PageMargins,

    /// Content area dimensions (page minus margins).
    content_width: f64,
    content_height: f64,

    /// Gap between pages in the output.
    page_gap: f64,

    /// Horizontal offset for multi-page rendering.
    page_offset_x: f64,

    /// Vertical offset for multi-page rendering.
    page_offset_y: f64,
}

impl PaginatedAdapter {
    /// Create a new paginated adapter with the given page dimensions.
    pub fn new(page_width: f64, page_height: f64, margins: PageMargins) -> Self {
        let content_width = page_width - margins.left - margins.right;
        let content_height = page_height - margins.top - margins.bottom;

        Self {
            page_width,
            page_height,
            margins,
            content_width,
            content_height,
            page_gap: constants::PAGE_GAP,
            page_offset_x: constants::PAGE_OFFSET_X,
            page_offset_y: constants::PAGE_OFFSET_Y,
        }
    }

    /// Get the content width (page width minus margins).
    pub fn content_width(&self) -> f64 {
        self.content_width
    }

    /// Get the content height (page height minus margins).
    pub fn content_height(&self) -> f64 {
        self.content_height
    }

    /// Calculate the Y offset for a page number (0-indexed internally).
    fn page_y_offset(&self, page_index: u32) -> f64 {
        self.page_offset_y + (page_index as f64) * (self.page_height + self.page_gap)
    }

    /// Finish the current page and add it to the pages list.
    fn finish_current_page(&mut self, state: &mut LayoutState) {
        // Calculate page position (pages are stacked vertically in this adapter)
        let page_index = state.page_number.saturating_sub(1);
        let y_offset = self.page_y_offset(page_index);

        // Create page layout record
        let page_layout = PageLayout {
            number: state.page_number,
            x_offset: self.page_offset_x,
            y_offset,
            width: self.page_width,
            height: self.page_height,
            systems: std::mem::take(&mut state.current_page_systems),
            margins: self.margins,
        };

        state.pages.push(page_layout);
    }
}

impl LayoutAdapter for PaginatedAdapter {
    fn needs_boundary(&self, state: &LayoutState, content_height: f64) -> bool {
        // Check if adding content would exceed the current page
        let available_height = self.content_height - (state.current_y - self.margins.top);
        content_height > available_height
    }

    fn handle_boundary(&mut self, root: &mut SceneNode, state: &mut LayoutState) {
        // Finish current page
        self.finish_current_page(state);

        // Start new page
        state.start_new_page(self.margins.top);

        // Paint the new page's background (white paper + drop shadow) at its offset.
        let page_index = state.page_number.saturating_sub(1);
        let page_y = self.page_y_offset(page_index);
        page_rendering::add_page_background(
            root,
            self.page_offset_x,
            page_y,
            self.page_width,
            self.page_height,
        );
    }

    fn finalize(mut self, root: SceneNode, mut state: LayoutState) -> ChartLayoutResult {
        // Finish the last page if there's content
        if !state.current_page_systems.is_empty() || state.pages.is_empty() {
            self.finish_current_page(&mut state);
        }

        // Calculate total dimensions
        let num_pages = state.pages.len();
        let total_height = self.page_offset_y * 2.0
            + (num_pages as f64) * self.page_height
            + ((num_pages.saturating_sub(1)) as f64) * self.page_gap;
        let total_width = self.page_offset_x * 2.0 + self.page_width;

        ChartLayoutResult {
            scene: root,
            pages: state.pages,
            total_height,
            total_width,
            beat_positions: state.beat_positions,
        }
    }

    fn add_background(&mut self, root: &mut SceneNode, state: &LayoutState) {
        // Paint the *first* page background. Subsequent pages are painted in
        // `handle_boundary` as the layout advances.
        let page_index = state.page_number.saturating_sub(1);
        let page_y = self.page_y_offset(page_index);
        page_rendering::add_page_background(
            root,
            self.page_offset_x,
            page_y,
            self.page_width,
            self.page_height,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engraver::layout::chart::pipeline::LayoutState;
    use crate::engraver::scene::id::{ElementType, SemanticId};

    fn test_margins() -> PageMargins {
        PageMargins {
            top: 50.0,
            right: 50.0,
            bottom: 50.0,
            left: 50.0,
        }
    }

    #[test]
    fn test_content_dimensions() {
        let adapter = PaginatedAdapter::new(800.0, 1000.0, test_margins());

        assert_eq!(adapter.content_width(), 700.0); // 800 - 50 - 50
        assert_eq!(adapter.content_height(), 900.0); // 1000 - 50 - 50
    }

    #[test]
    fn test_needs_boundary_at_start() {
        let adapter = PaginatedAdapter::new(800.0, 1000.0, test_margins());
        let state = LayoutState::for_paginated(50.0, 0.0, 0);

        // With plenty of room, should not need boundary
        assert!(!adapter.needs_boundary(&state, 100.0));
    }

    #[test]
    fn test_needs_boundary_when_full() {
        let adapter = PaginatedAdapter::new(800.0, 1000.0, test_margins());
        let mut state = LayoutState::for_paginated(50.0, 0.0, 0);

        // Move to near the bottom of the page
        state.current_y = 900.0; // 50 units left

        // Small content should fit
        assert!(!adapter.needs_boundary(&state, 50.0));

        // Large content should trigger boundary
        assert!(adapter.needs_boundary(&state, 100.0));
    }

    #[test]
    fn test_finalize_calculates_dimensions() {
        let adapter = PaginatedAdapter::new(800.0, 1000.0, test_margins());
        let state = LayoutState::for_paginated(50.0, 0.0, 0);

        let root = SceneNode::group(SemanticId::new(ElementType::Page, 0));
        let result = adapter.finalize(root, state);

        // Should have one page (the initial empty page)
        assert_eq!(result.pages.len(), 1);
        assert_eq!(result.total_width, 840.0); // 20 + 800 + 20
    }
}
