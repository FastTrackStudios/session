//! Time Signature Marker Component
//!
//! Displays time signature changes as visual markers on the timeline.
//! Pure presentation component - all state passed via props.

use crate::prelude::*;

/// A time signature marker to display on the timeline
#[derive(Clone, Debug, PartialEq)]
pub struct TimeSignatureMarkerData {
    /// Numerator (top number, e.g., 4 in 4/4)
    pub numerator: u32,
    /// Denominator (bottom number, e.g., 4 in 4/4)
    pub denominator: u32,
    /// Position as percentage (0-100)
    pub position_percent: f64,
    /// Whether this marker is visible (for filtering)
    pub is_visible: bool,
}

impl TimeSignatureMarkerData {
    /// Format as string (e.g., "4/4")
    pub fn format(&self) -> String {
        format!("{}/{}", self.numerator, self.denominator)
    }
}

/// Props for TimeSignatureMarker component
#[derive(Props, Clone, PartialEq)]
pub struct TimeSignatureMarkerProps {
    /// Numerator (top number)
    pub numerator: u32,
    /// Denominator (bottom number)
    pub denominator: u32,
    /// Position as percentage (0-100)
    pub position: f64,
    /// Whether to show the time signature label
    #[props(default = true)]
    pub show_label: bool,
    /// Height of the marker line in pixels
    #[props(default = 24)]
    pub height: u32,
}

/// Single time signature marker
///
/// Displays a vertical line at a specific position with an optional time signature label.
/// Used to indicate time signature changes in the timeline.
///
/// # Example
///
/// ```rust,no_run
/// use session_proto::ui::components::TimeSignatureMarker;
/// use dioxus::prelude::*;
///
/// fn MyComponent() -> Element {
///     rsx! {
///         TimeSignatureMarker {
///             numerator: 3,
///             denominator: 4,
///             position: 50.0,  // 50% into the song
///         }
///     }
/// }
/// ```
#[component]
pub fn TimeSignatureMarker(props: TimeSignatureMarkerProps) -> Element {
    let height_px = props.height;
    let position = props.position.clamp(0.0, 100.0);
    let time_sig = format!("{}/{}", props.numerator, props.denominator);

    rsx! {
        div {
            class: "time-signature-marker",
            style: "position: absolute; left: {position}%; top: 0; bottom: 0; z-index: 5;",

            // Vertical line (dashed)
            div {
                class: "time-signature-marker-line",
                style: "position: absolute; top: 0; left: 0; width: 1px; height: {height_px}px; background: linear-gradient(to bottom, #a855f7 50%, transparent 50%); background-size: 1px 4px; opacity: 0.8;"
            }

            // Time signature label
            if props.show_label {
                div {
                    class: "time-signature-marker-label",
                    style: "position: absolute; top: {height_px}px; left: 2px; font-size: 0.625rem; color: #a855f7; background: rgba(0, 0, 0, 0.7); padding: 1px 3px; border-radius: 2px; white-space: nowrap;",
                    "{time_sig}"
                }
            }
        }
    }
}

/// Time signature markers overlay
///
/// Renders multiple time signature markers on a timeline.
/// Filters out redundant markers that represent the same time signature as the previous one.
#[component]
pub fn TimeSignatureMarkersOverlay(
    /// List of time signature markers to display
    markers: Vec<TimeSignatureMarkerData>,
    /// Container height in pixels
    #[props(default = 48)]
    height: u32,
) -> Element {
    // Filter markers to only show changes (not every instance of same time sig)
    let mut last_time_sig = (0u32, 0u32);
    let significant_markers: Vec<_> = markers
        .iter()
        .filter(|m| m.is_visible)
        .filter(|m| {
            let current = (m.numerator, m.denominator);
            if current != last_time_sig || last_time_sig == (0, 0) {
                last_time_sig = current;
                true
            } else {
                false
            }
        })
        .collect();

    rsx! {
        div {
            class: "time-signature-markers-overlay",
            style: "position: relative; width: 100%; height: {height}px;",

            for (idx, marker) in significant_markers.iter().enumerate() {
                TimeSignatureMarker {
                    key: "{idx}",
                    numerator: marker.numerator,
                    denominator: marker.denominator,
                    position: marker.position_percent,
                    height: height,
                }
            }
        }
    }
}
