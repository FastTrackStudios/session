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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,blitz=info,usvg=error".into()),
        )
        .init();

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
    let (project, rows) = read_back().expect("read the project back");
    let shapes = shapes_of(&project);

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
        dioxus_native::launch_cfg_with_props(Shot, props, Vec::new(), Vec::new());
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
        pan(&mut document, w, h, frames);
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
fn pan(document: &mut DioxusDocument, width: u32, height: u32, frames: usize) {
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
            // A minute of session under the playhead, which moves every
            // item on screen and changes which ones are there at all.
            SCROLL.with(|scroll| {
                if let Some(mut scroll) = *scroll.borrow() {
                    document.vdom.in_runtime(|| scroll.set(t * 60.0 * PPS));
                }
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
    println!("\n  The component window, panning the golden session\n");
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
        "pan",
        frame.mean,
        frame.p99,
        frame.worst,
        solve.mean,
        paint.mean,
        1000.0 / frame.p99.max(0.001)
    );
    println!(
        "\n  reconciling {:.2}ms a frame — the pan re-renders nothing",
        diff.mean
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
fn read_back() -> Option<(ProjectRef, RowsRef)> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    let project = rt.block_on(daw_ui::studio::project::fetch())?;
    let project = ProjectRef(Arc::new(project));
    let (visible, depths) =
        daw_ui::components::folders::FolderState::default().visible(&project.tracks);
    let rows = RowsRef(Arc::new(visible.into_iter().zip(depths).collect()));
    Some((project, rows))
}

/// Every item's shape: its notes if it holds MIDI, its peaks otherwise.
///
/// The notes are read BEFORE anything is drawn, not after: a renderer
/// draws one frame and exits, so there is no later for them to arrive
/// in — and an item drawn from a waveform it does not have is why a
/// chord track once looked like a shaker.
fn shapes_of(project: &ProjectRef) -> Shapes {
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
    Shapes(Arc::new(shapes))
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
}

thread_local! {
    /// The scroll, reachable from outside the runtime so the benchmark
    /// can drive it the way a scrollbar would.
    static SCROLL: std::cell::RefCell<Option<Signal<f64>>> =
        const { std::cell::RefCell::new(None) };
}

#[component]
fn Shot(props: ShotProps) -> Element {
    let mut scroll = use_signal(|| props.view.scroll_x);
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
/// Keeps `size` in step with the window.
///
/// A component rather than a few hooks in `Window`, because the hooks it
/// needs only exist when a winit window does — and a hook cannot be
/// called conditionally, while a component can be mounted conditionally.
/// It draws nothing; it exists to hold three hooks.
#[component]
fn Surface(size: Signal<(f64, f64)>) -> Element {
    let handle = dioxus_native::use_window();
    let mut size = size;
    use_hook(move || {
        let px = handle.surface_size();
        size.set((f64::from(px.width.max(1)), f64::from(px.height.max(1))));
    });
    dioxus_native::use_window_event(move |event, _| {
        if let winit::event::WindowEvent::SurfaceResized(px) = event {
            size.set((f64::from(px.width.max(1)), f64::from(px.height.max(1))));
        }
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

    let mut scroll = use_signal(|| props.view.scroll_x);
    use_hook(|| {
        SCROLL.with(|slot| *slot.borrow_mut() = Some(scroll));
    });
    // What the gestures move that the scroll signal does not: down the
    // session, and both zooms. A pan is free because only one node reads
    // the scroll; a ZOOM is not, and cannot be — it changes where every
    // item is, which is a layout, which is the honest cost of the
    // gesture rather than a failure to optimise it.
    let mut down = use_signal(|| props.view.scroll_y);
    let mut zoom = use_signal(|| (1.0_f64, props.view.zoom_y));
    let mut gesture = use_signal(|| (GESTURES[0].0, 0.0_f64));

    // The two numbers the driver needs, taken out of the props before it
    // captures them — a closure that owned the project would own it
    // instead of the tree below.
    let span_x = (props.project.length_secs * PPS - props.view.width).max(1.0);
    if props.animate {
        use_future(move || async move {
            let started = std::time::Instant::now();
            let mut frames = 0u32;
            let mut last = started;
            loop {
                // Sixty times a second, which is what a window is asked
                // for. Anything the frame cannot finish in shows up as a
                // rate below it rather than as a faster loop.
                tokio::time::sleep(std::time::Duration::from_millis(16)).await;
                let now = std::time::Instant::now();
                let elapsed = started.elapsed().as_secs_f64();
                let index = ((elapsed / GESTURE_SECS) as usize) % GESTURES.len();
                let (name, at) = GESTURES[index];
                let t = (elapsed % GESTURE_SECS) / GESTURE_SECS;
                let (x, y, zx, zy) = at(t);

                // How far down a session this deep goes. Fixed rather
                // than measured because the point is a brutal gesture,
                // not a correct one.
                let span_y = 40_000.0_f64;
                scroll.set(x * span_x);
                down.set(y * span_y);
                zoom.set((zx, zy));

                frames = frames.saturating_add(1);
                if now.duration_since(last).as_secs_f64() >= 0.5 {
                    let fps = f64::from(frames) / now.duration_since(last).as_secs_f64();
                    gesture.set((name, fps));
                    frames = 0;
                    last = now;
                }
            }
        });
    }

    // What the controls are showing, held here so a press can change it.
    // The panel hands back which track and which control; deciding what
    // mute MEANS is the session's business, and in a demo window the
    // session is this.
    let mut live = use_signal(|| props.live.clone());
    let (zoom_x, zoom_y) = zoom();
    let lanes_x = lane_x();
    let lanes_y = lane_y();
    let moved = View {
        scroll_y: down(),
        pps: PPS * zoom_x,
        zoom_y,
        width,
        height,
        ..props.view
    };
    let lanes = View {
        width: frame_width(width),
        height: frame_height(height),
        ..moved
    };
    let ruler = View {
        width: width - session_daw::rails::SIDE * 2.0,
        ..moved
    };
    // How far down the session goes, for the scroller's extent.
    let content_height = props
        .rows
        .iter()
        .map(|(track, _)| props.sizing.height_of(track.height))
        .sum::<f64>()
        .mul_add(zoom_y, 0.0)
        .max(1.0);
    let panel_view = View {
        height: frame_height(height),
        ..moved
    };
    rsx! {
        div {
            style: "position:relative; width:{props.view.width}px; \
                    height:{props.view.height}px; overflow:hidden; \
                    background:{props.colors.surface};",
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
                }
            }
            div {
                style: "position:absolute; left:{lanes_x}px; top:{lanes_y}px;",
                Lanes {
                    project: props.project.clone(),
                    rows: props.rows.clone(),
                    view: lanes,
                    scroll: ReadSignal::from(scroll),
                    colors: props.colors.clone(),
                    shapes: props.shapes.clone(),
                    sizing: props.sizing,
                    grid: props.grid,
                }
            }
            div {
                style: "position:absolute; left:{session_daw::rails::SIDE}px; top:{lanes_y}px;",
                Panel {
                    project: props.project.clone(),
                    rows: props.rows.clone(),
                    view: panel_view,
                    colors: props.colors.clone(),
                    theme: props.theme.clone(),
                    sizing: props.sizing,
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
            Rails {
                width,
                height,
                colors: props.colors.clone(),
                top: props.rail_items.2.clone().into(),
                left: props.rail_items.0.clone().into(),
                right: props.rail_items.1.clone().into(),
            }
            ModeBar {
                width: session_daw::arrangement::TCP_WIDTH,
                height: session_daw::ruler::RULER_H,
                colors: props.colors.clone(),
                modes: props.modes.clone().into(),
            }

            // The thing the wheel actually turns.
            //
            // Blitz never dispatches a `wheel` event to the DOM: its
            // handler calls `scroll_by` on whatever the pointer is over
            // and redraws only if something moved. So an `onwheel`
            // handler can never fire, and a window with nothing
            // scrollable in it does not merely refuse to scroll — it
            // stops repainting, because no scroll means no redraw. That
            // is what "frozen" was.
            //
            // So there is a real scroller, and it is the input device:
            // an element the size of the lane rect holding a spacer the
            // size of the session, which Blitz scrolls natively. Its
            // offsets come back as a `Scroll` event and become the
            // numbers everything else is drawn from.
            //
            // The content is drawn UNDER it rather than inside it,
            // because Blitz has no `position: sticky` and a panel and a
            // ruler inside a scroller would scroll away with the
            // session. And it is LAST, so the pointer finds it — it
            // covers the lane rect and nothing else, which is why the
            // panel's buttons are still clickable: they are to the left
            // of it.
            if props.windowed {
                div {
                    style: "position:absolute; left:{lane_x()}px; top:{lane_y()}px; \
                            width:{frame_width(width)}px; height:{frame_height(height)}px; \
                            overflow:auto;",
                    onscroll: move |event| {
                        let data = event.data();
                        scroll.set(f64::from(data.scroll_left()).max(0.0));
                        down.set(f64::from(data.scroll_top()).max(0.0));
                    },
                    // The session's extent, and nothing else: what makes
                    // the scroller scrollable and tells its bars how far
                    // there is to go.
                    div {
                        style: "width:{(props.project.length_secs * PPS * zoom_x).max(1.0)}px; \
                                height:{content_height}px;",
                    }
                }
            }

            // What it is doing and how fast, on the window rather than in
            // a terminal behind it: the point of running the gestures
            // here is to watch them, and a rate you have to look away to
            // read is a rate you cannot match to what you just saw.
            if props.animate {
                {
                    let (name, fps) = gesture();
                    rsx! {
                        div {
                            style: "position:absolute; right:56px; bottom:12px; \
                                    padding:6px 10px; background:rgba(0,0,0,0.72); \
                                    color:#e8e8ea; font-size:12px; \
                                    font-family:{daw_ui::studio::lanes::FONT}; \
                                    white-space:nowrap;",
                            "{name} — {fps:.0} fps"
                        }
                    }
                }
            }
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
