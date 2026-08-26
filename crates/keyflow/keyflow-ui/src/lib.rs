//! Keyflow UI Components
//!
//! Dioxus components for Keyflow chart editing with live WGPU preview.
//! Provides a chart editor with syntax highlighting and GPU-accelerated rendering
//! powered by the engraver pipeline.
//!
//! # Architecture
//!
//! This crate provides UI components that work with the keyflow parser and engraver:
//!
//! - **Components**: Reusable UI primitives (highlighted editor, etc.)
//! - **Layouts**: Complete view layouts (ChartEditorLayout)
//! - **Signals**: Global state management via Dioxus signals
//! - **ChartLayoutManager**: Desktop rendering engine (parse → layout → vello::Scene)
//! - **Examples**: Built-in chart examples (Thriller, Empty)
//!
//! # Setup
//!
//! The consuming app provides the WGPU rendering surface (via ChartGraphics).
//! This crate produces `vello::Scene` objects that the app renders to the surface.
//!
//! ```rust,ignore
//! use keyflow_ui::{ChartEditorLayout, ChartLayoutManager};
//! use keyflow_ui::signals::CHART_SOURCE;
//!
//! // In the app's rendering loop:
//! let mut manager = ChartLayoutManager::new().unwrap();
//! let chart = keyflow::text::chart::parse_chart(&*CHART_SOURCE.read()).ok();
//! if let Some(chart) = &chart {
//!     manager.layout_chart_with_mode(chart, width, true);
//!     let mut scene = vello::Scene::new();
//!     manager.render_to_scene(&mut scene, width, height, transform);
//!     // Hand scene to ChartGraphics for WGPU rendering
//! }
//! ```

/// Re-export dioxus prelude based on feature flags **plus** the FTS shared
/// design system (`architect-ui`).
///
/// Every component / panel / layout in this crate is expected to compose
/// `architect-ui` primitives — `Button`, `Card`, `Tabs`, `Tooltip`, `Toast`,
/// theme tokens, etc. — instead of hand-rolling raw `<button>` / `<div>`
/// markup. The chart **renderer** itself (`chart_graphics`,
/// `chart_renderer`) stays raw because it owns a Vello scene mount; the
/// chrome around it (toolbars, panels, status footer, dialogs) goes
/// through `architect-ui`.
///
/// `use keyflow_ui::prelude::*;` therefore brings in:
/// - `dioxus::prelude` — `rsx!`, `#[component]`, signals, … (dioxus-native
///   is only the renderer; the component framework is always `dioxus`)
/// - `architect_ui::prelude` — every FTS component, layout primitive, theme
///   token, and the `cn!` class-merge macro.
///
/// Down-stream callers should never need to `use architect_ui::…` directly.
pub mod prelude {
    pub use dioxus::prelude::*;

    pub use architect_ui::cn;
    pub use architect_ui::prelude::*;
}

pub mod catalog;
pub mod chart_renderer;
pub mod components;
pub mod examples;
pub mod layouts;
pub mod signals;

#[cfg(any(
    feature = "desktop-panels",
    all(feature = "wasm-panels", target_arch = "wasm32"),
))]
pub mod chart_graphics;
#[cfg(feature = "desktop-panels")]
pub mod panels;

// Re-export key types for convenience
#[cfg(any(
    feature = "desktop-panels",
    all(feature = "wasm-panels", target_arch = "wasm32"),
))]
pub use chart_graphics::ChartGraphics;
pub use chart_renderer::{ChartLayoutManager, SceneHitResult};
pub use layouts::ChartEditorLayout;
#[cfg(feature = "desktop-panels")]
pub use panels::{ChartPreviewPanel, ChartView};
pub use signals::{
    ChartEditorBounds, ChartPageInfo, ChartViewport, PageMeta, PreviewMode, RenderStats,
    SemanticZoomLevel, SystemMeta, CHART_BASE_SCALE, CHART_CURSOR_POSITION,
    CHART_CURSOR_SCENE_CLICK, CHART_CURSOR_TICK, CHART_CURSOR_VISIBLE, CHART_EDITOR_BOUNDS,
    CHART_HOVER_SCENE_POINT, CHART_PAGE_INFO, CHART_PREVIEW_MODE, CHART_RENDER_STATS, CHART_SOURCE,
    CHART_VIEWPORT, SESSION_CHART_SOURCE,
};
