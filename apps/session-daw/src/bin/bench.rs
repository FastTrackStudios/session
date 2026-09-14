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
use session_daw::mcp::Mixer;

/// The phase these measurements are taken in — the one the rack was
/// built for, and the one every reference image was shot in.
const TONE: session::mix_phases::MixPhase = session::mix_phases::MixPhase::Tone;
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
    if std::env::var_os("FTS_BENCH_DEPTHS").is_some() {
        depths();
        return;
    }
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

    if std::env::var_os("FTS_BENCH_ANIMATE").is_some() {
        animate(&palette, &font, layout, width, height);
        return;
    }

    if let Ok(out) = std::env::var("FTS_BENCH_MIXER") {
        mixer_shot(&palette, &font, layout, &std::path::PathBuf::from(out), width, height);
        return;
    }

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
                        ruler::grid(painter, &palette, view, bars, &grid, FINEST, (0.0, 0.0));
                        ruler::ruler(painter, &palette, &font, view, bars, (0.0, 0.0));
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

/// Write one mixer frame to a PNG.
///
/// `FTS_BENCH_MIXER=/tmp/mixer.png`, with `FTS_BENCH_SCROLL=x,y` to move
/// along the strips.
fn mixer_shot(
    palette: &Palette,
    font: &session_daw::text::Font,
    layout: session_daw::layout::Layout,
    out: &std::path::Path,
    width: u32,
    height: u32,
) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let Some(project) = rt.block_on(daw_ui::studio::project::fetch()) else {
        eprintln!("could not read the project back");
        return;
    };
    let project = daw_ui::studio::ProjectRef(std::sync::Arc::new(project));
    let (visible, depths) =
        daw_ui::components::folders::FolderState::default().visible(&project.tracks);
    let mut planned: Vec<(daw_proto::Track, u32)> = visible.into_iter().zip(depths).collect();
    // A scene, if one is asked for: the visual track manager's answer
    // to which strips and how wide — `FTS_BENCH_SCENE=lead-vocal-fx`.
    let scene = std::env::var("FTS_BENCH_SCENE").ok().and_then(|slug| session_daw::plan::scene(&slug));
    if let Some(scene) = scene {
        planned = session_daw::plan::apply_scene(
            &planned,
            scene,
            session_daw::settings::Settings::default(),
            f64::from(height),
        );
    }
    let tracks: Vec<daw_proto::Track> = planned.iter().map(|(t, _)| t.clone()).collect();
    let rows = daw_ui::studio::RowsRef(std::sync::Arc::new(planned));

    let scroll_x = std::env::var("FTS_BENCH_SCROLL")
        .ok()
        .and_then(|v| v.split(',').next()?.trim().parse().ok())
        .unwrap_or(0.0);

    // The Tone rack, which is a mix sub-mode rather than a permanent
    // fixture — so the shot asks for it explicitly. A scene is a
    // picture of the racks, so it asks for it too — and a picture of
    // racks with nothing moving through them is a picture of half of
    // what they are, so a scene runs the simulation: the meters lit,
    // the compressors reducing, the de-essers firing, the tails
    // filled. `FTS_BENCH_LIVE=1` asks for the same without a scene.
    let tone = std::env::var("FTS_BENCH_TONE").is_ok() || scene.is_some();
    let live = std::env::var("FTS_BENCH_LIVE").is_ok() || scene.is_some();
    // The whole frame, so a shot matches the window.
    //
    // The mixer splits internally — the REAPER strip takes a third off
    // the bottom, the rack fills the rest — so handing it the frame is
    // handing it the same thing the window hands it. Passing a short
    // height here is how to shoot the DOCKED look instead.
    let mcp_height = std::env::var("FTS_BENCH_MCP_HEIGHT")
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|h| *h > 0.0)
        .unwrap_or_else(|| f64::from(height))
        .min(f64::from(height));
    // The rack's settings, seeded from the placeholder — see
    // `tone::Store`. The shot and the stress test both want the same
    // racks the window draws.
    let mut settings = session_daw::tone::Store::default();
    settings.seed(rows.as_slice());
    // The reverb tails, rendered before a frame is measured: a render
    // landing between two passes would make them differ, and the
    // verify run compares them byte for byte.
    settings.prerender_tails();
    let frame = session_daw::rails::Frame::new(f64::from(width), f64::from(height));
    let mixer = Mixer::build(
        palette,
        font,
        &project,
        &rows,
        mcp_height - session_daw::rails::TOP,
        layout,
        // The shot is of the Tone phase, which is the phase the rack
        // was built for and the one the reference images were taken in.
        if tone { session_daw::tone::panels_for(TONE) } else { &[] },
        live,
        session_daw::settings::Settings::default(),
        &settings,
    );
    // The signal, when the shot is live: seventy-two meter frames of
    // history per track ending at one instant, the way the window has
    // them after two and a half seconds of playback.
    let mut history: std::collections::HashMap<String, session_daw::tone::Levels> =
        std::collections::HashMap::new();
    let mut spectra: std::collections::HashMap<String, session_daw::tone::Analyser> =
        std::collections::HashMap::new();
    let mut levels: Vec<daw_proto::TrackLevels> = Vec::new();
    if live {
        const AT: f64 = 2.5;
        for (i, track) in tracks.iter().enumerate() {
            let Some(tone) = settings.get(&track.guid) else {
                continue;
            };
            let entry = history.entry(track.guid.clone()).or_default();
            let mut last = None;
            for k in 0..session_daw::tone::HISTORY {
                let t = AT - (session_daw::tone::HISTORY - 1 - k) as f64 / f64::from(session_daw::tone::PUBLISH_HZ);
                let meters = session_daw::simulate::meters(i, t, tone);
                entry.push(meters.sat_peak);
                entry.push_fire(meters.deess_deepest());
                entry.push_ess(meters.ess_db, meters.ess_ref_db);
                last = Some(meters);
            }
            if let Some(meters) = last {
                let peak = meters.sat_peak;
                if let Ok(index) = usize::try_from(track.index) {
                    if levels.len() <= index {
                        levels.resize(index + 1, daw_proto::TrackLevels::default());
                    }
                    levels[index] = daw_proto::TrackLevels {
                        peak_left: peak,
                        peak_right: peak * 0.85,
                        hold_left: peak,
                        hold_right: peak,
                    };
                }
                spectra.entry(track.guid.clone()).or_default().set(meters);
            }
        }
    }
    let mut racks = if live {
        session_daw::overlay::Racks {
            settings: &settings,
            history: &mut history,
            spectra: &mut spectra,
            lit: None,
            panels: session_daw::tone::panels_for(TONE),
            folded: Box::leak(Box::new(session_daw::tone::Fold::rest())),
        }
    } else {
        session_daw::overlay::Racks::none()
    };
    // The bench applies no preset, so the map is the identity — built
    // rather than skipped so the shot exercises the same lookup the
    // window does.
    let map = session_daw::plan::Rows::of(rows.as_slice(), &tracks);

    let mut image = VelloImageRenderer::new(width, height);
    let mut buffer = Vec::new();
    let mut counts = Counts::default();
    image.render_to_vec(
        |painter| {
            painter.reset();
            painter.fill(
                vello::peniko::Fill::NonZero,
                Affine::IDENTITY,
                palette.surface,
                None,
                &vello::kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
            );
            // Pinned to the BOTTOM of the window.
            //

            let at = Affine::translate((
                session_daw::rails::SIDE - scroll_x,
                session_daw::rails::TOP,
            ));
            let recorded = mixer.replay(painter, scroll_x, frame.content_width(), at);
            // The live controls, exactly as the window draws them — a
            // shot that skipped them would be a shot of a mixer with no
            // faders, and the whole point of the shot is to be the same
            // frame the window shows.
            let live = session_daw::overlay::controls(
                painter,
                palette,
                font,
                &mixer,
                &tracks,
                &map,
                &session_daw::pointer::Pointer::default(),
                // At rest unless live: the still shot is the reference
                // every viewport in the sweep is compared against, and
                // a lit meter in it would be a difference nobody asked
                // for. A scene is live.
                &levels,
                // Nothing has clipped in a still frame, and a latch in
                // the reference image would be a difference nobody
                // asked for.
                &session_daw::overlay::Clips::default(),
                // At the top: a shot is the reference every viewport is
                // compared against, and a scrolled rack in it would be
                // a difference nobody asked for.
                0.0,
                &mut racks,
                scroll_x,
                frame.content_width(),
                at,
            );
            counts.replayed = recorded.replayed + live.replayed;
            counts.submitted = recorded.submitted + live.submitted;
            let profile = session_daw::rails::profile(
                session_daw::rails::Surface::Mixer,
                session::modes::Mode::Mix,
                session::mix_phases::MixPhase::Tone,
                "Mix",
                session_daw::settings::Settings::default(),
            );
            session_daw::rails::draw(
                painter,
                palette,
                font,
                // Deliberately none: the shot is the reference the
                // sweep compares against, and it must not depend on
                // whether this machine has REAPER's resources on disk.
                &mut session_daw::icons::Icons::none(),
                // At rest, like every other control in the shot.
                (None, None),
                frame,
                &profile.left,
                &profile.right,
                &profile.top,
            );
        },
        &mut buffer,
    );
    image::save_buffer(out, &buffer, width, height, image::ColorType::Rgba8)
        .expect("write the frame");
    println!(
        // The content width is reported because opening a strip must
        // not change it: an opened strip borrows from the others. Two
        // shots of the same project, one with a selection and one
        // without, must print the same number.
        "  wrote {} — {} strips, {} deep, {:.0}px wide, {} commands submitted",
        out.display(),
        mixer.count,
        mixer.depth,
        mixer.content_width(),
        counts.submitted,
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

    // The arrangement is laid out inside the rails, the same as the
    // mixer — the panel has to know what it has or it draws rows under
    // the right rail and pays for every one.
    let frame = session_daw::rails::Frame::new(f64::from(width), f64::from(height));
    let view = Viewport {
        scroll_x,
        scroll_y,
        pps: PPS * zoom_x,
        zoom_y,
        width: frame.content_width(),
        height: frame.content_height(),
    };
    let rail_x = session_daw::rails::SIDE;
    let rail_y = session_daw::rails::TOP;
    let mut image = VelloImageRenderer::new(width, height);
    let mut buffer = Vec::new();
    let mut counts = Counts::default();
    // The window renderer clears to WHITE, while this buffer starts
    // zeroed and therefore hides an uncovered pixel as black. Painting
    // the gaps a colour the theme never uses makes them impossible to
    // miss, and makes this image show the same defect the window does.
    let gaps = std::env::var_os("FTS_BENCH_SHOT_GAPS").is_some();
    // The panel's live values need the rows they belong to.
    let (rows_for_panel, tracks_for_panel) = panel_rows();
    let panel_map = session_daw::plan::Rows::of(&rows_for_panel, &tracks_for_panel);
    image.render_to_vec(
        |painter| {
            painter.reset();
            // The theme's surface under everything.
            //
            // This buffer starts zeroed, so any pixel nothing covers
            // reads as transparent — which is why laying the panel
            // inside the rails put a white band across the top the
            // moment the ruler stopped starting at y=0. The mixer shot
            // has always painted one; this one relied on the lanes
            // covering the frame, which was true only by accident.
            painter.fill(
                vello::peniko::Fill::NonZero,
                Affine::IDENTITY,
                palette.surface,
                None,
                &vello::kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
            );
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
                Affine::translate((
                    rail_x + TCP_WIDTH - scroll_x,
                    rail_y + RULER_H - scroll_y,
                )) * Affine::scale_non_uniform(PPS * zoom_x, zoom_y),
            );
            session_daw::arrangement::titles(
                painter,
                palette,
                font,
                scene,
                view,
                (rail_x + TCP_WIDTH - scroll_x, rail_y + RULER_H - scroll_y),
            );
            let b = scene.replay_panel(
                painter,
                view,
                Affine::translate((rail_x, rail_y + RULER_H - scroll_y))
                    * Affine::scale_non_uniform(1.0, zoom_y),
            );
            ruler::grid(
                painter,
                palette,
                view,
                Bars::at(scene.bpm),
                &adaptive_grid::Adaptive::default(),
                1.0 / 16.0,
                (rail_x, rail_y),
            );
            let c = session_daw::overlay::panel_controls(
                painter,
                palette,
                font,
                scene,
                &rows_for_panel,
                &tracks_for_panel,
                &panel_map,
                view,
                // At rest: this is the reference shot every viewport in
                // the sweep is compared against, and a hover in it
                // would be a difference nobody asked for.
                &session_daw::pointer::Pointer::default(),
                Affine::translate((rail_x, rail_y + RULER_H - scroll_y)),
            );
            ruler::ruler(painter, palette, font, view, Bars::at(scene.bpm), (rail_x, rail_y));
            // The arrangement's left rail carries the same visual
            // presets the mixer's does — they are layouts of the
            // SESSION, not of one panel, so switching one switches
            // both. Its right rail is empty until the arrangement has
            // settings of its own worth switching.
            let profile = session_daw::rails::profile(
                session_daw::rails::Surface::Arrange,
                session::modes::Mode::Mix,
                session::mix_phases::MixPhase::Tone,
                "Mix",
                session_daw::settings::Settings::default(),
            );
            session_daw::rails::draw(
                painter,
                palette,
                font,
                // Deliberately none: the shot is the reference the
                // sweep compares against, and it must not depend on
                // whether this machine has REAPER's resources on disk.
                &mut session_daw::icons::Icons::none(),
                // At rest, like every other control in the shot.
                (None, None),
                frame,
                &profile.left,
                &profile.right,
                &profile.top,
            );
            // The mode selector sits in the corner the ruler leaves
            // above the track panel — the one piece of chrome the mode
            // does not re-populate.
            session_daw::rails::main_toolbar(
                painter,
                palette,
                font,
                &mut session_daw::icons::Icons::none(),
                (None, None),
                session::modes::Mode::Mix,
            );
            counts.replayed = a.replayed + b.replayed + c.replayed;
            counts.submitted = a.submitted + b.submitted + c.submitted;
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

/// Print the first rows' folder depths, to check nesting against the
/// project rather than against the picture.
fn depths() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let Some(project) = rt.block_on(daw_ui::studio::project::fetch()) else {
        return;
    };
    let project = daw_ui::studio::ProjectRef(std::sync::Arc::new(project));
    let (visible, depths) =
        daw_ui::components::folders::FolderState::default().visible(&project.tracks);
    for (track, depth) in visible.iter().zip(&depths).take(40) {
        println!(
            "  {:<10} depth {depth}  folder_depth {:>2}  is_folder {}",
            track.name, track.folder_depth, track.is_folder
        );
    }
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

/// Every parameter on every track, moving, measured.
///
/// `FTS_BENCH_ANIMATE=1`. The mixer's controls are drawn live so a mute
/// can change without the mixer being re-recorded; this is the frame
/// that says whether that is actually cheap.
///
/// Nothing scrolls. The question is not "can it draw a moving mixer",
/// it is "what does a still mixer cost when every control in it is
/// changing" — and mixing a scroll into that would hide the answer
/// under the cost of culling.
fn animate(
    palette: &Palette,
    font: &session_daw::text::Font,
    layout: session_daw::layout::Layout,
    width: u32,
    height: u32,
) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let Some(project) = rt.block_on(daw_ui::studio::project::fetch()) else {
        eprintln!("could not read the project back");
        return;
    };
    let project = daw_ui::studio::ProjectRef(std::sync::Arc::new(project));
    let (visible, depths) =
        daw_ui::components::folders::FolderState::default().visible(&project.tracks);
    let mut tracks: Vec<daw_proto::Track> = visible.clone();
    let rows = daw_ui::studio::RowsRef(std::sync::Arc::new(
        visible.into_iter().zip(depths).collect(),
    ));

    // The rack's settings, seeded from the placeholder — see
    // `tone::Store`. The shot and the stress test both want the same
    // racks the window draws.
    let mut settings = session_daw::tone::Store::default();
    settings.seed(rows.as_slice());
    // The reverb tails, rendered before a frame is measured: a render
    // landing between two passes would make them differ, and the
    // verify run compares them byte for byte.
    settings.prerender_tails();
    let frame = session_daw::rails::Frame::new(f64::from(width), f64::from(height));
    let mixer = Mixer::build(
        palette,
        font,
        &project,
        &rows,
        f64::from(height) - session_daw::rails::TOP,
        layout,
        session_daw::tone::panels_for(TONE),
        // The racks are driven here, so they are drawn live and the
        // recording reserves their space without filling it — which is
        // what the window does the moment anything feeds a spectrum.
        true,
        session_daw::settings::Settings::default(),
        &settings,
    );
    let map = session_daw::plan::Rows::of(rows.as_slice(), &tracks);
    let mut history: std::collections::HashMap<String, session_daw::tone::Levels> =
        std::collections::HashMap::new();
    // The analyser's bins per track — what makes a rack MOVE, and so
    // what makes it live rather than replayed. Driven here, because a
    // stress test of a mixer that shows audio has to show audio.
    let mut spectra: std::collections::HashMap<String, session_daw::tone::Analyser> =
        std::collections::HashMap::new();
    let pointer = session_daw::pointer::Pointer::default();

    let mut renderer = Headless::new(width, height).expect("a headless renderer");
    let mut stages = Stages::with_capacity(FRAMES);
    let mut counts = Counts::default();
    for batch in 0..FRAMES / BATCH {
        let batch_start = Instant::now();
        let mut painted = 0.0;
        for step in 0..BATCH {
            let frame_index = batch * BATCH + step;
            let t = frame_index as f64 / FRAMES as f64;
            // The session's state for this instant, the same on every
            // run — see `animate::drive`.
            session_daw::animate::drive(&mut tracks, t);
            // Meters arrive on their own subscription, so they are
            // driven separately — and per frame, which is faster than
            // the engine's pump will ever publish them.
            let levels = session_daw::animate::meters(tracks.len(), t);
            // And the compressor's display, at the rate the ENGINE
            // publishes meter frames — about 30 Hz — rather than at the
            // frame rate.
            //
            // Everything else here is driven per frame on purpose,
            // because a stress test should measure a case that cannot
            // happen. This one would measure a case that cannot happen
            // in the other direction: levels arriving faster than they
            // are drawn, which would defeat the trace's cache and
            // report a cost no session can produce. The draw still
            // happens every frame either way.
            if frame_index % 8 == 0 {
                for (i, track) in tracks.iter().enumerate() {
                    let Some(tone) = settings.get(&track.guid) else {
                        continue;
                    };
                    // One simulation per track: the peak the history
                    // takes is the peak the meters carry.
                    let meters = session_daw::simulate::meters(i, t * 8.0, tone);
                    let entry = history.entry(track.guid.clone()).or_default();
                    entry.push(meters.sat_peak);
                    entry.push_fire(meters.deess_deepest());
                entry.push_ess(meters.ess_db, meters.ess_ref_db);
                    spectra
                        .entry(track.guid.clone())
                        .or_default()
                        .set(meters);
                }
            }
            let mut drawn = Counts::default();
            painted += renderer
                .frame(|painter| {
                    painter.reset();
                    painter.fill(
                        vello::peniko::Fill::NonZero,
                        Affine::IDENTITY,
                        palette.surface,
                        None,
                        &vello::kurbo::Rect::new(0.0, 0.0, f64::from(width), f64::from(height)),
                    );
                    let at = Affine::translate((
                        session_daw::rails::SIDE,
                        session_daw::rails::TOP,
                    ));
                    // The recorded chrome, then the live values over it.
                    let a = mixer.replay(painter, 0.0, frame.content_width(), at);
                    let b = session_daw::overlay::controls(
                        painter,
                        palette,
                        font,
                        &mixer,
                        &tracks,
                        &map,
                        &pointer,
                        &levels,
                        &session_daw::overlay::Clips::default(),
                        0.0,
                        &mut session_daw::overlay::Racks {
                            folded: &session_daw::tone::Fold::default(),
                            settings: &settings,
                            history: &mut history,
                            spectra: &mut spectra,
                            lit: None,
                            panels: session_daw::tone::panels_for(TONE),
                        },
                        0.0,
                        frame.content_width(),
                        at,
                    );
                    drawn.replayed = a.replayed + b.replayed;
                    drawn.submitted = a.submitted + b.submitted;
                })
                .expect("render a frame");
            counts = drawn;
        }
        renderer.wait().expect("the gpu to finish the batch");
        let per_frame = batch_start.elapsed().as_secs_f64() * 1000.0 / BATCH as f64;
        stages.frame.push_ms(per_frame);
        stages.paint.push_ms(painted / BATCH as f64);
    }
    // The first batch warms geometry no earlier frame touched — the
    // cost of starting, not of running.
    stages.frame.drop_warmup(1);
    stages.paint.drop_warmup(1);

    let frame = stages.frame.summary().expect("batches");
    let paint = stages.paint.summary().expect("batches");
    println!(
        "  {:<20} {:>7.2}ms {:>7.2}ms {:>7.2}ms {:>9.0}   {:>7.2} {:>7.2}",
        "every parameter",
        frame.mean,
        frame.p99,
        frame.worst,
        frame.fps(),
        paint.mean,
        (frame.mean - paint.mean).max(0.0),
    );
    println!(
        "\n  {} strips, {} commands submitted a frame — every mute, solo, arm,",
        mixer.count, counts.submitted
    );
    println!("  fader, pan and meter on every visible strip changing on every frame.");
}

/// The visible rows and their tracks, for the panel's live controls.
fn panel_rows() -> (Vec<(daw_proto::Track, u32)>, Vec<daw_proto::Track>) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let Some(project) = rt.block_on(daw_ui::studio::project::fetch()) else {
        return (Vec::new(), Vec::new());
    };
    let (visible, depths) =
        daw_ui::components::folders::FolderState::default().visible(&project.tracks);
    let tracks = visible.clone();
    (visible.into_iter().zip(depths).collect(), tracks)
}
