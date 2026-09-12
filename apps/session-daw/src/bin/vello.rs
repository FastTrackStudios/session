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
    /// The tracks as they are NOW — what the live controls draw from.
    ///
    /// Kept beside the recorded mixer rather than inside it: the mixer
    /// records the chrome once and these change on every click, which
    /// is the whole reason the two are separate.
    tracks: Vec<daw_proto::Track>,
    /// What the pointer is on, and what it is doing to it.
    pointer: session_daw::pointer::Pointer,
    /// The same, for the arrangement's track panel. A separate state
    /// because the two surfaces have different controls and only one
    /// of them is on screen at a time.
    panel: session_daw::pointer::Pointer<session_daw::pointer::RowSpot>,
    /// Presses, drags and double-clicks, out of the raw events.
    gestures: session_daw::gesture::Gestures,
    /// Where the pointer is, since winit reports moves and clicks
    /// separately and a click carries no position.
    cursor: (f64, f64),
    /// Whether the fine-adjustment modifier is held.
    fine: bool,
    /// Edits on their way to the engine.
    applier: Option<session_daw::engine::Applier>,
    /// Where the transport is, polled off the event loop.
    transport: Option<session_daw::engine::Transport>,
    /// The play cursor, which glides between the transport's reports.
    playhead: session_daw::cursor::Playhead,
    /// The edit cursor and the time selection.
    edit: session_daw::cursor::Edit,
    /// Where a ruler drag started, in seconds.
    dragging_time: Option<f64>,
    /// The panel control the pointer went down on.
    pressed_row: Option<(usize, session_daw::row::Control)>,
    /// Where the last panel drag was measured from.
    row_drag: Option<(usize, session_daw::row::Control)>,
    /// The engine's own account of the tracks, as it changes.
    watch: Option<session_daw::engine::Watch>,
    /// An open rename, if a name is being edited.
    rename: Option<session_daw::rename::Rename>,
    /// When the last panel click was, for spotting a double.
    last_row_click: Option<(usize, std::time::Instant)>,
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
                // While a name is being edited the keyboard belongs to
                // it — `x` must type an x, not switch views.
                if self.rename.is_some() {
                    self.type_into_rename(&event);
                    self.redraw();
                    return;
                }
                if event.logical_key.to_text() == Some("x") {
                    self.view = self.view.toggled();
                    if let Some(window) = &self.window {
                        window.set_title(self.view.title());
                    }
                    tracing::info!(view = ?self.view, "view");
                    self.redraw();
                }
            }
            WindowEvent::PointerMoved { position, .. } => {
                self.cursor = (position.x, position.y);
                let spot = self.spot_at(position.x, position.y);
                // A drag outranks a hover: while the pointer is down on
                // a fader it is setting a level, not browsing.
                if let Some(from) = self.dragging_time {
                    if let Some(to) = self.ruler_time_unclamped(position.x) {
                        self.edit.drag(from, to);
                        self.redraw();
                        return;
                    }
                }
                if let Some((row, control)) = self.row_drag.or(self.pressed_row) {
                    if control.is_continuous() {
                        self.row_drag = Some((row, control));
                        let fine = self.fine;
                        let dy = position.y - self.cursor.1;
                        self.drag_row(row, control, dy, fine);
                        self.cursor = (position.x, position.y);
                        self.redraw();
                        return;
                    }
                }
                let over_row = self
                    .row_spot_at(position.x, position.y)
                    .map(|(row, control)| session_daw::pointer::RowSpot { row, control });
                if let Some(event) = self.gestures.moved(position.x, position.y, self.fine) {
                    self.act(event);
                    self.redraw();
                } else if self.panel.hover(over_row) | self.pointer.hover(spot) {
                    // Only when something actually changed — a pointer
                    // crossing a strip fires a move per pixel and
                    // changes control about twice.
                    self.redraw();
                }
            }
            WindowEvent::PointerLeft { .. } => {
                if self.panel.hover(None) | self.pointer.hover(None) {
                    self.redraw();
                }
            }
            WindowEvent::PointerButton {
                state,
                position,
                button,
                ..
            } => {
                if button.mouse_button() != Some(winit::event::MouseButton::Left) {
                    return;
                }
                // The event carries its own position, which is the one
                // the click actually happened at — the last move may
                // have been a pixel ago, or on another device.
                self.cursor = (position.x, position.y);
                let (x, y) = self.cursor;
                if state.is_pressed() {
                    if let Some(seconds) = self.ruler_time(x, y) {
                        // A press on the ruler moves the cursor at once
                        // — the click is what you meant, and waiting
                        // for the release to show it feels like a lag
                        // rather than like care.
                        self.dragging_time = Some(seconds);
                        self.edit.click(seconds);
                    }
                    self.pointer.hover(self.spot_at(x, y));
                    self.pointer.press();
                    self.pressed_row = self.row_spot_at(x, y);
                    self.panel.hover(
                        self.pressed_row
                            .map(|(row, control)| session_daw::pointer::RowSpot { row, control }),
                    );
                    self.panel.press();
                    if let Some(hit) = self.hit_at(x, y) {
                        self.gestures.press(hit, x, y);
                    }
                } else {
                    if let (Some(from), Some(to)) = (self.dragging_time.take(), self.ruler_time(x, y)) {
                        // A drag across the ruler is a time selection;
                        // a click is just the cursor. `Edit::drag`
                        // decides which, so a twitch does not leave a
                        // four-millisecond selection behind.
                        self.edit.drag(from, to);
                    }
                    // A click on a panel control acts if the pointer is
                    // still on the control it went down on — the same
                    // rule the mixer follows, so dragging away cancels.
                    if let (Some((row, control)), Some((now_row, now_control))) =
                        (self.pressed_row.take(), self.row_spot_at(x, y))
                    {
                        if row == now_row && control == now_control && !control.is_continuous() {
                            let now = std::time::Instant::now();
                            let double = control == session_daw::row::Control::Name
                                && self.last_row_click.is_some_and(|(last, when)| {
                                    last == row
                                        && now.saturating_duration_since(when)
                                            <= session_daw::gesture::DOUBLE
                                });
                            tracing::debug!(ui.row = row, ui.control = ?control, ui.double = double, "panel click");
                            if double {
                                // A double-click on a name edits it;
                                // the single click that preceded it
                                // selected the track, which is what you
                                // wanted on the way here anyway.
                                self.last_row_click = None;
                                if let Some(track) = self.tracks.get(row) {
                                    self.rename = Some(session_daw::rename::Rename::new(
                                        row,
                                        track.guid.clone(),
                                        &track.name,
                                    ));
                                }
                            } else {
                                self.last_row_click = Some((row, now));
                                self.act_on_row(row, control);
                            }
                        }
                    }
                    self.row_drag = None;
                    self.pointer.release();
                    self.panel.release();
                    if let Some(event) = self.gestures.release(x, y, std::time::Instant::now()) {
                        self.act(event);
                    }
                }
                self.redraw();
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
            self.tracks = loaded
                .rows
                .iter()
                .map(|(track, _)| track.clone())
                .collect();
            self.session = Some((loaded.project, loaded.rows));
            // The applier needs the facade, which only exists once the
            // project has been opened.
            if self.applier.is_none() {
                self.applier = session_daw::engine::Applier::start();
            }
            if self.transport.is_none() {
                self.transport = session_daw::engine::Transport::start();
            }
            if self.watch.is_none() {
                self.watch = session_daw::engine::Watch::start();
            }
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
        // The rails take their share before anything scrolls: the span
        // is how far the CONTENT can move inside them, not how far it
        // could move if it owned the window.
        let frame = session_daw::rails::Frame::new(self.surface_size.0, self.surface_size.1);
        let (width, height) = (frame.content_width(), frame.content_height());
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
    /// The panel row and control under a window point.
    ///
    /// The arrangement's counterpart to `spot_at`, and the same rule:
    /// the layout that answers this is the layout the row was drawn
    /// from.
    fn row_spot_at(&self, x: f64, y: f64) -> Option<(usize, session_daw::row::Control)> {
        if self.view != View::Arrangement {
            return None;
        }
        let scene = self.scene.as_ref()?;
        let panel_x = x - session_daw::rails::SIDE;
        if panel_x < 0.0 || panel_x >= TCP_WIDTH {
            return None;
        }
        let content_y =
            y - session_daw::rails::TOP - RULER_H + self.scroll_y;
        let index = scene.row_at(content_y)?;
        let (top, height) = scene.row_box(index)?;
        let (track, depth) = self.rows_at(index)?;
        let row = session_daw::row::Row::new(top, height, depth, track.is_folder);
        let control = row.control_at(panel_x, content_y)?;
        Some((index, control))
    }

    /// The track and depth of a visible row.
    fn rows_at(&self, index: usize) -> Option<(&daw_proto::Track, i32)> {
        let (_, rows) = self.session.as_ref()?;
        rows.get(index)
            .map(|(track, depth)| (track, i32::try_from(*depth).unwrap_or(0)))
    }

    /// The control under a window point, if the mixer is showing.
    fn spot_at(&self, x: f64, y: f64) -> Option<session_daw::pointer::Spot> {
        if self.view != View::Mixer {
            return None;
        }
        let mixer = self.mixer.as_ref()?;
        let content_x = x - session_daw::rails::SIDE + self.mixer_scroll;
        let row = mixer.strip_at(content_x)?;
        let (left, width, height) = mixer.strip_box(row)?;
        // The same layout the strip was drawn from — see `strip::Strip`.
        let control = session_daw::mcp::control_at(
            width,
            height,
            mixer.height,
            mixer.rack_h,
            mixer.buttons_top,
            content_x - left,
            y - session_daw::rails::TOP,
        )?;
        Some(session_daw::pointer::Spot { row, control })
    }

    /// The hit under a window point, for the gesture layer.
    fn hit_at(&self, x: f64, y: f64) -> Option<session_daw::hit::Hit> {
        let mixer = self.mixer.as_ref()?;
        Some(session_daw::hit::mixer(mixer, self.mixer_scroll, x, y))
    }

    /// The time under a point, if it is on the ruler.
    fn ruler_time(&self, x: f64, y: f64) -> Option<f64> {
        if self.view != View::Arrangement {
            return None;
        }
        let top = session_daw::rails::TOP;
        // The corner above the track panel is the mode selector, not
        // the ruler — the ruler measures the timeline, and the timeline
        // starts where the lanes do.
        let left = session_daw::rails::SIDE + session_daw::arrangement::TCP_WIDTH;
        (y >= top && y < top + session_daw::ruler::RULER_H && x >= left)
            .then(|| self.time_at(x))
            .flatten()
    }

    /// The time under an x, wherever the pointer is vertically — for
    /// continuing a drag that began on the ruler.
    fn ruler_time_unclamped(&self, x: f64) -> Option<f64> {
        (self.view == View::Arrangement).then(|| self.time_at(x)).flatten()
    }

    fn time_at(&self, x: f64) -> Option<f64> {
        if self.pps <= 0.0 {
            return None;
        }
        let content =
            x - session_daw::rails::SIDE - session_daw::arrangement::TCP_WIDTH + self.scroll_x;
        Some((content / self.pps).max(0.0))
    }

    /// One key, into an open rename.
    fn type_into_rename(&mut self, event: &winit::event::KeyEvent) {
        use winit::keyboard::{Key, NamedKey};
        let Some(rename) = self.rename.as_mut() else {
            return;
        };
        match &event.logical_key {
            Key::Named(NamedKey::Enter) => {
                let Some(rename) = self.rename.take() else {
                    return;
                };
                let Some(name) = rename.commit().map(str::to_owned) else {
                    // An empty name is refused, and refusing it by
                    // cancelling is kinder than leaving the field open
                    // with no way to tell why Enter did nothing.
                    return;
                };
                let edit = session_daw::engine::Edit::Rename(rename.guid, name);
                apply_locally(&mut self.tracks, rename.row, &edit);
                // The name is recorded chrome, not a live value, so the
                // prediction only shows once the panel is re-recorded.
                self.re_record();
                if let Some(applier) = &self.applier {
                    applier.send(edit);
                }
            }
            Key::Named(NamedKey::Escape) => {
                self.rename = None;
            }
            Key::Named(NamedKey::Backspace) => rename.backspace(),
            Key::Named(NamedKey::Delete) => rename.delete(),
            Key::Named(NamedKey::ArrowLeft) => rename.left(),
            Key::Named(NamedKey::ArrowRight) => rename.right(),
            Key::Named(NamedKey::Home) => rename.home(),
            Key::Named(NamedKey::End) => rename.end(),
            key => {
                if let Some(text) = key.to_text() {
                    for c in text.chars() {
                        rename.insert(c);
                    }
                }
            }
        }
    }

    /// A click on a panel control.
    fn act_on_row(&mut self, row: usize, control: session_daw::row::Control) {
        use session_daw::row::Control as C;
        let Some(track) = self.tracks.get(row) else {
            return;
        };
        let guid = track.guid.clone();
        let edit = match control {
            C::Mute => Some(session_daw::engine::Edit::ToggleMute(guid)),
            C::Solo => Some(session_daw::engine::Edit::ToggleSolo(guid)),
            C::RecArm => Some(session_daw::engine::Edit::ToggleArm(guid)),
            C::Name => Some(session_daw::engine::Edit::Select(guid)),
            // Folding, routing and the FX chain are not edits to a
            // track — they are edits to the VIEW and to models this
            // window has not read yet.
            C::Folder | C::Routing | C::Fx | C::Volume | C::Pan => None,
        };
        let Some(edit) = edit else { return };
        apply_locally(&mut self.tracks, row, &edit);
        if let Some(applier) = &self.applier {
            applier.send(edit);
        }
    }

    /// A drag on a panel knob.
    fn drag_row(&mut self, row: usize, control: session_daw::row::Control, dy: f64, fine: bool) {
        use session_daw::row::Control as C;
        let Some(track) = self.tracks.get(row) else {
            return;
        };
        let guid = track.guid.clone();
        // A knob's notional travel: REAPER's own, so a full sweep takes
        // about the same movement here as it does there.
        const KNOB_TRAVEL: f64 = 150.0;
        let scaled = if fine {
            dy * session_daw::gesture::FINE
        } else {
            dy
        };
        let fraction = session_daw::gesture::drag_fraction(scaled, KNOB_TRAVEL);
        let mapped = match control {
            C::Volume => session_daw::engine::drag(
                session_daw::mcp::Control::Volume,
                &guid,
                track,
                fraction,
            ),
            C::Pan => session_daw::engine::drag(
                session_daw::mcp::Control::Pan,
                &guid,
                track,
                fraction,
            ),
            _ => None,
        };
        let Some(edit) = mapped else { return };
        apply_locally(&mut self.tracks, row, &edit);
        if let Some(applier) = &self.applier {
            applier.send(edit);
        }
    }

    /// Do what a gesture meant.
    ///
    /// The edit goes to the engine AND is applied here, because the
    /// engine is in-process but not instant, and a fader that waited
    /// for a round trip would lag the hand moving it. The local copy is
    /// a prediction of what the engine will say; when the event stream
    /// is wired it becomes a correction instead.
    fn act(&mut self, event: session_daw::gesture::Event) {
        use session_daw::gesture::Event;
        use session_daw::hit::Target;

        let (hit, drag) = match event {
            Event::Click(hit) | Event::DoubleClick(hit) => (hit, None),
            Event::Drag { hit, delta } => (hit, Some(delta)),
            Event::DragEnd(_) => return,
        };
        let Target::Track { row } = hit.target else {
            return;
        };
        let Some(spot) = self.pointer.hovered().filter(|s| s.row == row).or_else(|| {
            // Mid-drag the pointer may have left the control; the
            // gesture still belongs to what it started on.
            self.pointer.active().map(|(spot, _)| spot)
        }) else {
            return;
        };
        let Some(track) = self.tracks.get(row) else {
            return;
        };
        let guid = track.guid.clone();

        let edit = if let Some((_, dy)) = drag {
            let travel = self
                .mixer
                .as_ref()
                .and_then(|m| m.strip_box(row))
                .map_or(1.0, |(_, _, h)| h * 0.4);
            let fraction = session_daw::gesture::drag_fraction(dy, travel);
            session_daw::engine::drag(spot.control, &guid, track, fraction)
        } else {
            session_daw::engine::click(spot.control, &guid)
        };
        let Some(edit) = edit else { return };

        apply_locally(&mut self.tracks, row, &edit);
        if let Some(applier) = &self.applier {
            applier.send(edit);
        }
    }

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
        let frame = session_daw::rails::Frame::new(width, height);
        if !self.mixer_for(frame.content_height()) {
            return;
        }
        let profile = session_daw::rails::profile(
            session_daw::rails::Surface::Mixer,
            session::modes::Mode::Mix,
            session::mix_phases::MixPhase::Tone,
            "Mix",
            session_daw::settings::Settings::default(),
        );

        // Split the borrow: `render` takes the renderer mutably and
        // everything drawn inside it is read.
        let Self {
            renderer,
            mixer,
            tracks,
            pointer,
            palette,
            font,
            ..
        } = self;
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
            let at = Affine::translate((
                session_daw::rails::SIDE - scroll,
                session_daw::rails::TOP,
            ));
            // The recorded chrome.
            let a = mixer.replay(painter, scroll, frame.content_width(), at);
            // Then every control whose value can change, from the
            // tracks as they are now — which is what makes a click show
            // up without the mixer being re-recorded.
            let b = session_daw::overlay::controls(
                painter,
                palette,
                font,
                mixer,
                tracks,
                pointer,
                scroll,
                frame.content_width(),
                at,
            );
            drawn.replayed = a.replayed + b.replayed;
            drawn.submitted = a.submitted + b.submitted;
            // The rails last, over everything that scrolled under them.
            session_daw::rails::draw(
                painter,
                palette,
                font,
                frame,
                &profile.left,
                &profile.right,
                &profile.top,
            );
        });
        self.after_frame(drawn);
    }

    /// Take the engine's word for what the tracks are.
    ///
    /// The window predicts an edit so the hand does not wait for a
    /// round trip; this is where the prediction is replaced by the
    /// truth. A frame that finds no events does nothing, which is most
    /// of them.
    fn reconcile(&mut self) {
        let Some(watch) = &self.watch else { return };
        let events: Vec<_> = watch.drain().collect();
        if events.is_empty() {
            return;
        }
        // A control the hand is still on keeps the hand's value. The
        // engine will agree in a moment; snapping to its last word
        // mid-drag is a fader that fights back, and the hand is the
        // more recent authority on a value it is still setting.
        let dragging = self.row_drag.and_then(|(row, _)| {
            self.tracks.get(row).map(|track| track.guid.clone())
        });
        let mut renamed = false;
        for event in &events {
            if let Some(held) = &dragging
                && session_daw::engine::continuous_for(event).is_some_and(|guid| guid == held)
            {
                continue;
            }
            renamed |= matches!(event, daw_proto::track::TrackEvent::Renamed { .. });
            session_daw::engine::apply_event(&mut self.tracks, event);
        }
        if renamed {
            self.re_record();
        }
    }

    /// Re-record the panels, because something the RECORDING holds has
    /// changed.
    ///
    /// Almost every edit this window makes is a live value — a fader, a
    /// mute, a meter — and the overlay pass redraws those over the
    /// recorded chrome for the cost of one control. A NAME is not: it
    /// is text baked into the recorded panel, and no overlay can change
    /// it without redrawing the whole plate on every frame forever.
    ///
    /// So a rename is the one edit that pays for a re-record. It costs
    /// about four milliseconds across sixty rows, once, when a name
    /// actually changes — which is the side of the trade the recorded
    /// scene exists to be on.
    fn re_record(&mut self) {
        let Some((project, rows)) = self.session.clone() else {
            return;
        };
        // The recording reads names from the ROWS, so the rows have to
        // carry what the tracks now say. The two are index-aligned:
        // `self.tracks` was built from these rows, in order.
        let mut next = rows.as_slice().to_vec();
        for (row, track) in next.iter_mut().zip(self.tracks.iter()) {
            row.0.name.clone_from(&track.name);
        }
        let rows = daw_ui::studio::RowsRef(std::sync::Arc::new(next));
        self.scene = Some(Arrangement::build(
            &self.palette,
            &self.font,
            &project,
            &rows,
            self.layout,
        ));
        // The mixer is recorded lazily against the window's height, so
        // dropping it rebuilds on the next mixer frame rather than
        // paying now for a surface nobody is looking at.
        self.mixer = None;
        self.session = Some((project, rows));
    }

    fn redraw(&mut self) {
        if !self.renderer.is_active() {
            return;
        }
        // The engine's corrections, before anything is drawn from the
        // window's copy — and before the scene is borrowed, because a
        // rename makes this rebuild it.
        self.reconcile();
        if self.view == View::Mixer {
            self.redraw_mixer();
            return;
        }
        let Some(scene) = &self.scene else { return };
        let (sx, sy, pps) = (self.scroll_x, self.scroll_y, self.pps);
        let (width, surface_h) = self.surface_size;
        let frame = session_daw::rails::Frame::new(width, surface_h);
        // What this frame can see. Everything outside it is skipped
        // before it reaches Vello's encoder — see `Arrangement::index`.
        // The rails are excluded, or the panel draws rows behind them
        // and pays for every one.
        let view = Viewport {
            scroll_x: sx,
            scroll_y: sy,
            pps,
            zoom_y: 1.0,
            width: frame.content_width(),
            height: frame.content_height(),
        };
        let surface = self.palette.surface;
        let palette = &self.palette;
        let font = &self.font;
        let bars = Bars::at(scene.bpm);
        let rows: &[(daw_proto::Track, u32)] =
            self.session.as_ref().map_or(&[], |(_, rows)| rows.as_slice());
        let tracks = self.tracks.as_slice();
        let rename = self.rename.as_ref();
        let panel = &self.panel;
        // The transport's last word, and where that puts the cursor
        // NOW — the reading is per-block, the drawing is per-frame, and
        // the difference between them is the glide.
        if let Some(transport) = &self.transport {
            let (at, playing) = transport.read();
            let now = std::time::Instant::now();
            if playing != self.playhead.playing() {
                self.playhead.set_playing(playing, 1.0, now);
            }
            self.playhead.report(at, 1.0, now);
        }
        let play_at = self.playhead.at_time(std::time::Instant::now());
        let edit = self.edit;
        let rail = (session_daw::rails::SIDE, session_daw::rails::TOP);
        let profile = session_daw::rails::profile(
            session_daw::rails::Surface::Arrange,
            session::modes::Mode::Mix,
            session::mix_phases::MixPhase::Tone,
            "Mix",
            session_daw::settings::Settings::default(),
        );

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
                &vello::kurbo::Rect::new(0.0, 0.0, width, surface_h),
            );
            // The lanes: scrolled both ways, and scaled horizontally by
            // the zoom. Recorded at one pixel per second, so the scale IS
            // the zoom — no rebuild, no re-record.
            let a = scene.replay_lanes(
                painter,
                view,
                Affine::translate((rail.0 + TCP_WIDTH - sx, rail.1 + RULER_H - sy))
                    * Affine::scale_non_uniform(pps, 1.0),
            );
            // The panel: the SAME vertical offset, which is the entire
            // point. It cannot drift from the lanes because there is
            // nothing to drift — one number moves both.
            let b = scene.replay_panel(
                painter,
                view,
                Affine::translate((rail.0, rail.1 + RULER_H - sy)),
            );
            // After the lanes — their backgrounds are opaque — and the
            // ruler last of all, over everything scrolled under it.
            ruler::grid(painter, &palette, view, bars, &grid, FINEST, rail);
            // An open rename, over the name it replaces.
            if let Some(open) = rename {
                if let Some((top, height)) = scene.row_box(open.row) {
                    let depth = rows
                        .get(open.row)
                        .map_or(0, |(_, d)| i32::try_from(*d).unwrap_or(0));
                    let is_folder = rows.get(open.row).is_some_and(|(t, _)| t.is_folder);
                    let row = session_daw::row::Row::new(top, height, depth, is_folder);
                    if let Some(field) = row.rect(session_daw::row::Control::Name) {
                        session_daw::rename::paint(
                            painter,
                            &palette,
                            &font,
                            open,
                            field,
                            Affine::translate((rail.0, rail.1 + RULER_H - sy)),
                        );
                    }
                }
            }

            // The panel's live values, over its recorded chrome.
            let c = session_daw::overlay::panel_controls(
                painter,
                &palette,
                &font,
                scene,
                rows,
                tracks,
                view,
                panel,
                Affine::translate((rail.0, rail.1 + RULER_H - sy)),
            );
            ruler::ruler(painter, &palette, &font, view, bars, rail);
            // The cursors last, over the lanes and under nothing: a
            // playhead behind an item is a playhead you cannot follow.
            let top = rail.1 + RULER_H;
            let bottom = rail.1 + view.height;
            session_daw::cursor::paint_edit(
                painter, &palette, &edit, view, rail, top, bottom,
            );
            let x = play_at.mul_add(
                view.pps,
                rail.0 + TCP_WIDTH - view.scroll_x,
            );
            session_daw::cursor::paint(
                painter,
                session_daw::cursor::Look::default(),
                x,
                top,
                bottom,
                rail.0 + TCP_WIDTH,
            );
            // The rails over everything that scrolled under them, and
            // the mode selector in the corner the ruler leaves.
            session_daw::rails::draw(
                painter,
                &palette,
                &font,
                frame,
                &profile.left,
                &profile.right,
                &profile.top,
            );
            session_daw::rails::main_toolbar(painter, &palette, &font, session::modes::Mode::Mix);
            drawn.replayed = a.replayed + b.replayed + c.replayed;
            drawn.submitted = a.submitted + b.submitted + c.submitted;
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
        tracks: Vec::new(),
        pointer: session_daw::pointer::Pointer::default(),
        panel: session_daw::pointer::Pointer::default(),
        gestures: session_daw::gesture::Gestures::default(),
        cursor: (0.0, 0.0),
        fine: false,
        applier: session_daw::engine::Applier::start(),
        transport: session_daw::engine::Transport::start(),
        playhead: session_daw::cursor::Playhead::stopped(0.0),
        edit: session_daw::cursor::Edit::default(),
        dragging_time: None,
        pressed_row: None,
        row_drag: None,
        watch: session_daw::engine::Watch::start(),
        rename: None,
        last_row_click: None,
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

/// Show an edit here, now, rather than waiting for the engine.
///
/// The engine is in-process but not instant, and a fader that waited
/// for a round trip would lag the hand moving it. This is a PREDICTION
/// of what the engine will say; once its event stream is subscribed it
/// becomes a correction instead, and the prediction being wrong stops
/// mattering because the truth arrives a frame later.
fn apply_locally(tracks: &mut [daw_proto::Track], row: usize, edit: &session_daw::engine::Edit) {
    use session_daw::engine::Edit;
    let Some(track) = tracks.get_mut(row) else {
        return;
    };
    match edit {
        Edit::ToggleMute(_) => track.muted = !track.muted,
        Edit::ToggleSolo(_) => track.soloed = !track.soloed,
        Edit::ToggleArm(_) => track.armed = !track.armed,
        Edit::SetVolume(_, v) => track.volume = *v,
        Edit::SetPan(_, p) => track.pan = *p,
        Edit::Rename(_, name) => track.name.clone_from(name),
        // Selection is the engine's to decide: it is exclusive, so
        // predicting it here would mean predicting which OTHER tracks
        // stop being selected. Getting that wrong looks worse than a
        // frame of lag looks slow.
        Edit::Select(_) => {}
    }
}
