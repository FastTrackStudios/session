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
    /// Which live track each mixer strip is showing. A preset can hide
    /// a track, so a strip's index is not a track's — see
    /// `plan::Rows`.
    mixer_map: session_daw::plan::Rows,
    /// The same, for the arrangement's rows.
    arrange_map: session_daw::plan::Rows,
    /// The arrangement's rows AFTER the preset — what the recorded
    /// scene was built from, and what the panel overlay reads.
    arrange_rows: daw_ui::studio::RowsRef,
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
    ///
    /// REAPER's Ctrl, for the panels' own controls. The EQ graph has
    /// its own convention — Shift is fine there, and Alt and Cmd mean
    /// other things — so the rack reads `mods` instead. Two surfaces,
    /// two established sets of modifiers; collapsing them into one
    /// would make one of them wrong.
    fine: bool,
    /// Every modifier, for the rack — the EQ's interaction model wants
    /// all three and resolves the chords itself.
    mods: eq_ui::eq_graph_interaction::Mods,
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
    /// The rail button the pointer is over.
    hovered_rail: Option<session_daw::rails::Action>,
    /// The rail button the pointer went down on. A rail click acts on
    /// RELEASE over the same button, like every other button here —
    /// dragging off one is how you change your mind.
    pressed_rail: Option<session_daw::rails::Action>,
    /// Where the last panel drag was measured from.
    row_drag: Option<(usize, session_daw::row::Control)>,
    /// The engine's own account of the tracks, as it changes.
    watch: Option<session_daw::engine::Watch>,
    /// The engine's live meter levels, latest-wins.
    meters: Option<session_daw::engine::Meters>,
    /// Which DAW mode the window is in — what the corner selects.
    mode: session::modes::Mode,
    /// Which mix phase, which is what the left rail's lower half
    /// selects and what decides how much processing a strip shows.
    phase: session::mix_phases::MixPhase,
    /// Which visual preset is recalled — which tracks show, and how
    /// wide. The left rail's upper half.
    preset: &'static str,
    /// The right rail's switches.
    settings: session_daw::settings::Settings,
    /// An open rename, if a name is being edited.
    rename: Option<session_daw::rename::Rename>,
    /// When the last panel click was, for spotting a double.
    last_row_click: Option<(usize, std::time::Instant)>,
    /// The row layout, kept for that re-record.
    layout: session_daw::layout::Layout,
    /// When the last rack grip was clicked, for spotting a double.
    last_grip_click: Option<((usize, session_daw::tone::Grip), std::time::Instant)>,
    /// The rack grip the pointer is over, and the strip it is on.
    hovered_grip: Option<(usize, session_daw::tone::Grip)>,
    /// The rack grip the pointer is dragging, and the strip it is on.
    ///
    /// A rack is RECORDED, so an edited one has to be drawn live until
    /// the drag ends — see `redraw_mixer`. Recording the mixer on every
    /// pointer move would be four milliseconds a frame to change one
    /// curve.
    rack_drag: Option<(usize, session_daw::tone::Grip)>,
    /// Each track's analyser bins, keyed by GUID — what the EQ's
    /// spectrum shows. A track with one has a rack that moves.
    tone_spectra: std::collections::HashMap<String, session_daw::tone::Analyser>,
    /// Each track's recent input levels, keyed by GUID — what the
    /// compressor's display draws and what its threshold line is read
    /// against. Fed from the meter frames, which arrive at about 30 Hz.
    tone_levels: std::collections::HashMap<String, session_daw::tone::Levels>,
    /// The Tone settings every rack is drawn from. Seeded from the
    /// placeholder until a chain can be read — see `tone::Store`.
    tone_settings: session_daw::tone::Store,
    /// Which folders are collapsed. A view state, not a track state:
    /// it changes which rows exist rather than what any track is.
    folders: daw_ui::components::folders::FolderState,
    /// The REAPER toolbar icons the rails draw, decoded once.
    icons: session_daw::icons::Icons,
    /// The theme, kept for a RELOAD — a track added or removed changes
    /// the folder tree, and the only honest way to recompute it is to
    /// read the session back, which needs a theme to record against.
    theme: daw_ui::theming::Theme,
    /// Whether the Tone rack is on.
    tone: bool,
    /// When the last level was recorded, so a simulation publishes at
    /// the engine's rate rather than the window's.
    last_level: Option<std::time::Instant>,
    /// Whether a simulated signal is running through every channel.
    ///
    /// The meters, the EQ's spectrum and the compressor's waveform only
    /// move when audio does, and a template session has none. This
    /// drives them from `simulate::frame` so the panels that exist to
    /// show a signal can be looked at while they are built.
    simulate: bool,
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
                // A wheel over a rack grip adjusts it rather than
                // scrolling the mixer past it. That is what the wheel
                // does over a band in the editor, and a strip that
                // scrolled away instead would be the one place the
                // gesture did something else.
                if let Some((row, grip)) = self.hovered_grip {
                    self.wheel_rack(row, grip, dy);
                    self.mixer = None;
                    self.redraw();
                    return;
                }
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
                // Space plays and stops, Home returns — REAPER's own
                // keys, and the two that make the playhead this window
                // already draws mean something.
                use winit::keyboard::{Key, NamedKey};
                // A space is a character key, not a named one — winit
                // reports it as the text " ".
                if event.logical_key.to_text() == Some(" ") {
                    session_daw::engine::transport(session_daw::engine::Move::PlayStop, 0.0);
                    self.redraw();
                    return;
                }
                if event.logical_key == Key::Named(NamedKey::Home) {
                    session_daw::engine::transport(session_daw::engine::Move::Home, 0.0);
                    self.playhead.report(0.0, 1.0, std::time::Instant::now());
                    self.redraw();
                    return;
                }
                // `s` runs a simulated signal through every channel, so
                // the meters and the rack's displays can be seen
                // without a session that plays.
                if event.logical_key.to_text() == Some("s") {
                    self.simulate = !self.simulate;
                    // The recording holds racks when nothing is live
                    // and reserves their space when something is, so
                    // flipping this changes what was recorded.
                    self.mixer = None;
                    if !self.simulate {
                        self.tone_spectra.clear();
                        self.tone_levels.clear();
                        // The racks stop moving, so they go back into
                        // the recording — which is where a still rack
                        // belongs.
                        self.mixer = None;
                    }
                    tracing::info!(ui.simulate = self.simulate, "simulation");
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
            // The fine-adjustment modifier. Held, a drag moves a
            // quarter as far — REAPER's own Ctrl, and the difference
            // between setting a fader and setting it exactly.
            //
            // This was a field nothing ever wrote: every drag in the
            // window has been coarse because no event set it.
            WindowEvent::ModifiersChanged(modifiers) => {
                let state = modifiers.state();
                self.fine = state.control_key();
                self.mods = eq_ui::eq_graph_interaction::Mods::new(
                    state.alt_key(),
                    state.shift_key(),
                    // Ctrl on Linux, Command on macOS — the plugin's
                    // own docs write it as one key and its model takes
                    // it as one field. winit calls the Mac one `meta`.
                    state.control_key() || state.meta_key(),
                );
            }
            WindowEvent::PointerMoved { position, .. } => {
                // Where the pointer WAS, before this event moved it.
                //
                // A drag is a delta, and the delta is against the last
                // position — so it has to be read before `self.cursor`
                // is overwritten. Setting the cursor first made every
                // drag delta exactly zero, which is why the panel's
                // knobs did not turn.
                let last = self.cursor;
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
                if let Some((row, grip)) = self.rack_drag {
                    let (dx, dy) = (position.x - last.0, position.y - last.1);
                    let (dx, dy) = if self.fine {
                        (
                            dx * session_daw::gesture::FINE,
                            dy * session_daw::gesture::FINE,
                        )
                    } else {
                        (dx, dy)
                    };
                    self.drag_rack(row, grip, dx, dy);
                    self.redraw();
                    return;
                }
                if let Some((row, control)) = self.row_drag.or(self.pressed_row) {
                    if control.is_continuous() {
                        self.row_drag = Some((row, control));
                        let fine = self.fine;
                        let dy = position.y - last.1;
                        self.drag_row(row, control, dy, fine);
                        self.redraw();
                        return;
                    }
                }
                let over_grip = self.rack_grip_at(position.x, position.y);
                if over_grip != self.hovered_grip {
                    self.hovered_grip = over_grip;
                    self.redraw();
                }
                let over_rail = self.rail_action_at(position.x, position.y);
                if over_rail != self.hovered_rail {
                    self.hovered_rail = over_rail;
                    self.redraw();
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
                self.hovered_rail = None;
                self.hovered_grip = None;
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
                    // The rails are drawn over everything, so they are
                    // asked first: a click on a phase button must not
                    // also move the edit cursor behind it.
                    self.pressed_rail = self.rail_action_at(x, y);
                    if self.pressed_rail.is_some() {
                        self.redraw();
                        return;
                    }
                    if let Some(seconds) = self.ruler_time(x, y) {
                        // A press on the ruler moves the cursor at once
                        // — the click is what you meant, and waiting
                        // for the release to show it feels like a lag
                        // rather than like care.
                        self.dragging_time = Some(seconds);
                        self.edit.click(seconds);
                        // And the transport goes there, which is what
                        // clicking a ruler means in every DAW: the edit
                        // cursor and the play position are the same
                        // thing until a time selection separates them.
                        session_daw::engine::transport(
                            session_daw::engine::Move::Seek,
                            seconds,
                        );
                    }
                    // The rack is above the strip's controls and is
                    // drawn over them, so it is claimed first.
                    self.rack_drag = self.rack_grip_at(x, y);
                    tracing::debug!(ui.x = x, ui.y = y, ui.grip = ?self.rack_drag, "rack press");
                    if let Some((row, grip)) = self.rack_drag {
                        // A modified click is an action on the band —
                        // bypass it, cycle its shape — not the start of
                        // a drag. `dot_click` says whether the chord
                        // meant one.
                        if self.click_rack(row, grip) {
                            self.rack_drag = None;
                            self.mixer = None;
                            self.redraw();
                            return;
                        }
                        // A second click inside the double-click window
                        // puts the grip back to its default, which is
                        // what makes one safe to explore: the way back
                        // is a gesture rather than a remembered number.
                        let now = std::time::Instant::now();
                        let again = self.last_grip_click.is_some_and(|(last, when)| {
                            last == (row, grip)
                                && now.saturating_duration_since(when)
                                    <= session_daw::gesture::DOUBLE
                        });
                        if again {
                            self.last_grip_click = None;
                            self.reset_rack(row, grip);
                            self.rack_drag = None;
                            self.mixer = None;
                        } else {
                            self.last_grip_click = Some(((row, grip), now));
                        }
                        self.redraw();
                        return;
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
                    if self.rack_drag.take().is_some() {
                        // Back into the recording: a rack that is not
                        // being dragged is constant again, and the
                        // live pass exists for what is not.
                        self.mixer = None;
                        self.redraw();
                        return;
                    }
                    if let Some(pressed) = self.pressed_rail.take() {
                        if self.rail_action_at(x, y) == Some(pressed) {
                            self.act_on_rail(pressed);
                        }
                        self.redraw();
                        return;
                    }
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
                                if let Some(track) = self
                                    .arrange_map
                                    .index(row)
                                    .and_then(|i| self.tracks.get(i))
                                {
                                    self.rename = Some(session_daw::rename::Rename::new(
                                        session_daw::rename::Surface::Arrange,
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
            // EVERY track, not the visible ones: folding changes which
            // rows exist, and live values kept only for what is on
            // screen would be lost the moment a folder closed over
            // them. The panels reach these through `plan::Rows`.
            self.tracks = loaded.project.0.tracks.clone();
            self.arrange_map = session_daw::plan::Rows::of(loaded.planned.as_slice(), &self.tracks);
            self.arrange_rows = loaded.planned;
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
            if self.meters.is_none() {
                self.meters = session_daw::engine::Meters::start();
            }
            // The mixer was recorded against the old track list; the
            // next mixer frame records it against this one.
            self.mixer = None;
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
        self.arrange_rows
            .get(index)
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

    /// The rack grip under a window point, if the pointer is in a rack.
    ///
    /// Asked before the strip's own controls, because the rack sits
    /// above them and a click has to land on what it looks like it
    /// landed on.
    fn rack_grip_at(&self, x: f64, y: f64) -> Option<(usize, session_daw::tone::Grip)> {
        if self.view != View::Mixer {
            return None;
        }
        let mixer = self.mixer.as_ref()?;
        let content_x = x - session_daw::rails::SIDE + self.mixer_scroll;
        let row = mixer.strip_at(content_x)?;
        let (left, width, height) = mixer.strip_box(row)?;
        let strip = session_daw::strip::Strip::new(
            width,
            height,
            mixer.height,
            mixer.rack_h,
            mixer.buttons_top,
        );
        let rack = session_daw::tone::Panel::of(strip.rack_rect()?, left);
        let track = self.mixer_map.index(row).and_then(|i| self.tracks.get(i))?;
        let tone = self.tone_settings.get(&track.guid)?;
        session_daw::tone::grip_at(
            self.rack_panels(),
            tone,
            rack,
            content_x,
            y - session_daw::rails::TOP,
        )
        .map(|grip| (row, grip))
    }

    /// Which panels the rack is showing, from the phase.
    fn rack_panels(&self) -> &'static [session_daw::tone::Which] {
        if self.tone {
            session_daw::tone::panels_for(self.phase)
        } else {
            &[]
        }
    }

    /// Turn the wheel over a rack grip.
    fn wheel_rack(&mut self, row: usize, grip: session_daw::tone::Grip, delta_y: f64) {
        let mods = self.mods;
        let Some(guid) = self
            .mixer_map
            .index(row)
            .and_then(|i| self.tracks.get(i))
            .map(|track| track.guid.clone())
        else {
            return;
        };
        if let Some(tone) = self.tone_settings.edit(&guid) {
            session_daw::tone::wheel(tone, grip, mods, delta_y);
        }
    }

    /// A modified click on a band — bypass, or cycle its shape.
    ///
    /// Returns whether it did anything, so a plain click falls through
    /// to taking hold of the band instead.
    fn click_rack(&mut self, row: usize, grip: session_daw::tone::Grip) -> bool {
        let session_daw::tone::Grip::Band(index) = grip else {
            return false;
        };
        let mods = self.mods;
        let Some(guid) = self
            .mixer_map
            .index(row)
            .and_then(|i| self.tracks.get(i))
            .map(|track| track.guid.clone())
        else {
            return false;
        };
        self.tone_settings
            .edit(&guid)
            .is_some_and(|tone| session_daw::tone::dot_click(tone, index, mods))
    }

    /// Put a rack grip back to its default.
    fn reset_rack(&mut self, row: usize, grip: session_daw::tone::Grip) {
        let Some(guid) = self
            .mixer_map
            .index(row)
            .and_then(|i| self.tracks.get(i))
            .map(|track| track.guid.clone())
        else {
            return;
        };
        if let Some(tone) = self.tone_settings.edit(&guid) {
            session_daw::tone::reset(tone, grip);
        }
    }

    /// Move a rack grip by a pointer delta.
    fn drag_rack(&mut self, row: usize, grip: session_daw::tone::Grip, dx: f64, dy: f64) {
        let mods = self.mods;
        let Some(mixer) = self.mixer.as_ref() else {
            return;
        };
        let Some((left, width, height)) = mixer.strip_box(row) else {
            return;
        };
        let strip = session_daw::strip::Strip::new(
            width,
            height,
            mixer.height,
            mixer.rack_h,
            mixer.buttons_top,
        );
        let Some(rack) = strip.rack_rect().map(|r| session_daw::tone::Panel::of(r, left)) else {
            return;
        };
        let panels = self.rack_panels();
        let Some(guid) = self
            .mixer_map
            .index(row)
            .and_then(|i| self.tracks.get(i))
            .map(|track| track.guid.clone())
        else {
            return;
        };
        if let Some(tone) = self.tone_settings.edit(&guid) {
            session_daw::tone::drag(tone, grip, panels, rack, mods, dx, dy);
        }
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

    /// The viewport this frame sees — the one number both the drawing
    /// and the hit tests resolve against.
    fn viewport(&self) -> Viewport {
        let (width, height) = self.surface_size;
        let frame = session_daw::rails::Frame::new(width, height);
        Viewport {
            scroll_x: self.scroll_x,
            scroll_y: self.scroll_y,
            pps: self.pps,
            zoom_y: 1.0,
            width: frame.content_width(),
            height: frame.content_height(),
        }
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
                let map = match rename.surface {
                    session_daw::rename::Surface::Arrange => &self.arrange_map,
                    session_daw::rename::Surface::Mixer => &self.mixer_map,
                };
                if let Some(index) = map.index(rename.row) {
                    apply_locally(&mut self.tracks, index, &edit);
                }
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
        // Folding is an edit to the VIEW: it changes which rows exist,
        // not what any track is, so it goes to the folder state and
        // re-records rather than to the engine. Handled before the
        // mapping because it is the one control whose answer is a row
        // and not a track.
        if control == C::Folder {
            self.fold(row);
            return;
        }
        let Some(row) = self.arrange_map.index(row) else {
            return;
        };
        let Some(track) = self.tracks.get(row) else {
            return;
        };
        let guid = track.guid.clone();
        let edit = match control {
            C::Mute => Some(session_daw::engine::Edit::ToggleMute(guid)),
            C::Solo => Some(session_daw::engine::Edit::ToggleSolo(guid)),
            C::RecArm => Some(session_daw::engine::Edit::ToggleArm(guid)),
            C::Name => Some(if self.fine {
                session_daw::engine::Edit::AddToSelection(guid)
            } else {
                session_daw::engine::Edit::Select(guid)
            }),
            C::Phase => Some(session_daw::engine::Edit::SetPhase(
                guid,
                !track.phase_inverted,
            )),
            C::Routing => Some(session_daw::engine::Edit::SetParentSend(
                guid,
                !track.parent_send,
            )),
            // Folding is an edit to the VIEW, not to the track, so it
            // is handled before this — see `act_on_row`'s caller. The
            // FX button waits for a chain; `bin/chain-probe` says why.
            C::Folder | C::Fx | C::Volume | C::Pan => None,
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
        let Some(row) = self.arrange_map.index(row) else {
            return;
        };
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

        let (hit, drag, double) = match event {
            Event::Click(hit) => (hit, None, false),
            Event::DoubleClick(hit) => (hit, None, true),
            Event::Drag { hit, delta } => (hit, Some(delta), false),
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
        // The strip's own index, for the geometry; the track's, for the
        // value. A preset can hide a track, so the two differ.
        let Some(index) = self.mixer_map.index(row) else {
            return;
        };
        let Some(track) = self.tracks.get(index) else {
            return;
        };
        let guid = track.guid.clone();

        // A double-click on a strip's name plate edits it, the same
        // gesture and the same editor the track panel uses.
        if double && spot.control == session_daw::mcp::Control::Name {
            self.rename = Some(session_daw::rename::Rename::new(
                session_daw::rename::Surface::Mixer,
                row,
                guid,
                &track.name,
            ));
            return;
        }

        let edit = if let Some((_, dy)) = drag {
            let travel = self
                .mixer
                .as_ref()
                .and_then(|m| m.strip_box(row))
                .map_or(1.0, |(_, _, h)| h * 0.4);
            let fraction = session_daw::gesture::drag_fraction(dy, travel);
            session_daw::engine::drag(spot.control, &guid, track, fraction)
        } else {
            session_daw::engine::click(spot.control, &guid, track, self.fine)
        };
        let Some(edit) = edit else { return };

        apply_locally(&mut self.tracks, index, &edit);
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
            // The visual preset decides which strips exist and how wide
            // each one opens, so it is applied BEFORE the recording —
            // the mixer records what it is given and has never heard of
            // a preset.
            let planned = daw_ui::studio::RowsRef(std::sync::Arc::new(session_daw::plan::apply(
                rows.as_slice(),
                session_daw::plan::slug(self.preset).unwrap_or(self.preset),
                session_daw::plan::Surface::Mixer,
                self.settings,
                height,
            )));
            self.tone_settings.seed(planned.as_slice());
            self.mixer_map = session_daw::plan::Rows::of(planned.as_slice(), &self.tracks);
            self.mixer = Some(session_daw::mcp::Mixer::build(
                &self.palette,
                &self.font,
                project,
                &planned,
                height,
                self.layout,
                // Which processing the rack shows is the PHASE's
                // decision — the left rail's lower half — and the env
                // switch turns the whole rack off whatever it says.
                if self.tone {
                    session_daw::tone::panels_for(self.phase)
                } else {
                    &[]
                },
                // Every rack is drawn live while anything is feeding
                // them a spectrum — see `mcp::Mixer::build`.
                !self.tone_spectra.is_empty(),
                self.settings,
                &self.tone_settings,
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
            self.mode,
            self.phase,
            self.preset,
            self.settings,
        );
        let levels = self.meter_levels();
        let levels = levels.as_slice();
        let rail_at = (self.hovered_rail, self.pressed_rail);
        let rename = self
            .rename
            .as_ref()
            .filter(|r| r.surface == session_daw::rename::Surface::Mixer);
        let panels = self.rack_panels();
        // A rack is drawn live when it MOVES — when the pointer is on
        // one of its grips, or when the track has a spectrum. Both are
        // rare enough that the rest of the mixer stays replayed.
        let lit = self.rack_drag.or(self.hovered_grip);

        // Split the borrow: `render` takes the renderer mutably and
        // everything drawn inside it is read.
        let Self {
            renderer,
            mixer,
            tracks,
            mixer_map: map,
            tone_levels: history,
            tone_spectra: spectra,
            tone_settings: settings,
            pointer,
            palette,
            font,
            icons,
            ..
        } = self;
        let Some(mixer) = mixer.as_ref() else { return };
        let mut racks = session_daw::overlay::Racks {
            settings,
            history,
            spectra,
            lit,
            panels,
        };
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
                map,
                pointer,
                levels,
                &mut racks,
                scroll,
                frame.content_width(),
                at,
            );
            // An open rename, over the plate it replaces.
            if let Some(open) = rename {
                if let Some((left, strip_w, strip_h)) = mixer.strip_box(open.row) {
                    let strip = session_daw::strip::Strip::new(
                        strip_w,
                        strip_h,
                        mixer.height,
                        mixer.rack_h,
                        mixer.buttons_top,
                    );
                    if let Some(field) = strip.rect(session_daw::mcp::Control::Name) {
                        session_daw::rename::paint(
                            painter,
                            palette,
                            font,
                            open,
                            field + vello::kurbo::Vec2::new(left, 0.0),
                            at,
                        );
                    }
                }
            }
            drawn.replayed = a.replayed + b.replayed;
            drawn.submitted = a.submitted + b.submitted;
            // The rails last, over everything that scrolled under them.
            session_daw::rails::draw(
                painter,
                palette,
                font,
                icons,
                rail_at,
                frame,
                &profile.left,
                &profile.right,
                &profile.top,
            );
        });
        self.after_frame(drawn);
    }

    /// The rails, as they are for the view now on screen.
    ///
    /// Built rather than stored: it is a handful of `Item`s and it has
    /// to agree with what was drawn, so deriving it from the same four
    /// values the drawing used is cheaper than keeping it in step.
    fn profile(&self) -> session_daw::rails::Profile {
        session_daw::rails::profile(
            match self.view {
                View::Arrangement => session_daw::rails::Surface::Arrange,
                View::Mixer => session_daw::rails::Surface::Mixer,
            },
            self.mode,
            self.phase,
            self.preset,
            self.settings,
        )
    }

    /// What a click at this point would do in the rails, if anything.
    ///
    /// Answered from the same `Profile` the rails were drawn from, so a
    /// button cannot act as the one beside it: the action is carried by
    /// the item, not looked up by index in a second table.
    fn rail_action_at(&self, x: f64, y: f64) -> Option<session_daw::rails::Action> {
        use session_daw::hit::{Side, Target};
        let (width, height) = self.surface_size;
        let frame = session_daw::rails::Frame::new(width, height);
        let profile = self.profile();
        // The corner above the track panel is the mode selector, and it
        // is drawn over the ruler, so it is asked first.
        if self.view == View::Arrangement
            && let Some(action) = self.mode_action_at(x, y)
        {
            return Some(action);
        }
        let hit = session_daw::hit::rails(frame, profile.left.len(), profile.right.len(), x, y)?;
        let Target::Rail { side, index } = hit.target else {
            return None;
        };
        match side {
            Side::Left => profile.left.get(index),
            Side::Right => profile.right.get(index),
            Side::Top => profile.top.get(index),
        }
        .map(|item| item.act)
    }

    /// The mode under a point in the corner above the track panel.
    fn mode_action_at(&self, x: f64, y: f64) -> Option<session_daw::rails::Action> {
        let modes = session::modes::Mode::ALL;
        let scene = self.scene.as_ref()?;
        let view = self.viewport();
        let hit = session_daw::hit::arrangement(scene, view, modes.len(), x, y);
        match hit.target {
            session_daw::hit::Target::Mode(index) => {
                modes.get(index).copied().map(session_daw::rails::Action::Mode)
            }
            _ => None,
        }
    }

    /// Do what a rail button says.
    ///
    /// Every arm ends in dropping something recorded, because that is
    /// what these buttons change: a preset decides which strips exist
    /// and how wide, a phase decides how much processing each one
    /// shows, and the settings decide what a selection costs its
    /// neighbours. None of them is a live value an overlay can redraw.
    fn act_on_rail(&mut self, action: session_daw::rails::Action) {
        use session_daw::rails::Action as A;
        match action {
            A::Preset(name) => {
                if self.preset == name {
                    return;
                }
                self.preset = name;
                self.re_record();
            }
            A::Phase(phase) => {
                if self.phase == phase {
                    return;
                }
                self.phase = phase;
                // The phase does not change the layout yet — it changes
                // which processing a rack shows, and the rack is drawn
                // from a placeholder. Dropping the mixer is still right:
                // when it does, this is where it happens.
                self.mixer = None;
            }
            A::Mode(mode) => {
                if self.mode == mode {
                    return;
                }
                self.mode = mode;
            }
            A::FocusSelected => {
                self.settings.focus_selected = !self.settings.focus_selected;
                self.mixer = None;
            }
            A::TakeFocusWidth => {
                self.settings.take_focus_width = !self.settings.take_focus_width;
                self.mixer = None;
            }
        }
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
            self.arrange_map
                .index(row)
                .and_then(|i| self.tracks.get(i))
                .map(|track| track.guid.clone())
        });
        let mut renamed = false;
        let mut listed = false;
        for event in &events {
            if let Some(held) = &dragging
                && session_daw::engine::continuous_for(event).is_some_and(|guid| guid == held)
            {
                continue;
            }
            // A name and a SELECTION are both recorded: the name is
            // text in the panel, and the selection decides how wide a
            // strip opens — which is the whole of "focus the selected
            // track". Neither is a live value an overlay can redraw.
            renamed |= matches!(
                event,
                daw_proto::track::TrackEvent::Renamed { .. }
                    | daw_proto::track::TrackEvent::SelectionChanged { .. }
            );
            listed |= matches!(
                event,
                daw_proto::track::TrackEvent::Added(_)
                    | daw_proto::track::TrackEvent::Removed(_)
                    | daw_proto::track::TrackEvent::Moved { .. }
            );
            session_daw::engine::apply_event(&mut self.tracks, event);
        }
        // A changed LIST outranks a changed name: the reload rebuilds
        // the names too, and doing both would record the panel twice.
        if listed {
            self.reload();
        } else if renamed {
            self.re_record();
        }
    }

    /// Add this frame's meter reading to each track's history.
    ///
    /// Once per FRAME rather than once per published meter frame,
    /// which means the history is sampled at the window's rate and not
    /// the engine's. That is the right way round for a display: it is
    /// drawn per frame, so a history at the frame rate has exactly one
    /// sample per drawn column and never aliases. The engine publishing
    /// slower just means some columns repeat.
    ///
    /// Only while the mixer is on screen. The arrangement draws no
    /// rack, and four seconds of history per track accumulated behind a
    /// panel nobody is looking at is memory spent on nothing.
    fn record_levels(&mut self) {
        if self.view != View::Mixer {
            return;
        }
        // A simulation stands in for the engine, not beside it: it
        // feeds the same `TrackLevels` and the same spectrum bins, so
        // every path below this is the one a real signal takes. A
        // simulation that took a shortcut would prove the shortcut.
        if self.simulate {
            let at = self.started.elapsed().as_secs_f64();
            // At the rate the ENGINE publishes meter frames, not at
            // the frame rate. The history is a fixed number of samples,
            // so pushing per frame at 240 fps would hold a third of a
            // second of it — less than one hit of a kick, and a
            // waveform that showed a single decay stretched across the
            // whole display.
            const PUBLISH: std::time::Duration = std::time::Duration::from_millis(33);
            if self.last_level.is_some_and(|last| last.elapsed() < PUBLISH) {
                return;
            }
            self.last_level = Some(std::time::Instant::now());
            for (index, track) in self.tracks.iter().enumerate() {
                let signal = session_daw::simulate::frame(index, at);
                self.tone_levels
                    .entry(track.guid.clone())
                    .or_default()
                    .push(signal.peak);
                self.tone_spectra
                    .entry(track.guid.clone())
                    .or_default()
                    .set(signal.spectrum);
            }
            return;
        }
        let Some(meters) = self.meters.as_ref() else {
            return;
        };
        let levels = meters.levels();
        if levels.is_empty() {
            return;
        }
        for track in &self.tracks {
            let Some(level) = usize::try_from(track.index).ok().and_then(|i| levels.get(i)) else {
                continue;
            };
            self.tone_levels
                .entry(track.guid.clone())
                .or_default()
                .push(level.peak_left.max(level.peak_right));
        }
    }

    /// The meter levels to draw, simulated or real.
    ///
    /// The mixer's meters read a frame rather than a history, so they
    /// take the simulation at this instant rather than the last thing
    /// pushed — which keeps them exactly as live as the traces beside
    /// them.
    fn meter_levels(&self) -> Vec<daw_proto::TrackLevels> {
        if self.simulate {
            let at = self.started.elapsed().as_secs_f64();
            return self
                .tracks
                .iter()
                .enumerate()
                .map(|(index, _)| {
                    let peak = session_daw::simulate::frame(index, at).peak;
                    daw_proto::TrackLevels {
                        peak_left: peak,
                        peak_right: peak * 0.85,
                        hold_left: peak,
                        hold_right: peak,
                    }
                })
                .collect();
        }
        self.meters
            .as_ref()
            .map(session_daw::engine::Meters::levels)
            .unwrap_or_default()
    }

    /// Re-read the session, because the track LIST changed.
    ///
    /// Added, Removed and Moved are the three events a re-record cannot
    /// absorb. Everything else is a field on a track this window
    /// already has; these change which tracks there ARE, and the rows,
    /// their depths, the scene's offsets and both recorded panels all
    /// follow from that. Patching a track into a folder tree from an
    /// event means recomputing the tree, and reading it back from the
    /// authority is both shorter and correct — four milliseconds, on a
    /// thread, while the window keeps drawing what it has.
    ///
    /// One reload at a time: a script adding forty tracks emits forty
    /// events, and forty reloads of the same session would be
    /// thirty-nine wasted. The last one wins because the read happens
    /// after all of them.
    fn reload(&mut self) {
        if self.loading.is_some() {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let theme = self.theme.clone();
        let layout = self.layout;
        let preset = session_daw::plan::slug(self.preset).unwrap_or(self.preset).to_owned();
        let settings = self.settings;
        if std::thread::Builder::new()
            .name("session-daw-reload".into())
            .spawn(move || {
                if let Some(loaded) = build_scene(&theme, layout, &preset, settings) {
                    let _ = tx.send(loaded);
                }
            })
            .is_ok()
        {
            self.loading = Some(rx);
        }
    }

    /// Fold or unfold the folder at a panel row.
    ///
    /// Only a folder can be folded, and clicking the rail of an
    /// ordinary track must do nothing rather than quietly hiding the
    /// track below it — the rail runs the full height of every row
    /// because it carries the indent colour, so most clicks on it are
    /// on a track that has no children.
    fn fold(&mut self, row: usize) {
        let Some((track, _)) = self.arrange_rows.get(row) else {
            return;
        };
        if !track.is_folder {
            return;
        }
        self.folders.toggle(&track.guid);
        self.re_record();
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
        let Some((project, _)) = self.session.clone() else {
            return;
        };
        // The rows are derived, not kept: which ones exist follows from
        // the folder state, and what they say follows from the live
        // tracks. Deriving both here is what makes folding a re-record
        // rather than a second list to keep in step.
        let live: Vec<daw_proto::Track> = project
            .0
            .tracks
            .iter()
            .map(|track| {
                self.tracks
                    .iter()
                    .find(|t| t.guid == track.guid)
                    .cloned()
                    .unwrap_or_else(|| track.clone())
            })
            .collect();
        let (visible, depths) = self.folders.visible(&live);
        let rows = daw_ui::studio::RowsRef(std::sync::Arc::new(
            visible.into_iter().zip(depths).collect(),
        ));
        // Then the preset, which decides which of those rows the
        // arrangement shows and how tall each one opens.
        let planned = daw_ui::studio::RowsRef(std::sync::Arc::new(session_daw::plan::apply(
            rows.as_slice(),
            session_daw::plan::slug(self.preset).unwrap_or(self.preset),
            session_daw::plan::Surface::Arrange,
            self.settings,
            self.surface_size.1,
        )));
        self.arrange_map = session_daw::plan::Rows::of(planned.as_slice(), &self.tracks);
        self.scene = Some(Arrangement::build(
            &self.palette,
            &self.font,
            &project,
            &planned,
            self.layout,
        ));
        self.arrange_rows = planned;
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
        self.record_levels();
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
        let view = self.viewport();
        let surface = self.palette.surface;
        let palette = &self.palette;
        let font = &self.font;
        let bars = Bars::at(scene.bpm);
        // The rows the SCENE was recorded from — after the preset, not
        // the session's full list. The scene's row indices are indices
        // into these, and reading the full list here is how a hidden
        // track makes every row below it name the wrong track.
        let rows: &[(daw_proto::Track, u32)] = self.arrange_rows.as_slice();
        let tracks = self.tracks.as_slice();
        let arrange_map = &self.arrange_map;
        let rename = self.rename.as_ref();
        let panel = &self.panel;
        let mode = self.mode;
        let rail_at = (self.hovered_rail, self.pressed_rail);
        let icons = &mut self.icons;
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
            self.mode,
            self.phase,
            self.preset,
            self.settings,
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
            if let Some(open) = rename.filter(|r| r.surface == session_daw::rename::Surface::Arrange)
            {
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
                arrange_map,
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
                icons,
                rail_at,
                frame,
                &profile.left,
                &profile.right,
                &profile.top,
            );
            session_daw::rails::main_toolbar(painter, &palette, &font, icons, rail_at, mode);
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
    let for_loader = theme.clone();
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
            match build_scene(
                &for_loader,
                layout,
                session_daw::plan::PRESETS[0].1,
                session_daw::settings::Settings::default(),
            ) {
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
        mods: eq_ui::eq_graph_interaction::Mods::default(),
        applier: session_daw::engine::Applier::start(),
        transport: session_daw::engine::Transport::start(),
        playhead: session_daw::cursor::Playhead::stopped(0.0),
        edit: session_daw::cursor::Edit::default(),
        dragging_time: None,
        pressed_row: None,
        hovered_rail: None,
        pressed_rail: None,
        row_drag: None,
        watch: session_daw::engine::Watch::start(),
        meters: session_daw::engine::Meters::start(),
        mode: session::modes::Mode::Mix,
        phase: session::mix_phases::MixPhase::Tone,
        preset: session_daw::rails::PRESETS[0],
        settings: session_daw::settings::Settings::default(),
        rename: None,
        last_row_click: None,
        session: None,
        mixer: None,
        mixer_map: session_daw::plan::Rows::default(),
        arrange_map: session_daw::plan::Rows::default(),
        arrange_rows: daw_ui::studio::RowsRef(std::sync::Arc::new(Vec::new())),
        mixer_scroll: 0.0,
        layout,
        last_grip_click: None,
        hovered_grip: None,
        rack_drag: None,
        tone_levels: std::collections::HashMap::new(),
        tone_spectra: std::collections::HashMap::new(),
        tone_settings: session_daw::tone::Store::default(),
        folders: daw_ui::components::folders::FolderState::default(),
        icons: session_daw::icons::Icons::new(),
        theme,
        // On by default: the rack is what this panel is being built
        // for, and a flag you have to remember is a feature nobody sees.
        tone: std::env::var_os("FTS_VELLO_NO_TONE").is_none(),
        last_level: None,
        simulate: std::env::var_os("FTS_VELLO_SIMULATE").is_some(),
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
    preset: &str,
    settings: session_daw::settings::Settings,
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
    // The opening preset is applied HERE rather than by the window,
    // so the first arrangement the window shows is already the one the
    // preset asks for. Recording it twice — once plain, once planned —
    // is four milliseconds nobody sees and a frame of the wrong layout
    // that they do.
    let planned = daw_ui::studio::RowsRef(std::sync::Arc::new(session_daw::plan::apply(
        rows.as_slice(),
        preset,
        session_daw::plan::Surface::Arrange,
        settings,
        0.0,
    )));
    // The project and its rows come back with the arrangement, because
    // the mixer is recorded against the WINDOW's height and that is not
    // known here — see `App::mixer_for`.
    Some(Loaded {
        arrangement: Arrangement::build(&palette, &font, &project, &planned, layout),
        project,
        rows,
        planned,
    })
}

/// What the loader thread hands back.
struct Loaded {
    arrangement: Arrangement,
    project: daw_ui::studio::ProjectRef,
    /// Every visible row, in the session's own order — the window's
    /// live values are indexed by this.
    rows: daw_ui::studio::RowsRef,
    /// And the subset the opening preset shows, at the sizes it gives
    /// them, which is what the recorded arrangement was built from.
    planned: daw_ui::studio::RowsRef,
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
        Edit::SetPhase(_, inverted) => track.phase_inverted = *inverted,
        Edit::SetParentSend(_, enabled) => track.parent_send = *enabled,
        // Selection is the engine's to decide: an exclusive select
        // changes every OTHER track too, and predicting which ones
        // stop being selected would be predicting the engine's whole
        // answer. Getting that wrong looks worse than a frame of lag
        // looks slow — and the frame is one round trip in-process.
        Edit::Select(_) | Edit::AddToSelection(_) => {}
    }
}
