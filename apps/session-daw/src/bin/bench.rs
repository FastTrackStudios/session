//! The arrangement, benchmarked headlessly.
//!
//! ```sh
//! just daw-bench                       # the synthetic 65/877 fixture
//! just daw-bench 200 20000             # an orchestral template
//! ```
//!
//! No window, no surface, no vsync. `anyrender`'s `ImageRenderer` draws
//! into a buffer, so this measures what the RENDERER can do rather than
//! what a monitor will accept — which is the number that says whether
//! there is headroom, and the only one that is comparable between
//! machines.
//!
//! The windowed build is pinned to whichever display it opens on: 240 fps
//! on the 240 Hz panel, 180 on the 180 Hz one, both of them simply vsync.
//! Useful for "does it keep up", useless for "how much room is left".
//! This answers the second question, and it runs on a headless box, in
//! CI, or over ssh.
//!
//! It is not a substitute for the windowed number. There is no
//! compositor here and no present, so this is the upper bound on drawing
//! alone. Quote it as that.

use std::time::Instant;

use anyrender::{ImageRenderer, PaintScene};
use anyrender_vello::VelloImageRenderer;
use vello::kurbo::Affine;

use session_daw::arrangement::{Arrangement, Palette, Viewport, TCP_WIDTH};
use session_daw::headless::{Headless, BATCH};
use session_daw::profile::{Counts, Stages, Summary};
use session_daw::ruler::{self, Bars, RULER_H};

/// One phase's motion over `0..1`, as (`scroll_x`, `scroll_y`,
/// `zoom_x`, `zoom_y`) fractions.
///
/// Boxed rather than a plain `fn` because the fit-everything phase has
/// to close over the scene's height — the zoom at which a session fits
/// depends on how tall it is, and a fixture-independent phase cannot be
/// a constant.
type Gesture = Box<dyn Fn(f64) -> (f64, f64, f64, f64)>;

/// The surface to draw into, `WIDTHxHEIGHT`.
///
/// Defaults to the ultrawide's native 5120x1440 — 7.4 megapixels, 3.2x a
/// 1600x900 window. A 2D renderer is fill-rate bound as much as
/// command-bound, so a number from a small surface says little about the
/// screen this actually runs on.
const SIZE_ENV: &str = "FTS_BENCH_SIZE";
const DEFAULT_SIZE: (u32, u32) = (5120, 1440);
/// Enough to see past the first-frame costs and get a stable median.
const FRAMES: usize = 240;
/// Pixels per second, matching the window's opening zoom.
const PPS: f64 = 40.0;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,bench=info,session_daw=info".into()),
        )
        .init();

    let path = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("SESSION_DAW_PROJECT").ok())
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists());
    let Some(path) = path else {
        eprintln!("bench needs a project: cargo run --bin bench -- <song.rpp>");
        std::process::exit(2);
    };

    let (width, height) = std::env::var(SIZE_ENV)
        .ok()
        .and_then(|s| {
            let (w, h) = s.split_once(['x', 'X'])?;
            Some((w.trim().parse().ok()?, h.trim().parse().ok()?))
        })
        .unwrap_or(DEFAULT_SIZE);

    let theme = daw_ui::theming::Theme::dark();
    let palette = Palette::from_theme(&theme);
    let font = session_daw::text::Font::embedded().expect("the embedded font");
    let layout = session_daw::layout::Layout::from_env();

    let opened = session_daw::open::open_and_serve(&path).expect("open project");
    let scene = build_scene(&palette, layout).expect("read project back");
    tracing::info!(
        project.tracks = opened.track_count,
        scene.rows = scene.rows,
        "benchmarking"
    );

    let mut renderer = Headless::new(width, height).expect("open a gpu device");
    // The bar grid, and the adaptive division that follows the zoom.
    let bars = Bars::at(scene.bpm);
    let grid = adaptive_grid::Adaptive::default();
    /// The finest the grid ever gets: sixteenths, as a fraction of a
    /// whole note. The zoom only ever coarsens away from it.
    const FINEST: f64 = 1.0 / 16.0;

    let span_y = (scene.content_height() - f64::from(height)).max(1.0);
    let span_x = (scene.length_secs * PPS - f64::from(width)).max(1.0);

    // One phase per GESTURE, because they are different work and a
    // blended average hides which one hurts. Scrolling down changes
    // which lanes are on screen; scrolling across changes which part of
    // every lane; zooming changes the geometry of everything at once and
    // is the only one that can make more of the session visible than the
    // screen has ever held.
    //
    // `t` runs 0..1 over the phase. Each returns (scroll_x, scroll_y,
    // zoom_x, zoom_y).
    /// How many full end-to-end traversals each phase performs.
    ///
    /// Deliberately brutal: at this rate a phase crosses all 2,000
    /// tracks in about eight frames — someone grabbing the scrollbar and
    /// yanking it from top to bottom, not a gentle sweep. That is the
    /// case that matters, and it is the worst case for anything we might
    /// later cache or cull, because consecutive frames share almost no
    /// visible content.
    const LAPS: f64 = 14.0;

    // Vertical zoom at which every track fits on screen at once.
    // Computed from the scene rather than guessed, so the phase means
    // the same thing whatever fixture it is pointed at.
    let fit = f64::from(height) / scene.content_height().max(1.0);

    /// A triangle wave: out to the far end of the session and all the way
    /// back, `LAPS` times over the phase.
    fn tri(t: f64) -> f64 {
        let t = (t * LAPS) % 1.0;
        if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 }
    }
    /// The zoom sweep: three passes over the range, not fourteen.
    fn slow(t: f64) -> f64 {
        let t = (t * 3.0) % 1.0;
        if t < 0.5 { t * 2.0 } else { 2.0 - t * 2.0 }
    }

    let phases: Vec<(&str, Gesture)> = vec![
        ("scroll down/up", Box::new(|t| (0.0, tri(t), 1.0, 1.0))),
        ("scroll right/left", Box::new(|t| (tri(t), 0.0, 1.0, 1.0))),
        ("scroll both", Box::new(|t| (tri(t), tri((t * 1.7) % 1.0), 1.0, 1.0))),
        // A zoom is a slower gesture than a yank — a wheel or a pinch,
        // not a thrown scrollbar — so these sweep their range a few times
        // rather than fourteen.
        ("zoom vertical", Box::new(|t| (0.0, 0.3, 1.0, 0.25 + slow(t) * 3.75))),
        ("zoom horizontal", Box::new(|t| (0.0, 0.3, 0.25 + slow(t) * 7.75, 1.0))),
        (
            "zoom both",
            Box::new(|t| (0.0, 0.3, 0.25 + slow(t) * 7.75, 0.25 + slow(t) * 3.75)),
        ),
        // The whole session at once, which is what per-track heights and
        // a two-pixel floor are FOR. Culling cannot help here by
        // definition — every row is on screen — so this is the genuine
        // worst case and it belongs in the table rather than in a
        // footnote about how fast the other phases are.
        (
            "fit whole session",
            Box::new(move |t| (0.0, 0.0, 1.0, fit + slow(t) * (0.25 - fit))),
        ),
    ];

    println!();
    println!("  scene         {} rows, {} items", scene.rows, scene.items());
    println!(
        "  surface       {width}x{height}, headless  ({:.1} megapixels)",
        f64::from(width) * f64::from(height) / 1_000_000.0
    );
    println!("  frames        {FRAMES} per phase");
    println!(
        "  measured      frames rendered to a texture in batches of {BATCH}, waited on once\n\
         \x20               per batch — no readback, because the app never does one\n"
    );
    println!(
        "  {:<20} {:>9} {:>9} {:>9} {:>9}   {:>7} {:>7}",
        "phase", "mean", "p99", "worst", "fps(p99)", "paint", "gpu"
    );
    println!("  {}", "-".repeat(78));

    if let Ok(out) = std::env::var("FTS_BENCH_SHOT") {
        // Look at a frame instead of arguing about one. Renders the
        // window's opening view and writes it to a PNG, which is the
        // fastest way to tell a culling bug (geometry missing) from a
        // palette bug (geometry there, wrong colour).
        shot(&scene, &palette, &font, &std::path::PathBuf::from(out), width, height);
        return;
    }

    if std::env::var("FTS_BENCH_VERIFY").is_ok() {
        // Verification is the one place a readback is the POINT: it
        // compares pixels, so it uses the image renderer rather than the
        // texture-only one the timings use.
        let mut image = VelloImageRenderer::new(width, height);
        verify(&mut image, &scene, &phases, span_x, span_y, width, height);
        return;
    }

    let mut all: Vec<(&str, Summary, Summary, Counts)> = Vec::new();
    for (name, gesture) in phases.iter() {
        let mut stages = Stages::with_capacity(FRAMES);
        let mut counts = Counts::default();
        // Frames go out in batches and the GPU is waited on once at the
        // end of each. Waiting per frame would measure a pipeline
        // deliberately drained, which no running app ever is; waiting
        // never would measure how fast work can be QUEUED.
        for batch in 0..FRAMES / BATCH {
            let batch_start = Instant::now();
            let mut painted = 0.0;
            for step in 0..BATCH {
                let frame = batch * BATCH + step;
                let t = frame as f64 / FRAMES as f64;
                let (fx, fy, zx, zy) = gesture(t);
                let (scroll_x, scroll_y) = (span_x * fx, span_y * fy);
                // The viewport and the transform are the same fact
                // stated twice — one culls, one draws — so they are
                // built next to each other and from the same values.
                let view = Viewport {
                    scroll_x,
                    scroll_y,
                    pps: PPS * zx,
                    zoom_y: zy,
                    width: f64::from(width),
                    height: f64::from(height),
                };
                let mut drawn = Counts::default();
                painted += renderer
                    .frame(|painter| {

                    let a = scene.replay_lanes(
                        painter,
                        view,
                        Affine::translate((TCP_WIDTH - scroll_x, -scroll_y))
                            * Affine::scale_non_uniform(PPS * zx, zy),
                    );
                    let b = scene.replay_panel(
                        painter,
                        view,
                        Affine::translate((0.0, -scroll_y)) * Affine::scale_non_uniform(1.0, zy),
                    );
                        // After the lanes, not before: the lane
                        // backgrounds are opaque and painted the grid
                        // straight out of the frame.
                        ruler::grid(painter, &palette, view, bars, &grid, FINEST);
                        ruler::ruler(painter, &palette, &font, view, bars);
                        drawn.replayed = a.replayed + b.replayed;
                        drawn.submitted = a.submitted + b.submitted;
                    })
                    .expect("render a frame");
                counts = drawn;
            }
            renderer.wait().expect("the gpu to finish the batch");
            // Per-frame, so a batch is comparable to a frame budget.
            let per_frame = batch_start.elapsed().as_secs_f64() * 1000.0 / BATCH as f64;
            stages.frame.push_ms(per_frame);
            stages.paint.push_ms(painted / BATCH as f64);
        }
        // The first batch warms up geometry the previous phase never
        // touched; that is the cost of CHANGING gesture, not of making it.
        stages.frame.drop_warmup(1);
        stages.paint.drop_warmup(1);
        let frame = stages.frame.summary().expect("batches");
        let paint = stages.paint.summary().expect("batches");
        println!(
            "  {name:<20} {:>7.2}ms {:>7.2}ms {:>7.2}ms {:>9.0}   {:>7.2} {:>7.2}",
            frame.mean,
            frame.p99,
            frame.worst,
            frame.fps(),
            paint.mean,
            (frame.mean - paint.mean).max(0.0),
        );
        all.push((*name, frame, paint, counts));
    }

    let worst = all
        .iter()
        .max_by(|a, b| a.1.p99.partial_cmp(&b.1.p99).expect("no NaN"))
        .expect("at least one phase");
    let (name, frame, paint, counts) = worst;
    let gpu = (frame.mean - paint.mean).max(0.0);
    println!(
        "\n  worst gesture: {name} at {:.2}ms p99 ({:.0} fps)",
        frame.p99,
        frame.fps()
    );
    // Where the worst frame's time went, and how much of the scene it
    // had to walk to spend it. If paint dominates, the win is in what we
    // submit; if gpu dominates, it is in what we ask Vello to rasterise.
    println!(
        "    paint {:.2}ms ({:.0}%)   gpu {:.2}ms ({:.0}%)",
        paint.mean,
        paint.mean * 100.0 / frame.mean.max(0.001),
        gpu,
        gpu * 100.0 / frame.mean.max(0.001),
    );
    println!(
        "    {} commands walked, {} submitted ({:.0}% kept)",
        counts.replayed,
        counts.submitted,
        counts.kept_pct().unwrap_or(0.0),
    );
    println!(
        "  headroom there at 240Hz: {:.2}x   at 60Hz: {:.2}x\n",
        (1000.0 / 240.0) / frame.p99,
        (1000.0 / 60.0) / frame.p99,
    );
}

/// Prove the culling draws the same frame the full replay does.
///
/// Every phase is sampled across its whole sweep and each viewport is
/// rendered twice — culled, then complete — and the two buffers compared
/// byte for byte. Any difference is a dropped or mis-clipped command,
/// reported with the viewport that produced it so it can be reproduced.
///
/// This is the control for the numbers the bench prints. Without it the
/// headline is "we made it five times faster by drawing less", which is
/// only interesting if the missing part was never on screen.
fn verify(
    renderer: &mut VelloImageRenderer,
    scene: &Arrangement,
    phases: &[(&str, Gesture)],
    span_x: f64,
    span_y: f64,
    width: u32,
    height: u32,
) {
    /// Sampled per phase. Enough to land on the ends of every sweep and
    /// the awkward fractions between them.
    const SAMPLES: usize = 120;

    let mut culled = Vec::new();
    let mut complete = Vec::new();
    let mut checked = 0usize;
    let mut bad = 0usize;
    // Viewports that matched only within the tolerance. Counted and
    // reported rather than ignored: if this climbs, the tolerance is
    // hiding something and wants looking at.
    let mut rounded = 0usize;

    println!("\n  verifying culled == complete over {SAMPLES} viewports per phase\n");
    for (name, gesture) in phases.iter() {
        let mut worst_kept = 100.0f64;
        let mut empty = 0usize;
        for sample in 0..SAMPLES {
            let t = sample as f64 / SAMPLES as f64;
            let (fx, fy, zx, zy) = gesture(t);
            let (scroll_x, scroll_y) = (span_x * fx, span_y * fy);
            let view = Viewport {
                scroll_x,
                scroll_y,
                pps: PPS * zx,
                zoom_y: zy,
                width: f64::from(width),
                height: f64::from(height),
            };
            let lanes = Affine::translate((TCP_WIDTH - scroll_x, -scroll_y))
                * Affine::scale_non_uniform(PPS * zx, zy);
            let panel = Affine::translate((0.0, -scroll_y)) * Affine::scale_non_uniform(1.0, zy);

            let mut counts = Counts::default();
            renderer.render_to_vec(
                |painter| {
                    painter.reset();
                    let a = scene.replay_lanes(painter, view, lanes);
                    let b = scene.replay_panel(painter, view, panel);
                    counts.replayed = a.replayed + b.replayed;
                    counts.submitted = a.submitted + b.submitted;
                },
                &mut culled,
            );
            renderer.render_to_vec(
                |painter| {
                    painter.reset();
                    session_daw::arrangement::replay_all(painter, &scene.lanes, lanes);
                    scene.replay_all_panel(painter, view, panel);
                },
                &mut complete,
            );

            checked += 1;
            // Compared with a one-LSB tolerance, not bit-for-bit.
            //
            // Vello accumulates coverage on the GPU, and the same curve
            // can land a single channel 1/255 apart depending on how
            // many commands preceded it — which culling changes by
            // design. Twelve such pixels showed up here, all of them on
            // the antialiased edge of a knob.
            //
            // The tolerance is deliberately tiny, because the thing this
            // guards against is not subtle: a control that got culled
            // while visible differs over its whole area, by the full
            // distance between it and the background. One LSB cannot
            // hide that, and demanding exactness instead would mean
            // reporting a renderer's rounding as a correctness failure
            // every run.
            const TOLERANCE: i16 = 1;

            let mut differing = 0_usize;
            let mut worst_delta = 0_i16;
            let mut at = (0_usize, 0_usize, 0_usize);
            for (i, (a, b)) in culled.iter().zip(&complete).enumerate() {
                let delta = i16::from(*a) - i16::from(*b);
                if delta == 0 {
                    continue;
                }
                differing += 1;
                if delta.abs() > worst_delta.abs() {
                    worst_delta = delta;
                    let pixel = i / 4;
                    at = (pixel % width as usize, pixel / width as usize, i % 4);
                }
            }
            if differing > 0 {
                rounded += 1;
            }
            if worst_delta.abs() > TOLERANCE {
                bad += 1;
                bad += 1;
                println!(
                    "  MISMATCH {name}: scroll ({scroll_x:.0}, {scroll_y:.0}) zoom ({zx:.2}, \
                     {zy:.2}) — {differing} bytes, worst {worst_delta:+} at ({}, {}) channel {}",
                    at.0, at.1, at.2,
                );
            }
            if let Some(kept) = counts.kept_pct() {
                worst_kept = worst_kept.min(kept);
            } else {
                empty += 1;
            }
        }
        let note = if empty > 0 {
            format!(", {empty} showed nothing at all")
        } else {
            String::new()
        };
        println!(
            "  {name:<20} {SAMPLES} viewports identical, kept as low as {worst_kept:.0}%{note}"
        );
    }

    if bad == 0 {
        println!(
            "\n  {checked} viewports: culled output matches complete \
             ({rounded} differed only by a rounding LSB)\n"
        );
    } else {
        println!("\n  {bad} of {checked} viewports DIFFER — the culling is dropping visible work\n");
        std::process::exit(1);
    }
}

/// Write one frame to a PNG.
///
/// `FTS_BENCH_SHOT=/tmp/frame.png`, with `FTS_BENCH_SCROLL=x,y` and
/// `FTS_BENCH_ZOOM=x,y` to move it. The view defaults to the window's
/// opening one so the image is directly comparable to what is on screen.
fn shot(
    scene: &Arrangement,
    palette: &Palette,
    font: &session_daw::text::Font,
    out: &std::path::Path,
    width: u32,
    height: u32,
) {
    let pair = |name: &str, default: (f64, f64)| {
        std::env::var(name)
            .ok()
            .and_then(|v| {
                let (a, b) = v.split_once(',')?;
                Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
            })
            .unwrap_or(default)
    };
    let (scroll_x, scroll_y) = pair("FTS_BENCH_SCROLL", (0.0, 0.0));
    let (zoom_x, zoom_y) = pair("FTS_BENCH_ZOOM", (1.0, 1.0));

    let view = Viewport {
        scroll_x,
        scroll_y,
        pps: PPS * zoom_x,
        zoom_y,
        width: f64::from(width),
        height: f64::from(height),
    };
    let mut image = VelloImageRenderer::new(width, height);
    let mut buffer = Vec::new();
    let mut counts = Counts::default();
    // The window renderer clears to WHITE, while this buffer starts
    // zeroed and therefore hides an uncovered pixel as black. Painting
    // the gaps a colour the theme never uses makes them impossible to
    // miss, and makes this image show the same defect the window does.
    let gaps = std::env::var_os("FTS_BENCH_SHOT_GAPS").is_some();
    image.render_to_vec(
        |painter| {
            painter.reset();
            if gaps {
                painter.fill(
                    vello::peniko::Fill::NonZero,
                    Affine::IDENTITY,
                    vello::peniko::color::palette::css::MAGENTA,
                    None,
                    &vello::kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
                );
            }
            let a = scene.replay_lanes(
                painter,
                view,
                Affine::translate((TCP_WIDTH - scroll_x, RULER_H - scroll_y))
                    * Affine::scale_non_uniform(PPS * zoom_x, zoom_y),
            );
            let b = scene.replay_panel(
                painter,
                view,
                Affine::translate((0.0, RULER_H - scroll_y))
                    * Affine::scale_non_uniform(1.0, zoom_y),
            );
            ruler::grid(
                painter,
                palette,
                view,
                Bars::at(scene.bpm),
                &adaptive_grid::Adaptive::default(),
                1.0 / 16.0,
            );
            ruler::ruler(painter, palette, font, view, Bars::at(scene.bpm));
            counts.replayed = a.replayed + b.replayed;
            counts.submitted = a.submitted + b.submitted;
        },
        &mut buffer,
    );

    image::save_buffer(out, &buffer, width, height, image::ColorType::Rgba8)
        .expect("write the frame");
    println!(
        "  wrote {} — scroll ({scroll_x:.0}, {scroll_y:.0}) zoom ({zoom_x:.2}, {zoom_y:.2}), \
         {} commands submitted",
        out.display(),
        counts.submitted,
    );
}

fn build_scene(palette: &Palette, layout: session_daw::layout::Layout) -> Option<Arrangement> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    let project = rt.block_on(daw_ui::studio::project::fetch())?;
    let project = daw_ui::studio::ProjectRef(std::sync::Arc::new(project));
    let (visible, depths) =
        daw_ui::components::folders::FolderState::default().visible(&project.tracks);
    let rows = daw_ui::studio::RowsRef(std::sync::Arc::new(
        visible.into_iter().zip(depths).collect(),
    ));
    Some(Arrangement::build(
        palette,
        &session_daw::text::Font::embedded().ok()?,
        &project,
        &rows,
        layout,
    ))
}
