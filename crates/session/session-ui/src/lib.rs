//! Session UI Components
//!
//! Dioxus components for rendering setlist and performance views.
//! Uses a signal-driven architecture with smart components that subscribe to granular
//! domain state.
//!
//! # Architecture
//!
//! This crate provides UI components that work with `session-proto` types and clients:
//!
//! - **Components**: Reusable UI primitives (progress bars, transport controls, etc.)
//! - **Layouts**: Complete view layouts (PerformanceLayout, etc.)
//! - **Signals**: Global state management via Dioxus signals
//! - **Session**: Singleton access to service clients (similar to `Daw::get()`)
//!
//! # Setup
//!
//! Initialize the `Session` singleton during app startup:
//!
//! ```rust,ignore
//! use session_ui::Session;
//! use session_proto::SetlistServiceClient;
//!
//! // During app startup
//! let client = SetlistServiceClient::new(connection);
//! Session::init(client).expect("Failed to init session");
//! ```
//!
//! # Usage
//!
//! Components call service methods directly via `Session::get()`:
//!
//! ```rust,ignore
//! use session_ui::{Session, PerformanceLayout};
//!
//! // In UI event handlers
//! spawn(async move {
//!     Session::get().setlist().play().await;
//! });
//!
//! // Render the performance view
//! rsx! {
//!     PerformanceLayout {}
//! }
//! ```

/// Re-export dioxus prelude based on feature flags.
/// When `native` feature is enabled, uses dioxus-native (GPU-accelerated Blitz renderer).
/// Otherwise, uses standard dioxus (WebView-based desktop or web).
pub mod prelude {
    #[cfg(feature = "native")]
    pub use dioxus_native::prelude::*;

    #[cfg(not(feature = "native"))]
    pub use dioxus::prelude::*;
}

pub mod components;
pub mod layouts;
pub mod panel_registration;
pub mod signals;

// Re-export key types for convenience
pub use layouts::top_bar::{ConnectionState, TopBar, VERSION};
pub use layouts::{PerformanceLayout, PerformanceSidebar, TransportPanel};
pub use panel_registration::register_panels;
pub use signals::{
    ChartAreaBounds, LatencyAction, LatencyInfo, LatencyMeasurement, LatencyTracker,
    PerfChartViewport, Session, TransportState, ACTIVE_INDICES, ACTIVE_PLAYBACK_IS_PLAYING,
    ACTIVE_PLAYBACK_MUSICAL, AUDIO_LATENCY_SECONDS, CHART_AREA_BOUNDS, LATENCY_INFO,
    LATENCY_TRACKER, PERF_CHART_BASE_SCALE, PERF_CHART_CLICK, PERF_CHART_HOVER,
    PERF_CHART_VIEWPORT, PLAYBACK_STATE, SETLIST_STRUCTURE, SHOW_CHART_SPLIT, SONG_CHARTS,
    SONG_TRANSPORT,
};
