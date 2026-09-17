//! A track panel's controls, as vector art and as CSS, measured.
//!
//! ```sh
//! cargo run -r -p session-daw --bin blitz_controls
//! ```
//!
//! # The question
//!
//! The component track panel draws its knobs, faders, meters and
//! buttons as inline `<svg>`, because that is what the REAPER theme's
//! art is and matching it was the point. Blitz renders an inline `<svg>`
//! by serialising the subtree back to markup and handing it to usvg —
//! so the art is not free the way it is in a browser, and a panel is
//! 154 of those nodes a row across sixty rows on screen.
//!
//! CSS can draw the same controls with no child nodes at all: a knob is
//! a `conic-gradient` in a `border-radius: 50%` box, a fader is a
//! `linear-gradient` rail with one grip div, a meter is a gradient
//! clipped by its own width, a button is a border and a background. One
//! element each, no markup to serialise, no parser to run.
//!
//! # What it measures
//!
//! Sixty rows — what a 1440px window shows at a 24px pitch — of the
//! same six controls, built both ways, in three phases:
//!
//! - **static** — nothing moves. The cost of simply having the panel.
//! - **one meter** — a single track's meter moves, which is one style
//!   write on one node and the most common thing a panel ever does.
//! - **every meter** — all sixty move, which is what a playing session
//!   does sixty times a second.
//!
//! The last phase is the one that decides it. A control whose art is
//! serialised and re-parsed when its value changes cannot be on a meter
//! feed, however cheap it is to hold still.
//!
//! # The answer
//!
//! CSS wins, but not the CSS you would write first.
//!
//! | | static | every meter | paint |
//! |---|---|---|---|
//! | svg | 1.11 ms | 1.97 ms | 0.72 ms |
//! | css, gradients and radii | 2.64 ms | 3.95 ms | 1.70 ms |
//! | css, flat fills | **1.08 ms** | **1.74 ms** | **0.69 ms** |
//!
//! Flat CSS beats the vector art on every phase, and beats it by most
//! where it matters: a meter feed moving sixty meters costs 1.74 ms
//! against the art's 1.97, with style and layout at 0.43 ms against
//! 0.65. No markup is serialised and no parser runs.
//!
//! The middle row is the trap. A `conic-gradient` knob and a
//! `border-radius` button are the obvious way to draw a control in CSS
//! and they cost MORE than the vector art they replaced — a full
//! millisecond a frame, all of it in paint, because Vello re-encodes
//! every gradient brush and turns every rounded corner into a path on
//! every frame, whether or not it changed. Paint is the one pass that is
//! never incremental, so a gradient is a per-frame cost forever.
//!
//! So the rule for anything that repeats per row: flat fills, square
//! corners, one element. Gradients and radii are affordable on chrome
//! that appears once — a header, a dialog — and nowhere else.

use std::time::Instant;

use blitz_dom::{Document as _, DocumentConfig};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;
use session_daw::headless::{BATCH, Headless};
use session_daw::profile::{Samples, Stages};

/// The same frame count the other benchmarks use.
const FRAMES: usize = 240;

/// What a 2560x1440 window shows at the panel's own row pitch.
const ROWS: usize = 60;

/// The panel's row height, from the measured REAPER geometry.
const ROW_H: f64 = 24.0;

/// The panel's width, likewise.
const PANEL_W: f64 = 343.0;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,blitz=info,usvg=error".into()),
        )
        .init();

    let (width, height) = size();
    println!("\n  A track panel's controls: vector art against CSS\n");
    println!("  surface       {width}x{height}");
    println!("  shape         {ROWS} rows, six controls each");
    println!();
    println!(
        "  {:<24} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "phase", "mean", "p99", "diff", "style+lay", "paint", "fps(p99)"
    );
    println!("  {}", "-".repeat(86));

    for art in [Art::Svg, Art::Css, Art::Flat] {
        for moving in [Moving::Nothing, Moving::OneMeter, Moving::EveryMeter] {
            measure(art, moving, width, height);
        }
    }
}

fn size() -> (u32, u32) {
    std::env::var("FTS_BLITZ_SIZE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
        })
        .unwrap_or((2560, 1440))
}

/// How a control is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Art {
    /// Inline `<svg>`, the way the component panel draws it today.
    Svg,
    /// Gradients, radii and borders — one element per control.
    Css,
    /// The same elements with flat fills: no gradient, no radius.
    ///
    /// The third arm exists because the first comparison does not
    /// separate "CSS" from "a gradient". Vello encodes a gradient brush
    /// every frame whether or not it changed, and a rounded corner is a
    /// path where a square one is a rect — so if CSS is expensive here,
    /// this says whether it is CSS or the paint those two properties ask
    /// for.
    Flat,
}

impl Art {
    const fn name(self) -> &'static str {
        match self {
            Self::Svg => "svg",
            Self::Css => "css",
            Self::Flat => "css flat",
        }
    }
}

/// What changes between frames.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Moving {
    Nothing,
    OneMeter,
    EveryMeter,
}

impl Moving {
    const fn name(self) -> &'static str {
        match self {
            Self::Nothing => "static",
            Self::OneMeter => "one meter",
            Self::EveryMeter => "every meter",
        }
    }
}

thread_local! {
    /// The meter level, driven from outside the runtime the way a meter
    /// feed drives it from a socket.
    static LEVEL: std::cell::RefCell<Option<Signal<f64>>> =
        const { std::cell::RefCell::new(None) };
}

fn measure(art: Art, moving: Moving, width: u32, height: u32) {
    let mut renderer = Headless::new(width, height).expect("a headless renderer");
    let mut stages = Stages::with_capacity(FRAMES);

    let vdom = VirtualDom::new_with_props(Panel, PanelProps { art, moving });
    let mut document = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(width, height, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    document.initial_build();
    document.poll(None);

    let mut diff = Samples::with_capacity(FRAMES);
    let mut solve = Samples::with_capacity(FRAMES);
    for batch in 0..FRAMES / BATCH {
        let batch_start = Instant::now();
        let mut painted = 0.0;
        for step in 0..BATCH {
            let frame = batch * BATCH + step;
            #[expect(
                clippy::cast_precision_loss,
                clippy::as_conversions,
                reason = "a frame index over a few hundred"
            )]
            let t = frame as f64 / FRAMES as f64;
            if moving != Moving::Nothing {
                let reached = document.vdom.in_runtime(|| {
                    LEVEL.with(|signal| {
                        let mut signal = signal.borrow_mut();
                        if let Some(signal) = signal.as_mut() {
                            signal.set((t * std::f64::consts::TAU).sin().abs());
                            true
                        } else {
                            false
                        }
                    })
                });
                assert!(reached, "the meter signal was never reachable");
            }
            let at = Instant::now();
            document.poll(None);
            diff.push_ms(at.elapsed().as_secs_f64() * 1000.0);
            let at = Instant::now();
            {
                let mut inner = document.inner_mut();
                inner.resolve(0.0);
            }
            solve.push_ms(at.elapsed().as_secs_f64() * 1000.0);
            painted += renderer
                .frame(|painter| {
                    let mut inner = document.inner_mut();
                    blitz_paint::paint_scene(painter, &mut inner, 1.0, width, height, 0, 0);
                })
                .expect("render a frame");
        }
        renderer.wait().expect("the gpu to finish the batch");
        #[expect(
            clippy::cast_precision_loss,
            clippy::as_conversions,
            reason = "a batch size of thirty"
        )]
        let per_frame = batch_start.elapsed().as_secs_f64() * 1000.0 / BATCH as f64;
        stages.frame.push_ms(per_frame);
        stages.paint.push_ms(painted / BATCH as f64);
    }

    let (Some(frame), Some(paint), Some(diff), Some(solve)) = (
        stages.frame.summary(),
        stages.paint.summary(),
        diff.summary(),
        solve.summary(),
    ) else {
        println!("  nothing measured");
        return;
    };
    let name = format!("{}, {}", art.name(), moving.name());
    println!(
        "  {name:<24} {:>7.2}ms {:>7.2}ms {:>7.2}ms {:>7.2}ms {:>7.2}ms {:>9.0}",
        frame.mean,
        frame.p99,
        diff.mean,
        solve.mean,
        paint.mean,
        1000.0 / frame.p99.max(0.001)
    );
}

#[derive(Props, Clone, PartialEq)]
struct PanelProps {
    art: Art,
    moving: Moving,
}

/// The panel: sixty rows of controls.
#[component]
fn Panel(props: PanelProps) -> Element {
    let level = use_signal(|| 0.0_f64);
    use_hook(|| {
        LEVEL.with(|slot| *slot.borrow_mut() = Some(level));
    });
    rsx! {
        div {
            style: "width:{PANEL_W}px; height:100%; background:#1e1e22; overflow:hidden;",
            for row in 0..ROWS {
                Row {
                    row,
                    art: props.art,
                    // Only the rows the phase says are moving take the
                    // signal. A row that does not read it is a row a
                    // meter tick cannot re-render, which is the whole
                    // difference between the second phase and the third.
                    level: match props.moving {
                        Moving::Nothing => None,
                        Moving::OneMeter => (row == 0).then_some(level),
                        Moving::EveryMeter => Some(level),
                    },
                }
            }
        }
    }
}

/// One row: a name, mute and solo, a pan knob, a fader and a meter.
#[component]
fn Row(row: usize, art: Art, level: Option<Signal<f64>>) -> Element {
    // Read here rather than inside the meter, so a row that is not on
    // the feed never subscribes to it at all.
    let lit = level.map_or(0.4, |level| level());
    rsx! {
        div {
            style: "display:flex; flex-direction:row; align-items:center; gap:4px; \
                    height:{ROW_H}px; padding:0 4px; border-bottom:1px solid #26262a;",
            div { style: "flex:1; font-size:11px; color:#c8c8cc; overflow:hidden;", "Track {row}" }
            Toggle { art, on: row % 3 == 0, label: "M", tint: "#c8a33a" }
            Toggle { art, on: row % 7 == 0, label: "S", tint: "#3a8ac8" }
            Knob { art, value: (row % 21) as f64 / 20.0 }
            Fader { art, value: 0.72 }
            Meter { art, level: lit }
        }
    }
}

/// Mute, solo — a square that lights.
#[component]
fn Toggle(art: Art, on: bool, label: String, tint: String) -> Element {
    let face = if on {
        tint.clone()
    } else {
        "#2a2a2e".to_owned()
    };
    let ink = if on { "#14141a" } else { "#8a8a92" };
    match art {
        Art::Flat => rsx! {
            div {
                style: "width:16px; height:14px; flex:none; background:{face}; \
                        color:{ink}; font-size:9px; text-align:center; line-height:14px;",
                "{label}"
            }
        },
        Art::Css => rsx! {
            div {
                style: "width:16px; height:14px; flex:none; border-radius:2px; \
                        border:1px solid #3a3a40; background:{face}; color:{ink}; \
                        font-size:9px; text-align:center; line-height:14px;",
                "{label}"
            }
        },
        Art::Svg => rsx! {
            svg {
                width: "16",
                height: "14",
                view_box: "0 0 16 14",
                rect {
                    x: "0.5",
                    y: "0.5",
                    width: "15",
                    height: "13",
                    rx: "2",
                    fill: "{face}",
                    stroke: "#3a3a40",
                }
                text { x: "8", y: "10", "text-anchor": "middle", "font-size": "9", fill: "{ink}",
                    "{label}"
                }
            }
        },
    }
}

/// The pan knob — a dial with a pointer.
#[component]
fn Knob(art: Art, value: f64) -> Element {
    // Three quarters of a turn, centred at the top, the way every DAW
    // draws a pan.
    let sweep = value.mul_add(270.0, -135.0);
    match art {
        Art::Flat => rsx! {
            div {
                style: "position:relative; width:18px; height:18px; flex:none; \
                        background:#32323a;",
                div {
                    style: "position:absolute; left:50%; top:2px; width:1px; height:6px; \
                            background:#d0d0d6; transform: translateX(-50%) rotate({sweep}deg); \
                            transform-origin: 50% 7px;",
                }
            }
        },
        Art::Css => rsx! {
            div {
                // The dial: a conic sweep for the filled arc, a radial
                // face over it, and the pointer as a rotated pseudo-free
                // child. Two elements, no markup.
                style: "position:relative; width:18px; height:18px; flex:none; \
                        border-radius:50%; border:1px solid #3a3a40; \
                        background: conic-gradient(from 225deg, #4a9ad8 0deg \
                        {value * 270.0}deg, #2a2a2e {value * 270.0}deg 270deg, \
                        transparent 270deg 360deg), radial-gradient(#3a3a42, #26262c);",
                div {
                    style: "position:absolute; left:50%; top:2px; width:1px; height:6px; \
                            background:#d0d0d6; transform: translateX(-50%) rotate({sweep}deg); \
                            transform-origin: 50% 7px;",
                }
            }
        },
        Art::Svg => rsx! {
            svg {
                width: "18",
                height: "18",
                view_box: "0 0 18 18",
                circle { cx: "9", cy: "9", r: "8", fill: "#32323a", stroke: "#3a3a40" }
                path {
                    d: "M9 9 L9 2",
                    stroke: "#d0d0d6",
                    "stroke-width": "1",
                    transform: "rotate({sweep} 9 9)",
                }
            }
        },
    }
}

/// The volume fader — a rail with a grip on it.
#[component]
fn Fader(art: Art, value: f64) -> Element {
    let grip = (1.0 - value) * 44.0;
    match art {
        Art::Flat => rsx! {
            div {
                style: "position:relative; width:52px; height:16px; flex:none; \
                        background:#202026;",
                div {
                    style: "position:absolute; top:2px; left:{44.0 - grip}px; width:6px; \
                            height:10px; background:#c8c8ce;",
                }
            }
        },
        Art::Css => rsx! {
            div {
                style: "position:relative; width:52px; height:16px; flex:none; \
                        background: linear-gradient(#1a1a1e, #26262c); \
                        border:1px solid #3a3a40; border-radius:2px;",
                div {
                    style: "position:absolute; top:2px; left:{44.0 - grip}px; width:6px; \
                            height:10px; border-radius:1px; \
                            background: linear-gradient(#d8d8de, #8a8a92);",
                }
            }
        },
        Art::Svg => rsx! {
            svg {
                width: "52",
                height: "16",
                view_box: "0 0 52 16",
                rect { x: "0.5", y: "0.5", width: "51", height: "15", rx: "2", fill: "#202026", stroke: "#3a3a40" }
                rect { x: "{44.0 - grip}", y: "3", width: "6", height: "10", rx: "1", fill: "#c8c8ce" }
            }
        },
    }
}

/// The meter — the one control that moves every frame.
#[component]
fn Meter(art: Art, level: f64) -> Element {
    let lit = (level.clamp(0.0, 1.0) * 40.0).round();
    match art {
        Art::Flat => rsx! {
            div {
                style: "position:relative; width:40px; height:8px; flex:none; \
                        background:#17171b;",
                div { style: "width:{lit}px; height:8px; background:#3ac86a;" }
            }
        },
        Art::Css => rsx! {
            div {
                style: "width:40px; height:8px; flex:none; background:#17171b; \
                        border:1px solid #2a2a30; \
                        background-image: linear-gradient(90deg, #3ac86a 0 {lit}px, \
                        transparent {lit}px 40px);",
            }
        },
        Art::Svg => rsx! {
            svg {
                width: "40",
                height: "8",
                view_box: "0 0 40 8",
                rect { x: "0", y: "0", width: "40", height: "8", fill: "#17171b" }
                rect { x: "0", y: "0", width: "{lit}", height: "8", fill: "#3ac86a" }
            }
        },
    }
}
