//! The mixer through Blitz, on the same GPU path, measured.
//!
//! ```sh
//! just daw-blitz
//! ```
//!
//! # The question
//!
//! Every layout bug in this window has been the same shape: a value
//! that has to match, worked out twice from different inputs. The
//! button line recomputed. The strip height recomputed. The bands
//! resolved once for the recorded chrome and again for the live
//! controls. A layout engine removes that class outright — you declare
//! once, it solves once, and draw and hit-test both read the answer.
//!
//! `daw-ui` already HAS the mixer as Dioxus components. So the question
//! is not whether to write it, it is whether laying it out costs the
//! frame budget the direct path currently meets: 1.03 ms p99 at
//! 2560x1440 with every parameter moving.
//!
//! # What this measures
//!
//! `blitz_paint::paint_scene` takes any `PaintScene`, which is the same
//! trait `anyrender_vello` implements and the same one the direct path
//! draws through. So both paths end in the identical rasteriser on the
//! identical GPU, and the difference between them is exactly the DOM:
//! style resolution, layout, and painting a tree instead of a list.
//!
//! Deliberately `vello` and not `vello-hybrid`. Hybrid does much of its
//! path work on the CPU — it exists for WebGL-class targets — and on a
//! desktop GPU it is what stood between the expression editor and its
//! budget: 35 fps at 5120x1440 with five notes on screen, where the
//! scene was 69 draw commands and 0.04 ms to build. Measuring hybrid
//! here would measure that, not Blitz.

use std::time::Instant;

use blitz_dom::{Document as _, DocumentConfig};
use blitz_traits::shell::{ColorScheme, Viewport};
use daw_proto::Track;
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;
use session_daw::headless::{BATCH, Headless};
use session_daw::profile::Stages;

/// How many frames each measurement runs for. The same count the direct
/// path uses, so the two numbers are comparable.
const FRAMES: usize = 240;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,blitz=info".into()),
        )
        .init();

    let (width, height) = size();
    let strips = std::env::var("FTS_BLITZ_STRIPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(36_usize);

    println!("\n  Blitz + vello, straight to a wgpu surface\n");
    println!("  surface       {width}x{height}");
    println!("  strips        {strips}");
    println!(
        "  measured      the SAME rasteriser the direct path uses, so the\n  \
              {:14}difference is the DOM: style, layout, and painting a tree",
        ""
    );
    println!();
    println!(
        "  {:<20} {:>9} {:>9} {:>9} {:>9}",
        "phase", "mean", "p99", "worst", "fps(p99)"
    );
    println!("  {}", "-".repeat(62));

    measure("static", width, height, strips, false);
    measure("every parameter", width, height, strips, true);
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

/// One measurement: build the document once, then drive it.
fn measure(name: &str, width: u32, height: u32, strips: usize, animate: bool) {
    let mut renderer = Headless::new(width, height).expect("a headless renderer");
    let mut stages = Stages::with_capacity(FRAMES);

    let mut document = build(strips, width, height);
    // The first render has to happen before anything can drive it: the
    // signal is created inside the component, so until the component
    // has run there is nothing to set.
    document.initial_build();
    document.poll(None);
    let mut drove = false;
    let mut dirty = 0_usize;
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
            if animate {
                // How a Dioxus app actually changes: set a signal,
                // let the runtime re-render what depends on it, and let
                // the mutation writer diff that into the DOM.
                //
                // Rebuilding the whole VirtualDom instead measured 24ms
                // a frame — which is the cost of throwing the
                // application away sixty times a second, not the cost
                // of moving a fader, and quoting it would have been a
                // straw man.
                let reached = document.vdom.in_runtime(|| {
                    TRACKS.with(|signal| {
                        let mut signal = signal.borrow_mut();
                        if let Some(signal) = signal.as_mut() {
                            signal.set(tracks(strips, t));
                            true
                        } else {
                            false
                        }
                    })
                });
                // A benchmark that measured nothing changing would
                // report a number that means nothing, and it would
                // report it as a win.
                assert!(reached, "the mixer's state signal was never reachable");
                drove = true;
            }
            if document.poll(None) {
                dirty += 1;
            }
            {
                let mut inner = document.inner_mut();
                inner.resolve(0.0);
            }
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
    if animate {
        assert!(drove, "nothing drove the state");
        assert!(
            dirty > FRAMES / 2,
            "only {dirty} of {FRAMES} frames had work to do — the signal is \
             not reaching the DOM, so this measures a static page"
        );
    }
    stages.frame.drop_warmup(1);
    let frame = stages.frame.summary().expect("batches");
    println!(
        "  {name:<20} {:>7.2}ms {:>7.2}ms {:>7.2}ms {:>9.0}",
        frame.mean,
        frame.p99,
        frame.worst,
        frame.fps(),
    );
}

fn build(strips: usize, width: u32, height: u32) -> DioxusDocument {
    build_at(strips, width, height, 0.0)
}

fn build_at(strips: usize, width: u32, height: u32, t: f64) -> DioxusDocument {
    let tracks = tracks(strips, t);
    let vdom = VirtualDom::new_with_props(Mixer, MixerProps { tracks });
    DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(width, height, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    )
}

fn tracks(n: usize, t: f64) -> Vec<Track> {
    let mut tracks: Vec<Track> = (0..n)
        .map(|i| Track {
            guid: format!("t{i}"),
            name: format!("Track {i}"),
            volume: 1.0,
            #[expect(
                clippy::cast_possible_truncation,
                clippy::as_conversions,
                reason = "a track index in a benchmark"
            )]
            index: i as u32,
            ..Track::default()
        })
        .collect();
    session_daw::animate::drive(&mut tracks, t);
    tracks
}

#[derive(Props, Clone, PartialEq)]
struct MixerProps {
    tracks: Vec<Track>,
}

/// A strip with the controls the direct path draws, as elements.
///
/// Not `daw-ui`'s own mixer component: that one reaches for a
/// `TrackStore` from context and a theme provider, and wiring those in
/// would measure the plumbing as much as the layout. This is the same
/// SHAPE — a row of strips, each a column of controls — which is what
/// the layout engine is being asked to solve.
thread_local! {
    /// The state the mixer reads, reachable from outside the runtime so
    /// the benchmark can drive it the way an engine event would.
    static TRACKS: std::cell::RefCell<Option<Signal<Vec<Track>>>> =
        const { std::cell::RefCell::new(None) };
}

#[component]
fn Mixer(props: MixerProps) -> Element {
    let tracks = use_signal(|| props.tracks.clone());
    use_hook(|| {
        TRACKS.with(|slot| *slot.borrow_mut() = Some(tracks));
    });
    let tracks = tracks.read().clone();
    rsx! {
        div {
            style: "display:flex; flex-direction:row; background:#18181b; width:100%; height:100%;",
            for track in tracks.iter().cloned() {
                Strip { track }
            }
        }
    }
}

#[component]
fn Strip(track: Track) -> Element {
    let lit = |on: bool| if on { "#4a9eff" } else { "#2a2a2e" };
    // The fader's fill, as a percentage — the same value the direct
    // path puts through the taper.
    let fill = (daw_theme_art::paint::tcp::gain_norm(track.volume) * 100.0).clamp(0.0, 100.0);
    let pan = (track.pan.clamp(-1.0, 1.0) + 1.0) * 50.0;
    rsx! {
        div {
            style: "display:flex; flex-direction:column; width:133px; \
                    margin-right:1px; background:#1e1e22; height:100%;",
            // The rack's three panels.
            for _ in 0..3 {
                div { style: "flex:1; margin:2px; background:#0d0d10;" }
            }
            // The coloured band, with the pan knob in it.
            div {
                style: "height:56px; background:#3a2a20; position:relative;",
                div {
                    style: "position:absolute; top:8px; left:{pan}%; width:10px; \
                            height:10px; border-radius:5px; background:#e0b040;",
                }
            }
            // The button column and the fader.
            div {
                style: "display:flex; flex-direction:row; height:40%;",
                div {
                    style: "width:22px; margin-left:56px; background:#0a0a0c; \
                            position:relative;",
                    div {
                        style: "position:absolute; bottom:0; width:100%; \
                                height:{fill}%; background:#4a9eff;",
                    }
                }
                div {
                    style: "display:flex; flex-direction:column; margin-left:8px;",
                    div { style: "width:21px; height:20px; background:{lit(track.muted)};", "M" }
                    div { style: "width:21px; height:20px; background:{lit(track.soloed)};", "S" }
                    div { style: "width:21px; height:20px; background:{lit(track.armed)};" }
                }
            }
            div { style: "height:26px; background:#26262a;", "{track.name}" }
        }
    }
}
