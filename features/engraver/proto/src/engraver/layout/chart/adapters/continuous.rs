//! Continuous layout adapter.
//!
//! Implements `LayoutAdapter` for infinite scroll rendering.
//! No page breaks - content flows continuously.

use crate::engraver::layout::chart::pipeline::{LayoutAdapter, LayoutState};
use crate::engraver::layout::chart::types::ChartLayoutResult;
use crate::engraver::layout::orchestrator::PageMargins;
use crate::engraver::scene::node::SceneNode;

/// Adapter for continuous (scroll-based) chart layout.
///
/// This adapter produces a single continuous layout without page breaks.
/// It's suitable for screen viewing with scrolling.
#[derive(Debug, Clone)]
pub struct ContinuousAdapter {
    /// Total width of the layout area.
    width: f64,

    /// Page margins (used for padding).
    margins: PageMargins,

    /// Content width (width minus margins).
    content_width: f64,
}

impl ContinuousAdapter {
    /// Create a new continuous adapter with the given width.
    pub fn new(width: f64, margins: PageMargins) -> Self {
        let content_width = width - margins.left - margins.right;

        Self {
            width,
            margins,
            content_width,
        }
    }

    /// Get the content width (total width minus margins).
    pub fn content_width(&self) -> f64 {
        self.content_width
    }

    /// Get the left margin (content X offset).
    pub fn content_x(&self) -> f64 {
        self.margins.left
    }
}

impl LayoutAdapter for ContinuousAdapter {
    fn needs_boundary(&self, _state: &LayoutState, _content_height: f64) -> bool {
        // Continuous mode never needs boundaries
        false
    }

    fn handle_boundary(&mut self, _root: &mut SceneNode, _state: &mut LayoutState) {
        // No-op for continuous mode
    }

    fn finalize(self, root: SceneNode, state: LayoutState) -> ChartLayoutResult {
        // Single page that encompasses all content
        let total_height = state.total_height + self.margins.bottom;

        ChartLayoutResult {
            scene: root,
            pages: vec![], // No pages in continuous mode
            total_height,
            total_width: self.width,
            beat_positions: state.beat_positions,
        }
    }

    fn add_background(&mut self, _root: &mut SceneNode, _state: &LayoutState) {
        // No background in continuous mode (transparent)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engraver::layout::chart::pipeline::LayoutState;
    use crate::engraver::scene::id::{ElementType, SemanticId};

    #[test]
    fn test_continuous_never_needs_boundary() {
        let margins = PageMargins {
            top: 50.0,
            right: 50.0,
            bottom: 50.0,
            left: 50.0,
        };
        let adapter = ContinuousAdapter::new(800.0, margins);
        let state = LayoutState::for_continuous(50.0);

        // Even with huge content, should never need boundary
        assert!(!adapter.needs_boundary(&state, 10000.0));
    }

    #[test]
    fn test_content_width_calculation() {
        let margins = PageMargins {
            top: 50.0,
            right: 100.0,
            bottom: 50.0,
            left: 100.0,
        };
        let adapter = ContinuousAdapter::new(1000.0, margins);

        assert_eq!(adapter.content_width(), 800.0); // 1000 - 100 - 100
    }

    #[test]
    fn test_finalize_returns_total_height() {
        let margins = PageMargins {
            top: 50.0,
            right: 50.0,
            bottom: 50.0,
            left: 50.0,
        };
        let adapter = ContinuousAdapter::new(800.0, margins);
        let mut state = LayoutState::for_continuous(50.0);
        state.total_height = 1000.0;

        let root = SceneNode::group(SemanticId::new(ElementType::Page, 0));
        let result = adapter.finalize(root, state);

        assert_eq!(result.total_height, 1050.0); // 1000 + bottom margin
        assert_eq!(result.total_width, 800.0);
        assert!(result.pages.is_empty());
    }
}
