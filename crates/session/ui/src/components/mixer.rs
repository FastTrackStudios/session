//! Mixer View — a full mixing-console UI.
//!
//! A dense, professional multi-channel console in the spirit of a digital
//! live-sound desk (X32 / Allen & Heath) or a DAW mixer: a left rail of
//! console section labels, then many vertical channel strips side by side,
//! horizontally scrollable. Each strip carries a pan control, routing icon
//! buttons, a tall vertical fader flanked by a dB scale and a (static) meter
//! gutter, the fader's current dB value, a mute button, and the track name on
//! a colored footer bar.
//!
//! Purely presentational: takes a flat `Vec<Track>` (REAPER folder-depth
//! order) and reports edits via callbacks. Folder tracks render as a
//! highlighted *bus / group master* strip — muting/riding a folder is the
//! group VCA (e.g. the Vocals bus). The app binds these callbacks to the
//! `Tracks` service; the state of record is always the `Track` props.

use crate::prelude::*;
use daw_proto::Track;

/// Console section labels for the left rail (decorative desk chrome).
const SECTIONS: &[&str] = &["CH", "FX", "SUB", "AUX", "DCA", "MTX"];

/// dB scale ticks drawn beside each fader (top → bottom). The fader taper
/// tops out at 0 dB (fader position `1.0`), so there is no positive region.
const DB_TICKS: &[&str] = &["0", "-5", "-10", "-20", "-40", "-∞"];

/// Map a normalized fader position (`0.0` = -∞, `1.0` = 0 dB) to a display dB
/// label. Uses a simple, monotonic log taper: `dB = 40·log10(v)` — so `0.5`
/// reads ≈ -12 dB and `0.1` reads -40 dB — clamped to `-∞` at the bottom.
fn fader_db_label(v: f64) -> String {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.0009 {
        return "-∞".to_string();
    }
    let db = 40.0 * v.log10();
    if db >= -0.05 {
        // Snap tiny negatives / the top of the taper to a clean 0.0.
        "0.0".to_string()
    } else {
        format!("{db:.1}")
    }
}

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
    /// Optional pan edit: `(track_guid, pan -1.0..=1.0)`. When absent the pan
    /// control renders read-only (the existing call sites don't wire it).
    #[props(default)]
    on_pan: Option<Callback<(String, f64)>>,
) -> Element {
    // Purely-visual focus: which strip is hovered (adds a subtle ring). The
    // authoritative selection still comes from `Track::selected`.
    let hovered = use_signal(|| Option::<String>::None);

    rsx! {
        div { class: "flex h-full w-full bg-background text-foreground select-none",

            // ── Left rail: console section legend ──────────────────────────
            div {
                class: "flex flex-col items-center gap-2 shrink-0 border-r border-border \
                        bg-card/60 px-2 py-3",
                span {
                    class: "text-[9px] font-bold uppercase tracking-widest text-muted-foreground",
                    "Console"
                }
                div { class: "flex flex-1 flex-col justify-center gap-1.5",
                    for s in SECTIONS.iter() {
                        span {
                            class: "rounded bg-muted px-1.5 py-0.5 text-center text-[9px] \
                                    font-semibold tracking-wide text-muted-foreground",
                            "{s}"
                        }
                    }
                }
                span {
                    class: "text-[8px] uppercase tracking-widest text-muted-foreground/60",
                    "dB"
                }
            }

            // ── Channel strips (horizontally scrollable) ───────────────────
            div { class: "flex-1 overflow-x-auto",
                div { class: "flex h-full items-stretch gap-1 p-2",
                    if tracks.is_empty() {
                        div { class: "flex h-full w-full items-center justify-center \
                                      text-xs text-muted-foreground",
                            "No channels"
                        }
                    }
                    for track in tracks.iter().cloned() {
                        ChannelStrip {
                            key: "{track.guid}",
                            track,
                            on_volume,
                            on_mute,
                            on_solo,
                            on_pan,
                            hovered,
                        }
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
    on_pan: Option<Callback<(String, f64)>>,
    hovered: Signal<Option<String>>,
) -> Element {
    let guid = track.guid.clone();
    let is_bus = track.is_folder;

    // The track's own REAPER color (0xRRGGBB) — the ONLY raw color allowed;
    // everything else is theme tokens.
    let accent = track
        .color
        .map(|c| format!("#{:06x}", c & 0xFF_FF_FF))
        .unwrap_or_else(|| "#52525b".to_string());

    let db_label = fader_db_label(track.volume);
    let is_hovered = hovered.read().as_deref() == Some(guid.as_str());
    let focused = track.selected || is_hovered;

    // Pan: -1..1 → percentage offset for the L/R readout.
    let pan_pct = (track.pan.clamp(-1.0, 1.0) * 50.0).round() as i32;
    let pan_label = if track.pan.abs() < 0.02 {
        "C".to_string()
    } else if track.pan < 0.0 {
        format!("L{}", (track.pan.abs() * 100.0).round() as i32)
    } else {
        format!("R{}", (track.pan * 100.0).round() as i32)
    };

    // Strip chrome: width, background, and a focus / bus ring.
    let width = if is_bus { "w-[68px]" } else { "w-[60px]" };
    let mut ring = if focused {
        "ring-2 ring-primary"
    } else if is_bus {
        "ring-1 ring-primary/40"
    } else {
        "ring-1 ring-transparent"
    };
    if is_bus && focused {
        ring = "ring-2 ring-primary";
    }
    let bg = if is_bus { "bg-card" } else { "bg-card/70" };
    let strip_class = format!(
        "group relative flex h-full shrink-0 flex-col items-stretch overflow-hidden \
         rounded-md border border-border {bg} {width} {ring}"
    );

    // Clones for each closure.
    let (g_vol, g_mute, g_solo, g_pan) = (guid.clone(), guid.clone(), guid.clone(), guid.clone());
    let h_enter = guid.clone();

    rsx! {
        div {
            class: "{strip_class}",
            onmouseenter: move |_| hovered.set(Some(h_enter.clone())),
            onmouseleave: move |_| hovered.set(None),

            // ── Header: index + bus tag ────────────────────────────────────
            div { class: "flex items-center justify-between px-1.5 pt-1",
                span { class: "text-[9px] font-mono text-muted-foreground", "{track.index}" }
                if is_bus {
                    span { class: "rounded bg-primary/20 px-1 text-[8px] font-bold uppercase \
                                   tracking-wide text-primary",
                        "Bus"
                    }
                } else if track.armed {
                    span { class: "h-2 w-2 rounded-full bg-red-600" }
                }
            }

            // ── Pan control ────────────────────────────────────────────────
            div { class: "px-1.5 pt-1",
                input {
                    r#type: "range",
                    min: "-1",
                    max: "1",
                    step: "0.02",
                    value: "{track.pan}",
                    disabled: on_pan.is_none(),
                    class: "h-1.5 w-full cursor-pointer accent-primary disabled:cursor-default \
                            disabled:opacity-60",
                    oninput: move |e| {
                        if let (Some(cb), Ok(p)) = (on_pan, e.value().parse::<f64>()) {
                            cb.call((g_pan.clone(), p));
                        }
                    },
                }
                div { class: "mt-0.5 flex items-center justify-between text-[7px] text-muted-foreground",
                    span { "L" }
                    span { class: "font-mono text-foreground", "{pan_label}" }
                    span { "R" }
                }
            }

            // ── Routing / headphone icon buttons (decorative desk chrome) ──
            div { class: "flex items-center justify-center gap-1 px-1.5 pt-1",
                span { class: "flex h-4 w-4 items-center justify-center rounded bg-muted \
                               text-[8px] text-muted-foreground",
                    "◇"
                }
                span { class: "flex h-4 w-4 items-center justify-center rounded bg-muted \
                               text-[8px] text-muted-foreground",
                    "⌂"
                }
                if track.phase_inverted {
                    span { class: "flex h-4 w-4 items-center justify-center rounded bg-muted \
                                   text-[8px] font-bold text-primary",
                        "ø"
                    }
                }
            }

            // ── Fader region: dB scale | meter | fader (fills height) ───────
            div { class: "flex flex-1 items-stretch justify-center gap-1 px-1 py-1 min-h-0",

                // dB scale ticks, aligned top(0 dB) → bottom(-∞).
                div { class: "flex flex-col justify-between py-0.5 text-right",
                    for t in DB_TICKS.iter() {
                        span { class: "text-[7px] leading-none text-muted-foreground", "{t}" }
                    }
                }

                // Static meter gutter (no live level data — decorative only).
                div { class: "flex w-1.5 flex-col justify-end overflow-hidden rounded-sm bg-muted/60",
                    div {
                        class: "w-full bg-muted-foreground/30",
                        style: "height: {(track.volume.clamp(0.0, 1.0) * 100.0) as i32}%;",
                    }
                }

                // Vertical fader — a rotated range input filling the height.
                div { class: "flex items-stretch justify-center",
                    input {
                        r#type: "range",
                        min: "0",
                        max: "1",
                        step: "0.005",
                        value: "{track.volume}",
                        class: "h-full cursor-pointer accent-primary",
                        style: "writing-mode: vertical-lr; direction: rtl; width: 14px;",
                        oninput: move |e| {
                            if let Ok(v) = e.value().parse::<f64>() {
                                on_volume.call((g_vol.clone(), v));
                            }
                        },
                    }
                }
            }

            // ── dB readout ─────────────────────────────────────────────────
            div { class: "px-1 pb-0.5 text-center",
                span {
                    class: "font-mono text-[10px] font-semibold text-foreground",
                    "{db_label}"
                }
            }

            // ── Solo / Mute ────────────────────────────────────────────────
            div { class: "flex gap-1 px-1.5 pb-1",
                button {
                    class: if track.soloed {
                        "flex-1 rounded py-0.5 text-[10px] font-bold bg-amber-400 text-black \
                         shadow-inner"
                    } else {
                        "flex-1 rounded py-0.5 text-[10px] font-bold bg-muted text-muted-foreground \
                         hover:bg-accent"
                    },
                    onclick: move |_| on_solo.call(g_solo.clone()),
                    "S"
                }
                button {
                    class: if track.muted {
                        "flex-1 rounded py-0.5 text-[10px] font-bold bg-red-600 text-white shadow-inner"
                    } else {
                        "flex-1 rounded py-0.5 text-[10px] font-bold bg-muted text-muted-foreground \
                         hover:bg-accent"
                    },
                    onclick: move |_| on_mute.call(g_mute.clone()),
                    "M"
                }
            }

            // ── Colored footer bar with the track name ─────────────────────
            div {
                class: "mt-auto flex h-7 items-center justify-center px-1 text-center",
                style: "background: {accent};",
                title: "{track.name}",
                span {
                    class: "w-full truncate text-[9px] font-semibold leading-tight text-white \
                            drop-shadow-[0_1px_1px_rgba(0,0,0,0.8)]",
                    "{track.name}"
                }
            }
        }
    }
}
