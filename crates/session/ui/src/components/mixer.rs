//! Mixer View
//!
//! A presentational mixer: one channel strip per track (volume fader, mute,
//! solo), grouped by folder. Folder tracks render as a highlighted *bus*
//! strip — muting/riding a folder is the group VCA (e.g. the Vocals bus).
//!
//! Domain-agnostic: takes a flat `Vec<Track>` (REAPER folder-depth order) and
//! reports edits via callbacks. The app binds these to the `Tracks` service.

use crate::prelude::*;
use daw_proto::Track;

/// A mixer bound to a flat, folder-depth-ordered track list.
#[component]
pub fn MixerView(
    tracks: Vec<Track>,
    /// `(track_guid, volume 0.0..=1.0)`
    on_volume: Callback<(String, f64)>,
    /// toggle mute for `track_guid`
    on_mute: Callback<String>,
    /// toggle solo for `track_guid`
    on_solo: Callback<String>,
) -> Element {
    rsx! {
        div {
            class: "h-full w-full overflow-x-auto bg-background",
            div {
                class: "flex items-stretch gap-1.5 p-3 h-full",
                for track in tracks.iter().cloned() {
                    ChannelStrip {
                        key: "{track.guid}",
                        track,
                        on_volume,
                        on_mute,
                        on_solo,
                    }
                }
            }
        }
    }
}

#[component]
fn ChannelStrip(
    track: Track,
    on_volume: Callback<(String, f64)>,
    on_mute: Callback<String>,
    on_solo: Callback<String>,
) -> Element {
    let guid = track.guid.clone();
    let is_bus = track.is_folder;
    // REAPER hex color (0xRRGGBB) if the track carries one.
    let accent = track
        .color
        .map(|c| format!("#{:06x}", c & 0xFF_FF_FF))
        .unwrap_or_else(|| "#52525b".to_string());

    let strip_class = if is_bus {
        "flex flex-col items-center rounded-md border border-border bg-card/80 px-2 py-2 w-16 ring-1 ring-primary/40"
    } else {
        "flex flex-col items-center rounded-md border border-border bg-card px-2 py-2 w-14"
    };

    let (g_vol, g_mute, g_solo) = (guid.clone(), guid.clone(), guid.clone());

    rsx! {
        div {
            class: strip_class,
            // Bus tag
            if is_bus {
                span { class: "text-[9px] font-semibold uppercase tracking-wide text-primary mb-1", "Bus" }
            }
            // Solo / Mute
            div {
                class: "flex gap-1 mb-2",
                button {
                    class: if track.soloed {
                        "w-6 h-5 rounded text-[10px] font-bold bg-yellow-500 text-black"
                    } else {
                        "w-6 h-5 rounded text-[10px] font-bold bg-muted text-muted-foreground hover:bg-accent"
                    },
                    onclick: move |_| on_solo.call(g_solo.clone()),
                    "S"
                }
                button {
                    class: if track.muted {
                        "w-6 h-5 rounded text-[10px] font-bold bg-red-600 text-white"
                    } else {
                        "w-6 h-5 rounded text-[10px] font-bold bg-muted text-muted-foreground hover:bg-accent"
                    },
                    onclick: move |_| on_mute.call(g_mute.clone()),
                    "M"
                }
            }
            // Fader (0.0..=1.0; 1.0 = 0 dB)
            input {
                r#type: "range",
                min: "0",
                max: "1",
                step: "0.01",
                value: "{track.volume}",
                class: "w-full my-1 accent-primary",
                oninput: move |e| {
                    if let Ok(v) = e.value().parse::<f64>() {
                        on_volume.call((g_vol.clone(), v));
                    }
                },
            }
            // Color dot + name
            div {
                class: "mt-1 w-3 h-3 rounded-full",
                style: "background:{accent};",
            }
            span {
                class: "mt-1 text-[9px] text-center leading-tight text-foreground truncate w-full",
                title: "{track.name}",
                "{track.name}"
            }
        }
    }
}
