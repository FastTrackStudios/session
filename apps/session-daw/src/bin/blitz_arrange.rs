//! The ARRANGEMENT through Blitz, as ordinary components, measured.
//!
//! ```sh
//! cargo run -r -p session-daw --bin blitz_arrange
//! ```
//!
//! # The answer
//!
//! Yes, with one condition: virtualise.
//!
//! Paint never moved — 0.75 ms in every phase, whatever was changing.
//! The GPU was never the question. Nor was Dioxus: reading the pan in
//! the component that owns the items costs 5.0 ms a frame rebuilding
//! eleven hundred vnodes, but reading it inside the node that actually
//! moves drops that to 0.17 ms. The cost that remains is Blitz's style
//! and layout, and it is a function of HOW MANY NODES ARE IN THE TREE,
//! not of how many of them changed: one dirty node in a tree of 1389
//! costs 6.7 ms, the same tree with nothing dirty costs 0.5 ms, and
//! cutting the tree down cuts it proportionally.
//!
//! | on screen | frame p99 | style+layout | fps |
//! |---|---|---|---|
//! | 277 tracks, 1112 items | 8.20 ms | 6.67 ms | 122 |
//! | 60 tracks, 300 items | 2.52 ms | 1.67 ms | 396 |
//! | 60 tracks, 150 items | 1.75 ms | 0.95 ms | 570 |
//!
//! The bottom two rows are what a 2560x1440 window at a 24 px row pitch
//! can actually show. Against the mixer's 3.77 ms and the recorded
//! scene's 1.03 ms, an arrangement of ordinary components is in budget —
//! so `Item`, `Lane` and `MuteButton` can be components, one `rsx!` can
//! run on Blitz, WRY and the browser, and the session's performance view
//! can live in the same window and the same signals as this one.
//!
//! The condition is not a workaround. Blitz has no `content-visibility`,
//! so an off-screen item is a node both passes walk; the viewport
//! already knows which items are on screen (`take_window`), so the fix
//! is to not render the rest. A browser would want the same.
//!
//! # The question
//!
//! `bin/blitz.rs` asked it for the mixer and answered yes: 36 strips as
//! Dioxus components cost 3.77 ms a frame with every parameter moving,
//! against 1.03 ms for the recorded scene — 3.7× the price and still
//! 260 fps. If the arrangement answers the same way, then `Item`,
//! `Lane` and `MuteButton` are components, the same `rsx!` runs on
//! Blitz, WRY and the browser with no platform code at all, and hot
//! reload and AccessKit come free. If it does not, exactly one leaf
//! becomes a custom-painted widget and everything else stays shared.
//!
//! The arrangement is a harder question than the mixer for three
//! reasons, all of them Blitz's:
//!
//! - **There is no `content-visibility`.** The browser's own
//!   virtualisation lever does not exist here, so every item is a node
//!   the style and layout passes see.
//! - **Culling is a top-down walk.** Paint skips an item outside the
//!   clip, but it visits the tree to discover that.
//! - **Paint is not incremental.** Style and layout are; the scene is
//!   re-encoded whole, every frame.
//!
//! # What it measures
//!
//! The golden session's shape — 277 tracks, 1112 items — at
//! 2560x1440, through the same `Headless` GPU path and the same frame
//! accounting `bin/blitz.rs` uses, so the numbers are directly
//! comparable to the mixer's and to the direct path's.
//!
//! Three phases, because how a pan is EXPRESSED is the whole question:
//!
//! - **static** — nothing moves. The floor: style, layout and a full
//!   re-encode of whatever is on screen.
//! - **pan, per item** — the offset is applied to every item's own
//!   style, which is what a naive port does: 1112 style writes a frame.
//! - **pan, one property** — the offset is written once, to a CSS
//!   custom property on the root, and `calc()` moves the items. This is
//!   the trick `daw_ui::studio::arrange` is built on ("Zooming a DAW
//!   changes one number… living in CSS instead, a zoom is a single
//!   custom-property write on the root"), and whether Stylo honours it
//!   here decides whether an arrangement of components can pan at all.
//! - **pan, transform** — the offset is a `translate` on the LANE, one
//!   style write on one node, and no item's own style changes at all.
//!   A pan is not a layout change and this is the only phrasing that
//!   says so; the other two ask the engine to re-solve a tree to
//!   express a scroll.
//!
//! - **pan, scoped** — the same transform, but the signal is read
//!   inside the node that moves rather than in the component that owns
//!   the items. Nothing above it re-renders, so the item vnodes are
//!   built once and never rebuilt. This is the phase that separates
//!   "Blitz cannot pan an arrangement" from "we asked Dioxus to rebuild
//!   eleven hundred components to express a scroll".
//!
//! Each phase reports the whole frame and the paint inside it, because
//! which of the two is moving is the whole diagnosis: paint is
//! re-encoded every frame no matter what, so anything above it is style
//! and layout being asked to redo work a pan should not cause.

use std::time::Instant;

use blitz_dom::{Document as _, DocumentConfig};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;
use session_daw::headless::{BATCH, Headless};
use session_daw::profile::{Samples, Stages};

/// The same frame count the other two benchmarks use.
const FRAMES: usize = 240;

/// The golden session's shape, so this measures the thing we ship.
const TRACKS: usize = 277;
const ITEMS: usize = 1112;

/// How many items are actually in the tree.
///
/// Blitz has no `content-visibility`, so an item off screen is not free:
/// it is a node style and layout both visit. A real arrangement never
/// needs them in the tree at all — the viewport says which items are on
/// screen and the rest simply are not rendered — so this sweeps the
/// count to find out whether that lever is worth pulling.
thread_local! {
    static SHOWN: std::cell::Cell<usize> = const { std::cell::Cell::new(ITEMS) };
    /// The same, for tracks. A 1440px window at a 24px row pitch shows
    /// sixty of them; the other two hundred are a panel nobody can see
    /// that style and layout walk anyway.
    static SHOWN_TRACKS: std::cell::Cell<usize> = const { std::cell::Cell::new(TRACKS) };
}

/// How tall a lane is, in CSS pixels — the window's own row pitch.
const ROW_H: f64 = 24.0;

/// Pixels per second at rest.
const PPS: f64 = 12.0;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,blitz=info".into()),
        )
        .init();

    let (width, height) = size();
    println!("\n  The arrangement as components, Blitz + vello, on a wgpu surface\n");
    println!("  surface       {width}x{height}");
    println!("  shape         {TRACKS} tracks, {ITEMS} items (the golden session)");
    println!("  compare       mixer components 3.77ms · recorded scene 1.03ms p99");
    println!();
    println!(
        "  {:<20} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "phase", "mean", "p99", "diff", "style+lay", "paint", "fps(p99)"
    );
    println!("  {}", "-".repeat(82));

    measure("static", width, height, Drive::Still);
    measure("pan, per item", width, height, Drive::PerItem);
    measure("pan, one property", width, height, Drive::OneProperty);
    measure("pan, transform", width, height, Drive::Transform);
    measure("pan, scoped", width, height, Drive::Scoped);

    println!();
    println!("  Virtualised: the same scoped pan with fewer items in the tree");
    println!();
    println!(
        "  {:<20} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "items", "mean", "p99", "diff", "style+lay", "paint", "fps(p99)"
    );
    println!("  {}", "-".repeat(82));
    for shown in [600, 300, 150, 60] {
        SHOWN.with(|s| s.set(shown));
        measure(&format!("{shown} on screen"), width, height, Drive::Scoped);
    }

    println!();
    println!("  What is actually on a 2560x1440 screen: 60 rows at a 24px pitch");
    println!();
    println!(
        "  {:<20} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "tracks x items", "mean", "p99", "diff", "style+lay", "paint", "fps(p99)"
    );
    println!("  {}", "-".repeat(82));
    SHOWN_TRACKS.with(|s| s.set(60));
    for shown in [300, 150, 60] {
        SHOWN.with(|s| s.set(shown));
        measure(&format!("60 x {shown}"), width, height, Drive::Scoped);
    }
    SHOWN.with(|s| s.set(ITEMS));
    SHOWN_TRACKS.with(|s| s.set(TRACKS));
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

/// What moves, and how it is expressed.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Drive {
    Still,
    /// Every item's own style carries the offset.
    PerItem,
    /// One custom property on the root carries it; `calc()` does the rest.
    OneProperty,
    /// The lane is translated. Nothing inside it is touched.
    Transform,
    /// The same translate, read inside the node that moves.
    Scoped,
}

thread_local! {
    /// The pan, reachable from outside the runtime so the benchmark can
    /// drive it the way a scroll would.
    static PAN: std::cell::RefCell<Option<Signal<f64>>> =
        const { std::cell::RefCell::new(None) };
}

fn measure(name: &str, width: u32, height: u32, drive: Drive) {
    let mut renderer = Headless::new(width, height).expect("a headless renderer");
    let mut stages = Stages::with_capacity(FRAMES);

    let vdom = VirtualDom::new_with_props(Arrangement, ArrangementProps { drive });
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
    let mut drove = false;
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
            if drive != Drive::Still {
                // A pan across a minute of session, which moves every
                // item on screen — the arrangement's equivalent of the
                // mixer's "every parameter".
                let reached = document.vdom.in_runtime(|| {
                    PAN.with(|signal| {
                        let mut signal = signal.borrow_mut();
                        if let Some(signal) = signal.as_mut() {
                            signal.set(t * 60.0 * PPS);
                            true
                        } else {
                            false
                        }
                    })
                });
                assert!(reached, "the pan signal was never reachable");
                drove = true;
            }
            // The two CPU passes, apart: Dioxus reconciling the tree,
            // then Stylo and Taffy solving what came out of it. A frame
            // time says a pan is slow; only this says which pass is.
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
    if drive != Drive::Still {
        assert!(drove, "nothing drove the pan");
    }

    let (Some(frame), Some(paint), Some(diff), Some(solve)) = (
        stages.frame.summary(),
        stages.paint.summary(),
        diff.summary(),
        solve.summary(),
    ) else {
        println!("  {name:<20} nothing measured");
        return;
    };
    println!(
        "  {name:<20} {:>7.2}ms {:>7.2}ms {:>7.2}ms {:>7.2}ms {:>7.2}ms {:>9.0}",
        frame.mean,
        frame.p99,
        diff.mean,
        solve.mean,
        paint.mean,
        1000.0 / frame.p99.max(0.001)
    );
}

#[derive(Props, Clone, PartialEq)]
struct ArrangementProps {
    drive: Drive,
}

/// The arrangement's SHAPE as elements: a ruler, a track panel, and a
/// lane per track with its items in it.
///
/// Not the real components — `daw-ui`'s reach for a store and a theme
/// provider would measure the plumbing. This is what the layout engine
/// is actually asked to solve: a tall column of lanes, each holding a
/// few absolutely-positioned boxes, under a ruler.
#[component]
fn Arrangement(props: ArrangementProps) -> Element {
    let pan = use_signal(|| 0.0_f64);
    use_hook(|| {
        PAN.with(|slot| *slot.borrow_mut() = Some(pan));
    });
    // Read here, EXCEPT when the point of the phase is not to: `Scoped`
    // leaves the signal untouched at this level so this component never
    // re-runs, and `Panner` below reads it instead.
    let offset = if props.drive == Drive::Scoped {
        0.0
    } else {
        pan()
    };
    let items = items();

    // The two ways to express the pan. Per item, every box carries the
    // offset in its own style. One property, the root carries it and
    // `calc()` reads it — one write instead of eleven hundred.
    let (root_var, per_item) = match props.drive {
        Drive::OneProperty => (format!("--pan: {offset}px;"), 0.0),
        Drive::Transform => (String::new(), 0.0),
        _ => (String::new(), offset),
    };
    // A pan as what it is: the lane slides. Nothing in it is restyled.
    let lane_transform = if props.drive == Drive::Transform {
        format!("transform: translateX({}px);", -offset)
    } else {
        String::new()
    };

    rsx! {
        div {
            style: "display:flex; flex-direction:row; background:#18181b; \
                    width:100%; height:100%; overflow:hidden; {root_var}",

            // The track panel: a name and three buttons per track, the
            // same column the window draws.
            div {
                style: "width:280px; flex:none; background:#1e1e22;",
                for track in 0..SHOWN_TRACKS.with(std::cell::Cell::get) {
                    div {
                        style: "display:flex; flex-direction:row; align-items:center; \
                                height:{ROW_H}px; border-bottom:1px solid #26262a;",
                        div { style: "flex:1; padding-left:6px; font-size:11px;", "Track {track}" }
                        div { style: "width:16px; height:14px; background:#2a2a2e; margin-right:2px;", "M" }
                        div { style: "width:16px; height:14px; background:#2a2a2e; margin-right:2px;", "S" }
                        div { style: "width:16px; height:14px; background:#2a2a2e; margin-right:4px;" }
                    }
                }
            }

            // The lanes, with the items in them.
            if props.drive == Drive::Scoped {
                Panner {
                    pan,
                    children: rsx! {
                        for (row, start, length) in items.clone() {
                            Item { row, start, length, offset: 0.0, by_property: false }
                        }
                    },
                }
            } else {
                div {
                    style: "flex:1; position:relative; overflow:hidden; {lane_transform}",
                    for (row, start, length) in items {
                        Item {
                            row,
                            start,
                            length,
                            offset: per_item,
                            by_property: props.drive == Drive::OneProperty,
                        }
                    }
                }
            }
        }
    }
}

/// The node that moves, and the only one that knows the pan.
///
/// Its children are built by the component above, which does not read
/// the signal — so a pan re-runs exactly this function and hands the
/// same item vnodes straight back.
#[component]
fn Panner(pan: Signal<f64>, children: Element) -> Element {
    let offset = pan();
    rsx! {
        div {
            style: "flex:1; position:relative; overflow:hidden; \
                    transform: translateX({-offset}px);",
            {children}
        }
    }
}

/// One item.
#[component]
fn Item(row: usize, start: f64, length: f64, offset: f64, by_property: bool) -> Element {
    #[expect(
        clippy::cast_precision_loss,
        clippy::as_conversions,
        reason = "a row index under a thousand"
    )]
    let top = row as f64 * ROW_H;
    let width = (length * PPS).max(2.0);
    let height = ROW_H - 2.0;
    // `calc(var(--pan))` when the root carries the offset; a resolved
    // number when every item carries its own.
    let left = if by_property {
        format!("calc({}px - var(--pan, 0px))", start * PPS)
    } else {
        format!("{}px", start.mul_add(PPS, -offset))
    };
    rsx! {
        div {
            style: "position:absolute; top:{top}px; left:{left}; width:{width}px; \
                    height:{height}px; background:#7a2a2a; border:1px solid #9a3a3a;",
        }
    }
}

/// The golden session's items, spread over its tracks.
///
/// Deterministic rather than random: two runs have to be comparable,
/// and a benchmark whose input changes measures the input.
fn items() -> Vec<(usize, f64, f64)> {
    (0..SHOWN.with(std::cell::Cell::get))
        .map(|i| {
            let row = i % TRACKS;
            #[expect(
                clippy::cast_precision_loss,
                clippy::as_conversions,
                reason = "an item index under a few thousand"
            )]
            let n = i as f64;
            // Spread across about three minutes, in clumps, the way a
            // session actually is.
            let start = (n * 7.3) % 180.0;
            let length = 4.0 + (n * 1.7) % 12.0;
            (row, start, length)
        })
        .collect()
}
