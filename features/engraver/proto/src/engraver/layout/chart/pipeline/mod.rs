//! Chart layout pipeline.
//!
//! Abstracts the common layout logic between paginated and continuous modes
//! using a `LayoutAdapter` trait. This allows the core layout algorithm to be
//! shared while adapters handle mode-specific concerns like page breaks.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │                  LayoutPipeline<A>                       │
//! │  (orchestrates common layout logic)                     │
//! │                                                         │
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────────┐ │
//! │  │ Section     │  │ System      │  │ Measure         │ │
//! │  │ Iterator    │──│ Builder     │──│ Builder         │ │
//! │  └─────────────┘  └─────────────┘  └─────────────────┘ │
//! │                                                         │
//! └─────────────────────────────────────────────────────────┘
//!                          │
//!                          │ calls
//!                          ▼
//!                ┌───────────────────┐
//!                │   LayoutAdapter   │
//!                │   (trait)         │
//!                └───────────────────┘
//!                       ╱    ╲
//!                      ╱      ╲
//!     ┌────────────────┐      ┌──────────────────┐
//!     │ PaginatedAdapter│      │ ContinuousAdapter│
//!     │ (page breaks)  │      │ (scroll mode)    │
//!     └────────────────┘      └──────────────────┘
//! ```

mod state;

pub use state::{LayoutState, SystemState};

use crate::engraver::layout::chart::types::ChartLayoutResult;
use crate::engraver::scene::node::SceneNode;

/// Adapter trait for mode-specific layout behavior.
///
/// Implementors handle the differences between paginated and continuous layout:
/// - When to break to a new page/section
/// - How to finalize the result
///
/// The common layout logic (measure spacing, chord rendering, etc.) is shared
/// in the pipeline orchestrator.
pub trait LayoutAdapter: Sized {
    /// Check if a boundary is needed before adding content of the given height.
    ///
    /// For paginated layout: checks if there's room on the current page
    /// For continuous layout: always returns false (no boundaries)
    fn needs_boundary(&self, state: &LayoutState, content_height: f64) -> bool;

    /// Handle a boundary (page break for paginated, no-op for continuous).
    ///
    /// For paginated layout: finishes current page, starts new one
    /// For continuous layout: no-op
    fn handle_boundary(&mut self, root: &mut SceneNode, state: &mut LayoutState);

    /// Finalize the layout and produce the result.
    ///
    /// For paginated layout: collects all pages, computes total dimensions
    /// For continuous layout: wraps single content tree
    fn finalize(self, root: SceneNode, state: LayoutState) -> ChartLayoutResult;

    /// Optional: Add page/section background.
    ///
    /// For paginated layout: adds page background rectangle
    /// For continuous layout: no-op
    fn add_background(&mut self, _root: &mut SceneNode, _state: &LayoutState) {}

    /// Optional: Add header content (title, song info).
    ///
    /// For paginated layout: adds header on first page
    /// For continuous layout: adds header at top
    fn add_header(&mut self, _root: &mut SceneNode, _state: &mut LayoutState) {}
}
