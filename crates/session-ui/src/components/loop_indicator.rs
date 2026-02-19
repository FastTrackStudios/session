//! Loop Indicator Component
//!
//! Displays loop region as a visual overlay on the timeline.
//! Pure presentation component - all state passed via props.

use crate::prelude::*;

/// Props for LoopIndicator component
#[derive(Props, Clone, PartialEq)]
pub struct LoopIndicatorProps {
    /// Start position as percentage (0-100)
    pub start_percent: f64,
    /// End position as percentage (0-100)
    pub end_percent: f64,
    /// Height in pixels
    #[props(default = 48)]
    pub height: u32,
    /// Overlay color (hex format)
    #[props(default = "#3b82f6".to_string())]
    pub color: String,
    /// Overlay opacity (0.0 to 1.0)
    #[props(default = 0.3)]
    pub opacity: f64,
}

/// Loop region indicator
///
/// Displays a colored overlay showing the loop region on a timeline.
/// The overlay has semi-transparent fill and visible boundary markers.
///
/// # Example
///
/// ```rust,no_run
/// use session_proto::ui::components::LoopIndicator;
/// use dioxus::prelude::*;
///
/// fn MyComponent() -> Element {
///     rsx! {
///         LoopIndicator {
///             start_percent: 20.0,
///             end_percent: 80.0,
///         }
///     }
/// }
/// ```
#[component]
pub fn LoopIndicator(props: LoopIndicatorProps) -> Element {
    let height_px = props.height;
    let start = props.start_percent.clamp(0.0, 100.0);
    let end = props.end_percent.clamp(0.0, 100.0);
    let width = (end - start).max(0.0);
    let opacity = props.opacity.clamp(0.0, 1.0);

    // Convert hex color to RGB for opacity
    let color = &props.color;

    rsx! {
        div {
            class: "loop-indicator",
            style: "position: absolute; left: {start}%; top: 0; width: {width}%; height: {height_px}px; z-index: 3;",

            // Loop region overlay (semi-transparent fill)
            div {
                class: "loop-region-fill",
                style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0; background: {color}; opacity: {opacity};"
            }

            // Start boundary marker (left edge)
            div {
                class: "loop-start-marker",
                style: "position: absolute; top: 0; left: 0; width: 2px; height: 100%; background: {color}; opacity: 0.9;",

                // Triangle marker at top
                div {
                    class: "loop-start-triangle",
                    style: "position: absolute; top: -6px; left: -4px; width: 0; height: 0; border-left: 5px solid transparent; border-right: 5px solid transparent; border-top: 8px solid {color};"
                }
            }

            // End boundary marker (right edge)
            div {
                class: "loop-end-marker",
                style: "position: absolute; top: 0; right: 0; width: 2px; height: 100%; background: {color}; opacity: 0.9;",

                // Triangle marker at top
                div {
                    class: "loop-end-triangle",
                    style: "position: absolute; top: -6px; right: -4px; width: 0; height: 0; border-left: 5px solid transparent; border-right: 5px solid transparent; border-top: 8px solid {color};"
                }
            }

            // Loop label (centered)
            if width > 10.0 {
                div {
                    class: "loop-label",
                    style: "position: absolute; top: 50%; left: 50%; transform: translate(-50%, -50%); font-size: 0.75rem; font-weight: 600; color: white;",
                    "LOOP"
                }
            }
        }
    }
}

/// Loop toggle button component
///
/// A styled button for toggling loop mode on/off.
#[component]
pub fn LoopToggleButton(
    is_looping: bool,
    on_toggle: EventHandler<()>,
    #[props(default = false)] disabled: bool,
) -> Element {
    rsx! {
        button {
            class: if is_looping {
                "loop-toggle-btn px-3 py-2 rounded-lg bg-blue-600 hover:bg-blue-700 text-white transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            } else {
                "loop-toggle-btn px-3 py-2 rounded-lg bg-gray-700 hover:bg-gray-600 text-white transition-colors disabled:opacity-50 disabled:cursor-not-allowed"
            },
            disabled: disabled,
            onclick: move |_| on_toggle.call(()),
            if is_looping {
                "🔁 Loop ON"
            } else {
                "🔁 Loop OFF"
            }
        }
    }
}
