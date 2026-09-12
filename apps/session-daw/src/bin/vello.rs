//! The arrangement on the GPU.
//!
//! ```sh
//! just daw-vello
//! ```
//!
//! `x` toggles between the arrangement and the mixer. One window, one
//! view: multi-window comes later, and a split would have to decide how
//! to divide the height before either half has earned it.
//!
//! A winit window, a wgpu surface, and the arrangement drawn through
//! `anyrender` by Vello. No DOM, no WebView, no canvas — one scene
//! containing the track panel AND the lanes, replayed under a transform
//! that carries the scroll and the zoom.
//!
//! # Why this exists beside the dioxus one
//!
//! The WebView studio is capped at ~62 fps by WebKit's own
//! `requestAnimationFrame` throttle (measured: the same ceiling in plain
//! MiniBrowser, so it is the engine, not us). Vello presents at the
//! surface's real rate — `PresentMode::AutoVsync` — which on this
//! machine's displays is 144 to 240 Hz.
//!
//! It also fixes, structurally, the tearing that a canvas overlay could
//! not: the panel and the lanes are one scene under one transform, so
//! they cannot be a frame apart.

use std::sync::Arc;

use anyrender::{PaintScene, WindowRenderer};
use anyrender_vello::VelloWindowRenderer;
use vello::kurbo::Affine;
use winit::application::ApplicationHandler;
use winit::event::{MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, WindowHandle,
};
use winit::window::{Window, WindowAttributes, WindowId};

/// Hands winit's window to anyrender.
///
/// `anyrender::WindowHandle` is a blanket trait over
/// `HasWindowHandle + HasDisplayHandle`, and winit's `dyn Window` cannot
/// be coerced to it directly — one trait object does not become another.
/// A concrete newtype can implement both and then coerce, which is all
/// this is.
struct Surface(Arc<dyn Window>);

impl HasWindowHandle for Surface {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.0.window_handle()
    }
}

impl HasDisplayHandle for Surface {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.0.display_handle()
    }
}

use session_daw::arrangement::{Arrangement, Palette, Viewport, TCP_WIDTH};
use session_daw::ruler::{self, Bars, RULER_H};
use session_daw::{open, theme};

/// Pixels per second at rest.
const DEFAULT_PPS: f64 = 40.0;

struct App {
    window: Option<Arc<dyn Window>>,
    renderer: VelloWindowRenderer,
    /// Built when the project lands; `None` until then.
    scene: Option<Arrangement>,
    palette: Palette,
    /// Both in PIXELS of content, at the current zoom.
    ///
    /// Not seconds. The wheel handler, the autoscroll and the draw used
    /// to disagree about this — the draw multiplied by `pps` while the
    /// other two had already done so — which sent the sweep forty times
    /// past the end of the session, into a region with no lanes in it.
    /// The window renderer clears to white, so that is exactly what you
    /// saw. One unit, named here, for all three.
    scroll_x: f64,
    scroll_y: f64,
    pps: f64,
    /// The surface size, tracked so the draw can cull to it. Culling is
    /// the difference between encoding 30,000 commands a frame and 367;
    /// it needs to know how much fits on screen, and the surface is the
    /// only thing that knows.
    surface_size: (f64, f64),
    /// The font the panel and the ruler print with. Loaded once: every
    /// label in the window is laid out against it.
    font: session_daw::text::Font,
    /// The grid that follows the zoom.
    grid: adaptive_grid::Adaptive,
    /// The project loads on a worker thread so the window opens now.
    loading: Option<std::sync::mpsc::Receiver<Loaded>>,
    /// Which view `x` last left the window on.
    view: View,
    /// The session, kept so the mixer can be re-recorded at whatever
    /// height the window is. `None` until the project lands.
    session: Option<(daw_ui::studio::ProjectRef, daw_ui::studio::RowsRef)>,
    /// The mixer, recorded for the height it was last drawn at.
    mixer: Option<session_daw::mcp::Mixer>,
    /// The row layout, kept for that re-record.
    layout: session_daw::layout::Layout,
    /// Whether the Tone rack is on.
    tone: bool,
    /// How far along the strips the mixer is scrolled. Its own axis:
    /// the arrangement's horizontal scroll is in seconds of timeline and
    /// the mixer's is in strips, and sharing one number would mean a
    /// zoom moved the mixer.
    mixer_scroll: f64,
    /// Frames presented since the last report, and when that was.
    ///
    /// Counted HERE, at the point a frame is actually handed to the
    /// surface — not renders, not redraw requests. The WebView studio
    /// had to ask the page for this through `requestAnimationFrame`;
    /// owning the loop means simply counting.
    frames: u32,
    last_report: std::time::Instant,
    worst_frame: std::time::Duration,
    last_frame: std::time::Instant,
    started: std::time::Instant,
    autoscroll: bool,
}

impl ApplicationHandler for App {
    // winit 0.31's required hook: the point at which a surface may be
    // created. `resumed` is no longer the one to hang window creation on.
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        tracing::info!("can_create_surfaces");
        if self.window.is_some() {
            return;
        }
        let attrs = WindowAttributes::default()
            .with_title(self.view.title())
            .with_surface_size(winit::dpi::LogicalSize::new(1600.0, 900.0));
        let window: Arc<dyn Window> = Arc::from(
            event_loop.create_window(attrs).expect("create window"),
        );
        let size = window.surface_size();
        self.surface_size = (f64::from(size.width), f64::from(size.height));
        let surface: Arc<dyn anyrender::WindowHandle> = Arc::new(Surface(window.clone()));
        self.renderer
            .resume(surface, size.width.max(1), size.height.max(1), || {});
        // NOT assumed to finish here. `VelloWindowRenderer` brings the
        // wgpu device up asynchronously, so the first `complete_resume`
        // returns false and the renderer is not yet active — see
        // `about_to_wait`, which keeps asking.
        self.renderer.complete_resume();
        window.request_redraw();
        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        _id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::SurfaceResized(size) => {
                self.surface_size = (f64::from(size.width), f64::from(size.height));
                self.renderer.set_size(size.width.max(1), size.height.max(1));
                self.redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // Lines or pixels depending on the device; treating both
                // as pixels is how a trackpad ends up feeling wrong.
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (f64::from(x) * 53.0, f64::from(y) * 53.0),
                    MouseScrollDelta::PixelDelta(p) => (p.x, p.y),
                };
                if self.view == View::Mixer {
                    // The mixer has one axis. Either wheel direction
                    // moves along the strips, because a mixer scrolled
                    // vertically by a wheel that meant "sideways" is the
                    // most common way to lose your place in one.
                    self.scroll_mixer(dx + dy);
                } else {
                    self.scroll_to(self.scroll_x - dx, self.scroll_y - dy);
                }
                self.redraw();
            }
            // `x` between the arrangement and the mixer — REAPER's own
            // key for it, and the whole of the window's navigation until
            // there is more than one window to put them in.
            WindowEvent::KeyboardInput { event, .. } if event.state.is_pressed() => {
                if event.logical_key.to_text() == Some("x") {
                    self.view = self.view.toggled();
                    if let Some(window) = &self.window {
                        window.set_title(self.view.title());
                    }
                    tracing::info!(view = ?self.view, "view");
                    self.redraw();
                }
            }
            WindowEvent::RedrawRequested => self.redraw(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &dyn ActiveEventLoop) {
        // A sweep the window drives itself, for measuring. The same
        // triangle wave the WebView studio's `autoscroll` uses, so the
        // two rates are comparable — and a direction change is where a
        // renderer that rebuilds anything per frame shows it.
        if self.autoscroll && self.scene.is_some() {
            // The same two-axis sweep the headless bench runs, so what
            // you watch and what it measures are the same gesture. Fast
            // on purpose: a slow pan hides the stalls a real drag finds.
            let (span_x, span_y) = self.spans();
            // A yank, not a drift: the whole session top to bottom in a
            // third of a second, which is what grabbing the scrollbar and
            // throwing it looks like. A slow pan hides the stalls a real
            // gesture finds.
            let secs = self.last_frame.duration_since(self.started).as_secs_f64();
            let tri = |t: f64| if t < 1.0 { t } else { 2.0 - t };
            self.scroll_to(
                span_x * tri((secs / 0.5) % 2.0),
                span_y * tri((secs / 0.35) % 2.0),
            );
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        // Finish the resume the moment the device is ready.
        //
        // This is the whole liveness of the window. `redraw` bails when
        // the renderer is not active, and an early return does not ask
        // for another frame — so without this the window is created,
        // never draws, and never even maps. It presents as a process
        // sitting idle with its Vulkan threads asleep, which is exactly
        // how it presented.
        if !self.renderer.is_active() && self.renderer.complete_resume() {
            tracing::info!("renderer active");
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
        // The project arriving is the one thing that rebuilds the scene.
        if let Some(rx) = &self.loading
            && let Ok(loaded) = rx.try_recv()
        {
            self.scene = Some(loaded.arrangement);
            self.session = Some((loaded.project, loaded.rows));
            self.loading = None;
            self.redraw();
        }
    }
}

impl App {
    /// How far the content can scroll before it runs out, in pixels.
    ///
    /// Measured against the SURFACE, not against the size the window was
    /// asked for at creation: those differ the moment anyone resizes it,
    /// and scrolling past the end shows the renderer's white clear
    /// colour rather than the session.
    fn spans(&self) -> (f64, f64) {
        let Some(scene) = &self.scene else {
            return (1.0, 1.0);
        };
        let (width, height) = self.surface_size;
        (
            (scene.length_secs * self.pps - (width - TCP_WIDTH)).max(1.0),
            // The ruler takes a strip off the top, so there is that much
            // more to scroll before the last row reaches the bottom.
            (scene.content_height() - (height - RULER_H)).max(1.0),
        )
    }

    /// Move the view, clamped to the session.
    ///
    /// The single place scroll position is written, so the wheel and the
    /// autoscroll cannot end up with different ideas about the bounds or
    /// the units.
    fn scroll_to(&mut self, x: f64, y: f64) {
        let (span_x, span_y) = self.spans();
        self.scroll_x = x.clamp(0.0, span_x);
        self.scroll_y = y.clamp(0.0, span_y);
    }

    /// The mixer, recorded for a panel `height` tall.
    ///
    /// Re-recorded when the height changes, which is a toggle or a
    /// resize — not a frame. A strip resolves its sections against the
    /// height it is DRAWN at, so a mixer recorded for one height and
    /// replayed at another has its fader, its buttons and its rack in
    /// the wrong places; scaling it would make the controls the wrong
    /// size, which is the thing the whole panel is built not to do.
    /// Move along the strips, stopping at both ends.
    ///
    /// Clamped to the content rather than left free: a mixer scrolled
    /// past its last strip is a blank screen with no cue about which
    /// way to go back, and the arrangement has a ruler and a playhead
    /// to orient by where this has nothing.
    fn scroll_mixer(&mut self, by: f64) {
        let width = self.surface_size.0;
        let content = self.mixer.as_ref().map_or(0.0, session_daw::mcp::Mixer::content_width);
        let most = (content - width).max(0.0);
        self.mixer_scroll = (self.mixer_scroll - by).clamp(0.0, most);
    }

    fn mixer_for(&mut self, height: f64) -> bool {
        let Some((project, rows)) = self.session.as_ref() else {
            return false;
        };
        let stale = self
            .mixer
            .as_ref()
            .is_none_or(|m| (m.height - height).abs() > 0.5);
        if stale {
            self.mixer = Some(session_daw::mcp::Mixer::build(
                &self.palette,
                &self.font,
                project,
                rows,
                height,
                self.layout,
                self.tone,
            ));
        }
        self.mixer.is_some()
    }

    fn redraw_mixer(&mut self) {
        let (width, height) = self.surface_size;
        let surface = self.palette.surface;
        let scroll = self.mixer_scroll;
        // The panel is REAPER's own height, not the window's, and it
        // sits on the bottom edge.
        //
        // A strip stretches to whatever height it is given, and given a
        // 1440-pixel window that is a nine-hundred-pixel fader — travel
        // nobody wants and precision nobody asked for. REAPER's mixer
        // fills its panel, but nobody docks that panel to a whole 1440p
        // screen, so "fills the window" is the wrong reading of it.
        let panel = session_daw::mcp::DEFAULT_HEIGHT.min(height);
        if !self.mixer_for(panel) {
            return;
        }
        let dock = (height - self.mixer.as_ref().map_or(0.0, |m| m.height)).max(0.0);
        // Split the borrow: the renderer is taken mutably by `render`
        // and the mixer is only read inside it.
        let Self { renderer, mixer, .. } = self;
        let Some(mixer) = mixer.as_ref() else { return };
        let mut drawn = session_daw::profile::Counts::default();
        renderer.render(|painter| {
            painter.reset();
            painter.fill(
                vello::peniko::Fill::NonZero,
                Affine::IDENTITY,
                surface,
                None,
                &vello::kurbo::Rect::new(0.0, 0.0, width, height),
            );
            drawn = mixer.replay(painter, scroll, width, Affine::translate((-scroll, dock)));
        });
        self.after_frame(drawn);
    }

    fn redraw(&mut self) {
        if !self.renderer.is_active() {
            return;
        }
        if self.view == View::Mixer {
            self.redraw_mixer();
            return;
        }
        let Some(scene) = &self.scene else { return };
        let (sx, sy, pps) = (self.scroll_x, self.scroll_y, self.pps);
        let (width, height) = self.surface_size;
        // What this frame can see. Everything outside it is skipped
        // before it reaches Vello's encoder — see `Arrangement::index`.
        let view = Viewport {
            scroll_x: sx,
            scroll_y: sy,
            pps,
            zoom_y: 1.0,
            width,
            height,
        };
        let surface = self.palette.surface;
        let palette = &self.palette;
        let font = &self.font;
        let bars = Bars::at(scene.bpm);

        let grid = &self.grid;
        /// The finest the grid ever gets — sixteenths, as a fraction of
        /// a whole note. The zoom only ever coarsens away from it.
        const FINEST: f64 = 1.0 / 16.0;
        let mut drawn = session_daw::profile::Counts::default();
        self.renderer.render(|painter| {
            painter.reset();
            // The theme's surface, under everything.
            //
            // `VelloWindowRenderer` clears to WHITE, so any pixel the
            // arrangement does not cover is not merely undrawn, it is
            // bright white on a dark theme — during a fast scroll, at
            // the end of the session, or in the gap under the last row.
            // One rectangle makes the window's background the theme's
            // instead of the renderer's.
            painter.fill(
                vello::peniko::Fill::NonZero,
                Affine::IDENTITY,
                surface,
                None,
                &vello::kurbo::Rect::new(0.0, 0.0, width, height),
            );
            // The lanes: scrolled both ways, and scaled horizontally by
            // the zoom. Recorded at one pixel per second, so the scale IS
            // the zoom — no rebuild, no re-record.
            let a = scene.replay_lanes(
                painter,
                view,
                Affine::translate((TCP_WIDTH - sx, RULER_H - sy))
                    * Affine::scale_non_uniform(pps, 1.0),
            );
            // The panel: the SAME vertical offset, which is the entire
            // point. It cannot drift from the lanes because there is
            // nothing to drift — one number moves both.
            let b = scene.replay_panel(painter, view, Affine::translate((0.0, RULER_H - sy)));
            // After the lanes — their backgrounds are opaque — and the
            // ruler last of all, over everything scrolled under it.
            ruler::grid(painter, &palette, view, bars, &grid, FINEST);
            ruler::ruler(painter, &palette, &font, view, bars);
            drawn.replayed = a.replayed + b.replayed;
            drawn.submitted = a.submitted + b.submitted;
        });
        self.after_frame(drawn);
    }

    /// The per-frame bookkeeping both views share.
    ///
    /// The rate, twice a second, the same shape the WebView studio
    /// reports so the two are comparable: the worst frame in the window
    /// beside the mean, because a scroll that stutters averages
    /// beautifully and feels terrible.
    fn after_frame(&mut self, drawn: session_daw::profile::Counts) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
        let now = std::time::Instant::now();
        self.worst_frame = self.worst_frame.max(now - self.last_frame);
        self.last_frame = now;
        self.frames += 1;
        let elapsed = now - self.last_report;
        if elapsed >= std::time::Duration::from_millis(500) {
            // The counts ride along with the rate. A frame rate on its
            // own cannot tell "fast because the scene is culled well"
            // from "fast because half the screen is not being drawn",
            // and those look identical in a log and nothing alike on a
            // monitor.
            tracing::info!(
                ui.view = ?self.view,
                ui.fps = f64::from(self.frames) / elapsed.as_secs_f64(),
                ui.worst_frame_ms = self.worst_frame.as_secs_f64() * 1000.0,
                ui.surface = ?self.surface_size,
                ui.scroll = ?(self.scroll_x, self.scroll_y),
                ui.submitted = drawn.submitted,
                "frame rate"
            );
            self.frames = 0;
            self.worst_frame = std::time::Duration::ZERO;
            self.last_report = now;
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,session_daw=info".into()),
        )
        .init();

    let Some(path) = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("SESSION_DAW_PROJECT").ok())
        .map(std::path::PathBuf::from)
        .filter(|p| p.exists())
    else {
        eprintln!("session-daw vello needs a project: cargo run --bin vello -- <song.rpp>");
        std::process::exit(2);
    };

    let theme = theme::resolve().theme;
    let palette = Palette::from_theme(&theme);
    let layout = session_daw::layout::Layout::from_env();

    // The window opens now; the project fills in behind it.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("session-daw-load".into())
        .spawn(move || {
            tracing::info!("loader thread started");
            match open::open_and_serve(&path) {
                Ok(opened) => tracing::info!(
                    project.name = opened.name,
                    project.tracks = opened.track_count,
                    "project open"
                ),
                Err(e) => {
                    tracing::error!(error = %e, "the project did not open");
                    return;
                }
            }
            // The facade is up; read it and record the scene.
            match build_scene(&theme, layout) {
                Some(loaded) => {
                    tracing::info!(rows = loaded.arrangement.rows, "session recorded");
                    let _ = tx.send(loaded);
                }
                None => tracing::error!("could not read the project back"),
            }
        })
        .expect("spawn loader");
    tracing::info!("loader spawned; building event loop");

    let event_loop = EventLoop::new().expect("event loop");
    tracing::info!("event loop built; running");
    let app = App {
        window: None,
        renderer: VelloWindowRenderer::new(),
        scene: None,
        // The RESOLVED theme, not a fresh default. Building a palette
        // from `Theme::dark()` here is how the window quietly stopped
        // using the user's REAPER theme once already.
        palette,
        scroll_x: 0.0,
        scroll_y: 0.0,
        pps: DEFAULT_PPS,
        // Replaced the moment the surface exists; until then it culls to
        // nothing, which is correct — there is no surface to draw on.
        surface_size: (0.0, 0.0),
        font: match session_daw::text::Font::embedded() {
            Ok(font) => font,
            Err(e) => {
                tracing::error!(error = %e, "the embedded font did not load");
                std::process::exit(1);
            }
        },
        grid: adaptive_grid::Adaptive::default(),
        loading: Some(rx),
        view: View::Arrangement,
        session: None,
        mixer: None,
        mixer_scroll: 0.0,
        layout,
        // On by default: the rack is what this panel is being built
        // for, and a flag you have to remember is a feature nobody sees.
        tone: std::env::var_os("FTS_VELLO_NO_TONE").is_none(),
        frames: 0,
        last_report: std::time::Instant::now(),
        worst_frame: std::time::Duration::ZERO,
        last_frame: std::time::Instant::now(),
        started: std::time::Instant::now(),
        autoscroll: std::env::var_os("FTS_VELLO_AUTOSCROLL").is_some(),
    };
    event_loop.run_app(app).expect("run");
}

/// Read the project through the facade and record it.
fn build_scene(
    theme: &daw_ui::theming::Theme,
    layout: session_daw::layout::Layout,
) -> Option<Loaded> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    let project = rt.block_on(daw_ui::studio::project::fetch())?;
    let project = daw_ui::studio::ProjectRef(std::sync::Arc::new(project));
    let (visible, depths) = daw_ui::components::folders::FolderState::default().visible(&project.tracks);
    let rows = daw_ui::studio::RowsRef(std::sync::Arc::new(
        visible.into_iter().zip(depths).collect(),
    ));
    let palette = Palette::from_theme(theme);
    let font = session_daw::text::Font::embedded().ok()?;
    // The project and its rows come back with the arrangement, because
    // the mixer is recorded against the WINDOW's height and that is not
    // known here — see `App::mixer_for`.
    Some(Loaded {
        arrangement: Arrangement::build(&palette, &font, &project, &rows, layout),
        project,
        rows,
    })
}

/// What the loader thread hands back.
struct Loaded {
    arrangement: Arrangement,
    project: daw_ui::studio::ProjectRef,
    rows: daw_ui::studio::RowsRef,
}

/// Which of the two the window is showing.
///
/// One window, one view, `x` between them — REAPER's own key for it.
/// Multi-window comes later; a single window that can reach both is
/// what makes the mixer usable at all today, and a split would have to
/// decide how to divide the height before either half has earned it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Arrangement,
    Mixer,
}

impl View {
    const fn toggled(self) -> Self {
        match self {
            Self::Arrangement => Self::Mixer,
            Self::Mixer => Self::Arrangement,
        }
    }

    const fn title(self) -> &'static str {
        match self {
            Self::Arrangement => "Session — arrangement (Vello)",
            Self::Mixer => "Session — mixer (Vello)",
        }
    }
}
