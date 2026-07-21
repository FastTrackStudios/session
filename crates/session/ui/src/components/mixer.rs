//! Mixer View — a full mixing-console UI.
//!
//! A dense, professional multi-channel console in the spirit of a digital
//! live-sound desk (X32 / Allen & Heath) or a DAW mixer: a left rail of
//! instrument-group jump buttons, then many vertical channel strips side by
//! side, horizontally scrollable. Each strip carries an instrument icon, a pan
//! control, routing icon buttons, a tall vertical fader (with a real console
//! fader cap) flanked by a dB scale and a level meter, the fader's current dB
//! value, a mute button, and the track name on a colored footer bar.
//!
//! Purely presentational: takes a flat `Vec<Track>` (REAPER folder-depth
//! order) and reports edits via callbacks. Folder tracks render as a
//! highlighted *bus / group master* strip. Meters are prop-driven — the app
//! feeds real peak levels from per-stem Web-Audio AnalyserNodes; this
//! component never invents motion. The state of record is always the `Track`
//! props.

use crate::prelude::*;
use daw_proto::Track;
use lucide_dioxus::{AlarmClock, Drum, Guitar, Mic, Music2, Piano};
use std::collections::HashMap;

/// dB scale ticks drawn beside each fader (top → bottom). The fader taper
/// tops out at 0 dB (fader position `1.0`), so there is no positive region.
const DB_TICKS: &[&str] = &["0", "-5", "-10", "-20", "-40", "-∞"];

/// Custom fader styling: a wide, short, rounded console fader CAP with a
/// horizontal center notch — injected once so the range thumb stops reading as
/// a plain circle. Uses theme CSS variables for chrome (the notch is
/// `--primary`), so it tracks light/dark like the rest of the console.
const FADER_CSS: &str = r#"
.fts-fader{-webkit-appearance:none;appearance:none;background:transparent;cursor:pointer;}
.fts-fader::-webkit-slider-runnable-track{width:6px;border-radius:3px;
  background:linear-gradient(to right,var(--muted),color-mix(in oklab,var(--muted) 70%,#000));}
.fts-fader::-moz-range-track{width:6px;border-radius:3px;background:var(--muted);}
.fts-fader::-webkit-slider-thumb{-webkit-appearance:none;appearance:none;
  width:28px;height:14px;border-radius:3px;border:1px solid var(--border);
  box-shadow:0 1px 2px rgba(0,0,0,.6);
  background:
    linear-gradient(to bottom,transparent 43%,var(--primary) 43%,var(--primary) 57%,transparent 57%),
    linear-gradient(to bottom,color-mix(in oklab,var(--foreground) 22%,var(--card)),var(--card));}
.fts-fader::-moz-range-thumb{
  width:28px;height:14px;border-radius:3px;border:1px solid var(--border);
  box-shadow:0 1px 2px rgba(0,0,0,.6);
  background:
    linear-gradient(to bottom,transparent 43%,var(--primary) 43%,var(--primary) 57%,transparent 57%),
    linear-gradient(to bottom,color-mix(in oklab,var(--foreground) 22%,var(--card)),var(--card));}
"#;

/// Instrument category derived from a track / group name.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Inst {
    Drums,
    Bass,
    Guitars,
    Keys,
    Vocals,
    Click,
    Other,
}

impl Inst {
    /// Case-insensitive keyword match on a track or group name.
    fn from_name(name: &str) -> Self {
        let n = name.to_lowercase();
        let has = |kws: &[&str]| kws.iter().any(|k| n.contains(k));
        if has(&["click", "cue", "guide", "metronome", "count"]) {
            Inst::Click
        } else if has(&["drum", "kick", "snare", "tom", "hat", "perc", "cymbal", "ride", "crash"]) {
            Inst::Drums
        } else if has(&["bass"]) {
            Inst::Bass
        } else if has(&["guitar", "gtr", "electric", "acoustic"]) {
            Inst::Guitars
        } else if has(&["key", "piano", "synth", "organ", "pad", "rhodes"]) {
            Inst::Keys
        } else if has(&["vox", "vocal", "choir", "bgv", "lead", "singer"]) {
            Inst::Vocals
        } else {
            Inst::Other
        }
    }

    /// Group label for the left rail.
    fn label(self) -> &'static str {
        match self {
            Inst::Drums => "Drums",
            Inst::Bass => "Bass",
            Inst::Guitars => "Guitars",
            Inst::Keys => "Keys",
            Inst::Vocals => "Vocals",
            Inst::Click => "Click",
            Inst::Other => "Other",
        }
    }
}

/// A small lucide instrument icon chosen from a track/group name.
fn instrument_icon(name: &str) -> Element {
    let sz = 13usize;
    match Inst::from_name(name) {
        Inst::Drums => rsx! { Drum { size: sz, color: "currentColor" } },
        // Bass shares the guitar-family icon.
        Inst::Bass | Inst::Guitars => rsx! { Guitar { size: sz, color: "currentColor" } },
        Inst::Keys => rsx! { Piano { size: sz, color: "currentColor" } },
        Inst::Vocals => rsx! { Mic { size: sz, color: "currentColor" } },
        Inst::Click => rsx! { AlarmClock { size: sz, color: "currentColor" } },
        Inst::Other => rsx! { Music2 { size: sz, color: "currentColor" } },
    }
}

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
        "0.0".to_string()
    } else {
        format!("{db:.1}")
    }
}

/// A rail jump-target: DOM `id`, display label, and a name to pick an icon.
#[derive(Clone, PartialEq)]
struct RailGroup {
    anchor: String,
    label: String,
    icon_name: String,
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
    /// Optional live meter levels: `track_guid -> peak 0.0..=1.0`. When absent
    /// (or missing a guid) the strip shows a static gutter. The app feeds this
    /// from per-stem AnalyserNodes; this component only reads it.
    #[props(default)]
    levels: Option<HashMap<String, f32>>,
) -> Element {
    // Purely-visual hover focus (adds a ring). Authoritative selection is
    // `Track::selected`.
    let hovered = use_signal(|| Option::<String>::None);

    // Build the instrument-group rail + decide which strips get a scroll
    // anchor id. Folder/bus tracks are the natural groups; if the list has no
    // folders, fall back to instrument categories (first strip of each).
    let has_folders = tracks.iter().any(|t| t.is_folder);
    let mut rail: Vec<RailGroup> = Vec::new();
    let mut anchor_for: HashMap<String, String> = HashMap::new();
    if has_folders {
        for t in tracks.iter().filter(|t| t.is_folder) {
            let anchor = format!("mixgrp-{}", t.guid);
            anchor_for.insert(t.guid.clone(), anchor.clone());
            rail.push(RailGroup {
                anchor,
                label: t.name.clone(),
                icon_name: t.name.clone(),
            });
        }
    } else {
        let mut seen: Vec<Inst> = Vec::new();
        for t in tracks.iter() {
            let cat = Inst::from_name(&t.name);
            if !seen.contains(&cat) {
                seen.push(cat);
                let anchor = format!("mixgrp-{}", t.guid);
                anchor_for.insert(t.guid.clone(), anchor.clone());
                rail.push(RailGroup {
                    anchor,
                    label: cat.label().to_string(),
                    icon_name: t.name.clone(),
                });
            }
        }
    }

    rsx! {
        div { class: "flex h-full w-full bg-background text-foreground select-none",
            // Inject the fader-cap styling once for the whole console.
            style { dangerous_inner_html: FADER_CSS }

            // ── Left rail: instrument-group jump buttons ───────────────────
            if !rail.is_empty() {
                div {
                    class: "flex flex-col gap-1 shrink-0 border-r border-border bg-card/60 \
                            px-1.5 py-2 overflow-y-auto",
                    span {
                        class: "px-1 pb-1 text-[9px] font-bold uppercase tracking-widest \
                                text-muted-foreground",
                        "Groups"
                    }
                    for g in rail.iter().cloned() {
                        button {
                            class: "flex items-center gap-1.5 rounded px-1.5 py-1 text-left \
                                    text-[10px] font-semibold text-muted-foreground \
                                    hover:bg-accent hover:text-foreground",
                            onclick: {
                                let anchor = g.anchor.clone();
                                move |_| {
                                    let _ = dioxus::document::eval(&format!(
                                        "document.getElementById('{anchor}')?.scrollIntoView({{inline:'start',block:'nearest',behavior:'smooth'}});"
                                    ));
                                }
                            },
                            span { class: "shrink-0", {instrument_icon(&g.icon_name)} }
                            span { class: "truncate", "{g.label}" }
                        }
                    }
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
                        {
                            let guid = track.guid.clone();
                            let level = levels.as_ref().and_then(|m| m.get(&guid).copied());
                            let anchor = anchor_for.get(&guid).cloned();
                            rsx! {
                                ChannelStrip {
                                    key: "{guid}",
                                    track,
                                    on_volume,
                                    on_mute,
                                    on_solo,
                                    on_pan,
                                    hovered,
                                    level,
                                    anchor,
                                }
                            }
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
    /// Live peak level `0.0..=1.0`, or `None` for a static gutter.
    level: Option<f32>,
    /// DOM id for the group-rail scroll target, when this strip starts a group.
    anchor: Option<String>,
) -> Element {
    let guid = track.guid.clone();
    let is_bus = track.is_folder;

    // The track's own REAPER color (0xRRGGBB) — the ONLY raw color allowed for
    // chrome; everything else is theme tokens.
    let accent = track
        .color
        .map(|c| format!("#{:06x}", c & 0xFF_FF_FF))
        .unwrap_or_else(|| "#52525b".to_string());

    let db_label = fader_db_label(track.volume);
    let is_hovered = hovered.read().as_deref() == Some(guid.as_str());
    let focused = track.selected || is_hovered;

    let pan_label = if track.pan.abs() < 0.02 {
        "C".to_string()
    } else if track.pan < 0.0 {
        format!("L{}", (track.pan.abs() * 100.0).round() as i32)
    } else {
        format!("R{}", (track.pan * 100.0).round() as i32)
    };

    // Strip chrome: width, background, and a focus / bus ring.
    let width = if is_bus { "w-[68px]" } else { "w-[60px]" };
    let ring = if focused {
        "ring-2 ring-primary"
    } else if is_bus {
        "ring-1 ring-primary/40"
    } else {
        "ring-1 ring-transparent"
    };
    let bg = if is_bus { "bg-card" } else { "bg-card/70" };
    let strip_class = format!(
        "group relative flex h-full shrink-0 flex-col items-stretch overflow-hidden \
         rounded-md border border-border {bg} {width} {ring}"
    );

    // Meter fill: live level → height + green/yellow/red gradient (color rises
    // toward the top as the fill grows). Static fallback is a faint
    // volume-proportional bar (no motion, no fake data).
    let (meter_h, meter_style) = match level {
        Some(l) => {
            let pct = (l.clamp(0.0, 1.0) * 100.0) as i32;
            (
                pct,
                "background:linear-gradient(to top,#22c55e,#eab308 70%,#ef4444);".to_string(),
            )
        }
        None => {
            let pct = (track.volume.clamp(0.0, 1.0) * 100.0) as i32;
            (
                pct,
                "background:var(--muted-foreground);opacity:0.3;".to_string(),
            )
        }
    };

    let (g_vol, g_mute, g_solo, g_pan) = (guid.clone(), guid.clone(), guid.clone(), guid.clone());
    let h_enter = guid.clone();

    rsx! {
        div {
            class: "{strip_class}",
            id: anchor.map(|a| a.to_string()),
            onmouseenter: move |_| hovered.set(Some(h_enter.clone())),
            onmouseleave: move |_| hovered.set(None),

            // ── Instrument icon ────────────────────────────────────────────
            div { class: "flex items-center justify-center pt-1 text-foreground",
                {instrument_icon(&track.name)}
            }

            // ── Header: index + bus tag / arm ──────────────────────────────
            div { class: "flex items-center justify-between px-1.5",
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

            // ── Routing icon buttons (decorative desk chrome) ──────────────
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

                // Level meter gutter — prop-driven; no live data ⇒ static.
                div { class: "flex w-1.5 flex-col justify-end overflow-hidden rounded-sm bg-muted/60",
                    div {
                        class: "w-full",
                        style: "height: {meter_h}%; {meter_style}",
                    }
                }

                // Vertical fader — rotated range input with a real fader cap.
                div { class: "flex items-stretch justify-center",
                    input {
                        r#type: "range",
                        min: "0",
                        max: "1",
                        step: "0.005",
                        value: "{track.volume}",
                        class: "fts-fader h-full",
                        style: "writing-mode: vertical-lr; direction: rtl; width: 30px;",
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
                        "flex-1 rounded py-0.5 text-[10px] font-bold bg-amber-400 text-black shadow-inner"
                    } else {
                        "flex-1 rounded py-0.5 text-[10px] font-bold bg-muted text-muted-foreground hover:bg-accent"
                    },
                    onclick: move |_| on_solo.call(g_solo.clone()),
                    "S"
                }
                button {
                    class: if track.muted {
                        "flex-1 rounded py-0.5 text-[10px] font-bold bg-red-600 text-white shadow-inner"
                    } else {
                        "flex-1 rounded py-0.5 text-[10px] font-bold bg-muted text-muted-foreground hover:bg-accent"
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
