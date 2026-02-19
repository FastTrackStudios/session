//! Top Bar Layout Component
//!
//! Navigation bar with connection status indicator.
//! Based on FastTrackStudio desktop app design.

use crate::prelude::*;
use crate::signals::LATENCY_INFO;
use lucide_dioxus::{CircleCheck, CircleX};

/// Application version string
pub const VERSION: &str = "v0.1.0-alpha";

/// Connection state for the top bar
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ConnectionState {
    #[default]
    Disconnected,
    Connecting,
    Connected,
}

/// Top bar component with navigation and connection status
///
/// Displays:
/// - Connection status indicator (left)
/// - Navigation tabs (center-left)
/// - App title (center)
/// - Optional action buttons (right)
#[component]
pub fn TopBar(
    /// Current connection state
    connection_state: ConnectionState,
    /// Current active tab/route
    #[props(default = "performance".to_string())]
    active_tab: String,
    /// Callback when a tab is clicked
    #[props(default)]
    on_tab_click: Option<Callback<String>>,
    /// Optional app title override
    #[props(default = "FTS Control".to_string())]
    app_title: String,
) -> Element {
    rsx! {
        div {
            class: "h-12 flex-shrink-0 border-b border-border bg-card flex items-center justify-between px-4",

            // Left section: Version + Connection status + navigation
            div {
                class: "flex items-center gap-4",

                // Version label
                span {
                    class: "text-xs text-muted-foreground font-mono",
                    "{VERSION}"
                }

                // Connection status badge
                ConnectionStatusBadge {
                    state: connection_state,
                }

                // Navigation tabs
                div {
                    class: "flex items-center gap-1 ml-6",

                    NavTab {
                        label: "Performance",
                        tab_id: "performance",
                        is_active: active_tab == "performance",
                        on_click: on_tab_click.clone(),
                    }

                    NavTab {
                        label: "Chart",
                        tab_id: "chart",
                        is_active: active_tab == "chart",
                        on_click: on_tab_click.clone(),
                    }

                    NavTab {
                        label: "Setlist",
                        tab_id: "setlist",
                        is_active: active_tab == "setlist",
                        on_click: on_tab_click.clone(),
                    }

                    NavTab {
                        label: "Rig",
                        tab_id: "rig",
                        is_active: active_tab == "rig",
                        on_click: on_tab_click.clone(),
                    }

                    NavTab {
                        label: "FX",
                        tab_id: "fx",
                        is_active: active_tab == "fx",
                        on_click: on_tab_click.clone(),
                    }

                    NavTab {
                        label: "Settings",
                        tab_id: "settings",
                        is_active: active_tab == "settings",
                        on_click: on_tab_click.clone(),
                    }
                }
            }

            // Center: App title
            h1 {
                class: "text-lg font-semibold text-card-foreground",
                "{app_title}"
            }

            // Right section: latency badge
            div {
                class: "flex items-center gap-2",
                LatencyBadge {}
            }
        }
    }
}

/// Latency information badge component
///
/// Displays input, output, network, and total latency in a compact badge.
/// Clicking expands to show detailed breakdown.
#[component]
pub fn LatencyBadge() -> Element {
    let latency_info = LATENCY_INFO.read();
    let mut expanded = use_signal(|| false);

    // Input latency is already in ms
    let input_ms = latency_info.input_ms;

    let output_ms = latency_info.output_ms;
    let network_ms = latency_info.network_rtt_ms;
    let total_ms = latency_info.total_compensation_ms();

    // Determine badge color based on total latency
    let (bg_class, text_class) = if !latency_info.is_running {
        ("bg-gray-500/20", "text-gray-400")
    } else if total_ms < 20.0 {
        ("bg-green-500/20", "text-green-500")
    } else if total_ms < 50.0 {
        ("bg-yellow-500/20", "text-yellow-500")
    } else {
        ("bg-red-500/20", "text-red-500")
    };

    rsx! {
        div {
            class: "relative",

            // Main badge - always visible
            button {
                class: "flex items-center gap-2 px-3 py-1.5 rounded-full text-sm font-medium {bg_class} {text_class} hover:opacity-80 transition-opacity",
                onclick: move |_| expanded.set(!expanded()),

                // Latency icon (clock with arrow)
                svg {
                    width: "14",
                    height: "14",
                    view_box: "0 0 24 24",
                    fill: "none",
                    stroke: "currentColor",
                    stroke_width: "2",
                    stroke_linecap: "round",
                    stroke_linejoin: "round",
                    circle { cx: "12", cy: "12", r: "10" }
                    polyline { points: "12 6 12 12 16 14" }
                }

                // Total latency display
                span {
                    class: "font-mono tabular-nums",
                    if latency_info.is_running {
                        "{total_ms:.1}ms"
                    } else {
                        "—"
                    }
                }
            }

            // Expanded dropdown
            if expanded() {
                div {
                    class: "absolute right-0 top-full mt-2 w-56 p-3 rounded-lg shadow-lg border border-border bg-popover text-popover-foreground z-50",

                    // Header
                    div {
                        class: "text-xs font-semibold text-muted-foreground tracking-wider mb-3",
                        "LATENCY BREAKDOWN"
                    }

                    // Latency rows
                    div {
                        class: "space-y-2 text-sm",

                        // Input latency
                        LatencyRow {
                            label: "Input",
                            value_ms: input_ms,
                            is_running: latency_info.is_running,
                        }

                        // Output latency
                        LatencyRow {
                            label: "Output",
                            value_ms: output_ms,
                            is_running: latency_info.is_running,
                        }

                        // Network latency (RTT / 2 for one-way)
                        LatencyRow {
                            label: "Network",
                            value_ms: network_ms / 2.0,
                            is_running: true, // Network is always "running"
                        }

                        // Divider
                        div {
                            class: "border-t border-border my-2",
                        }

                        // Total
                        div {
                            class: "flex justify-between items-center font-semibold",
                            span { "Total" }
                            span {
                                class: "font-mono tabular-nums {text_class}",
                                if latency_info.is_running {
                                    "{total_ms:.1}ms"
                                } else {
                                    "—"
                                }
                            }
                        }
                    }

                    // Sample rate info
                    if latency_info.sample_rate > 0 {
                        div {
                            class: "mt-3 pt-2 border-t border-border text-xs text-muted-foreground",
                            "Sample Rate: {latency_info.sample_rate} Hz"
                        }
                    }
                }
            }
        }
    }
}

/// Single row in the latency breakdown
#[component]
fn LatencyRow(label: &'static str, value_ms: f64, is_running: bool) -> Element {
    rsx! {
        div {
            class: "flex justify-between items-center",
            span {
                class: "text-muted-foreground",
                "{label}"
            }
            span {
                class: "font-mono tabular-nums",
                if is_running {
                    "{value_ms:.1}ms"
                } else {
                    "—"
                }
            }
        }
    }
}

/// Connection status badge component
#[component]
fn ConnectionStatusBadge(state: ConnectionState) -> Element {
    let (bg_class, text_class, label, icon_color) = match state {
        ConnectionState::Connected => (
            "bg-green-500/20",
            "text-green-500",
            "Connected",
            "currentColor",
        ),
        ConnectionState::Connecting => (
            "bg-yellow-500/20",
            "text-yellow-500",
            "Connecting...",
            "currentColor",
        ),
        ConnectionState::Disconnected => (
            "bg-red-500/20",
            "text-red-500",
            "Disconnected",
            "currentColor",
        ),
    };

    rsx! {
        div {
            class: "flex items-center gap-2 px-3 py-1.5 rounded-full text-sm font-medium {bg_class} {text_class}",

            // Status icon
            match state {
                ConnectionState::Connected => rsx! {
                    CircleCheck { size: 16, color: icon_color }
                },
                ConnectionState::Connecting => rsx! {
                    div {
                        class: "w-4 h-4 border-2 border-current border-t-transparent rounded-full animate-spin",
                    }
                },
                ConnectionState::Disconnected => rsx! {
                    CircleX { size: 16, color: icon_color }
                },
            }

            // Label
            span { "{label}" }
        }
    }
}

/// Navigation tab component
#[component]
fn NavTab(
    label: String,
    tab_id: String,
    is_active: bool,
    on_click: Option<Callback<String>>,
) -> Element {
    let active_class = if is_active {
        "bg-primary text-primary-foreground"
    } else {
        "text-muted-foreground hover:text-foreground hover:bg-accent"
    };

    rsx! {
        button {
            class: "px-4 py-2 rounded-md font-medium text-sm transition-colors {active_class}",
            onclick: move |_| {
                if let Some(callback) = &on_click {
                    callback.call(tab_id.clone());
                }
            },
            "{label}"
        }
    }
}
