//! Global signals for Chart Editor state.
//!
//! These signals drive the chart editor's reactive data flow:
//! source text changes → parse → layout → render.

use crate::examples::DEFAULT_CHART;
use crate::prelude::*;

/// The current keyflow source text being edited.
pub static CHART_SOURCE: GlobalSignal<String> = GlobalSignal::new(|| DEFAULT_CHART.to_string());

/// Session-driven chart source for runtime views (performance split, etc.).
///
/// This is separate from `CHART_SOURCE` so live DAW hydration doesn't overwrite
/// manual chart editor text.
pub static SESSION_CHART_SOURCE: GlobalSignal<Option<String>> = GlobalSignal::new(|| None);

/// The current preview mode.
pub static CHART_PREVIEW_MODE: GlobalSignal<PreviewMode> = GlobalSignal::new(|| PreviewMode::Page);

/// The physical pixel bounds of the chart preview area.
/// Updated by the UI after layout, consumed by the WGPU rendering loop.
pub static CHART_EDITOR_BOUNDS: GlobalSignal<ChartEditorBounds> =
    GlobalSignal::new(ChartEditorBounds::zero);

/// Preview mode for the chart renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PreviewMode {
    /// Content-sized layout with no fixed page dimensions.
    Snippet,
    /// Printable paginated layout.
    Page,
    /// iReal Pro-inspired responsive screen layout for phones/tablets.
    Responsive,
}

/// Physical pixel bounds of the chart preview area in the window.
/// Used to tell the WGPU renderer where to draw the chart.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChartEditorBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Device pixel ratio (e.g. 2.0 on Retina displays).
    pub dpr: f64,
}

/// Viewport state for the chart preview (scroll offset + zoom).
/// Mouse events captured by the WebView are translated into viewport changes.
pub static CHART_VIEWPORT: GlobalSignal<ChartViewport> = GlobalSignal::new(ChartViewport::default);

/// Chart render stats (FPS, frame time). Updated by the render loop.
pub static CHART_RENDER_STATS: GlobalSignal<RenderStats> = GlobalSignal::new(RenderStats::default);

/// Lightweight render performance stats based on vello's stats.rs.
/// Uses a sliding window of frame times to compute FPS and min/max.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderStats {
    pub fps: f64,
    pub frame_time_ms: f64,
    pub frame_time_min_ms: f64,
    pub frame_time_max_ms: f64,
}

impl RenderStats {
    pub const fn default() -> Self {
        Self {
            fps: 0.0,
            frame_time_ms: 0.0,
            frame_time_min_ms: 0.0,
            frame_time_max_ms: 0.0,
        }
    }
}

/// Semantic zoom level presets for the chart preview.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticZoomLevel {
    /// Fit entire page in viewport.
    FullPage,
    /// Show approximately half the page.
    HalfPage,
    /// Show 2-3 systems (lines of music).
    SystemView,
    /// Show current line + next line only.
    LineView,
    /// Free-form zoom (user has manually zoomed via scroll wheel).
    Custom,
}

/// Viewport transform for the chart preview area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChartViewport {
    /// Horizontal scroll offset in points.
    pub scroll_x: f64,
    /// Vertical scroll offset in points.
    pub scroll_y: f64,
    /// Zoom scale factor (1.0 = 100%).
    pub zoom: f64,
    /// Active semantic zoom level. Set to Custom when user manually zooms.
    pub zoom_level: SemanticZoomLevel,
}

impl ChartViewport {
    pub const fn default() -> Self {
        Self {
            scroll_x: 0.0,
            scroll_y: 0.0,
            zoom: 1.0,
            zoom_level: SemanticZoomLevel::FullPage,
        }
    }
}

/// Page navigation and layout metadata for the chart preview.
/// Written by the render pipeline (derived from scroll_y and layout result).
/// Read by the UI for page indicator and navigation buttons.
#[derive(Debug, Clone, PartialEq)]
pub struct ChartPageInfo {
    /// Current page being viewed (1-indexed). Derived from scroll_y.
    pub current_page: u32,
    /// Total number of pages in the layout.
    pub total_pages: u32,
    /// Per-page metadata for navigation calculations.
    pub page_metadata: Vec<PageMeta>,
    /// Current semantic zoom level (synced from viewport).
    pub zoom_level: SemanticZoomLevel,
}

impl ChartPageInfo {
    #[allow(clippy::should_implement_trait)]
    pub fn default() -> Self {
        Self {
            current_page: 1,
            total_pages: 1,
            page_metadata: Vec::new(),
            zoom_level: SemanticZoomLevel::FullPage,
        }
    }
}

/// Lightweight page metadata for navigation calculations.
#[derive(Debug, Clone, PartialEq)]
pub struct PageMeta {
    /// Page number (1-indexed).
    pub number: u32,
    /// X offset in scene coordinates (pages are laid out side-by-side).
    pub x_offset: f64,
    /// Y offset in scene coordinates.
    pub y_offset: f64,
    /// Page width in points.
    pub width: f64,
    /// Page height in points.
    pub height: f64,
    /// Systems (lines of music) on this page.
    pub systems: Vec<SystemMeta>,
}

/// Lightweight system metadata for zoom-to-system calculations.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemMeta {
    /// Y position relative to page top (in points).
    pub y: f64,
    /// System height (in points).
    pub height: f64,
}

/// Page navigation info. Written by the render pipeline, read by the UI.
pub static CHART_PAGE_INFO: GlobalSignal<ChartPageInfo> = GlobalSignal::new(ChartPageInfo::default);

/// The current base scale factor (fit-to-width) for the chart preview.
/// Written by the render pipeline so UI navigation functions can compute scroll offsets.
pub static CHART_BASE_SCALE: GlobalSignal<f64> = GlobalSignal::new(|| 1.0);

/// Current cursor position in the chart, expressed as an absolute tick.
/// Written by click-to-position or external playback sync.
/// Read by the render pipeline to compute highlight commands.
pub static CHART_CURSOR_TICK: GlobalSignal<i64> = GlobalSignal::new(|| 0);

/// Whether the cursor should be visible.
pub static CHART_CURSOR_VISIBLE: GlobalSignal<bool> = GlobalSignal::new(|| true);

/// Musical position display string (e.g. "4.3.050").
/// Derived from cursor tick by the render pipeline.
pub static CHART_CURSOR_POSITION: GlobalSignal<String> =
    GlobalSignal::new(|| "1.1.000".to_string());

/// Scene-coordinate hover point for symbol highlighting.
/// Written continuously by the UI on mouse-move over the chart preview.
/// Consumed by the render pipeline to find the nearest beat and render
/// a blue highlight overlay.
pub static CHART_HOVER_SCENE_POINT: GlobalSignal<Option<(f64, f64)>> = GlobalSignal::new(|| None);

/// Scene-coordinate click point for click-to-position.
/// Written by the UI when the user clicks on the chart preview.
/// Consumed (read + cleared) by the render pipeline which has access
/// to `ChartLayoutManager::tick_at_scene_point()`.
pub static CHART_CURSOR_SCENE_CLICK: GlobalSignal<Option<(f64, f64)>> = GlobalSignal::new(|| None);

impl ChartEditorBounds {
    pub const fn new(x: f64, y: f64, width: f64, height: f64, dpr: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
            dpr,
        }
    }

    pub const fn zero() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            width: 0.0,
            height: 0.0,
            dpr: 1.0,
        }
    }

    /// Returns true if the bounds have a non-zero area.
    pub fn is_valid(&self) -> bool {
        self.width > 0.0 && self.height > 0.0
    }
}
