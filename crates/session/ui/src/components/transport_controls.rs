//! Transport Control Components
//!
//! Transport control bar with grid layout.
//! Copied from FastTrackStudio for exact styling match.

use crate::prelude::*;
use lucide_dioxus::{
    Circle as RecordIcon, Mic as ArmIcon, Pause as PauseIcon, Play as PlayIcon,
    Repeat2 as LoopIcon, SkipBack as BackIcon, SkipForward as ForwardIcon,
};

/// Transport control bar component.
///
/// Provides arm, record, back, play/pause, loop, and forward controls.
/// All actions are handled via callbacks to keep the component domain-agnostic.
///
/// The six controls always lay out as a single 6-column grid. In the default
/// (desktop) mode each cell is a roomy `icon + label` row at `text-lg`. Set
/// `compact` for tight hosts (e.g. a narrow note pane): the icon stacks over a
/// small label so all six fit without clipping regardless of the frame width.
#[component]
pub fn TransportControlBar(
    is_playing: bool,
    is_looping: bool,
    is_recording: bool,
    is_armed: bool,
    on_play_pause: Callback<()>,
    on_loop_toggle: Callback<()>,
    on_record_toggle: Callback<()>,
    on_arm_toggle: Callback<()>,
    on_back: Callback<()>,
    on_forward: Callback<()>,
    /// Tight layout: stack a small label under a smaller icon so all six
    /// controls fit a narrow container. Defaults false (desktop row layout).
    #[props(default)]
    compact: bool,
) -> Element {
    let playing = is_playing;
    let looping = is_looping;
    let recording = is_recording;
    let armed = is_armed;

    let icon = if compact { 20 } else { 28 };
    // Shared cell layout — the only difference between modes is icon size,
    // stacking direction, and type scale. State-specific fills are appended
    // per button below.
    let base = if compact {
        "flex flex-col items-center justify-center gap-1 px-1 text-center leading-none cursor-pointer transition-colors text-[11px] font-medium"
    } else {
        "flex items-center justify-center gap-3 cursor-pointer transition-colors text-lg font-medium"
    };
    let cls = |extra: &str| format!("{base} {extra}");

    rsx! {
        div {
            // Layout-critical: state it inline so the six controls always
            // lay out as one 6-column row regardless of whether the Tailwind
            // `grid grid-cols-6` utilities survived the consumer's CSS purge.
            // Without this the children collapse to block rows and overlap
            // inside the caller's fixed-height (`h-16 overflow-hidden`) frame.
            style: "display:grid; grid-template-columns:repeat(6,minmax(0,1fr)); align-items:stretch;",
            class: "h-full w-full bg-card grid grid-cols-6 divide-x divide-border",

            // Arm Button — arms/disarms the selected tracks in the active song
            div {
                class: if armed {
                    cls("bg-red-600/80 text-white hover:bg-red-600")
                } else {
                    cls("border border-border hover:bg-accent")
                },
                onclick: move |_| {
                    on_arm_toggle.call(());
                },
                ArmIcon { size: icon, color: "currentColor" }
                "Arm"
            }

            // Record Button — toggles recording into the active song's project
            div {
                class: if recording {
                    cls("bg-red-600 text-white hover:bg-red-700")
                } else {
                    cls("border border-border hover:bg-accent text-red-500")
                },
                onclick: move |_| {
                    on_record_toggle.call(());
                },
                RecordIcon { size: icon, color: "currentColor" }
                if recording { "Recording" } else { "Record" }
            }

            // Back Button
            div {
                class: cls("hover:bg-accent"),
                onclick: move |_| {
                    if !playing {
                        on_back.call(());
                    }
                },
                BackIcon { size: icon, color: "currentColor" }
                "Back"
            }

            // Play/Pause Button
            div {
                class: if playing {
                    cls("bg-primary text-primary-foreground hover:bg-primary/90")
                } else {
                    cls("border border-border hover:bg-accent")
                },
                onclick: move |_| {
                    on_play_pause.call(());
                },
                if playing {
                    PauseIcon { size: icon, color: "currentColor" }
                } else {
                    PlayIcon { size: icon, color: "currentColor" }
                }
                if playing { "Pause" } else { "Play" }
            }

            // Loop Button
            div {
                class: if looping {
                    cls("bg-primary text-primary-foreground hover:bg-primary/90")
                } else {
                    cls("border border-border hover:bg-accent")
                },
                onclick: move |_| {
                    on_loop_toggle.call(());
                },
                LoopIcon { size: icon, color: "currentColor" }
                "Loop"
            }

            // Advance Button
            div {
                class: cls("hover:bg-accent"),
                onclick: move |_| {
                    if !playing {
                        on_forward.call(());
                    }
                },
                ForwardIcon { size: icon, color: "currentColor" }
                "Advance"
            }
        }
    }
}
