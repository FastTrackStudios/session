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
    let part = std::env::var("FTS_BLITZ_PART").unwrap_or_else(|_| "lanes".to_owned());
    let (width, height) = size();
    // The lane rect: what is left of the frame once the rails and the
    // track panel and the ruler have taken theirs. The component
    // renderer draws only this, because this is the part that was in
    // question — the panel beside it is already components.
    let ruler = part == "ruler";
    let rails = part == "rails";
    let lane_w = if rails {
        width
    } else if ruler {
        (width - session_daw::rails::SIDE * 2.0).max(1.0)
    } else {
        frame_width(width)
    };
    let lane_h = if rails {
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
    let marks = marks_of(&project);
    let sections = project.sections.clone().into();
    let markers = project.markers.clone().into();

    let vdom = VirtualDom::new_with_props(
        Shot,
        ShotProps {
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
            rail_items: rail_items(),
            modes: modes(),
        },
    );
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
            ..Default::default()
        },
    );
    document.initial_build();
    // Twice: the first poll builds the tree, and anything that resolves
    // on mount — a memo, a shape that was not ready — lands on the
    // second. A picture taken between the two is a picture of a UI
    // mid-construction, which is how a renderer gets accused of dropping
    // content it simply had not been given yet.
    document.poll(None);
    {
        let mut inner = document.inner_mut();
        inner.resolve(0.0);
    }
    document.poll(None);
    {
        let mut inner = document.inner_mut();
        inner.resolve(0.0);
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
    println!("\n  The component lanes, panning the golden session\n");
    println!("  surface       {width}x{height}");
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
    rail_items: (Vec<Item>, Vec<Item>, Vec<Item>),
    modes: Vec<Item>,
}

thread_local! {
    /// The scroll, reachable from outside the runtime so the benchmark
    /// can drive it the way a scrollbar would.
    static SCROLL: std::cell::RefCell<Option<Signal<f64>>> =
        const { std::cell::RefCell::new(None) };
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
        style { "html, body {{ margin: 0; padding: 0; }}" }
        if props.rails {
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
