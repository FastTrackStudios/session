//! Desktop panel components for chart rendering.
//!
//! These components provide the WGPU-rendered chart views that integrate
//! with the dock layout system. They consume a `ChartGraphics` context
//! (provided by the binary) and use `ChartLayoutManager` for parse/layout/render.
//!
//! # Components
//!
//! - [`ChartView`]: Full chart editor with layout manager, cursor tracking, click-to-position
//! - [`ChartPreviewPanel`]: Live performance preview with auto-follow, 120Hz cursor, click-to-seek

mod chart_view;
mod preview_panel;
mod render_stats;

pub use chart_view::ChartView;
pub use preview_panel::ChartPreviewPanel;
pub use render_stats::{
    FpsTracker, PerfCursorMotionState, PerfRenderAccumulator, PerfStaticSceneKey,
};
