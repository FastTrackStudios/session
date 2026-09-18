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

use anyrender::ImageRenderer as _;
use anyrender_vello::VelloImageRenderer;
use blitz_dom::{Document as _, DocumentConfig};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;

use daw_ui::studio::lanes::{Colors, Grid, Lanes, Note, Rows, Shape, Shapes, View};
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

    let (width, height) = size();
    // The lane rect: what is left of the frame once the rails and the
    // track panel and the ruler have taken theirs. The component
    // renderer draws only this, because this is the part that was in
    // question — the panel beside it is already components.
    let lane_w = frame_width(width);
    let lane_h = frame_height(height);

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
}

#[component]
fn Shot(props: ShotProps) -> Element {
    rsx! {
        // The user-agent stylesheet gives the body an eight-pixel
        // margin, which is eight pixels of the session pushed off the
        // bottom of the window and every row eight pixels from where the
        // reference draws it. A window is not a document.
        style { "html, body {{ margin: 0; padding: 0; }}" }
        Lanes {
            project: props.project,
            rows: props.rows,
            view: props.view,
            colors: props.colors,
            shapes: props.shapes,
            sizing: props.sizing,
            grid: props.grid,
        }
    }
}
