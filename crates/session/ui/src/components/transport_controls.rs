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
) -> Element {
    let playing = is_playing;
    let looping = is_looping;
    let recording = is_recording;
    let armed = is_armed;

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
                    "flex items-center justify-center gap-3 cursor-pointer bg-red-600/80 text-white hover:bg-red-600 transition-colors text-lg font-medium"
                } else {
                    "flex items-center justify-center gap-3 cursor-pointer border border-border hover:bg-accent transition-colors text-lg font-medium"
                },
                onclick: move |_| {
                    on_arm_toggle.call(());
                },
                ArmIcon { size: 28, color: "currentColor" }
                "Arm"
            }

            // Record Button — toggles recording into the active song's project
            div {
                class: if recording {
                    "flex items-center justify-center gap-3 cursor-pointer bg-red-600 text-white hover:bg-red-700 transition-colors text-lg font-medium"
                } else {
                    "flex items-center justify-center gap-3 cursor-pointer border border-border hover:bg-accent transition-colors text-lg font-medium text-red-500"
                },
                onclick: move |_| {
                    on_record_toggle.call(());
                },
                RecordIcon { size: 28, color: "currentColor" }
                if recording { "Recording" } else { "Record" }
            }

            // Back Button
            div {
                class: "flex items-center justify-center gap-3 cursor-pointer hover:bg-accent transition-colors text-lg font-medium",
                onclick: move |_| {
                    if !playing {
                        on_back.call(());
                    }
                },
                BackIcon {
                    size: 28,
                    color: "currentColor",
                }
                "Back"
            }

            // Play/Pause Button
            div {
                class: if playing {
                    "flex items-center justify-center gap-3 cursor-pointer bg-primary text-primary-foreground hover:bg-primary/90 transition-colors text-lg font-medium"
                } else {
                    "flex items-center justify-center gap-3 cursor-pointer border border-border hover:bg-accent transition-colors text-lg font-medium"
                },
                onclick: move |_| {
                    on_play_pause.call(());
                },
                if playing {
                    PauseIcon {
                        size: 28,
                        color: "currentColor",
                    }
                } else {
                    PlayIcon {
                        size: 28,
                        color: "currentColor",
                    }
                }
                if playing { "Pause" } else { "Play" }
            }

            // Loop Button
            div {
                class: if looping {
                    "flex items-center justify-center gap-3 cursor-pointer bg-primary text-primary-foreground hover:bg-primary/90 transition-colors text-lg font-medium"
                } else {
                    "flex items-center justify-center gap-3 cursor-pointer border border-border hover:bg-accent transition-colors text-lg font-medium"
                },
                onclick: move |_| {
                    on_loop_toggle.call(());
                },
                LoopIcon {
                    size: 28,
                    color: "currentColor",
                }
                "Loop"
            }

            // Advance Button
            div {
                class: "flex items-center justify-center gap-3 cursor-pointer hover:bg-accent transition-colors text-lg font-medium",
                onclick: move |_| {
                    if !playing {
                        on_forward.call(());
                    }
                },
                "Advance"
                ForwardIcon {
                    size: 28,
                    color: "currentColor",
                }
            }
        }
    }
}
