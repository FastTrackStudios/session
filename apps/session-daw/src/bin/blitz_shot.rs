//! The component arrangement, rendered to a picture.
//!
//! ```sh
//! cargo run -r -p session-daw --bin blitz_shot -- \
//!     features/dynamic-template/fixtures/golden/template.rpp /tmp/lanes.png
//! ```
//!
//! The same lanes the recorded scene draws, built from
//! [`daw_ui::studio::lanes`] and rendered through Blitz on to a wgpu
//! surface — so the component renderer and the direct one can be
//! compared as two PNGs rather than as two opinions.
//!
//! What comes out is the LANE RECT alone: the region the reference shot
//! (`FTS_BENCH_SHOT`) draws from the track panel's right edge to the
//! frame's, under the ruler. Cropping the reference to the same rect is
//! what makes the comparison a comparison — see
//! `apps/session-daw/tests/component_lanes.rs`.

use std::collections::HashMap;
use std::sync::Arc;

use std::time::Instant;

use anyrender::ImageRenderer as _;
use anyrender_vello::VelloImageRenderer;
use blitz_dom::{Document as _, DocumentConfig};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;

use daw_ui::studio::lanes::{Colors, Grid, Lanes, Note, Rows, Shape, Shapes, View};
use daw_ui::studio::panel::Panel;
use daw_ui::studio::rails::{Item, ModeBar, Rails};
use daw_ui::studio::ruler::{Marks, Reading, Ruler, Tick};
use daw_ui::studio::{ProjectRef, RowsRef};

/// Pixels per second, the reference shot's own.
const PPS: f64 = 40.0;

/// How many peaks a second an item's shape carries.
///
/// The recorded scene's `WAVE_POINTS_PER_SECOND`, because a shape
/// sampled at a different rate is a different picture however right
/// each one is on its own.
const PEAKS_PER_SECOND: f64 = 12.0;

/// How many sub-samples each peak holds the maximum of.
///
/// The scene's `WAVE_HOLD`. A peak is the loudest thing in the stretch
/// it stands for, not the level at the instant it was sampled — which is
/// the difference between a waveform and a sampling of one.
const HOLD: usize = 4;

fn main() {
    // A window writes to a LOG rather than to a terminal it does not
    // have. `FTS_BLITZ_LOG` names the file; a window defaults to one, so
    // that what the run did can be read afterwards instead of watched
    // live — which is the only way to know what a window did on somebody
    // else's screen.
    let log = std::env::var("FTS_BLITZ_LOG")
        .ok()
        .or_else(|| std::env::var_os("FTS_BLITZ_WINDOW").map(|_| "/tmp/fts-studio.log".to_owned()));
    let filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(
            |_| // `blitz_shot` and not `session_daw`: a binary's tracing target is
            // the BINARY's crate name, so a filter naming the library it
            // lives beside silently drops everything this file says.
            "warn,blitz=info,usvg=error,session_daw=info,blitz_shot=info,daw_ui=info".into(),
        )
    };
    if let Some(path) = log.as_deref() {
        match std::fs::File::create(path) {
            Ok(file) => {
                tracing_subscriber::fmt()
                    .with_env_filter(filter())
                    .with_ansi(false)
                    .with_writer(std::sync::Mutex::new(file))
                    .init();
                println!("logging to {path}");
            }
            Err(error) => {
                tracing_subscriber::fmt().with_env_filter(filter()).init();
                tracing::warn!(%error, path, "could not open the log; using stderr");
            }
        }
    } else {
        tracing_subscriber::fmt().with_env_filter(filter()).init();
    }

    let mut args = std::env::args().skip(1);
    let (Some(project_path), Some(out)) = (args.next(), args.next()) else {
        eprintln!("blitz_shot <song.rpp> <out.png> [scroll_x] [scroll_y]");
        std::process::exit(2);
    };
    let scroll_x = args.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let scroll_y = args.next().and_then(|v| v.parse().ok()).unwrap_or(0.0);

    // Which part of the window is being drawn. Each is compared against
    // the same part of the reference shot, so they are converted — and
    // proven — one at a time rather than all at once.
    // A window is the WHOLE window unless something asks otherwise. The
    // single-surface modes exist so each one can be compared against the
    // reference on its own; opening one of those as a window and calling
    // it the studio is how you end up looking at an arrangement with no
    // panel beside it and no ruler over it.
    let part = std::env::var("FTS_BLITZ_PART").unwrap_or_else(|_| {
        if std::env::var_os("FTS_BLITZ_WINDOW").is_some() {
            "all".to_owned()
        } else {
            "lanes".to_owned()
        }
    });
    let (width, height) = size();
    // The lane rect: what is left of the frame once the rails and the
    // track panel and the ruler have taken theirs. The component
    // renderer draws only this, because this is the part that was in
    // question — the panel beside it is already components.
    let ruler = part == "ruler";
    let rails = part == "rails";
    // The whole window, with every converted surface in its place. The
    // point of this one is not any single picture — it is that the
    // pieces compose: one tree, one document, one paint.
    let all = part == "all";
    let panel = part == "panel";
    let lane_w = if panel {
        daw_ui::studio::panel::ROW_W
    } else if rails || all {
        width
    } else if ruler {
        (width - session_daw::rails::SIDE * 2.0).max(1.0)
    } else {
        frame_width(width)
    };
    let lane_h = if panel {
        frame_height(height)
    } else if rails || all {
        height
    } else if ruler {
        session_daw::ruler::RULER_H
    } else {
        frame_height(height)
    };

    session_daw::open::open_silent(std::path::Path::new(&project_path)).expect("open project");
    // A window opens the session the way the visual track manager lays
    // it out; a comparison shot does not, because the reference it is
    // compared against does not either.
    let scene = std::env::var("FTS_BLITZ_SCENE")
        .ok()
        .or_else(|| std::env::var_os("FTS_BLITZ_WINDOW").map(|_| "drum-mixing".to_owned()));
    let (project, rows) = read_back(scene.as_deref(), std::path::Path::new(&project_path))
        .expect("read the project back");
    // `FTS_BLITZ_SHAPES=0` draws the lanes without waveforms or notes.
    // Not a mode anyone wants to look at — it is the control that says
    // how much of a frame the shapes are, which is the only way to know
    // whether an optimisation aimed at them is aimed at anything.
    // Read once, used twice: the component tree turns these into
    // `Shapes` and the widget's recording draws them directly. Building
    // the widget from an empty set is how its trigger rows came out
    // blank while every waveform beside them was right.
    let previews = previews_of(&project);
    let shapes = if std::env::var("FTS_BLITZ_SHAPES").as_deref() == Ok("0") {
        Shapes::default()
    } else {
        shapes_of(&project, &previews)
    };

    let view = View {
        scroll_x,
        scroll_y,
        pps: PPS,
        zoom_y: 1.0,
        width: lane_w,
        height: lane_h,
    };
    // The reference shot's own theme and row sizing. A picture drawn
    // from different numbers is a different picture, however right each
    // set is on its own.
    let theme = daw_ui::theming::Theme::dark();
    let colors = Colors::from_theme(&theme);
    let layout = session_daw::layout::Layout::from_env();
    let sizing = Rows {
        default: layout.height_of(None),
        min: layout.height_of(Some(1)),
    };
    let grid = grid_of(&project, view);
    let project_for_live = project.clone();
    let marks = marks_of(&project);
    let sections = project.sections.clone().into();
    let markers = project.markers.clone().into();

    // The arrangement as ONE node, which is how this window runs.
    //
    // `FTS_BLITZ_WIDGET=0` builds it as a tree instead. Everything above
    // is the same — the same project, the same rows, the same scene — so
    // the two are the same window drawn two ways, which is what
    // `tests/component_lanes.rs` compares and what the numbers in
    // `session_daw::widget` were measured from. The tree is kept for
    // exactly that: it is the thing the widget is checked against, not
    // a mode anybody is meant to run.
    let widget = std::env::var("FTS_BLITZ_WIDGET").as_deref() != Ok("0");
    // The frame-time graph. On in a window and off in a shot, because a
    // shot is compared pixel for pixel against the painted renderer and
    // an overlay is a difference. `FTS_BLITZ_FPS=0` turns it off in a
    // window too; `=1` forces it on in a shot, which is the only way to
    // get a picture of the readout itself.
    let readout = match std::env::var("FTS_BLITZ_FPS").as_deref() {
        Ok("0") => false,
        Ok(_) => true,
        Err(_) => std::env::var_os("FTS_BLITZ_WINDOW").is_some(),
    };
    // What this run actually resolved to, on the record. Three switches
    // decide what you are looking at and none of them is visible in the
    // picture; "the readout is missing" and "the readout is off" look
    // identical on screen and take a log line to tell apart.
    tracing::info!(
        widget,
        readout,
        present = std::env::var("FTS_PRESENT").unwrap_or_else(|_| "vsync".to_owned()),
        "drawing"
    );
    let arrangement = widget.then(|| {
        let shared: session_daw::widget::Shared =
            std::rc::Rc::new(std::cell::RefCell::new(session_daw::widget::View {
                scroll_x,
                scroll_y,
                zoom_x: 1.0,
                zoom_y: 1.0,
                play_at: 0.0,
            }));
        WIDGET_VIEW.with(|slot| *slot.borrow_mut() = Some(std::rc::Rc::clone(&shared)));
        let recorded = session_daw::arrangement::Arrangement::build(
            &session_daw::arrangement::Palette::from_theme(&theme),
            &session_daw::text::Font::embedded().expect("the embedded font"),
            &project,
            &rows,
            layout,
            &previews,
        );
        let bpm = recorded.bpm;
        let built = session_daw::widget::ArrangementWidget::new(
            recorded,
            session_daw::arrangement::Palette::from_theme(&theme),
            session_daw::text::Font::embedded().expect("the embedded font"),
            bpm,
            PPS,
            rows.as_slice().to_vec(),
            layout,
            // The widget's own copy, which its editor moves before the
            // engine has — see `ArrangementWidget::project`.
            (*project.0).clone(),
            previews.clone(),
            shared,
            readout,
        );
        // What the widget wants done to the session. It queues an
        // `Edit`; the window owns the connection that can carry one
        // out. See `session_daw::widget::ArrangementWidget::act`.
        WIDGET_EDITS.with(|slot| *slot.borrow_mut() = Some(built.edits()));
        dioxus_native_dom::CustomWidgetAttr::new(built)
    });

    let props = ShotProps {
        project,
        rows,
        view,
        colors,
        shapes,
        sizing,
        grid,
        marks,
        sections,
        markers,
        ruler,
        rails,
        all,
        panel,
        theme: theme.clone(),
        live: live_of(&project_for_live),
        rail_items: rail_items(),
        modes: modes(),
        animate: false,
        windowed: false,
        widget,
        arrangement,
    };

    // `FTS_BLITZ_WINDOW=1` opens the studio in a real window instead of
    // rendering one frame of it. Everything above is the same — the same
    // project, the same components, the same props — so what this adds
    // is the half a headless renderer cannot reach: a surface that
    // resizes, a pointer, a keyboard, and a frame after the first one.
    if let Some(mode) = std::env::var_os("FTS_BLITZ_WINDOW") {
        // `animate` runs the benchmark's own gestures on screen, so the
        // numbers in the table and what the window feels like are the
        // same thing measured twice. Anything else is a window you drive
        // yourself — the wheel scrolls, shift makes it sideways, and
        // control zooms.
        let mut props = props;
        props.animate = mode.to_string_lossy() == "animate";
        props.windowed = true;
        println!(
            "opening the studio{} — close the window to exit",
            if props.animate {
                ", running the benchmark's gestures"
            } else {
                "; wheel to scroll, shift for sideways, control to zoom"
            }
        );
        // Opened at the size the rest of this program is measured at,
        // rather than winit's 800x600 default — a window that opens at a
        // size nobody benchmarks is a window whose frame rate cannot be
        // compared to anything.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::as_conversions,
            reason = "a window size, which is small positive integers"
        )]
        let wanted = winit::dpi::PhysicalSize::new(width as u32, height as u32);
        let attributes = winit::window::WindowAttributes::default()
            .with_title("FastTrackStudio — studio")
            .with_surface_size(wanted);
        dioxus_native::launch_cfg_with_props(Shot, props, Vec::new(), vec![Box::new(attributes)]);
        return;
    }

    let vdom = VirtualDom::new_with_props(Shot, props);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions,
        reason = "a lane rect is a window, and windows are small positive integers"
    )]
    let (w, h) = (lane_w as u32, lane_h as u32);
    let mut document = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(w, h, 1.0, ColorScheme::Dark)),
            // Without a provider the document's default one is a no-op
            // that never answers, so every image in the page stays in
            // flight forever — which looks exactly like art that was
            // never built. The panel's controls are one `data:` image,
            // so the shot needs a provider that answers for those.
            net_provider: Some(std::sync::Arc::new(DataUris)),
            ..Default::default()
        },
    );
    document.initial_build();
    // Twice: the first poll builds the tree, and anything that resolves
    // on mount — a memo, a shape that was not ready — lands on the
    // second. A picture taken between the two is a picture of a UI
    // mid-construction, which is how a renderer gets accused of dropping
    // content it simply had not been given yet.
    // Several times, with a moment between: a `data:` image is fetched
    // through the document's resource provider, which answers on another
    // thread. A picture taken before it answers is a picture of the UI
    // with its art still in flight — which looks exactly like art that
    // was never built.
    for _ in 0..12 {
        document.poll(None);
        {
            let mut inner = document.inner_mut();
            inner.resolve(0.0);
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    // `FTS_BLITZ_FRAMES=240` measures a pan instead of taking a
    // picture. The same tree, the same data and the same window — so the
    // number is the real UI's, not a model of it.
    if let Some(frames) = std::env::var("FTS_BLITZ_FRAMES")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
    {
        // `FTS_BLITZ_PROFILE=/tmp/frame.svg` writes a flame graph of the
        // run. Reading a renderer's source to work out where a frame
        // goes is how two days disappear into a phase that turns out not
        // to be the one; this says which function, and it costs a
        // feature flag.
        let profiler = profiler();
        pan(&mut document, w, h, frames);
        report(profiler);
        return;
    }

    let mut image = VelloImageRenderer::new(w, h);
    let mut buffer = Vec::new();
    image.render_to_vec(
        |painter| {
            let mut inner = document.inner_mut();
            blitz_paint::paint_scene(painter, &mut inner, 1.0, w, h, 0, 0);
        },
        &mut buffer,
    );

    let Some(image) = image::RgbaImage::from_raw(w, h, buffer) else {
        eprintln!("the renderer returned a buffer the wrong size");
        std::process::exit(1);
    };
    image.save(&out).expect("write the picture");
    println!("wrote {out} — {w}x{h}, scroll ({scroll_x}, {scroll_y})");
}

/// Pan across the session, and say what a frame of it costs.
///
/// Split the same way the other benchmarks split it, because a frame
/// time says a pan is slow and only the split says which pass is: Dioxus
/// reconciling the tree, then Stylo and Taffy solving what came out, then
/// the scene being encoded for the GPU.
/// Tell the painted arrangement where the view is.
///
/// The bridge between the two worlds: the window keeps the view in
/// signals because that is how a component tree hears about a change,
/// and the widget reads a plain cell because its paint runs outside the
/// Dioxus runtime. One copy a frame, of four numbers.
/// Hand whatever the widget has asked for to the engine.
///
/// The widget has already moved its own copy of the state, so the
/// picture is right whether or not this reaches anything — see
/// `ArrangementWidget::assume`. With no engine (a session opened from a
/// file, with no facade up) the window is a viewer that looks live,
/// which is better than one whose buttons do nothing visible.
fn drain_edits(applier: Option<&session_daw::engine::Applier>) {
    WIDGET_EDITS.with(|slot| {
        let Some(queue) = slot.borrow().as_ref().map(std::rc::Rc::clone) else {
            return;
        };
        let pending: Vec<_> = queue.borrow_mut().drain(..).collect();
        for edit in pending {
            match applier {
                Some(applier) => applier.send(edit),
                None => tracing::debug!(?edit, "no engine to carry out the edit"),
            }
        }
    });
}

fn tell_the_widget(scroll: f64, down: f64, zoom: (f64, f64), play_at: f64) {
    WIDGET_VIEW.with(|slot| {
        if let Some(shared) = slot.borrow().as_ref() {
            *shared.borrow_mut() = session_daw::widget::View {
                scroll_x: scroll,
                scroll_y: down,
                zoom_x: zoom.0,
                zoom_y: zoom.1,
                play_at,
            };
        }
    });
}

/// A sampling profiler over the benchmark, when one is asked for.
///
/// `pprof` samples on a SIGPROF timer rather than through `perf`, which
/// matters because this box has no `perf` and
/// `kernel.perf_event_paranoid` would not allow it anyway.
#[cfg(feature = "profile")]
fn profiler() -> Option<(pprof::ProfilerGuard<'static>, String)> {
    let out = std::env::var("FTS_BLITZ_PROFILE").ok()?;
    let guard = pprof::ProfilerGuardBuilder::default()
        .frequency(2000)
        // The frames are short and the interesting work is deep, so the
        // default blocklist matters: without it every sample lands in
        // libc and the graph says nothing.
        .blocklist(&["libc", "libgcc", "pthread", "vdso"])
        .build()
        .ok()?;
    Some((guard, out))
}

#[cfg(not(feature = "profile"))]
const fn profiler() -> Option<()> {
    None
}

/// Write the flame graph out, if one was being collected.
#[cfg(feature = "profile")]
fn report(profiler: Option<(pprof::ProfilerGuard<'static>, String)>) {
    let Some((guard, out)) = profiler else {
        return;
    };
    match guard.report().build() {
        Ok(report) => match std::fs::File::create(&out) {
            Ok(file) => {
                if let Err(error) = report.flamegraph(file) {
                    eprintln!("could not write {out}: {error}");
                } else {
                    println!("  flame graph: {out}");
                }
            }
            Err(error) => eprintln!("could not open {out}: {error}"),
        },
        Err(error) => eprintln!("the profiler reported nothing: {error}"),
    }
}

#[cfg(not(feature = "profile"))]
const fn report(_profiler: Option<()>) {}

/// Write one frame of a gesture out as a picture.
fn shot_frame(document: &mut DioxusDocument, dir: &str, frame: usize, width: u32, height: u32) {
    let mut image = VelloImageRenderer::new(width, height);
    let mut buffer = Vec::new();
    image.render_to_vec(
        |painter| {
            let mut inner = document.inner_mut();
            blitz_paint::paint_scene(painter, &mut inner, 1.0, width, height, 0, 0);
        },
        &mut buffer,
    );
    if let Some(image) = image::RgbaImage::from_raw(width, height, buffer) {
        let _ = image.save(format!("{dir}/{frame:04}.png"));
    }
}

/// Put the view where the gesture says it is, `t` of the way through.
///
/// Writing the signals from outside the runtime rather than simulating
/// input, because what is being measured is what the tree costs at a
/// given view — not what winit costs to deliver a wheel notch.
fn drive(document: &mut DioxusDocument, gesture: Gesture, t: f64) {
    match gesture {
        Gesture::Pan => {
            // A minute of session under the playhead, which moves every
            // item on screen and changes which ones are there at all.
            SCROLL.with(|scroll| {
                if let Some(mut scroll) = *scroll.borrow() {
                    document.vdom.in_runtime(|| scroll.set(t * 60.0 * PPS));
                }
            });
        }
        Gesture::Down => {
            DOWN.with(|down| {
                if let Some(mut down) = *down.borrow() {
                    document.vdom.in_runtime(|| down.set(t * DOWN_TRAVEL));
                }
            });
        }
        Gesture::ZoomX | Gesture::ZoomY => {
            // In and back out again, over the range a hand actually
            // covers in one gesture. Exponential because a zoom is
            // multiplicative — a linear sweep spends most of its frames
            // at the far end and measures the wrong thing.
            let swing = (t * std::f64::consts::TAU).sin();
            let scale = ZOOM_SWING.powf(swing);
            ZOOM.with(|zoom| {
                if let Some(mut zoom) = *zoom.borrow() {
                    document.vdom.in_runtime(|| {
                        if gesture == Gesture::ZoomX {
                            zoom.set((scale.clamp(ZOOM_X.0, ZOOM_X.1), 1.0));
                        } else {
                            zoom.set((1.0, scale.clamp(ZOOM_Y.0, ZOOM_Y.1)));
                        }
                    });
                }
            });
        }
    }
}

/// The zoom a still is taken at, from `FTS_BLITZ_ZOOM=x,y`.
///
/// So that a picture can be taken BETWEEN two of the steps the tree is
/// built at — which is the only place the residual scaling does any
/// work, and therefore the only place a mistake in it would show.
fn initial_zoom() -> (f64, f64) {
    std::env::var("FTS_BLITZ_ZOOM")
        .ok()
        .and_then(|v| {
            let (x, y) = v.split_once(',')?;
            Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
        })
        .unwrap_or((1.0, 1.0))
}

/// How far down a benchmarked vertical scroll travels.
///
/// Past the end of most sessions on purpose: the interesting frames are
/// the ones where a band of rows is built and thrown away, and the last
/// of those is at the bottom.
const DOWN_TRAVEL: f64 = 6000.0;

/// How far either way a benchmarked zoom travels.
///
/// Four times in and four times out is a gesture a hand makes in about a
/// second; anything wider stops being a gesture and starts being a
/// different session.
const ZOOM_SWING: f64 = 4.0;

fn pan(document: &mut DioxusDocument, width: u32, height: u32, frames: usize) {
    let gesture = Gesture::from_env();
    let dump = std::env::var("FTS_BLITZ_DUMP").ok();
    if let Some(dir) = dump.as_deref() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut renderer =
        session_daw::headless::Headless::new(width, height).expect("a headless renderer");
    let mut stages = session_daw::profile::Stages::with_capacity(frames);
    let mut diff = session_daw::profile::Samples::with_capacity(frames);
    let mut solve = session_daw::profile::Samples::with_capacity(frames);
    let batch = session_daw::headless::BATCH;

    for chunk in 0..frames / batch {
        let started = Instant::now();
        let mut painted = 0.0;
        for step in 0..batch {
            let frame = chunk * batch + step;
            #[expect(
                clippy::cast_precision_loss,
                clippy::as_conversions,
                reason = "a frame index over a few hundred"
            )]
            let t = frame as f64 / frames as f64;
            drive(document, gesture, t);
            SCROLL.with(|s| {
                DOWN.with(|d| {
                    ZOOM.with(|z| {
                        let at = |sig: &std::cell::RefCell<Option<Signal<f64>>>| {
                            sig.borrow().map_or(0.0, |s| s.peek().to_owned())
                        };
                        let zoom = z
                            .borrow()
                            .map_or((1.0, 1.0), |z: Signal<(f64, f64)>| z.peek().to_owned());
                        // No transport in a headless shot: nothing is
                        // playing, so the playhead is at the start.
                        tell_the_widget(at(s), at(d), zoom, 0.0);
                    });
                });
            });
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
            // `FTS_BLITZ_DUMP=/tmp/frames` writes every frame out. A
            // fault that only exists while something is moving cannot be
            // found by rendering a still at the same place — the state
            // that is wrong is the state left over from the frame
            // before.
            if let Some(dir) = dump.as_deref() {
                shot_frame(document, dir, frame, width, height);
            }
        }
        renderer.wait().expect("the gpu to finish the batch");
        #[expect(
            clippy::cast_precision_loss,
            clippy::as_conversions,
            reason = "a batch size of thirty"
        )]
        let per_frame = started.elapsed().as_secs_f64() * 1000.0 / batch as f64;
        stages.frame.push_ms(per_frame);
        stages.paint.push_ms(painted / batch as f64);
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
    // How many nodes are in the tree, because that is the unit the cost
    // is in: style and layout scale with what is THERE, not with what
    // changed. An optimisation that does not move this number is an
    // optimisation of something else.
    let nodes = document.inner().tree().len();
    println!(
        "\n  The component window, {} across the golden session\n",
        gesture.name()
    );
    println!("  surface       {width}x{height}");
    println!("  tree          {nodes} nodes");
    println!(
        "  {:<12} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}",
        "", "mean", "p99", "worst", "style+lay", "paint", "fps(p99)"
    );
    println!("  {}", "-".repeat(74));
    // The worst frame rather than the diff pass, because the diff is the
    // part this design made free and the worst frame is the part it did
    // not: a window boundary rebuilds the items, and that frame is the
    // one a fling would show a hitch on if it were slow.
    println!(
        "  {:<12} {:>7.2}ms {:>7.2}ms {:>7.2}ms {:>7.2}ms {:>7.2}ms {:>9.0}",
        gesture.name(),
        frame.mean,
        frame.p99,
        frame.worst,
        solve.mean,
        paint.mean,
        1000.0 / frame.p99.max(0.001)
    );
    // The middle frame and the worst one, not the mean of the two. A
    // snapped zoom is cheap between two steps and rebuilds when it
    // crosses one, so the mean is a blend of two costs that never
    // actually happen — and whether a gesture feels smooth is decided by
    // how far apart they are.
    println!(
        "\n  reconciling   p50 {:>6.2}ms   p99 {:>6.2}ms   worst {:>6.2}ms",
        diff.p50, diff.p99, diff.worst
    );
    println!(
        "  style+layout  p50 {:>6.2}ms   p99 {:>6.2}ms   worst {:>6.2}ms",
        solve.p50, solve.p99, solve.worst
    );
}

/// A resource provider that answers `data:` URIs and nothing else.
///
/// Blitz ships a real one in `blitz-net`, which fetches over HTTP and
/// off the filesystem and is asynchronous. A renderer that draws one
/// frame and exits wants neither: it wants the image it just built,
/// now, on the thread that asked. So this decodes the URI and answers
/// inline, and anything else is left unanswered on purpose — a shot
/// that quietly reached the network would be a shot whose picture
/// depended on this machine.
struct DataUris;

impl blitz_traits::net::NetProvider for DataUris {
    fn fetch(
        &self,
        _doc: usize,
        request: blitz_traits::net::Request,
        handler: Box<dyn blitz_traits::net::NetHandler>,
    ) {
        let url = request.url.as_str();
        let Some(payload) = url.strip_prefix("data:") else {
            return;
        };
        let Some((_, body)) = payload.split_once(',') else {
            return;
        };
        handler.bytes(url.to_owned(), unescape(body).into());
    }
}

/// Undo the percent-encoding a `data:` URI carries.
fn unescape(body: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len());
    let mut bytes = body.as_bytes().iter().copied();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let hex: String = bytes.by_ref().take(2).map(char::from).collect();
            if let Ok(decoded) = u8::from_str_radix(&hex, 16) {
                out.push(decoded);
                continue;
            }
        }
        out.push(byte);
    }
    out
}

/// The surface, which is the window the reference was shot at.
fn size() -> (f64, f64) {
    std::env::var("FTS_BLITZ_SIZE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
        })
        .unwrap_or((2560.0, 1440.0))
}

/// Where the lane rect starts across: past the left rail and the panel.
pub fn lane_x() -> f64 {
    session_daw::rails::SIDE + session_daw::arrangement::TCP_WIDTH
}

/// And down: past the top rail and the ruler.
pub fn lane_y() -> f64 {
    session_daw::rails::TOP + session_daw::ruler::RULER_H
}

/// How wide it is — to the right rail.
fn frame_width(width: f64) -> f64 {
    (width - lane_x() - session_daw::rails::SIDE).max(1.0)
}

/// And how tall — to the bottom of the frame.
fn frame_height(height: f64) -> f64 {
    (height - lane_y()).max(1.0)
}

/// The open project, as the studio's own refs.
fn read_back(scene: Option<&str>, project_path: &std::path::Path) -> Option<(ProjectRef, RowsRef)> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    let project = rt.block_on(daw_ui::studio::project::fetch())?;
    let project = ProjectRef(Arc::new(project));
    let (visible, depths) =
        daw_ui::components::folders::FolderState::default().visible(&project.tracks);
    let mut planned: Vec<(daw_proto::Track, u32)> = visible.into_iter().zip(depths).collect();
    // A SCENE, if one is asked for: the visual track manager's answer to
    // which rows show and how tall each opens. Without it every track is
    // the same height, because the session file carries no per-track
    // height — the heights are a layout the scene decides, not a fact
    // the file records.
    // `FTS_BLITZ_FOLDED_TAKES=0` draws a shut folder the way REAPER
    // does, which is empty — the switch on the arrangement's right rail,
    // reachable from here so both pictures can be taken.
    let settings = session_daw::settings::Settings {
        folded_takes: std::env::var("FTS_BLITZ_FOLDED_TAKES").as_deref() != Ok("0"),
        ..session_daw::settings::Settings::default()
    };
    if let Some(slug) = scene.and_then(dynamic_template::scenes::scene) {
        let kinds = session_daw::plan::Kinds::read(project_path);
        planned = session_daw::plan::apply_scene(
            &planned,
            &kinds,
            slug,
            session_daw::plan::Panel {
                surface: session_daw::plan::Surface::Arrange,
                mode: None,
                settings,
                extent: 1440.0,
                active_language: None,
            },
        );
    }
    // A folder the scene shut keeps its row and loses its audio, which
    // is REAPER's behaviour and is the wrong one here: fold the kit away
    // and you have folded the take away with it. So the row gets its
    // children's items, folded, and goes on being the folder in every
    // other respect — its name, its fader and its colour are what the
    // panel beside it shows, because the mix happens on the folder.
    let mut project = project;
    {
        let shown: Vec<daw_proto::Track> = planned.iter().map(|(t, _)| t.clone()).collect();
        daw_ui::studio::folded::refold(
            Arc::make_mut(&mut project.0),
            &shown,
            settings.folded_takes,
        );
    }
    let rows = RowsRef(Arc::new(planned));
    Some((project, rows))
}

/// Every item's shape: its notes if it holds MIDI, its peaks otherwise.
///
/// The notes are read BEFORE anything is drawn, not after: a renderer
/// draws one frame and exits, so there is no later for them to arrive
/// in — and an item drawn from a waveform it does not have is why a
/// chord track once looked like a shaker.
fn previews_of(project: &ProjectRef) -> session_daw::midi::Previews {
    let previews = session_daw::midi::Previews::default();
    previews.fill_blocking(
        project
            .0
            .items
            .values()
            .flatten()
            .filter(|item| project.0.is_midi(&item.guid))
            .map(|item| (item.guid.clone(), item.length.as_seconds()))
            .collect(),
    );
    previews
}

fn shapes_of(project: &ProjectRef, previews: &session_daw::midi::Previews) -> Shapes {
    let mut shapes: HashMap<String, Shape> = HashMap::new();
    for (row, track) in project.tracks.iter().enumerate() {
        let index = usize::try_from(track.index).unwrap_or(row);
        for item in project.lane(&track.guid) {
            let shape = match previews.get(&item.guid) {
                Some(notes) if !notes.is_empty() => roll(&notes),
                // Nothing drawn until the notes arrive, rather than a
                // fake shape: a wrong picture that later corrects itself
                // is worse than an honest empty one.
                _ if project.is_midi(&item.guid) => continue,
                _ => {
                    let x0 = item.position.as_seconds();
                    let span = item.length.as_seconds().max(0.001);
                    Shape::Wave(peaks(index, x0, span))
                }
            };
            shapes.insert(item.guid.clone(), shape);
        }
    }
    // A folded row's shape is its children's, not its own. The loop
    // above gave each folded item the waveform the FOLDER's index
    // generates, which is a plausible picture of nothing: the folder
    // carries no audio, and what the row is showing is the mics under
    // it. Done after, because a child's shape has to exist before it can
    // be folded.
    for (folder, fold) in &project.0.folds {
        for (index, span) in fold.spans.iter().enumerate() {
            let guid = daw_ui::studio::folded::guid_of(folder, index);
            let waves: Vec<&[f32]> = span
                .from
                .iter()
                .filter_map(|child| match shapes.get(child) {
                    Some(Shape::Wave(peaks)) => Some(&**peaks),
                    _ => None,
                })
                .collect();
            if waves.is_empty() {
                // Every child a trigger lane: a folded row of MIDI is
                // still MIDI, and drawing a flat waveform over it would
                // say the drum was miked when it was sampled.
                let notes: Vec<Note> = span
                    .from
                    .iter()
                    .filter_map(|child| match shapes.get(child) {
                        Some(Shape::Notes(notes)) => Some(notes.iter().copied()),
                        _ => None,
                    })
                    .flatten()
                    .collect();
                if notes.is_empty() {
                    shapes.remove(&guid);
                } else {
                    shapes.insert(guid, Shape::Notes(notes.into()));
                }
                continue;
            }
            shapes.insert(
                guid,
                Shape::Wave(daw_ui::studio::folded::wave(&waves).into()),
            );
        }
    }
    Shapes::new(shapes)
}

/// The notes of one item, as fractions of it.
///
/// The pitch range is the item's own, and how tall a note is drawn comes
/// from how many pitches it has to share the lane with: tall enough to
/// see, short enough that neighbouring pitches do not merge into a
/// block.
fn roll(notes: &[session_daw::midi::Note]) -> Shape {
    let (low, high) = notes.iter().fold((u8::MAX, u8::MIN), |(lo, hi), note| {
        (lo.min(note.pitch), hi.max(note.pitch))
    });
    // A part on one pitch has no range to spread over, so it is drawn
    // down the middle rather than divided by zero.
    let span = f64::from(high.saturating_sub(low)).max(1.0);
    let height = (1.0 / span.min(24.0)).clamp(0.0, 1.0 / 3.0);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a fraction of an item, and the shape is f32"
    )]
    let height = height as f32;
    Shape::Notes(
        notes
            .iter()
            .map(|note| {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "a fraction of a pitch range under 128 wide"
                )]
                let from_top = (f64::from(high.saturating_sub(note.pitch)) / span) as f32;
                Note {
                    at: note.at,
                    len: note.len,
                    from_top,
                    height,
                }
            })
            .collect(),
    )
}

/// One item's peaks: the loudest sample in each stretch.
fn peaks(track: usize, x0: f64, span: f64) -> Arc<[f32]> {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions,
        reason = "a count of peaks over an item, bounded by its length"
    )]
    let count = ((span * PEAKS_PER_SECOND).ceil() as usize).max(2);
    let step = 1.0 / PEAKS_PER_SECOND / peak_count(HOLD);
    (0..=count)
        .map(|i| {
            let t = span.mul_add(peak_count(i) / peak_count(count), x0);
            let peak = (0..HOLD)
                .map(|k| session_daw::simulate::waveform(track, peak_count(k).mul_add(-step, t)))
                .fold(0.0_f64, f64::max);
            #[expect(
                clippy::cast_possible_truncation,
                reason = "an amplitude in 0..1, and f32 is what a peak file holds"
            )]
            let peak = peak as f32;
            peak
        })
        .collect()
}

/// An index as a coordinate, without an `as` in the middle of the maths.
fn peak_count(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// The grid spacings at this zoom.
///
/// The adaptive division is the ruler's own: whichever division still
/// fits at this many pixels a bar, and none at all when nothing does.
fn grid_of(project: &ProjectRef, view: View) -> Grid {
    /// The finest the grid ever gets, as a fraction of a whole note.
    const FINEST: f64 = 1.0 / 16.0;
    let bars = session_daw::ruler::Bars::at(project.bpm);
    let bar = bars.secs_per_bar();
    let beat = adaptive_grid::Adaptive::default()
        .fit(FINEST, bar * view.pps)
        .map(|division| division * 4.0 * bars.secs_per_beat);
    Grid { bar, beat }
}

#[derive(Props, Clone, PartialEq)]
struct ShotProps {
    project: ProjectRef,
    rows: RowsRef,
    view: View,
    colors: Colors,
    shapes: Shapes,
    sizing: Rows,
    grid: Grid,
    marks: Marks,
    sections: std::sync::Arc<[daw_ui::studio::project::Section]>,
    markers: std::sync::Arc<[daw_ui::studio::project::Marker]>,
    ruler: bool,
    rails: bool,
    all: bool,
    panel: bool,
    rail_items: (Vec<Item>, Vec<Item>, Vec<Item>),
    modes: Vec<Item>,
    theme: daw_ui::theming::Theme,
    live: HashMap<String, daw_ui::studio::panel::Live>,
    /// Whether the window drives itself through the benchmark's gestures.
    animate: bool,
    /// Whether there is a real window to ask about its size.
    windowed: bool,
    /// Whether the arrangement is painted as one node rather than built
    /// as a tree — see [`session_daw::widget`].
    widget: bool,
    /// The widget itself, when it is. Write-once: the first render hands
    /// it to the document and every later one finds the slot empty,
    /// which is what makes it safe to carry in props that get cloned.
    arrangement: Option<dioxus_native_dom::CustomWidgetAttr>,
}

thread_local! {
    /// The scroll, reachable from outside the runtime so the benchmark
    /// can drive it the way a scrollbar would.
    static SCROLL: std::cell::RefCell<Option<Signal<f64>>> =
        const { std::cell::RefCell::new(None) };
    /// And both zooms, for the same reason.
    ///
    /// A separate slot rather than a field on the same one because only
    /// the composed window has a zoom to drive — the lanes on their own
    /// take a fixed `pps` as a prop, which is what makes them comparable
    /// against the reference shot.
    static ZOOM: std::cell::RefCell<Option<Signal<(f64, f64)>>> =
        const { std::cell::RefCell::new(None) };
    /// And how far DOWN, so a vertical scroll can be driven the way the
    /// horizontal one is. The two are not the same measurement: the
    /// panel travels with the lanes down the session and does not
    /// travel with them across it, so a fault in one axis says nothing
    /// about the other.
    static DOWN: std::cell::RefCell<Option<Signal<f64>>> =
        const { std::cell::RefCell::new(None) };
    /// Where the painted arrangement thinks the view is.
    ///
    /// A cell rather than a prop for the reason the others are: the
    /// widget's paint happens inside Blitz's traversal, which is not the
    /// Dioxus runtime, so what it reads cannot be a signal.
    /// What the arrangement widget has asked for and nobody has done
    /// yet. Drained on the window's own thread, which is where the
    /// engine's sender lives.
    static WIDGET_EDITS: std::cell::RefCell<
        Option<std::rc::Rc<std::cell::RefCell<Vec<session_daw::engine::Edit>>>>,
    > = const { std::cell::RefCell::new(None) };
    static WIDGET_VIEW: std::cell::RefCell<Option<session_daw::widget::Shared>> =
        const { std::cell::RefCell::new(None) };
}

/// Which gesture the headless benchmark runs.
///
/// A pan and a zoom are not the same measurement and never were: a pan
/// moves one transform, a zoom changes where every item is. Reporting
/// one number for "the window" hid that for weeks.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Gesture {
    /// Across the session, which is what `pan` always did.
    Pan,
    /// And down it, which is the axis the track panel shares.
    Down,
    /// In and out horizontally, around the middle of the travel.
    ZoomX,
    /// And vertically.
    ZoomY,
}

impl Gesture {
    /// The gesture named by `FTS_BLITZ_GESTURE`, defaulting to the pan.
    fn from_env() -> Self {
        match std::env::var("FTS_BLITZ_GESTURE").as_deref() {
            Ok("down") => Self::Down,
            Ok("zoom-x" | "zoomx") => Self::ZoomX,
            Ok("zoom-y" | "zoomy") => Self::ZoomY,
            _ => Self::Pan,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Pan => "pan",
            Self::Down => "down",
            Self::ZoomX => "zoom-x",
            Self::ZoomY => "zoom-y",
        }
    }
}

#[component]
fn Shot(props: ShotProps) -> Element {
    let scroll = use_signal(|| props.view.scroll_x);
    use_hook(|| {
        SCROLL.with(|slot| *slot.borrow_mut() = Some(scroll));
    });

    rsx! {
        // The user-agent stylesheet gives the body an eight-pixel
        // margin, which is eight pixels of the session pushed off the
        // bottom of the window and every row eight pixels from where the
        // reference draws it. A window is not a document.
        // A window, not a document.
        //
        // The margin is the user-agent's eight pixels. The HEIGHT is the
        // one that matters: `height: 100%` on the studio below resolves
        // against its parent, and a parent with no height of its own
        // gives it none — so the studio grew to its content instead, the
        // body ended up taller than the surface, and the SHELL scrolled
        // the whole page. Rails, ruler and all, which is exactly what it
        // looked like. Giving the chain a height makes `100%` mean the
        // window, and `overflow: hidden` leaves nothing for the shell to
        // scroll, so a wheel event reaches the arrangement.
        style {
            "html, body {{ margin: 0; padding: 0; width: 100%; height: 100%; \
             overflow: hidden; }}"
        }
        if props.panel {
            Panel {
                project: props.project,
                rows: props.rows,
                view: props.view,
                colors: props.colors,
                theme: props.theme,
                sizing: props.sizing,
                live: props.live,
            }
        } else if props.all {
            Window { ..props.clone() }
        } else if props.rails {
            Rails {
                width: props.view.width,
                height: props.view.height,
                colors: props.colors.clone(),
                top: props.rail_items.2.into(),
                left: props.rail_items.0.into(),
                right: props.rail_items.1.into(),
            }
            ModeBar {
                width: session_daw::arrangement::TCP_WIDTH,
                height: session_daw::ruler::RULER_H,
                colors: props.colors,
                modes: props.modes.into(),
            }
        } else if props.ruler {
            Ruler {
                view: props.view,
                colors: props.colors,
                marks: props.marks,
                sections: props.sections,
                markers: props.markers,
                scroll: ReadSignal::from(scroll),
            }
        } else {
        Lanes {
            project: props.project,
            rows: props.rows,
            view: props.view,
            scroll: ReadSignal::from(scroll),
            colors: props.colors,
            shapes: props.shapes,
            sizing: props.sizing,
            grid: props.grid,
        }
        }
    }
}

/// The rails the reference shot draws: the same profile, at rest.
///
/// Sized here rather than in the component, for the reason the rail
/// says: measuring a word in a face is the host's job, and the painted
/// window shrinks a label half a point at a time until it fits its
/// plate. A component that guessed would guess differently on every
/// renderer.
fn rail_items() -> (Vec<Item>, Vec<Item>, Vec<Item>) {
    let profile = session_daw::rails::profile(
        session_daw::rails::Surface::Arrange,
        session::modes::Mode::Mix,
        session::mix_phases::MixPhase::Tone,
        Some("drum-mixing"),
        session_daw::settings::Settings::default(),
        dynamic_template::scenes::Audience::Engineer,
        "drums",
    );
    let font = session_daw::text::Font::embedded().expect("the embedded font");
    let fit = |items: &[session_daw::rails::Item<'_>], room: f64| {
        items
            .iter()
            .map(|item| {
                let (label, size) = font.fit(item.label, 10.0, 6.0, room - 4.0);
                // A word that still needs an ellipsis is not drawn at
                // all: a clipped word in a 38-pixel button is a smear,
                // and the plate's lit state already says which is
                // current.
                let label = if label.contains('…') {
                    String::new()
                } else {
                    label
                };
                Item {
                    label,
                    on: item.on,
                    size: Some(f64::from(size)),
                }
            })
            .collect()
    };
    (
        fit(&profile.left, session_daw::rails::SIDE - 6.0),
        fit(&profile.right, session_daw::rails::SIDE - 6.0),
        fit(&profile.top, session_daw::rails::TOP_ITEM_W - 3.0),
    )
}

/// Every converted surface, in its place in the window.
///
/// The lanes sit past the panel and under the ruler, the ruler past the
/// panel, the rails over both, and the mode bar in the corner the ruler
/// leaves. The track panel's own column is the one thing missing, and it
/// is left as the window's ground rather than faked — a picture with a
/// wrong panel in it would be worse than one with none.
/// Keeps `size` in step with the window, and drives the gestures.
///
/// Named for what it tracks rather than `Surface`, which this file
/// already uses for the rails' own enum — two `Surface`s in one file is
/// a name that resolves to whichever one the reader is not thinking of.
///
/// A component rather than a few hooks in `Window`, because the hooks it
/// needs only exist when a winit window does — and a hook cannot be
/// called conditionally, while a component can be mounted conditionally.
/// It draws nothing; it exists to hold three hooks.
#[component]
#[expect(
    clippy::too_many_arguments,
    reason = "the window's clock drives every signal a gesture moves"
)]
fn WindowSize(
    size: Signal<(f64, f64)>,
    animate: bool,
    /// How far the session runs across and down in its own pixels,
    /// before any zoom, and the size of the frame it is seen through.
    ///
    /// The travel is worked out from these HERE rather than handed down
    /// ready-made, because a ready-made one would have to be computed by
    /// whoever reads the zoom — and the window may not be the one that
    /// does. See the note where this is mounted.
    span_x: f64,
    span_y: f64,
    frame: (f64, f64),
    scroll: Signal<f64>,
    down: Signal<f64>,
    zoom: Signal<(f64, f64)>,
    gesture: Signal<(&'static str, f64)>,
) -> Element {
    // Asked for rather than demanded: `use_window` consumes the context
    // and PANICS when it is missing, which a component swallows — the
    // window goes on drawing at whatever size it started with and
    // nothing says why. Every symptom of that looks like a resize that
    // did not arrive.
    let handle = use_hook(|| {
        let found: Option<std::sync::Arc<dyn winit::window::Window>> = try_consume_context();
        if found.is_none() {
            // Worth a warning rather than a panic: without it the window
            // draws for ever at the size it started with, and every
            // symptom of that looks like a resize that never arrived.
            tracing::warn!("no winit window in context; the studio cannot follow its surface");
        }
        found
    });
    let mut size = size;
    // What was last written to the log, so the same number is not
    // written twice. A plain cell and not a signal: nothing renders
    // from it, and a signal written every frame would re-render the
    // window to report that the window re-rendered.
    let last = use_hook(|| std::rc::Rc::new(std::cell::Cell::new(0.0_f64)));
    let redraw = handle.clone();
    use_hook(move || {
        if let Some(handle) = handle.as_ref() {
            let px = handle.surface_size();
            tracing::info!(width = px.width, height = px.height, "opened at");
            size.set((f64::from(px.width.max(1)), f64::from(px.height.max(1))));
        }
    });
    // What a frame COST, not how often one happened.
    //
    // Two earlier readouts were both wrong, in opposite directions.
    // Counting redraws over wall-clock says four per second when nothing
    // is moving, because Blitz only redraws when something changed and an
    // idle window changed nothing — that reads as "this is slow" when it
    // means "nothing was asked of it". Timing the interval BETWEEN
    // redraws is no better during a gesture: a burst of discrete wheel
    // events makes the gaps the input's cadence, not the renderer's.
    //
    // So the number comes from the shell, which is the only place that
    // knows: `blitz-shell` writes how long resolve + encode + present
    // took into `LAST_FRAME_MICROS` at the end of its own `redraw()`, and
    // this reads it. Kept in a cell and published only when the median of
    // the recent ones actually moves, because a signal written every
    // frame would re-render to show a number that changed because it
    // re-rendered.
    // What the pointer and the keyboard are doing.
    //
    // Handled at the WINIT level rather than as DOM events, because
    // Blitz does not dispatch a wheel event to the DOM at all — it
    // scrolls whatever is under the pointer and redraws if something
    // moved. A modifier that turns the wheel into a zoom therefore
    // cannot be a handler on an element; it has to be here, where the
    // wheel arrives before anything has decided what it means.
    let input = use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(Input::default())));
    // The one thing in this window that can change the session. `None`
    // when no facade is up, in which case the window is a viewer —
    // which is what opening a `.rpp` from disk with no engine running
    // actually is.
    let applier = use_hook(|| std::rc::Rc::new(session_daw::engine::Applier::start()));
    // Where the transport is, polled rather than subscribed to: a
    // position is a LEVEL, and a subscription would deliver a backlog
    // after a stall, which is the one thing a playhead must not replay.
    let transport = use_hook(|| std::rc::Rc::new(session_daw::engine::Transport::start()));
    let counted =
        use_hook(|| std::rc::Rc::new(std::cell::RefCell::new(Vec::<f64>::with_capacity(RECENT))));
    let started = use_hook(std::time::Instant::now);
    // How far the view may travel, at whatever the zoom is when a
    // gesture asks. A closure rather than a value because reading the
    // zoom to make a value would subscribe this component to it, and a
    // component that re-renders on every frame of a zoom is the thing
    // this file has spent itself getting rid of.
    // How far out a zoom may go: far enough to see all of it, and no
    // further.
    //
    // Derived, not declared. The floor used to be a constant — two
    // hundredths across and six across down — and a constant cannot be
    // right, because how far out is worth going is a fact about the
    // SESSION. Two hundredths put this song in a fifth of the window
    // and left four fifths of empty timeline to scroll around in; on a
    // session ten times longer the same constant would stop while there
    // was still more to see.
    //
    // What the floor actually is: the zoom at which the content exactly
    // fills the frame. Past that you are not looking at more of the
    // session, because there is no more of it — and that number changes
    // when the window resizes, when a scene shows or hides tracks, and
    // when a row's height changes, all of which it now follows on its
    // own.
    //
    // Never above one, though. A session shorter than the window would
    // otherwise be unable to reach its own natural scale, which is a
    // stranger thing than a little empty space after the last bar.
    let floor = move |frame: f64, span: f64| {
        if span > 0.0 && frame > 0.0 {
            (frame / span).min(1.0)
        } else {
            1.0
        }
    };
    let limits = move || {
        (
            (floor(frame.0, span_x), ZOOM_X.1),
            (floor(frame.1, span_y), ZOOM_Y.1),
        )
    };
    let extent = move || {
        let (zx, zy) = zoom.peek().to_owned();
        // Zoom FIRST, then take off the window: how far you may travel
        // is how much session there is at this zoom, less what is
        // already on screen. Nothing left over means nothing to scroll.
        (
            (span_x * zx - frame.0).max(0.0),
            (span_y * zy - frame.1).max(0.0),
        )
    };
    let mut scroll = scroll;
    let mut down = down;
    let mut zoom = zoom;
    let mut gesture = gesture;
    let driving = input.clone();
    dioxus_native::use_window_event(move |event, _| match event {
        // `z` is a spring: hold it and the wheel and the drag become the
        // zoom tool for the length of the hold, which is the gesture the
        // expression editor already uses and the one the hands here
        // already know.
        winit::event::WindowEvent::KeyboardInput { event, .. } => {
            if event.logical_key == winit::keyboard::Key::Character("z".into()) {
                driving.borrow_mut().zooming = event.state.is_pressed();
            }
        }
        winit::event::WindowEvent::ModifiersChanged(state) => {
            driving.borrow_mut().shift = state.state().shift_key();
        }
        winit::event::WindowEvent::PointerMoved { position, .. } => {
            let at = (position.x, position.y);
            let mut input = driving.borrow_mut();
            let moved = (at.0 - input.pointer.0, at.1 - input.pointer.1);
            input.pointer = at;
            match input.drag {
                // The hand: middle-drag moves the session under the
                // pointer, which is the one navigation gesture every DAW
                // agrees on.
                Some(Drag::Pan) => {
                    scroll.set((scroll() - moved.0).clamp(0.0, extent().0));
                    down.set((down() - moved.1).clamp(0.0, extent().1));
                }
                // And the zoom tool: sideways for time, up and down for
                // rows, both at once if the hand moves both ways.
                Some(Drag::Zoom) => {
                    let (zx, zy) = zoom();
                    zoom.set((
                        (zx * factor(moved.0)).clamp(limits().0.0, limits().0.1),
                        (zy * factor(-moved.1)).clamp(limits().1.0, limits().1.1),
                    ));
                }
                None => {}
            }
        }
        winit::event::WindowEvent::PointerButton { state, button, .. } => {
            let mut input = driving.borrow_mut();
            let pressed = state.is_pressed();
            let which = match button {
                winit::event::ButtonSource::Mouse(button) => *button,
                _ => winit::event::MouseButton::Left,
            };
            input.drag = match (pressed, which) {
                (true, winit::event::MouseButton::Middle) => Some(Drag::Pan),
                (true, winit::event::MouseButton::Left) if input.zooming => Some(Drag::Zoom),
                (true, _) => None,
                (false, _) => None,
            };
        }
        winit::event::WindowEvent::MouseWheel { delta, .. } => {
            let (dx, dy) = match delta {
                winit::event::MouseScrollDelta::LineDelta(x, y) => {
                    (f64::from(*x) * WHEEL_LINE, f64::from(*y) * WHEEL_LINE)
                }
                winit::event::MouseScrollDelta::PixelDelta(at) => (at.x, at.y),
            };
            let input = driving.borrow();
            if input.zooming {
                // Held `z` zooms the rows; with shift it zooms time.
                // Two axes, one key, and shift picks which — the same
                // split the scrollbars use.
                let (zx, zy) = zoom();
                if input.shift {
                    zoom.set((
                        (zx * factor(dy * 2.0)).clamp(limits().0.0, limits().0.1),
                        zy,
                    ));
                } else {
                    zoom.set((
                        zx,
                        (zy * factor(dy * 2.0)).clamp(limits().1.0, limits().1.1),
                    ));
                }
            } else if input.shift {
                scroll.set((scroll() - dx - dy).clamp(0.0, extent().0));
            } else {
                scroll.set((scroll() - dx).clamp(0.0, extent().0));
                down.set((down() - dy).clamp(0.0, extent().1));
            }
        }
        winit::event::WindowEvent::SurfaceResized(px) => {
            tracing::info!(width = px.width, height = px.height, "resized to");
            size.set((f64::from(px.width.max(1)), f64::from(px.height.max(1))));
        }
        winit::event::WindowEvent::RedrawRequested => {
            // The gestures are driven from the window's own clock rather
            // than from a timer. `use_future` with a sleep never ticks
            // here — nothing polls the task runtime unless the document
            // already has work — so the thing that IS guaranteed to
            // happen every frame is the frame, and asking for the next
            // one from inside it is an animation loop at whatever rate
            // the window can actually present.
            if animate {
                let elapsed = started.elapsed().as_secs_f64();
                let index =
                    usize::try_from((elapsed / GESTURE_SECS) as u64).unwrap_or(0) % GESTURES.len();
                let (name, at) = GESTURES[index];
                let t = (elapsed % GESTURE_SECS) / GESTURE_SECS;
                let (x, y, zx, zy) = at(t);
                // Set the zoom BEFORE reading how far the view may
                // travel, or the gesture spends the frame somewhere the
                // previous zoom allowed and this one does not.
                zoom.set((zx, zy));
                // Across the session and down it, as fractions of what
                // there actually is to travel. It used to be a fixed
                // forty thousand pixels down, which on a session with
                // less than that in it spent most of the gesture below
                // the last track looking at nothing — and a benchmark
                // that measures an empty screen is measuring nothing.
                let (across, deep) = extent();
                scroll.set(x * across);
                down.set(y * deep);
                gesture.set((name, 0.0));
                if let Some(handle) = redraw.as_ref() {
                    handle.request_redraw();
                }
            }

            // Where the painted arrangement should be looking, if there
            // is one. Written every frame rather than on change: it is
            // four numbers, and a missed write is a frame drawn at the
            // wrong place.
            let play_at = transport.as_ref().as_ref().map_or(0.0, |t| t.read().0);
            tell_the_widget(scroll(), down(), zoom(), play_at);
            // And whatever it wants done to the session, carried out.
            // Drained here, on the window's thread, because that is
            // where the engine's sender is — the widget is a painter
            // and holds no connection to anything.
            drain_edits((*applier).as_ref());

            // What the frame the shell just drew actually cost.
            let cost = f64::from(
                u32::try_from(
                    blitz_traits::LAST_FRAME_MICROS.load(std::sync::atomic::Ordering::Relaxed),
                )
                .unwrap_or(u32::MAX),
            ) / 1000.0;
            let mut state = counted.borrow_mut();
            let gap = cost.max(0.01);
            // This long is the shell having done something other than
            // draw this window, not a frame that took this long.
            const IDLE: f64 = 200.0;
            if gap < IDLE {
                if state.len() == RECENT {
                    state.remove(0);
                }
                state.push(gap);
            }
            if state.len() >= 8 {
                let mut recent = state.clone();
                recent.sort_by(f64::total_cmp);
                let middle = recent[recent.len() / 2];
                let worst = recent.last().copied().unwrap_or(middle);
                // To the log only. What the WINDOW shows is the
                // widget's own graph; this is for reading a run back
                // afterwards, which a graph on a closed window cannot
                // do. Rate-limited the same way — a line per frame is
                // a log nobody reads.
                if (middle - last.get()).abs() > 0.5 {
                    last.set(middle);
                    tracing::info!(
                        frame_ms = format!("{middle:.1}"),
                        worst_ms = format!("{worst:.1}"),
                        "presented"
                    );
                }
            }
        }
        _ => {}
    });
    rsx! {}
}

/// The benchmark's gestures, as the window runs them.
///
/// The same seven the table measures, in the same order and at the same
/// rates — so "260 fps in the table" and "this is what it feels like"
/// are the same thing said twice. A gesture returns where the view
/// should be at `t`, which runs 0..1 across it.
const GESTURES: [(&str, fn(f64) -> (f64, f64, f64, f64)); 7] = [
    ("scroll down/up", |t| (0.0, tri(t), 1.0, 1.0)),
    ("scroll right/left", |t| (tri(t), 0.0, 1.0, 1.0)),
    ("scroll both", |t| (tri(t), tri((t * 1.7) % 1.0), 1.0, 1.0)),
    ("zoom vertical", |t| (0.0, 0.3, 1.0, 0.25 + slow(t) * 3.75)),
    ("zoom horizontal", |t| {
        (0.0, 0.3, 0.25 + slow(t) * 7.75, 1.0)
    }),
    ("zoom both", |t| {
        (0.0, 0.3, 0.25 + slow(t) * 7.75, 0.25 + slow(t) * 3.75)
    }),
    ("fit whole session", |t| {
        (0.0, 0.0, 1.0, 0.08 + slow(t) * 0.4)
    }),
];

/// How many full traversals a scrolling gesture makes.
///
/// Deliberately brutal: someone grabbing the scrollbar and throwing it
/// from one end to the other, not a gentle sweep. That is the case that
/// matters, and it is the worst case for anything cached or culled,
/// because consecutive frames share almost nothing.
const LAPS: f64 = 14.0;

/// A triangle wave: out to the far end and all the way back.
fn tri(t: f64) -> f64 {
    let t = (t * LAPS) % 1.0;
    if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 }
}

/// The zoom's sweep — three passes over the range, not fourteen. A zoom
/// is a wheel or a pinch, not a yank.
fn slow(t: f64) -> f64 {
    let t = (t * 3.0) % 1.0;
    if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 }
}

/// How long each gesture runs before the next.
const GESTURE_SECS: f64 = 6.0;

/// How many recent frames the readout averages over.
const RECENT: usize = 32;

/// How far a wheel notch moves the session.
const WHEEL_LINE: f64 = 40.0;

/// How far a zoom may go, across and down.
/// The nominal zoom range, which is the BENCHMARK's sweep and not the
/// window's limits.
///
/// What a window lets you zoom out to is worked out from the session and
/// the frame — see `limits` in `WindowSize`. These two numbers survive
/// because a stress test wants a fixed range to sweep, not one that
/// changes with the content it is stressing.
const ZOOM_X: (f64, f64) = (0.02, 32.0);
const ZOOM_Y: (f64, f64) = (0.06, 6.0);

/// How much a pixel of movement zooms by.
///
/// Exponential, so the gesture feels the same at every scale: dragging
/// an inch doubles it whether you were far out or close in, where a
/// linear step crawls at one end and jumps at the other.
fn factor(pixels: f64) -> f64 {
    (pixels / 260.0).exp()
}

/// How thick a scrollbar is.
const BAR: f64 = 12.0;

/// One scrollbar: a track, and a thumb saying where in the session the
/// window is and how much of it is on screen.
///
/// Dragged rather than clicked-through, which is the gesture a DAW's
/// bars actually get: the thumb IS the view, and moving it is moving
/// the view.
#[component]
#[expect(
    clippy::too_many_arguments,
    reason = "a bar is its place, its extent and where the view is in it"
)]
fn Bar(
    across: bool,
    at: f64,
    travel: f64,
    window: f64,
    left: f64,
    top: f64,
    length: f64,
    colors: daw_ui::studio::lanes::Colors,
    on_move: EventHandler<f64>,
) -> Element {
    let whole = (travel + window).max(1.0);
    // Never smaller than a thumb you can hit, however long the session.
    let thumb = (window / whole * length).max(24.0);
    let along = (at / travel.max(1.0)) * (length - thumb);
    let dragging = use_signal(|| Option::<f64>::None);
    let mut held = dragging;

    let (size, place) = if across {
        (
            format!("left:{left}px; top:{top}px; width:{length}px; height:{BAR}px;"),
            format!(
                "left:{along:.1}px; top:2px; width:{thumb:.1}px; height:{}px;",
                BAR - 4.0
            ),
        )
    } else {
        (
            format!("left:{left}px; top:{top}px; width:{BAR}px; height:{length}px;"),
            format!(
                "left:2px; top:{along:.1}px; width:{}px; height:{thumb:.1}px;",
                BAR - 4.0
            ),
        )
    };

    rsx! {
        div {
            style: "position:absolute; {size} background:{colors.tcp_column};",
            onmousedown: move |event| {
                let at = event.data().element_coordinates();
                held.set(Some(if across { at.x } else { at.y }));
            },
            onmouseup: move |_| held.set(None),
            onmouseleave: move |_| held.set(None),
            onmousemove: move |event| {
                if dragging().is_none() {
                    return;
                }
                let at = event.data().element_coordinates();
                let along = if across { at.x } else { at.y };
                // Where the thumb's middle would put the view.
                let room = (length - thumb).max(1.0);
                let fraction = ((along - thumb / 2.0) / room).clamp(0.0, 1.0);
                on_move.call(fraction * travel.max(0.0));
            },
            div {
                style: "position:absolute; {place} background:{colors.text_dim}; \
                        border-radius:2px;",
            }
        }
    }
}

/// What the pointer and the keyboard are doing to the view.
#[derive(Default)]
struct Input {
    /// Whether the zoom spring is held.
    zooming: bool,
    shift: bool,
    pointer: (f64, f64),
    drag: Option<Drag>,
}

/// A drag in flight.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Drag {
    /// The hand: the session moves under the pointer.
    Pan,
    /// The zoom tool, sprung from a held `z`.
    Zoom,
}

#[component]
fn Window(props: ShotProps) -> Element {
    // The window's ACTUAL size, not the one the shot was configured
    // with. Laying out at a fixed number is what made the window ignore
    // a resize, show its left third in fullscreen, and scroll the rails
    // and the ruler along with the arrangement: a document larger than
    // the surface is a document the shell scrolls, and once the shell is
    // scrolling nothing inside it can decide what stays put.
    let size = use_signal(|| (props.view.width, props.view.height));

    let (width, height) = size();

    let scroll = use_signal(|| props.view.scroll_x);
    use_hook(|| {
        SCROLL.with(|slot| *slot.borrow_mut() = Some(scroll));
    });
    // What the gestures move that the scroll signal does not: down the
    // session, and both zooms. A pan is free because only one node reads
    // the scroll; a ZOOM is not, and cannot be — it changes where every
    // item is, which is a layout, which is the honest cost of the
    // gesture rather than a failure to optimise it.
    let down = use_signal(|| props.view.scroll_y);
    use_hook(|| {
        DOWN.with(|slot| *slot.borrow_mut() = Some(down));
    });
    let zoom = use_signal(|| (initial_zoom().0, initial_zoom().1 * props.view.zoom_y));
    use_hook(|| {
        ZOOM.with(|slot| *slot.borrow_mut() = Some(zoom));
    });
    let gesture = use_signal(|| (GESTURES[0].0, 0.0_f64));

    // How far the session runs across and down, in its OWN pixels — no
    // zoom in either, because this component may not read one.
    //
    // That is the rule the window kept breaking. Reading a signal here
    // re-renders this component, and this component builds the props for
    // every surface under it; measured at 5120x1440 that cost 11 ms a
    // frame on its own, on top of whatever actually changed. So the
    // scroll, the zoom, the frame rate and the gesture are read by the
    // three leaves that show them and by nothing else, and what travels
    // down from here is only ever a number that does not move.
    // How far the session runs, in its OWN pixels — the whole length at
    // the base zoom, with nothing subtracted.
    //
    // It used to have the window's width taken off before the zoom was
    // applied, which is the wrong order and made the travel wrong at
    // every zoom but one. Zoomed out far enough to see the whole song
    // you could still scroll a third of a screen past the end of it,
    // because the subtraction had happened while the session was still
    // notionally twenty-four thousand pixels wide.
    let span_x = (props.project.length_secs * PPS).max(1.0);
    let span_y = props
        .rows
        .iter()
        .map(|(track, _)| props.sizing.height_of(track.height))
        .sum::<f64>();
    // Mounted only when there IS a window: the same tree renders
    // headless for the comparisons, where there is no winit to ask.
    let tracking = if props.windowed {
        rsx! {
            WindowSize {
                size,
                animate: props.animate,
                span_x,
                span_y,
                frame: (frame_width(width), frame_height(height)),
                scroll,
                down,
                zoom,
                gesture,
            }
        }
    } else {
        rsx! {}
    };
    // What the controls are showing, held here so a press can change it.
    let mut live = use_signal(|| props.live.clone());
    // The same two numbers, as the lanes want them. A memo rather than a
    // prop so that reading it is subscribing to it: the one node that
    // has to move on every frame of a zoom does, and nothing else does.
    let magnified = use_memo(move || {
        let (x, y) = zoom();
        daw_ui::studio::lanes::Zoom { x, y }
    });
    // The placeholders, and a ground for them to sit on.
    let (top_rail, right_rail) = use_hook(placeholders);
    let rail_colors = daw_ui::studio::lanes::Colors {
        tcp_gutter: props.colors.tcp_tint.clone(),
        ..props.colors.clone()
    };
    let lanes_x = lane_x();
    let lanes_y = lane_y();
    // Deliberately WITHOUT the vertical scroll: reading it here would
    // re-render this component, and with it every row, item and control
    // below — which is the forty-millisecond frame. It travels as a
    // signal to the two components that move, and nothing between them
    // and it ever reads it.
    let moved = View {
        pps: PPS,
        width,
        height,
        ..props.view
    };
    // The lanes do NOT get the zoom folded into their view, and that is
    // the point of the whole exercise: a view that carried it changed
    // every frame of a gesture, and a changed view prop re-rendered every
    // row, every item and every lane's `<svg>`. They take the base view
    // and the zoom as a signal, and decide for themselves how much of it
    // is worth rebuilding for.
    let lanes = View {
        width: frame_width(width),
        height: frame_height(height),
        ..moved
    };
    // Neither of these carries the axis it does not use, and that is
    // not tidiness: a view is a prop, a changed prop re-renders the
    // component, and a ruler that carried the vertical zoom re-rendered
    // every mark in it every time a row got taller. The ruler has no
    // rows and the panel has no timeline.
    let ruler = View {
        width: width - session_daw::rails::SIDE * 2.0,
        ..moved
    };
    let panel_view = View {
        height: frame_height(height),
        ..moved
    };
    rsx! {
        div {
            // Sized from the surface the window reports, which is the
            // configured size when there is no window to ask — so the
            // same tree fills a resized window and still renders the
            // comparison shots at the size they are compared at.
            //
            // Not `100%`: a percentage needs a parent with a height to
            // resolve against, and in the headless document there is
            // none, so the studio collapsed to nothing and the composed
            // comparison rendered a blank page. A number the window
            // keeps up to date is the thing that works in both.
            style: "position:relative; width:{width}px; height:{height}px; \
                    overflow:hidden; background:{props.colors.surface};",
            {tracking}
            // The arrangement, as ONE node or as ten thousand.
            //
            // `FTS_BLITZ_WIDGET=0` paints the ruler, the panel and the
            // lanes as components instead of through
            // `session_daw::widget` — see that module for the
            // measurement that justifies the default. The switch exists
            // so the same window can be run both ways on the same
            // session and compared, which is a thing a commit message
            // cannot do.
            if props.widget {
                {
                    let w = frame_width(width) + session_daw::arrangement::TCP_WIDTH;
                    let h = height - session_daw::rails::TOP - session_daw::rails::SIDE;
                    rsx! {
                        object {
                            style: "position:absolute; left:{session_daw::rails::SIDE}px; \
                                    top:{session_daw::rails::TOP}px; \
                                    width:{w}px; height:{h}px;",
                            // The arrangement takes the keyboard — a
                            // rename is a text field inside it.
                            //
                            // Spelled out rather than relied on: the
                            // HTML default tabindex for `<object>` is
                            // 0, and Blitz's own comment says so, but
                            // its list of default-focusable elements
                            // leaves `object` out. Without this the
                            // node never focuses, key events go
                            // somewhere else, and a rename field opens
                            // that nothing can type into — which is
                            // exactly what happened.
                            tabindex: "0",
                            data: props.arrangement.clone(),
                        }
                    }
                }
            } else {
            div {
                style: "position:absolute; left:{session_daw::rails::SIDE}px; \
                        top:{session_daw::rails::TOP}px;",
                Ruler {
                    view: ruler,
                    colors: props.colors.clone(),
                    marks: props.marks.clone(),
                    sections: props.sections.clone(),
                    markers: props.markers.clone(),
                    scroll: ReadSignal::from(scroll),
                    zoom: ReadSignal::from(magnified),
                }
            }
            div {
                style: "position:absolute; left:{lanes_x}px; top:{lanes_y}px;",
                Lanes {
                    project: props.project.clone(),
                    rows: props.rows.clone(),
                    view: lanes,
                    scroll: ReadSignal::from(scroll),
                    scroll_y: ReadSignal::from(down),
                    colors: props.colors.clone(),
                    shapes: props.shapes.clone(),
                    sizing: props.sizing,
                    grid: props.grid,
                    zoom: ReadSignal::from(magnified),
                }
            }
            div {
                style: "position:absolute; left:{session_daw::rails::SIDE}px; top:{lanes_y}px;",
                Panel {
                    project: props.project.clone(),
                    rows: props.rows.clone(),
                    view: panel_view,
                    scroll_y: ReadSignal::from(down),
                    colors: props.colors.clone(),
                    theme: props.theme.clone(),
                    sizing: props.sizing,
                    zoom: ReadSignal::from(magnified),
                    live: live(),
                    on_press: move |(guid, control): (String, daw_ui::studio::panel::Control)| {
                        use daw_ui::studio::panel::Control;
                        let mut all = live.write();
                        let state = all.entry(guid).or_default();
                        match control {
                            Control::Mute => state.muted = !state.muted,
                            Control::Solo => state.soloed = !state.soloed,
                            Control::RecArm => state.armed = !state.armed,
                        }
                    },
                }
            }
            }
            Rails {
                width,
                height,
                // The rails lift off the window's ground so they read as
                // chrome around the session rather than as more of it —
                // a frame the same colour as what it frames is not a
                // frame. The mode bar keeps the painted window's own
                // colours, because that one IS in REAPER and matching it
                // is the point.
                colors: rail_colors,
                top: top_rail.into(),
                left: props.rail_items.0.clone().into(),
                right: right_rail.into(),
            }
            ModeBar {
                width: session_daw::arrangement::TCP_WIDTH,
                height: session_daw::ruler::RULER_H,
                colors: props.colors.clone(),
                modes: props.modes.clone().into(),
            }

            // The scrollbars: where you are, how much there is, and
            // the other way to move. Ours rather than the shell's,
            // because the wheel is handled above at the winit level —
            // Blitz would otherwise scroll a container underneath every
            // gesture that was meant to zoom — and because a DAW's bars
            // are a control, not a decoration.
            if props.windowed {
                Scrollbars {
                    scroll,
                    down,
                    zoom,
                    span_x,
                    span_y,
                    width,
                    height,
                    colors: props.colors.clone(),
                }
            }

            // What a frame costs is drawn by the widget itself now —
            // see `session_daw::fps`. A hundred bars with the budget
            // ruled across them says everything the element that used
            // to be here said, and the thing it could not: whether a
            // slow window is slow or is fast with a hitch in it.
        }
    }
}

/// The two scrollbars, and the only thing that works out how far the
/// view may travel.
///
/// A leaf on purpose. The extent depends on the zoom, the thumbs depend
/// on the scroll, and both of those change every frame of a gesture — so
/// whoever reads them re-renders every frame, and that has to be two
/// nodes rather than the window.
#[component]
#[expect(
    clippy::too_many_arguments,
    reason = "the two bars' signals and the frame they sit in"
)]
fn Scrollbars(
    mut scroll: Signal<f64>,
    mut down: Signal<f64>,
    zoom: Signal<(f64, f64)>,
    /// How far the session runs across and down in its own pixels,
    /// before any zoom.
    span_x: f64,
    span_y: f64,
    width: f64,
    height: f64,
    colors: daw_ui::studio::lanes::Colors,
) -> Element {
    let (zoom_x, zoom_y) = zoom();
    // The session's own extent less the window it is seen through.
    let travel = (
        (span_x * zoom_x - frame_width(width)).max(0.0),
        (span_y * zoom_y - frame_height(height)).max(0.0),
    );
    rsx! {
        Bar {
            across: true,
            at: scroll(),
            travel: travel.0,
            window: frame_width(width),
            left: lane_x(),
            top: height - session_daw::rails::SIDE - BAR,
            length: frame_width(width),
            colors: colors.clone(),
            on_move: move |to: f64| scroll.set(to.clamp(0.0, travel.0)),
        }
        Bar {
            across: false,
            at: down(),
            travel: travel.1,
            window: frame_height(height),
            left: width - session_daw::rails::SIDE - BAR,
            top: lane_y(),
            length: frame_height(height),
            colors,
            on_move: move |to: f64| down.set(to.clamp(0.0, travel.1)),
        }
    }
}

/// What each track's controls are showing.
///
/// Read off the project rather than invented, so the picture is the
/// session's and the comparison means something: a knob at whatever
/// value the file says is a knob the recorded renderer draws at the same
/// angle.
fn live_of(project: &ProjectRef) -> HashMap<String, daw_ui::studio::panel::Live> {
    project
        .tracks
        .iter()
        .map(|track| {
            (
                track.guid.clone(),
                daw_ui::studio::panel::Live {
                    volume: daw_theme_art::paint::tcp::gain_norm(track.volume),
                    pan: track.pan,
                    muted: track.muted,
                    soloed: track.soloed,
                    armed: track.armed,
                    parent_send: track.parent_send,
                    sends: track.send_count > 0,
                    receives: track.receive_count > 0,
                    effects: track.fx_count > 0,
                    phase_inverted: track.phase_inverted,
                },
            )
        })
        .collect()
}

/// What the empty rails hold until the real things exist.
///
/// The top rail is where the TRANSPORT goes — that is what the painted
/// window's own note says it is waiting for, and why its buttons are 74
/// wide "because the modes are named things". The right rail is for the
/// arrangement's own settings. Neither exists yet, so these are labelled
/// plates that do nothing, and they are here rather than in the shared
/// component on purpose: the components still draw what the painted
/// renderer draws, and the comparison that proves it still measures zero
/// differing pixels. A window is allowed to show more than a reference
/// does; a component is not.
fn placeholders() -> (Vec<Item>, Vec<Item>) {
    let font = session_daw::text::Font::embedded().expect("the embedded font");
    let fit = |labels: &[&str], room: f64| -> Vec<Item> {
        labels
            .iter()
            .map(|label| {
                let (label, size) = font.fit(label, 10.0, 6.0, room - 4.0);
                Item {
                    label,
                    on: false,
                    size: Some(f64::from(size)),
                }
            })
            .collect()
    };
    (
        fit(
            &["Play", "Stop", "Rec", "Loop", "Click", "Tempo"],
            session_daw::rails::TOP_ITEM_W - 3.0,
        ),
        fit(
            &["Grid", "Snap", "Fit", "Zoom"],
            session_daw::rails::SIDE - 6.0,
        ),
    )
}

/// The modes, abbreviated and fitted the way the corner draws them.
fn modes() -> Vec<Item> {
    let all = session::modes::Mode::ALL;
    let each =
        session_daw::arrangement::TCP_WIDTH / f64::from(u32::try_from(all.len()).unwrap_or(1));
    let font = session_daw::text::Font::embedded().expect("the embedded font");
    all.iter()
        .map(|mode| {
            // Three letters is what fits a tenth of the corner. A
            // placeholder for an icon, not a naming decision.
            let name = mode.display_name();
            let short = name
                .char_indices()
                .nth(3)
                .map_or(name, |(byte, _)| &name[..byte]);
            let (label, size) = font.fit(short, 10.0, 6.0, each - 2.0 - 4.0);
            Item {
                label,
                on: *mode == session::modes::Mode::Mix,
                size: Some(f64::from(size)),
            }
        })
        .collect()
}

/// The bar numbers and tempo readings of the open session.
///
/// Walked through the tempo map here rather than in the component, for
/// the reason the component says: a bar is however many beats the
/// signature says it is, and only whoever owns the map can count them.
fn marks_of(project: &ProjectRef) -> Marks {
    let view = session_daw::arrangement::Viewport {
        scroll_x: 0.0,
        scroll_y: 0.0,
        pps: PPS,
        zoom_y: 1.0,
        width: f64::MAX / 4.0,
        height: 0.0,
    };
    let bars = session_daw::ruler::Bars::at(project.bpm);
    let step = session_daw::ruler::step_beats(
        bars.secs_per_bar() * PPS,
        f64::from(project.tempo.first().map_or(4, |t| t.beats_per_bar)),
    );
    let beats = session_daw::ruler::Timeline::new(&project.tempo)
        .beats(project.length_secs.max(1.0), 100_000);
    let _ = view;

    let mut ticks = Vec::new();
    let mut since = 0.0_f64;
    for (index, beat) in beats.iter().enumerate() {
        if index > 0 {
            since += 1.0;
        }
        let on_step = if step >= 1.0 {
            (since % step).abs() < 1e-6 || (since % step - step).abs() < 1e-6
        } else {
            true
        };
        if !on_step {
            continue;
        }
        if step < 1.0 {
            let mut fraction = 0.0_f64;
            while fraction < 1.0 - 1e-9 {
                ticks.push(Tick {
                    at: fraction.mul_add(beat.secs_per_beat, beat.at),
                    label: session_daw::ruler::written(
                        beat.measure,
                        f64::from(beat.beat - 1) + fraction,
                        f64::from(beat.per_bar),
                        step,
                    ),
                });
                fraction += step;
            }
        } else {
            ticks.push(Tick {
                at: beat.at,
                label: session_daw::ruler::written(
                    beat.measure,
                    f64::from(beat.beat - 1),
                    f64::from(beat.per_bar),
                    step,
                ),
            });
        }
    }

    Marks {
        ticks: ticks.into(),
        tempo: project
            .tempo
            .iter()
            .map(|change| Reading {
                at: change.at,
                text: session_daw::ruler::reading(change),
            })
            .collect::<Vec<_>>()
            .into(),
        tempo_before: None,
    }
}
