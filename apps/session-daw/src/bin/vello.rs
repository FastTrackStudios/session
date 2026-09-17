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
use vello::kurbo::{Affine, Rect};
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

use session_daw::arrangement::{Arrangement, Palette, TCP_WIDTH, Viewport};
use session_daw::ruler::{Bars, RULER_H};
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
    /// The rows' scale — the zoom tool's vertical axis.
    zoom_y: f64,
    /// `z` is down: the next press on the lanes is the zoom tool.
    zoom_held: bool,
    /// Whether `g` is down — the tempo-mapping tool, where a click
    /// moves the nearest bar line to the pointer and the tempo is
    /// whatever makes that true.
    grid_held: bool,
    /// Counts and the dot: `4t`, then `.` over and over.
    repeat: session_daw::repeat::Repeat,
    /// The transients of the track being mapped, read once and kept
    /// until the track or the view changes. Reading a minute of audio
    /// per keypress would make `.` feel like a round trip, which is the
    /// one thing it must not.
    transients: Vec<f64>,
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
    /// The album's plan, resolved against the machine's studio profile
    /// — read once when the window opens, because it is the ALBUM's
    /// file and not this session's state. `None` when the project is
    /// not part of an album.
    patch_list: session_daw::patch_list::Panel,
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
    /// Where a ruler drag started, in seconds.
    /// The arrangement's editing: the selection, the edit cursor and
    /// the time selection, and whatever is being dragged — see
    /// `arrange_edit`. The window forwards what the pointer and the
    /// keys did and carries out what it asks for.
    editor: session_daw::arrange_edit::Editor,
    /// The expression editor, once `e` has opened it on an item. Kept
    /// while the dock is closed so reopening finds the same zoom and
    /// selection.
    expression: Option<session_daw::expression::Expression>,
    /// The dock's height along the bottom of the arrangement, while
    /// the editor is docked there.
    dock: Option<f64>,
    /// Whether the keyboard belongs to the dock — the last press was
    /// in it.
    dock_focus: bool,
    /// The dock's top edge being dragged to resize it.
    dock_drag: bool,
    /// The item under the pointer in the lanes, whose fade handles are
    /// drawn.
    hovered_item: Option<usize>,
    /// The keys held, for the mouse map.
    keys: session_daw::mousemap::Mods,
    /// A scrollbar thumb being dragged: which bar, and where on the
    /// thumb it was taken.
    bar_drag: Option<(session_daw::scrollbar::Axis, f64)>,
    /// Whether the view turns the page to follow the playhead. A
    /// setting: `FTS_FOLLOW=off` opens with it off; `f` toggles it.
    follow: bool,
    /// The keyboard, through the FTS profile's bindings.
    bindings: session_daw::keys::Keys,
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
    /// When the window last tried to find a REAPER again. A dial a
    /// frame would spend the whole of REAPER's startup failing.
    last_dial: Option<std::time::Instant>,
    /// The notes of the MIDI items, as they are read. An item is drawn
    /// from what it CONTAINS, and the notes arrive after the window is
    /// already standing.
    midi: session_daw::midi::Previews,
    /// Whether anything that is NOT a track has changed — items,
    /// takes, markers, regions, the tempo map. A flag, because all of
    /// them are drawn from one snapshot and the only useful question
    /// is whether that snapshot is old.
    refresh: Option<session_daw::engine::Refresh>,
    /// The engine's live meter levels, latest-wins.
    meters: Option<session_daw::engine::Meters>,
    /// Which DAW mode the window is in — what the corner selects.
    mode: session::modes::Mode,
    /// Which mix phase, which is what the left rail's lower half
    /// selects and what decides how much processing a strip shows.
    phase: session::mix_phases::MixPhase,
    /// Which scene is showing, and why — the visibility manager.
    ///
    /// `flow.scenes.follow-mode`: entering a mode shows that mode's
    /// scene for the instrument and audience, the number keys recall
    /// inside it, and a hand-chosen scene stays until the mode changes.
    visibility: dynamic_template::scenes::Follow,
    /// The taxonomy the template wrote into the project this window
    /// opened — what a scene's selectors match against.
    kinds: session_daw::plan::Kinds,
    /// The right rail's switches.
    settings: session_daw::settings::Settings,
    /// An open rename, if a name is being edited.
    rename: Option<session_daw::rename::Rename>,
    /// When the last panel click was, for spotting a double.
    last_row_click: Option<(usize, std::time::Instant)>,
    /// When the last click on a mark in the ruler was, and on which —
    /// for spotting the double that opens its name.
    last_ruler_click: Option<(session_daw::ruler::On, std::time::Instant)>,
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
    /// The last reverb-tail generation the mixer was recorded against
    /// — see `record_levels`.
    tails_seen: u64,
    /// The Tone settings every rack is drawn from. Seeded from the
    /// placeholder until a chain can be read — see `tone::Store`.
    tone_settings: session_daw::tone::Store,
    /// Which tracks have clipped since anyone last cleared them.
    ///
    /// Here rather than in the levels, because it is the one piece of
    /// metering that does NOT decay: a peak-hold is a reading and this
    /// is a report, and the value of a report is that it is still there
    /// when you look up.
    clips: session_daw::overlay::Clips,
    /// How far the racks are scrolled, shared by every strip.
    ///
    /// ONE scroll for the whole mixer, not one per track. The reason to
    /// put the chains side by side is to compare the same processor
    /// across tracks, and racks that scrolled independently would take
    /// that away the first time you moved one — you would be comparing
    /// a compressor against somebody else's saturator.
    rack_scroll: f64,
    /// Which phase containers are folded shut.
    rack_folds: session_daw::tone::Fold,
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
        let (want_w, want_h) = window_size();
        let attrs = WindowAttributes::default()
            .with_title(self.view.title())
            .with_surface_size(winit::dpi::LogicalSize::new(want_w, want_h));
        let window: Arc<dyn Window> =
            Arc::from(event_loop.create_window(attrs).expect("create window"));
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
                self.renderer
                    .set_size(size.width.max(1), size.height.max(1));
                self.redraw();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // The expression editor takes the wheel in notches —
                // one line of a mouse wheel — which is what its zoom
                // and pan gains are tuned for.
                if self.dock_at(self.cursor.0, self.cursor.1) {
                    let (nx, ny) = match delta {
                        MouseScrollDelta::LineDelta(x, y) => (f64::from(x), f64::from(y)),
                        MouseScrollDelta::PixelDelta(p) => (
                            p.x / expression_editor_paint::scroll::PIXELS_PER_NOTCH,
                            p.y / expression_editor_paint::scroll::PIXELS_PER_NOTCH,
                        ),
                    };
                    let (x, y) = self.cursor;
                    if let Some(ex) = self.expression.as_mut()
                        && ex.wheel(x, y, nx, ny, self.keys)
                    {
                        self.redraw();
                    }
                    return;
                }
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
                // Over the rack but not on a grip: scroll the CHAIN.
                // Every strip shows every processor now, which makes
                // the rack taller than its box as soon as there are
                // more than a few — so the box travels over it.
                if self.in_rack(self.cursor.0, self.cursor.1) {
                    self.scroll_rack(dy);
                    self.redraw();
                    return;
                }
                // Ctrl+wheel is REAPER's horizontal zoom, about the
                // pointer: the second under it stays under it.
                if self.view == View::Arrangement && self.keys.ctrl {
                    self.zoom_about(self.cursor.0, dy);
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
                // `z` held is the zoom tool, on the arrangement as on
                // the roll: the next press on the lanes drags a zoom.
                // The dock's own `z` is the editor's when it has focus.
                if event.logical_key.to_text() == Some("z")
                    && !self.keys.ctrl
                    && !self.keys.alt
                    && !(self.dock_focus && self.dock.is_some())
                {
                    self.zoom_held = true;
                    return;
                }
                // The tempo-mapping keys: `t` snaps the nearest bar
                // line to the next transient, a count in front of it
                // takes the Nth, and `.` does the same thing again. In
                // the arrangement only, and not while the dock has the
                // keyboard — `t` is a letter everywhere else.
                if self.view == View::Arrangement
                    && !(self.dock_focus && self.dock.is_some())
                    && !self.keys.ctrl
                    && !self.keys.alt
                    && let Some(key) = event.logical_key.to_text().and_then(|t| t.chars().next())
                    && let session_daw::repeat::Press::Run { command, times } =
                        self.repeat.press(key, &['t'])
                {
                    if command == 't' {
                        self.snap_to_transient(times);
                        self.redraw();
                    }
                    return;
                }
                // `g` held is the tempo-mapping tool: a click while it
                // is down puts the nearest bar line where you clicked.
                // Held rather than a mode, because mapping a song is a
                // hundred clicks and a mode you have to leave between
                // them is a mode you forget you are in.
                if event.logical_key.to_text() == Some("g")
                    && !self.keys.ctrl
                    && !(self.dock_focus && self.dock.is_some())
                {
                    self.grid_held = true;
                    return;
                }
                // `e` docks the expression editor under the arrangement
                // on the selected item, and closes the dock again.
                // Before the editor sees the key, or there would be no
                // way out of a dock that binds it.
                if event.logical_key.to_text() == Some("e") && !self.keys.ctrl && !self.keys.alt {
                    self.toggle_dock();
                    self.redraw();
                    return;
                }
                // While the dock has focus the keyboard is the
                // editor's: its own keymap, its own actions.
                if self.dock_focus
                    && self.dock.is_some()
                    && let Some(name) = expression_key_name(&event.logical_key)
                    && let Some(ex) = self.expression.as_mut()
                    && ex.key(&name, self.keys)
                {
                    self.redraw();
                    return;
                }
                // The profile first: what a key does here is what it
                // does in REAPER with the extension loaded. A binding
                // the window cannot do yet is logged and falls through
                // to the window's own keys, so `x` still switches views
                // while it is bound to a cut nobody has built.
                {
                    let named = match &event.logical_key {
                        winit::keyboard::Key::Named(n) => Some(format!("{n:?}")),
                        _ => None,
                    };
                    let code =
                        session_daw::keys::key_code(named.as_deref(), event.logical_key.to_text());
                    if let Some(code) = code {
                        let modifiers = input::Modifiers {
                            ctrl: self.keys.ctrl,
                            alt: self.keys.alt,
                            shift: self.keys.shift,
                            meta: false,
                        };
                        let actions = self.bindings.press(code, modifiers);
                        let mut handled = false;
                        for action in actions {
                            handled |= self.act_on_key(action);
                        }
                        if handled {
                            self.redraw();
                            return;
                        }
                    }
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
                // `f` follows the playhead, or stops following it.
                if event.logical_key.to_text() == Some("f") {
                    self.follow = !self.follow;
                    tracing::info!(ui.follow = self.follow, "follow playhead");
                    self.redraw();
                    return;
                }
                if event.logical_key == Key::Named(NamedKey::Home) {
                    session_daw::engine::transport(session_daw::engine::Move::Home, 0.0);
                    self.playhead.report(0.0, 1.0, std::time::Instant::now());
                    self.redraw();
                    return;
                }
                // The number keys recall scenes inside the mode — `1`
                // is the mode's own scene, and a digit past the end
                // goes back to no scene at all.
                if let Some(text) = event.logical_key.to_text()
                    && text.len() == 1
                    && let Some(digit) = text.chars().next().and_then(|c| c.to_digit(10))
                {
                    // The keys reach this instrument's scenes, not
                    // every instrument's: with four sets in the table
                    // there are more scenes in Record than there are
                    // digits, and the keys are for moving around what
                    // is in front of you.
                    let instrument = self.instrument();
                    let shown = self
                        .visibility
                        .recall(dynamic_template::scenes::scenes(), digit, &instrument)
                        .map(str::to_owned);
                    tracing::info!(ui.scene = shown.as_deref().unwrap_or("none"), "scene");
                    self.sync_folds_to_scene();
                    self.mixer = None;
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
                // The fold setting: together, or per track. Carrying
                // the current track's folds across when it goes
                // together, because the fold you just made is almost
                // always the one you want everywhere.
                if event.logical_key.to_text() == Some("f") {
                    self.settings.fold_phases_together = !self.settings.fold_phases_together;
                    let from = self
                        .pointer
                        .hovered()
                        .and_then(|spot| self.mixer_map.index(spot.row))
                        .and_then(|i| self.tracks.get(i))
                        .map(|track| track.guid.clone())
                        .unwrap_or_default();
                    self.rack_folds
                        .sync(self.settings.fold_phases_together, &from);
                    tracing::info!(
                        ui.fold_together = self.settings.fold_phases_together,
                        "phase folds"
                    );
                    self.mixer = None;
                    self.redraw();
                    return;
                }
                // `p` shows the album's plan. It is a view of the
                // ALBUM, not of this session, so it is here rather than
                // on the mixer's rail: it reads the same whatever the
                // window has open.
                if event.logical_key.to_text() == Some("p") {
                    self.show(if self.view == View::PatchList {
                        View::Arrangement
                    } else {
                        View::PatchList
                    });
                    return;
                }
                if event.logical_key.to_text() == Some("x") {
                    self.show(self.view.toggled());
                }
            }
            // The fine-adjustment modifier. Held, a drag moves a
            // quarter as far — REAPER's own Ctrl, and the difference
            // between setting a fader and setting it exactly.
            //
            // This was a field nothing ever wrote: every drag in the
            // window has been coarse because no event set it.
            WindowEvent::KeyboardInput { event, .. } => {
                if event.logical_key.to_text() == Some("z") {
                    self.zoom_held = false;
                }
                if event.logical_key.to_text() == Some("g") {
                    self.grid_held = false;
                }
                // A release. The editor's keymap has to hear it, or a
                // held prefix repeats its way down the sequence tree;
                // and a spring-loaded tool springs back on it.
                if self.dock_focus
                    && self.dock.is_some()
                    && let Some(name) = expression_key_name(&event.logical_key)
                    && let Some(ex) = self.expression.as_mut()
                    && ex.key_up(&name, self.keys)
                {
                    self.redraw();
                }
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                let state = modifiers.state();
                self.fine = state.control_key();
                self.keys = session_daw::mousemap::Mods {
                    shift: state.shift_key(),
                    ctrl: state.control_key() || state.meta_key(),
                    alt: state.alt_key(),
                };
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
                // The dock's edge, being dragged: the dock follows the
                // pointer and the arrangement gives way above it.
                if self.dock_drag {
                    let height = self.surface_size.1;
                    let most = height - session_daw::rails::TOP - DOCK_MIN;
                    self.dock = Some((height - position.y).clamp(DOCK_MIN, most.max(DOCK_MIN)));
                    self.redraw();
                    return;
                }
                // A gesture in the dock keeps the pointer after it
                // leaves the box; otherwise the dock takes only what is
                // over it.
                if self.dock_at(position.x, position.y)
                    || self.expression.as_ref().is_some_and(|ex| ex.dragging())
                {
                    if let Some(ex) = self.expression.as_mut()
                        && ex.moved(position.x, position.y, self.keys)
                    {
                        self.redraw();
                    }
                    return;
                }
                let spot = self.spot_at(position.x, position.y);
                // A fade in flight follows the pointer along the item:
                // the fade-in is as long as the pointer is past the
                // item's start, the fade-out as long as it is short of
                // its end, either clamped to the item.
                if self.editor.zoom.is_some() {
                    let view = self.viewport();
                    let lanes = self.lanes_origin();
                    if let Some(next) =
                        self.editor
                            .zoom_move((position.x, position.y), &view, lanes, self.keys)
                    {
                        self.apply_view(next);
                    }
                    self.redraw();
                    return;
                }
                if let Some((axis, _)) = self.bar_drag {
                    let (dx, dy) = (position.x - last.0, position.y - last.1);
                    if let Some(bars) = self.scrollbars() {
                        let (bar, along) = match axis {
                            session_daw::scrollbar::Axis::X => (bars.0, dx),
                            session_daw::scrollbar::Axis::Y => (bars.1, dy),
                        };
                        let by = bar.scroll_per_thumb(along);
                        match axis {
                            session_daw::scrollbar::Axis::X => {
                                self.scroll_to(self.scroll_x + by, self.scroll_y)
                            }
                            session_daw::scrollbar::Axis::Y => {
                                self.scroll_to(self.scroll_x, self.scroll_y + by)
                            }
                        }
                    }
                    self.redraw();
                    return;
                }
                // What is being dragged in the arrangement — an item, a
                // fade, a time selection — follows the pointer.
                let at_time = self.ruler_time_unclamped(position.x);
                let bpm = self.scene.as_ref().map_or(120.0, |s| s.bpm);
                if self.editor.moved(at_time, self.pps, bpm, self.keys) {
                    self.redraw();
                    return;
                }
                // The item under the pointer, for its handles.
                if self.view == View::Arrangement && !self.editor.dragging() {
                    let over = match self
                        .arrange_hit_at(position.x, position.y)
                        .map(|h| h.target)
                    {
                        Some(session_daw::hit::Target::Item { index, .. }) => Some(index),
                        _ => None,
                    };
                    if over != self.hovered_item {
                        self.hovered_item = over;
                        self.redraw();
                    }
                }
                // A drag outranks a hover: while the pointer is down on
                // a fader it is setting a level, not browsing.
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
                // The dock's top edge resizes it; the dock itself takes
                // every button — middle pans, right opens the editor's
                // menu. A press anywhere else takes the keyboard back.
                if self.view == View::Arrangement && self.dock.is_some() {
                    self.cursor = (position.x, position.y);
                    if !state.is_pressed() && std::mem::take(&mut self.dock_drag) {
                        self.redraw();
                        return;
                    }
                    if state.is_pressed() && self.dock_grip_at(position.x, position.y) {
                        self.dock_drag = true;
                        self.dock_focus = true;
                        return;
                    }
                    let in_dock = self.dock_at(position.x, position.y)
                        || self.expression.as_ref().is_some_and(|ex| ex.dragging());
                    if in_dock {
                        let code = match button.mouse_button() {
                            Some(winit::event::MouseButton::Right) => 2,
                            Some(winit::event::MouseButton::Middle) => 1,
                            _ => 0,
                        };
                        let keys = self.keys;
                        let changed = match self.expression.as_mut() {
                            Some(ex) if state.is_pressed() => {
                                ex.press(position.x, position.y, keys, code)
                            }
                            Some(ex) => ex.release(position.x, position.y, keys),
                            None => false,
                        };
                        if state.is_pressed() {
                            self.dock_focus = true;
                        }
                        if let Some(asked) =
                            self.expression.as_mut().and_then(|ex| ex.take_pending())
                        {
                            tracing::info!(
                                expression.pending = ?asked,
                                "the editor asked for a panel this window does not draw yet"
                            );
                        }
                        if changed {
                            self.redraw();
                        }
                        return;
                    }
                    if state.is_pressed() {
                        self.dock_focus = false;
                    }
                }
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
                    // The tempo-mapping tool, while `g` is held: the
                    // click is where a downbeat IS, and the nearest bar
                    // line is moved there. Asked before the zoom tool
                    // and before the arrangement, because a click meant
                    // for the grid must not also seek or select.
                    if self.grid_held && self.view == View::Arrangement {
                        self.map_tempo_at(x);
                        self.redraw();
                        return;
                    }
                    // The zoom tool, while `z` is held: a drag from here
                    // zooms rather than selects.
                    if self.zoom_held && self.view == View::Arrangement {
                        let view = self.viewport();
                        if self
                            .editor
                            .zoom_press((x, y), &view, self.lanes_origin(), self.keys)
                        {
                            self.redraw();
                            return;
                        }
                    }
                    // The scrollbars are drawn over the lanes, so they
                    // are pressed before them: a thumb is taken hold
                    // of, the track beside it turns a page.
                    if let Some(action) = self.press_scrollbar(x, y) {
                        self.bar_drag = action;
                        self.redraw();
                        return;
                    }
                    // What the press landed on in the arrangement — an
                    // item, a fade handle, the ruler, the empty area —
                    // through the mouse map, to the editor.
                    if let Some(scene) = self.scene.as_ref() {
                        let hit = self.arrange_hit_at(x, y);
                        // How wide the band under the pointer is. The
                        // hit map knows WHICH band; only the window has
                        // the list to ask how far it runs, and a drag
                        // has to move it from where it was rather than
                        // from where the last frame left it.
                        self.editor.ruler_span = self.ruler_span_of(hit);
                        let mut effects = Vec::new();
                        if self.editor.press(hit, self.keys, scene, &mut effects) {
                            self.run(effects);
                            self.redraw();
                            return;
                        }
                    }
                    // The rack is above the strip's controls and is
                    // drawn over them, so it is claimed first.
                    self.rack_drag = self.rack_grip_at(x, y);
                    tracing::debug!(ui.x = x, ui.y = y, ui.grip = ?self.rack_drag, "rack press");
                    if let Some((row, grip)) = self.rack_drag {
                        // A switch acts on the press and starts no
                        // drag. Everything else in a rack is a value,
                        // and a value is taken hold of.
                        if grip.is_switch() {
                            self.toggle_rack(row, grip);
                            self.rack_drag = None;
                            self.mixer = None;
                            self.redraw();
                            return;
                        }
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
                    if self.editor.zoom.is_some() {
                        let view = self.viewport();
                        let lanes = self.lanes_origin();
                        if let Some(next) = self.editor.zoom_release(&view, lanes) {
                            self.apply_view(next);
                        }
                        self.redraw();
                        return;
                    }
                    if self.bar_drag.take().is_some() {
                        self.redraw();
                        return;
                    }
                    {
                        // A second click on the same mark opens its
                        // name. Asked BEFORE the release so the move
                        // that click would otherwise be — to the place
                        // it is already in — is dropped rather than
                        // sent: a rename should not also nudge the
                        // thing being renamed.
                        let naming = self.ruler_double_click(x, y);
                        let at = self.ruler_time_unclamped(x);
                        let mut effects = Vec::new();
                        let released = match self.session.as_mut() {
                            Some((project, _)) => {
                                let project = std::sync::Arc::make_mut(&mut project.0);
                                self.editor.release(at, self.keys, project, &mut effects)
                            }
                            None => false,
                        };
                        if naming {
                            self.redraw();
                            return;
                        }
                        if released {
                            self.run(effects);
                            self.redraw();
                            return;
                        }
                    }
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
                                if let Some(track) =
                                    self.arrange_map.index(row).and_then(|i| self.tracks.get(i))
                                {
                                    self.rename = Some(session_daw::rename::Rename::new(
                                        session_daw::rename::Surface::Arrange,
                                        row,
                                        session_daw::rename::What::Track(track.guid.clone()),
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
            // An attached window learns its project's location only
            // after connecting, so the two things read from the file —
            // the taxonomy and the album's patch list — arrive with the
            // first load rather than before the window opened. Taken
            // once; after that the window's own copies are the ones
            // being edited.
            if let Some(kinds) = LIVE_KINDS.get()
                && self.kinds.is_empty()
            {
                self.kinds = kinds.clone();
            }
            if let Some(panel) = LIVE_PATCH_LIST.get()
                && matches!(self.patch_list, session_daw::patch_list::Panel::Absent)
            {
                self.patch_list = panel.clone();
                self.redraw_patch_list();
            }

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
            if self.refresh.is_none() {
                self.refresh = session_daw::engine::Refresh::start();
            }
            if self.meters.is_none() {
                self.meters = session_daw::engine::Meters::start();
            }
            // Ask for the notes of every MIDI item this project has.
            // Skipped for anything already read, so a reload after a
            // track change costs nothing for what is already known.
            self.request_midi();
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
        let frame = self.frame();
        let (width, height) = (frame.content_width(), frame.content_height());
        (
            (scene.length_secs * self.pps - (width - TCP_WIDTH)).max(1.0),
            // The ruler takes a strip off the top, so there is that much
            // more to scroll before the last row reaches the bottom.
            (scene.content_height() * self.zoom_y - (height - RULER_H)).max(1.0),
        )
    }

    /// Move the view, clamped to the session.
    ///
    /// The single place scroll position is written, so the wheel and the
    /// autoscroll cannot end up with different ideas about the bounds or
    /// the units.
    /// Zoom the arrangement horizontally by a wheel travel of `dy`
    /// pixels, keeping the time under window x `x` where it is.
    ///
    /// One wheel notch is about sixteen percent; the scale is clamped
    /// between two pixels a second, where an hour fits a screen, and
    /// two thousand, where a millisecond is two pixels.
    /// Where the lanes start in the window.
    fn lanes_origin(&self) -> session_daw::arrange_edit::LanesOrigin {
        (
            session_daw::rails::SIDE + TCP_WIDTH,
            session_daw::rails::TOP + RULER_H,
        )
    }

    /// Show the view a zoom asked for: its scale on both axes, and its
    /// scroll clamped to the spans that scale gives.
    fn apply_view(&mut self, next: Viewport) {
        self.pps = next.pps;
        self.zoom_y = next.zoom_y;
        self.scroll_to(next.scroll_x, next.scroll_y);
        self.mixer = None;
    }

    fn zoom_about(&mut self, x: f64, dy: f64) {
        const MIN_PPS: f64 = 2.0;
        const MAX_PPS: f64 = 2000.0;
        let factor = (dy / 53.0 * 0.15).exp();
        let anchor = x - session_daw::rails::SIDE - TCP_WIDTH;
        let at = (anchor + self.scroll_x) / self.pps.max(f64::EPSILON);
        self.pps = (self.pps * factor).clamp(MIN_PPS, MAX_PPS);
        let scroll_x = at.mul_add(self.pps, -anchor);
        self.scroll_to(scroll_x, self.scroll_y);
        // The bars, titles and ruler all read `pps` from the viewport,
        // so nothing else has to be told; the recording is untouched.
        self.mixer = None;
    }

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
        let content_y = y - session_daw::rails::TOP - RULER_H + self.scroll_y;
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
        let _ = (width, height);
        let control = session_daw::mcp::control_at(
            mixer,
            row,
            content_x - left,
            y - session_daw::rails::TOP,
        )?;
        Some(session_daw::pointer::Spot { row, control })
    }

    /// Whether a window point is inside some strip's rack box.
    ///
    /// Asked after the grips, so a wheel over a band still moves the
    /// band — the rack scrolls only where there is nothing else for the
    /// gesture to mean.
    fn in_rack(&self, x: f64, y: f64) -> bool {
        if self.view != View::Mixer {
            return false;
        }
        let Some(mixer) = self.mixer.as_ref() else {
            return false;
        };
        let content_x = x - session_daw::rails::SIDE + self.mixer_scroll;
        let Some(row) = mixer.strip_at(content_x) else {
            return false;
        };
        let Some((left, width, height)) = mixer.strip_box(row) else {
            return false;
        };
        let _ = (width, height);
        let Some(strip) = mixer.strip(row) else {
            return false;
        };
        strip.rack_rect().is_some_and(|box_| {
            let at = y - session_daw::rails::TOP;
            content_x >= left + box_.x0
                && content_x < left + box_.x1
                && at >= box_.y0
                && at < box_.y1
        })
    }

    /// Move the chain under its box.
    ///
    /// Clamped at both ends: a rack scrolled past its last processor is
    /// a screen of nothing with no cue about which way back, and the
    /// mixer has no ruler to orient by.
    fn scroll_rack(&mut self, by: f64) {
        let Some(mixer) = self.mixer.as_ref() else {
            return;
        };
        let panels = if self.tone {
            session_daw::tone::panels_for(self.phase)
        } else {
            &[][..]
        };
        // The LONGEST column, not the first track's. The scroll is
        // shared, and with per-track folds the strips have different
        // heights — clamping to one of them would put the bottom of
        // every other chain out of reach.
        //
        // Synced, they are all the same and this costs one pass over
        // the tracks to agree with itself.
        let span = self
            .tracks
            .iter()
            .map(|track| {
                session_daw::tone::scroll_span(
                    self.tone_settings
                        .get(&track.guid)
                        .map_or(panels, |t| t.panels(panels)),
                    mixer.rack_h,
                    self.rack_folds.of(&track.guid),
                )
            })
            .fold(0.0_f64, f64::max);
        self.rack_scroll = (self.rack_scroll - by).clamp(0.0, span);
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
        let _ = (width, height);
        let strip = mixer.strip(row)?;
        // Scrolled, like the drawing — see `tone::layout`. A hit test
        // against the unscrolled box would grab whatever USED to be
        // under the pointer before the chain moved.
        let rack = session_daw::tone::Panel::of(strip.rack_rect()?, left).up(self.rack_scroll);
        let track = self.mixer_map.index(row).and_then(|i| self.tracks.get(i))?;
        let tone = self.tone_settings.get(&track.guid)?;
        session_daw::tone::grip_at(
            tone.panels(self.rack_panels()),
            tone,
            rack,
            self.rack_folds.of(&track.guid),
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
        self.invalidate_rack(row);
    }

    /// A modified click on a band — bypass, or cycle its shape.
    ///
    /// Returns whether it did anything, so a plain click falls through
    /// to taking hold of the band instead.
    fn click_rack(&mut self, row: usize, grip: session_daw::tone::Grip) -> bool {
        let session_daw::tone::Grip::Band(which, index) = grip else {
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
        let changed = self
            .tone_settings
            .edit(&guid)
            .is_some_and(|tone| session_daw::tone::dot_click(tone, which, index, mods));
        if changed {
            self.invalidate_rack(row);
        }
        changed
    }

    /// Flip a rack switch — the grips that are not values.
    fn toggle_rack(&mut self, row: usize, grip: session_daw::tone::Grip) {
        use session_daw::tone::Grip;
        if !grip.is_switch() {
            return;
        }
        let Some(guid) = self
            .mixer_map
            .index(row)
            .and_then(|i| self.tracks.get(i))
            .map(|track| track.guid.clone())
        else {
            return;
        };
        // Folding is not a setting on the track — it is what the window
        // is showing — so it is handled before the per-track settings
        // and never reaches them.
        if let Grip::Phase(phase) = grip {
            self.rack_folds.toggle(&guid, phase);
            self.invalidate_rack(row);
            self.redraw();
            return;
        }
        if let Some(tone) = self.tone_settings.edit(&guid) {
            match grip {
                Grip::Bypass(which) => tone.bypass.toggle(which),
                // Clicking the zoom steps it to the next stop; the
                // wheel walks it either way and a double-click puts it
                // back. Wrapping, because a chip you click is a cycle.
                Grip::Scale(_) => tone.cycle_eq_range(),
                // The machine glyph cycles the machine: the delay's
                // style, the reverb's algorithm.
                Grip::Family(session_daw::tone::Which::Sat) => tone.cycle_sat(),
                Grip::Family(session_daw::tone::Which::Delay) => tone.delay.cycle_style(),
                Grip::Family(session_daw::tone::Which::Reverb) => tone.reverb.cycle_algorithm(),
                // A chip in the selector strip picks its family.
                Grip::Choose(session_daw::tone::Which::Sat, i) => tone.choose_sat_family(i),
                Grip::Choose(session_daw::tone::Which::Delay, i) => tone.delay.choose_family(i),
                Grip::Choose(session_daw::tone::Which::Reverb, i) => tone.reverb.choose_family(i),
                // A preset chip loads the preset: the whole rack follows.
                Grip::Preset(i) => tone.load_preset(i),
                _ => {}
            }
        }
        self.invalidate_rack(row);
    }

    /// Throw away a strip's cached rack, because its settings moved.
    ///
    /// The cache is keyed on the SPECTRUM — what usually changes — so
    /// a knob or a switch has to say so, or the picture would wait for
    /// the next meter frame to catch up and lag the gesture by a
    /// thirtieth of a second.
    fn invalidate_rack(&mut self, row: usize) {
        let Some(guid) = self
            .mixer_map
            .index(row)
            .and_then(|i| self.tracks.get(i))
            .map(|track| track.guid.clone())
        else {
            return;
        };
        if let Some(analyser) = self.tone_spectra.get_mut(&guid) {
            analyser.invalidate();
        }
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
        self.invalidate_rack(row);
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
        let _ = (width, height);
        let Some(strip) = mixer.strip(row) else {
            return;
        };
        let Some(rack) = strip
            .rack_rect()
            .map(|r| session_daw::tone::Panel::of(r, left).up(self.rack_scroll))
        else {
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
        let folded = self.rack_folds.of(&guid);
        if let Some(tone) = self.tone_settings.edit(&guid) {
            let panels = tone.panels(panels);
            session_daw::tone::drag(tone, grip, panels, rack, folded, mods, dx, dy);
        }
        self.invalidate_rack(row);
    }

    /// The hit under a window point, for the gesture layer.
    fn hit_at(&self, x: f64, y: f64) -> Option<session_daw::hit::Hit> {
        let mixer = self.mixer.as_ref()?;
        Some(session_daw::hit::mixer(mixer, self.mixer_scroll, x, y))
    }

    /// The box the lanes are drawn in: under the ruler, right of the
    /// panel, inside the rails.
    fn lanes_box(&self) -> Rect {
        let frame = self.frame();
        let rail = (session_daw::rails::SIDE, session_daw::rails::TOP);
        Rect::new(
            rail.0 + TCP_WIDTH,
            rail.1 + RULER_H,
            rail.0 + frame.content_width(),
            rail.1 + frame.content_height(),
        )
    }

    /// The arrangement's two scrollbars, for this frame's scroll.
    fn scrollbars(&self) -> Option<(session_daw::scrollbar::Bar, session_daw::scrollbar::Bar)> {
        if self.view != View::Arrangement || self.scene.is_none() {
            return None;
        }
        Some(session_daw::scrollbar::bars(
            self.lanes_box(),
            (self.scroll_x, self.scroll_y),
            self.spans(),
        ))
    }

    /// A press on a scrollbar: the thumb taken (returned as the drag to
    /// hold), or a page turned and nothing held. `None` if the press
    /// was not on a bar.
    fn press_scrollbar(
        &mut self,
        x: f64,
        y: f64,
    ) -> Option<Option<(session_daw::scrollbar::Axis, f64)>> {
        use session_daw::scrollbar::{Axis, Press};
        let bars = self.scrollbars()?;
        for bar in [bars.0, bars.1] {
            match bar.press(x, y) {
                Press::Miss => {}
                Press::Thumb => return Some(Some((bar.axis, 0.0))),
                Press::PageBack | Press::PageForward => {
                    let by = if bar.press(x, y) == Press::PageBack {
                        -bar.page()
                    } else {
                        bar.page()
                    };
                    match bar.axis {
                        Axis::X => self.scroll_to(self.scroll_x + by, self.scroll_y),
                        Axis::Y => self.scroll_to(self.scroll_x, self.scroll_y + by),
                    }
                    return Some(None);
                }
            }
        }
        None
    }

    /// What moves the view before a frame is drawn: a drag held at the
    /// edge of the lanes scrolls under it, and a playing transport
    /// turns the page when the playhead leaves the view.
    fn before_arrange_frame(&mut self) {
        let lanes = self.lanes_box();
        if self.editor.dragging() {
            let (dx, dy) = session_daw::scrollbar::autoscroll(lanes, self.cursor.0, self.cursor.1);
            if dx != 0.0 || dy != 0.0 {
                self.scroll_to(self.scroll_x + dx, self.scroll_y + dy);
                // The thing being dragged follows the pointer, which
                // is now over a different time than a frame ago.
                let at = self.ruler_time_unclamped(self.cursor.0);
                let bpm = self.scene.as_ref().map_or(120.0, |s| s.bpm);
                self.editor.moved(at, self.pps, bpm, self.keys);
            }
            return;
        }
        if self.follow && self.playhead.playing() && self.bar_drag.is_none() {
            let play_x = self.playhead.at_time(std::time::Instant::now()) * self.pps;
            if let Some(to) = session_daw::scrollbar::follow(self.scroll_x, lanes.width(), play_x) {
                self.scroll_to(to, self.scroll_y);
            }
        }
    }

    /// An edit to the engine, if one is listening.
    fn send(&self, edit: session_daw::engine::Edit) {
        if let Some(applier) = &self.applier {
            applier.send(edit);
        }
    }

    /// Carry out what the editor asked for.
    fn run(&mut self, effects: Vec<session_daw::arrange_edit::Effect>) {
        use session_daw::arrange_edit::Effect;
        for effect in effects {
            match effect {
                Effect::Send(edit) => self.send(edit),
                Effect::ReRecord => self.re_record(),
                Effect::Transport(command, at) => session_daw::engine::transport(command, at),
                Effect::Playhead(at) => self.playhead.report(at, 1.0, std::time::Instant::now()),
            }
        }
    }

    /// Do what a bound key asks. `false` when the window cannot, so the
    /// key falls through to what the window binds itself.
    fn act_on_key(&mut self, action: session_daw::keys::Action) -> bool {
        let Some((project_ref, _)) = self.session.as_mut() else {
            return false;
        };
        let Some(scene) = self.scene.as_ref() else {
            return false;
        };
        let project = std::sync::Arc::make_mut(&mut project_ref.0);
        let bpm = scene.bpm;
        let mut effects = Vec::new();
        let handled = self.editor.key(
            action,
            project,
            scene,
            &self.arrange_rows,
            &mut self.tracks,
            &self.arrange_map,
            bpm,
            &mut effects,
        );
        self.run(effects);
        handled
    }

    /// What the pointer is over in the arrangement, if that is the view.
    fn arrange_hit_at(&self, x: f64, y: f64) -> Option<session_daw::hit::Hit> {
        if self.view != View::Arrangement {
            return None;
        }
        let scene = self.scene.as_ref()?;
        let modes = session::modes::Mode::ALL.len();
        let (sections, markers) = self
            .session
            .as_ref()
            .map_or((&[] as &[_], &[] as &[_]), |(project, _)| {
                (project.0.sections.as_slice(), project.0.markers.as_slice())
            });
        Some(session_daw::hit::arrangement(
            scene,
            self.viewport(),
            modes,
            sections,
            markers,
            x,
            y,
        ))
    }

    /// Read the selected track's audio and find its transients.
    ///
    /// Once, not per keypress: `.` has to feel like a key, and a round
    /// trip per press would make mapping a song feel like waiting for
    /// one.
    ///
    /// Real samples through the audio accessor rather than peaks. A
    /// peak is the loudest value across a block, so a transient read
    /// off peaks is located to the nearest block — and that edge is the
    /// one measurement a tempo map is made of.
    fn read_transients(&mut self) {
        use expression_editor_audio::detect::{DetectConfig, transients};
        let Some(track) = self
            .tracks
            .iter()
            .find(|t| t.selected)
            .map(|t| t.guid.clone())
        else {
            self.transients.clear();
            return;
        };
        let Some(runtime) = session_daw::open::runtime() else {
            return;
        };
        let length = self.scene.as_ref().map_or(0.0, |s| s.length_secs);
        const RATE: f64 = 48_000.0;
        let samples = runtime.block_on(async {
            let daw = daw::rpc::Daw::try_get()?;
            let project = daw.current_project().await.ok()?;
            daw.audio()
                .mono(
                    project.guid(),
                    daw_proto::TrackRef::Guid(track),
                    0.0,
                    length,
                    RATE,
                )
                .await
                .ok()
        });
        let Some(samples) = samples else {
            self.transients.clear();
            return;
        };
        self.transients = transients(&samples, RATE, DetectConfig::default())
            .into_iter()
            .map(|hit| hit.at)
            .collect();
        tracing::debug!(found = self.transients.len(), "transients read");
    }

    /// Snap the nearest bar line to the Nth transient after it.
    ///
    /// The half of tempo mapping that does the work: rather than
    /// clicking each downbeat, you let the audio say where it is. A
    /// tune that puts a downbeat every fourth hit is `4t` once and a
    /// dot for the rest of the song.
    fn snap_to_transient(&mut self, times: u32) {
        use session_daw::tempo_map::{Move, Set, nth_transient};
        // Read before the scene is borrowed: the read needs `self` and
        // the walk below holds a reference into it.
        if self.transients.is_empty() {
            self.read_transients();
        }
        let Some(scene) = self.scene.as_ref() else {
            return;
        };
        let changes = scene.tempo();
        let at = self.editor.cursor.at;
        let beats = session_daw::ruler::Timeline::new(changes).beats(at + 600.0, 100_000);
        let lines: Vec<&session_daw::ruler::Beat> =
            beats.iter().filter(|b| b.is_downbeat()).collect();
        // The line being mapped is the one at the EDIT CURSOR: tempo
        // mapping walks forward through a song and the cursor is where
        // you have got to. Using the pointer would mean holding the
        // mouse still while typing a count.
        let Some(index) = lines
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| (a.at - at).abs().total_cmp(&(b.at - at).abs()))
            .map(|(i, _)| i)
        else {
            return;
        };
        if index == 0 {
            return;
        }
        let line = lines[index];
        let Some(to) = nth_transient(&self.transients, line.at, times) else {
            tracing::debug!(after = line.at, times, "no transient there to snap to");
            return;
        };
        let Some(marker) = changes
            .iter()
            .take_while(|c| c.at <= lines[index - 1].at + 1e-9)
            .last()
        else {
            return;
        };
        let moved = Move {
            line: line.at,
            to,
            previous_line: lines[index - 1].at,
            next_line: lines.get(index + 1).map(|b| b.at),
            marker: Set {
                at: marker.at,
                bpm: marker.bpm,
            },
        };
        self.write_tempo(moved);
        // Forward to the line just placed, so a dot maps the NEXT bar
        // rather than the same one again. Tempo mapping is a walk.
        self.editor.cursor.click(to);
    }

    /// Put the nearest bar line where the pointer is, and write the
    /// tempo that makes it true.
    ///
    /// You hear a downbeat, you click on it, and the bar line comes to
    /// you. What holds still while it moves is the modifier's to say.
    fn map_tempo_at(&mut self, x: f64) {
        use session_daw::tempo_map::{Move, Set};
        let Some(to) = self.time_at(x) else { return };
        let Some(scene) = self.scene.as_ref() else {
            return;
        };
        let changes = scene.tempo();
        let beats = session_daw::ruler::Timeline::new(changes).beats(to + 600.0, 100_000);
        let lines: Vec<&session_daw::ruler::Beat> =
            beats.iter().filter(|b| b.is_downbeat()).collect();
        let Some(index) = lines
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| (a.at - to).abs().total_cmp(&(b.at - to).abs()))
            .map(|(i, _)| i)
        else {
            return;
        };
        // The first bar line has nothing before it to stretch.
        if index == 0 {
            return;
        }
        let Some(marker) = changes
            .iter()
            .take_while(|c| c.at <= lines[index - 1].at + 1e-9)
            .last()
        else {
            return;
        };
        let moved = Move {
            line: lines[index].at,
            to,
            previous_line: lines[index - 1].at,
            next_line: lines.get(index + 1).map(|b| b.at),
            marker: Set {
                at: marker.at,
                bpm: marker.bpm,
            },
        };
        self.write_tempo(moved);
    }

    /// Work out the tempos a move needs and send them.
    ///
    /// Shared by the click and the transient snap because they differ
    /// only in how they choose the target — the anchoring, the
    /// arithmetic and the refusal are the same question either way.
    fn write_tempo(&self, moved: session_daw::tempo_map::Move) {
        use session_daw::tempo_map::{Anchor, align};
        let anchor = if self.keys.alt {
            Anchor::BothSides
        } else if self.keys.shift {
            Anchor::MeasureBefore
        } else {
            Anchor::Nothing
        };
        let Some(sets) = align(moved, anchor) else {
            // Refused rather than clamped: a clamped tempo puts the
            // line somewhere other than where you asked, silently.
            tracing::debug!(?moved, "that tempo would not be one");
            return;
        };
        for set in sets {
            self.send(session_daw::engine::Edit::SetTempo(
                String::new(),
                set.at,
                set.bpm,
            ));
        }
    }

    /// The time under an x, wherever the pointer is vertically — for
    /// continuing a drag that began on the ruler.
    fn ruler_time_unclamped(&self, x: f64) -> Option<f64> {
        (self.view == View::Arrangement)
            .then(|| self.time_at(x))
            .flatten()
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
        let frame = self.frame();
        Viewport {
            scroll_x: self.scroll_x,
            scroll_y: self.scroll_y,
            pps: self.pps,
            zoom_y: self.zoom_y,
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
                use session_daw::rename::What;
                let edit = match rename.what {
                    What::Track(guid) => {
                        let edit = session_daw::engine::Edit::Rename(guid, name);
                        let map = match rename.surface {
                            session_daw::rename::Surface::Mixer => &self.mixer_map,
                            // The ruler renames no track, so its rows
                            // are not in either map; it never reaches
                            // here with a track anyway.
                            session_daw::rename::Surface::Arrange
                            | session_daw::rename::Surface::Ruler => &self.arrange_map,
                        };
                        if let Some(index) = map.index(rename.row) {
                            apply_locally(&mut self.tracks, index, &edit);
                        }
                        // The name is recorded chrome, not a live
                        // value, so the prediction only shows once the
                        // panel is re-recorded.
                        self.re_record();
                        edit
                    }
                    // A mark's name is drawn from the project snapshot,
                    // which the refresh re-reads as soon as the engine
                    // has it — so there is nothing to predict here and
                    // nothing to re-record.
                    What::Marker(id) => {
                        session_daw::engine::Edit::RenameMarker(String::new(), id, name)
                    }
                    What::Region(id) => {
                        session_daw::engine::Edit::RenameRegion(String::new(), id, name)
                    }
                };
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
        self.commit(row, edit);
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
            C::Volume => {
                session_daw::engine::drag(session_daw::mcp::Control::Volume, &guid, track, fraction)
            }
            C::Pan => {
                session_daw::engine::drag(session_daw::mcp::Control::Pan, &guid, track, fraction)
            }
            _ => None,
        };
        let Some(edit) = mapped else { return };
        self.commit(row, edit);
    }

    /// Apply an edit here and send it to the engine — and, for a
    /// fader in a balance group, the edits that keep the group
    /// balanced with it. See `balance`.
    fn commit(&mut self, row: usize, edit: session_daw::engine::Edit) {
        use session_daw::engine::Edit;
        let companions = match &edit {
            Edit::SetVolume(guid, to) => {
                session_daw::balance::Groups::seed(&self.tracks).companions(&self.tracks, guid, *to)
            }
            _ => Vec::new(),
        };
        apply_locally(&mut self.tracks, row, &edit);
        if let Some(applier) = &self.applier {
            applier.send(edit);
        }
        for companion in companions {
            if let Edit::SetVolume(guid, _) = &companion
                && let Some(index) = self.tracks.iter().position(|t| t.guid == *guid)
            {
                apply_locally(&mut self.tracks, index, &companion);
            }
            if let Some(applier) = &self.applier {
                applier.send(companion);
            }
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
        let Some(mut spot) = self.pointer.hovered().filter(|s| s.row == row).or_else(|| {
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
        let is_folder = track.is_folder;

        // The fold at the foot of a folder's strip: a view edit, like
        // the panel's — it changes which strips exist — so it goes to
        // the folder state and re-records rather than to the engine.
        if spot.control == session_daw::mcp::Control::Folder {
            if is_folder && !double {
                self.folders.toggle(&guid);
                self.re_record();
            }
            return;
        }

        // The clip latch, which is a band across the top of the meter
        // and only a target while it is lit. Clearing it is the window
        // forgetting something, not the engine being told something —
        // so it returns here rather than becoming an edit.
        //
        // With nothing clipped there is nothing to clear and the click
        // falls through to the fader the band sits on top of, which is
        // what makes the latch cost the fader no travel at all.
        if spot.control == session_daw::mcp::Control::Clip {
            if self.clips.clear(&guid) {
                self.redraw();
                return;
            }
            spot.control = session_daw::mcp::Control::Volume;
        }

        // A double-click on a strip's name plate edits it, the same
        // gesture and the same editor the track panel uses.
        if double && spot.control == session_daw::mcp::Control::Name {
            self.rename = Some(session_daw::rename::Rename::new(
                session_daw::rename::Surface::Mixer,
                row,
                session_daw::rename::What::Track(guid),
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
        self.commit(index, edit);
    }

    /// Move along the strips, stopping at both ends.
    ///
    /// Clamped to the content rather than left free: a mixer scrolled
    /// past its last strip is a blank screen with no cue about which
    /// way to go back, and the arrangement has a ruler and a playhead
    /// to orient by where this has nothing.
    fn scroll_mixer(&mut self, by: f64) {
        let width = self.surface_size.0;
        let content = self
            .mixer
            .as_ref()
            .map_or(0.0, session_daw::mcp::Mixer::content_width);
        let most = (content - width).max(0.0);
        self.mixer_scroll = (self.mixer_scroll - by).clamp(0.0, most);
    }

    /// Which instrument the window is looking at.
    ///
    /// The selected track's, else the shown scene's, else the default
    /// — the three-deep fallback the spec asks for, so the question has
    /// an answer before anything is selected.
    fn instrument(&self) -> String {
        let rows = self.session.as_ref().map(|(_, rows)| rows.as_slice());
        let selected =
            self.tracks
                .iter()
                .find(|t| t.selected)
                .zip(rows)
                .and_then(|(track, rows)| {
                    session_daw::plan::instrument_of(rows, &self.kinds, &track.guid)
                });
        dynamic_template::scenes::follow::instrument_for(
            selected.as_deref(),
            self.visibility
                .shown()
                .and_then(dynamic_template::scenes::scene),
        )
    }

    /// The scene this window is showing, if any.
    fn shown_scene(&self) -> Option<&'static dynamic_template::scenes::Scene> {
        self.visibility
            .shown()
            .and_then(dynamic_template::scenes::scene)
    }

    /// Where this window applies a scene, for one panel.
    fn panel(
        &self,
        surface: session_daw::plan::Surface,
        extent: f64,
    ) -> session_daw::plan::Panel<'_> {
        session_daw::plan::Panel {
            surface,
            mode: Some(self.mode.slug()),
            settings: self.settings,
            extent,
            active_language: None,
        }
    }

    /// Put the window's folder state where the scene says.
    ///
    /// A scene folds folders, and the strips' fold icons have to agree
    /// with it — otherwise a click on one carries on from a state
    /// nobody is looking at.
    fn sync_folds_to_scene(&mut self) {
        let Some(scene) = self.shown_scene() else {
            return;
        };
        let (all, depths) =
            daw_ui::components::folders::FolderState::default().visible(&self.tracks);
        let rows: Vec<(daw_proto::Track, u32)> = all.into_iter().zip(depths).collect();
        self.folders = daw_ui::components::folders::FolderState::default();
        for guid in
            session_daw::plan::collapsed_by(&rows, &self.kinds, scene, Some(self.mode.slug()))
        {
            self.folders.toggle(&guid);
        }
        self.re_record();
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
            // The scene decides which strips exist and how wide each
            // one opens, so it is applied BEFORE the recording — the
            // mixer records what it is given and has never heard of a
            // scene.
            let planned = daw_ui::studio::RowsRef(std::sync::Arc::new(match self.shown_scene() {
                Some(scene) => session_daw::plan::apply_scene(
                    rows.as_slice(),
                    &self.kinds,
                    scene,
                    self.panel(session_daw::plan::Surface::Mixer, height),
                ),
                None => rows.as_slice().to_vec(),
            }));
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

    /// The frame the view now on screen is laid out in — with the
    /// dock taken off the arrangement's bottom while one is open.
    fn frame(&self) -> session_daw::rails::Frame {
        let (width, height) = self.surface_size;
        match (self.view, self.dock) {
            (View::Arrangement, Some(dock)) => {
                session_daw::rails::Frame::docked(width, height, dock)
            }
            _ => session_daw::rails::Frame::new(width, height),
        }
    }

    /// Whether a window point is on the docked editor.
    fn dock_at(&self, x: f64, y: f64) -> bool {
        self.view == View::Arrangement
            && self.dock.is_some()
            && self.expression.as_ref().is_some_and(|ex| ex.contains(x, y))
    }

    /// Whether a window point is on the dock's top edge — the grip
    /// that resizes it.
    fn dock_grip_at(&self, x: f64, y: f64) -> bool {
        let Some(dock) = self.frame().dock_box() else {
            return false;
        };
        x >= dock.x0 && x < dock.x1 && (y - dock.y0).abs() <= DOCK_GRIP
    }

    /// Dock the expression editor under the arrangement on the
    /// selected item, or close the dock.
    ///
    /// The first selected item's active take, if it has notes; the
    /// demo groove otherwise, so the editor can be exercised on a
    /// session with no MIDI in it yet. A track whose name says drums
    /// opens as a kit; anything else as a roll.
    fn toggle_dock(&mut self) {
        if self.dock.take().is_some() {
            self.dock_focus = false;
            self.dock_drag = false;
        } else {
            let (width, height) = self.surface_size;
            let dock = (height * DOCK_SHARE).max(DOCK_MIN);
            let frame = session_daw::rails::Frame::docked(width, height, dock);
            let dock_box = frame.dock_box().unwrap_or_default();
            let origin = (dock_box.x0, dock_box.y0);
            let size = (dock_box.width(), dock_box.height());
            let wanted = self.editor.selected.iter().next().cloned();
            let from_item = wanted.as_deref().and_then(|guid| {
                let scene = self.scene.as_ref()?;
                let item = scene.item_by_guid(guid)?;
                let track = self
                    .arrange_rows
                    .get(item.row)
                    .map(|(t, _)| t.name.as_str());
                let drums = track.is_some_and(session_daw::expression::is_drum_track);
                let snapshot =
                    session_daw::expression::load_take(guid, scene.bpm, item.x1 - item.x0)?;
                Some(session_daw::expression::Expression::from_take(
                    &snapshot,
                    guid.to_owned(),
                    drums,
                    origin,
                    size,
                ))
            });
            let already = self
                .expression
                .as_ref()
                .is_some_and(|ex| ex.item.is_some() && ex.item == wanted);
            match from_item {
                Some(ex) => self.expression = Some(ex),
                None if already => {}
                None => {
                    tracing::info!(
                        expression.item = wanted.as_deref().unwrap_or("none"),
                        "no MIDI take to edit; opening the demo groove"
                    );
                    if self.expression.as_ref().is_none_or(|ex| ex.item.is_some()) {
                        self.expression =
                            Some(session_daw::expression::Expression::demo(origin, size));
                    }
                }
            }
            // The window's colours, not the standalone editor's: a
            // docked roll is part of this arrangement.
            if let Some(ex) = self.expression.as_mut() {
                ex.set_look(session_daw::expression::look_of(&self.palette));
            }
            self.dock = Some(dock);
            self.dock_focus = true;
            self.view = View::Arrangement;
        }
        if let Some(window) = &self.window {
            window.set_title(self.view.title());
        }
        tracing::info!(view = ?self.view, dock = self.dock.unwrap_or(0.0), "view");
    }

    /// The album's plan, drawn inside the rails.
    ///
    /// Nothing here is recorded: the table is a few hundred rows of
    /// text that change only when the album file or the profile does,
    /// so it is painted straight into the frame like the rails are.
    /// Show a view: the title follows it, and so does the frame.
    ///
    /// One place rather than one per key, because the title and the
    /// redraw are not optional — a switch that forgot either left the
    /// window naming a view it was not drawing.
    fn show(&mut self, view: View) {
        self.view = view;
        if let Some(window) = &self.window {
            window.set_title(self.view.title());
        }
        tracing::info!(
            view = ?self.view,
            patch.unresolved = self.patch_list.table().map_or(0, |t| t.unresolved),
            "view"
        );
        self.redraw();
    }

    fn redraw_patch_list(&mut self) {
        let (width, height) = self.surface_size;
        let surface = self.palette.surface;
        let frame = session_daw::rails::Frame::new(width, height);
        let profile = session_daw::rails::profile(
            session_daw::rails::Surface::Mixer,
            self.mode,
            self.phase,
            self.visibility.shown(),
            self.settings,
            self.visibility.audience(),
            &self.instrument(),
        );
        let Self {
            renderer,
            palette,
            font,
            icons,
            patch_list,
            ..
        } = self;
        let rail_at = (None, None);
        renderer.render(|painter| {
            painter.reset();
            painter.fill(
                vello::peniko::Fill::NonZero,
                Affine::IDENTITY,
                surface,
                None,
                &vello::kurbo::Rect::new(0.0, 0.0, width, height),
            );
            session_daw::patch_list::paint_panel(
                painter,
                palette,
                font,
                patch_list,
                (session_daw::rails::SIDE, session_daw::rails::TOP),
                frame.content_width(),
            );
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
            self.visibility.shown(),
            self.settings,
            self.visibility.audience(),
            &self.instrument(),
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
            rack_scroll,
            rack_folds,
            ..
        } = self;
        let Some(mixer) = mixer.as_ref() else { return };
        let mut racks = session_daw::overlay::Racks {
            settings,
            history,
            spectra,
            lit,
            panels,
            folded: rack_folds,
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
            let at =
                Affine::translate((session_daw::rails::SIDE - scroll, session_daw::rails::TOP));
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
                &self.clips,
                *rack_scroll,
                &mut racks,
                scroll,
                frame.content_width(),
                at,
            );
            // An open rename, over the plate it replaces.
            if let Some(open) = rename {
                if let Some((left, strip_w, strip_h)) = mixer.strip_box(open.row) {
                    let _ = (strip_w, strip_h);
                    let strip = mixer.strip(open.row);
                    if let Some(field) = strip.and_then(|s| s.rect(session_daw::mcp::Control::Name))
                    {
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
                // The patch list has no rail buttons of its own yet —
                // editing it is #57 — so it borrows the mixer's, which
                // is what the frame around it is drawn with.
                View::Mixer | View::PatchList => session_daw::rails::Surface::Mixer,
            },
            self.mode,
            self.phase,
            self.visibility.shown(),
            self.settings,
            self.visibility.audience(),
            &self.instrument(),
        )
    }

    /// What a click at this point would do in the rails, if anything.
    ///
    /// Answered from the same `Profile` the rails were drawn from, so a
    /// button cannot act as the one beside it: the action is carried by
    /// the item, not looked up by index in a second table.
    fn rail_action_at(&self, x: f64, y: f64) -> Option<session_daw::rails::Action> {
        use session_daw::hit::{Side, Target};
        let frame = self.frame();
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

    /// A click on a mark in the ruler: the second one opens its name.
    ///
    /// Returns whether this click was that second one. Everything else
    /// — a click on empty lane, on the bars, anywhere but the ruler —
    /// only forgets whatever was remembered, so a double has to be two
    /// clicks on the SAME mark and nothing in between.
    fn ruler_double_click(&mut self, x: f64, y: f64) -> bool {
        use session_daw::ruler::On;
        let now = std::time::Instant::now();
        let on = match self.arrange_hit_at(x, y).map(|hit| hit.target) {
            Some(session_daw::hit::Target::Ruler { on, .. })
                if matches!(on, On::Marker { .. } | On::Region { .. }) =>
            {
                on
            }
            _ => {
                self.last_ruler_click = None;
                return false;
            }
        };
        let again = self.last_ruler_click.is_some_and(|(last, when)| {
            // A region's zone is where in the band the pointer was —
            // an edge or the body — and two clicks on one band are two
            // clicks on one band wherever they land in it.
            same_mark(last, on)
                && now.saturating_duration_since(when) <= session_daw::gesture::DOUBLE
        });
        if !again {
            self.last_ruler_click = Some((on, now));
            return false;
        }
        self.last_ruler_click = None;
        self.open_ruler_rename(on)
    }

    /// Open the name of a mark in the ruler, if it is still there.
    fn open_ruler_rename(&mut self, on: session_daw::ruler::On) -> bool {
        use session_daw::rename::{Rename, Surface, What};
        use session_daw::ruler::On;
        let Some((project, _)) = self.session.as_ref() else {
            return false;
        };
        let open = match on {
            On::Marker { id } => project.0.markers.iter().find(|m| m.idx == id).map(|m| {
                Rename::new(
                    Surface::Ruler,
                    session_daw::ruler::lane_row(m.lane),
                    What::Marker(id),
                    &m.name,
                )
            }),
            On::Region { id, .. } => project.0.sections.iter().find(|s| s.id == id).map(|s| {
                Rename::new(
                    Surface::Ruler,
                    session_daw::ruler::lane_row(s.lane),
                    What::Region(id),
                    &s.name,
                )
            }),
            On::Lane { .. } | On::Bars => None,
        };
        let opened = open.is_some();
        if opened {
            self.rename = open;
        }
        opened
    }

    /// The span of the band a hit landed on, in seconds.
    fn ruler_span_of(&self, hit: Option<session_daw::hit::Hit>) -> Option<(f64, f64)> {
        let session_daw::hit::Target::Ruler { on, .. } = hit?.target else {
            return None;
        };
        let (project, _) = self.session.as_ref()?;
        match on {
            session_daw::ruler::On::Region { id, .. } => project
                .0
                .sections
                .iter()
                .find(|section| section.id == id)
                .map(|section| (section.start, section.end)),
            session_daw::ruler::On::Marker { id } => project
                .0
                .markers
                .iter()
                .find(|marker| marker.idx == id)
                .map(|marker| (marker.at, marker.at)),
            session_daw::ruler::On::Lane { .. } | session_daw::ruler::On::Bars => None,
        }
    }

    /// The mode under a point in the corner above the track panel.
    fn mode_action_at(&self, x: f64, y: f64) -> Option<session_daw::rails::Action> {
        let modes = session::modes::Mode::ALL;
        let scene = self.scene.as_ref()?;
        let view = self.viewport();
        // Only the corner is being asked about, so the ruler's lists
        // are beside the point here.
        let hit = session_daw::hit::arrangement(scene, view, modes.len(), &[], &[], x, y);
        match hit.target {
            session_daw::hit::Target::Mode(index) => modes
                .get(index)
                .copied()
                .map(session_daw::rails::Action::Mode),
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
            A::Scene(slug) => {
                // Pressing the lit button again is how you get back to
                // the session as the project has it.
                if self.visibility.shown() == Some(slug) {
                    self.visibility.clear();
                } else {
                    // A scene chosen on the rail is a hand choice, the
                    // same as one recalled by a number key — by slug,
                    // because the button knows which scene it is.
                    self.visibility
                        .choose(dynamic_template::scenes::scenes(), slug);
                }
                self.sync_folds_to_scene();
                self.mixer = None;
                self.re_record();
            }
            A::Audience => {
                // Who the window is for, and therefore which of a
                // flow's two views its mode opens.
                // `flow.scenes.two-audiences`.
                let next = match self.visibility.audience() {
                    dynamic_template::scenes::Audience::Engineer => {
                        dynamic_template::scenes::Audience::Player
                    }
                    dynamic_template::scenes::Audience::Player => {
                        dynamic_template::scenes::Audience::Engineer
                    }
                };
                let instrument = self.instrument();
                self.visibility
                    .set_audience(dynamic_template::scenes::scenes(), next, &instrument);
                self.sync_folds_to_scene();
                self.mixer = None;
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
                // Scenes are the visibility manager and the visibility
                // manager follows the mode: entering Record shows the
                // instrument's tracking scene without a second choice
                // being made. `flow.scenes.follow-mode`.
                let instrument = self.instrument();
                self.visibility
                    .enter(dynamic_template::scenes::scenes(), mode.slug(), &instrument);
                self.sync_folds_to_scene();
                self.mixer = None;
                self.re_record();
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
    /// Notice a REAPER that has gone, and find the one that replaced it.
    ///
    /// A quit REAPER ends every stream at once, and nothing used to
    /// notice: the handles stayed, the restart check only fires when a
    /// handle is MISSING, and the window went on drawing the last thing
    /// it heard. Connected in appearance and dead in fact is the one
    /// state a mirror must not be able to reach.
    ///
    /// Tried once a second rather than once a frame. REAPER takes a
    /// while to come back, and a window that dials on every frame
    /// spends the whole wait spawning threads that immediately fail.
    fn reconnect(&mut self) {
        if !session_daw::open::is_attached() {
            return;
        }
        let dead = self.watch.as_ref().is_none_or(|w| !w.is_live())
            || self.refresh.as_ref().is_none_or(|r| !r.is_live())
            || self.meters.as_ref().is_none_or(|m| !m.is_live());
        if !dead {
            return;
        }
        let now = std::time::Instant::now();
        if self
            .last_dial
            .is_some_and(|last| now.duration_since(last) < std::time::Duration::from_secs(1))
        {
            return;
        }
        self.last_dial = Some(now);
        match session_daw::open::reattach() {
            Ok(attached) => {
                tracing::info!(
                    project = %attached.name,
                    tracks = attached.track_count,
                    "attached again"
                );
                // Dropped so the load path's restart check re-makes
                // them against the new connection, and the reload is
                // what re-reads a project that may not be the same one.
                self.watch = None;
                self.refresh = None;
                self.meters = None;
                self.reload();
            }
            Err(error) => {
                // Expected while REAPER is starting: one line, not a
                // line a second — the rate limit is above.
                tracing::debug!(error = %error, "nothing to attach to yet");
            }
        }
    }

    /// Ask for the notes of the MIDI items that have not been read.
    fn request_midi(&mut self) {
        let Some((project, _)) = self.session.as_ref() else {
            return;
        };
        let wanted: Vec<(String, f64)> = project
            .0
            .items
            .values()
            .flatten()
            .filter(|item| project.0.is_midi(&item.guid))
            .map(|item| (item.guid.clone(), item.length.as_seconds()))
            .collect();
        self.midi.fetch(wanted);
    }

    fn reconcile(&mut self) {
        // Notes that have landed since the last frame. A re-record is
        // the cheap half of a reload — it redraws from what the window
        // already holds and asks the engine for nothing.
        if self.midi.take_fresh() {
            self.re_record();
        }
        self.reconnect();
        // Anything that is not a track changed, so the snapshot every
        // item, marker, region and bar line is drawn from is old. This
        // is checked first and unconditionally: a project with no
        // track events at all still has items being dragged in it, and
        // gating it behind the track stream is how it used to be right
        // only by accident.
        // A hand still on an item is the more recent authority on where
        // it is going. Re-reading the project mid-drag would replace
        // the item under the pointer with where the engine last thought
        // it was, which is an item that jumps backwards while being
        // dragged. The flag is not cleared, so the read happens the
        // moment the hand comes off.
        if self.loading.is_none()
            && !self.editor.dragging()
            && self
                .refresh
                .as_ref()
                .is_some_and(session_daw::engine::Refresh::pending)
        {
            self.reload();
            if let Some(refresh) = &self.refresh {
                refresh.settled();
            }
        }
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
            // The applier says whether the LIST changed; asking it
            // beats re-matching the variants here, which is a second
            // list of which events are structural and would drift from
            // the first the day one is added.
            listed |= session_daw::engine::apply_event(&mut self.tracks, event)
                == session_daw::engine::Applied::Structure;
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
        // A reverb tail that finished rendering after the mixer was
        // recorded is a tail the recording does not have. One counter
        // says whether any landed; the re-record is what puts them in.
        let tails = session_daw::live::tails_generation();
        if tails != self.tails_seen {
            self.tails_seen = tails;
            self.mixer = None;
            for analyser in self.tone_spectra.values_mut() {
                analyser.invalidate();
            }
        }
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
                // The whole payload, as the engine would publish it:
                // the suppressors' reduction and the wet returns depend
                // on the track's settings, so the simulation reads them.
                let Some(tone) = self.tone_settings.get(&track.guid) else {
                    continue;
                };
                let meters = session_daw::simulate::meters(index, at, tone);
                let peak = meters.sat_peak;
                self.clips.note(
                    &track.guid,
                    daw_proto::TrackLevels {
                        peak_left: peak,
                        peak_right: peak * 0.85,
                        hold_left: peak,
                        hold_right: peak,
                    },
                );
                let history = self.tone_levels.entry(track.guid.clone()).or_default();
                history.push(peak);
                history.push_fire(meters.deess_deepest());
                history.push_ess(meters.ess_db, meters.ess_ref_db);
                self.tone_spectra
                    .entry(track.guid.clone())
                    .or_default()
                    .set(meters);
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
            let Some(level) = usize::try_from(track.index)
                .ok()
                .and_then(|i| levels.get(i))
            else {
                continue;
            };
            self.clips.note(&track.guid, *level);
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
        let shown = self.visibility.shown().map(str::to_owned);
        let kinds = self.kinds.clone();
        let mode = self.mode.slug().to_owned();
        if std::thread::Builder::new()
            .name("session-daw-reload".into())
            .spawn(move || {
                if let Some(loaded) = build_scene(&theme, layout, shown.as_deref(), &kinds, &mode) {
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
        // Then the scene, which decides which of those rows the
        // arrangement shows and how tall each one opens.
        let planned = daw_ui::studio::RowsRef(std::sync::Arc::new(match self.shown_scene() {
            Some(scene) => session_daw::plan::apply_scene(
                rows.as_slice(),
                &self.kinds,
                scene,
                self.panel(session_daw::plan::Surface::Arrange, self.surface_size.1),
            ),
            None => rows.as_slice().to_vec(),
        }));
        self.arrange_map = session_daw::plan::Rows::of(planned.as_slice(), &self.tracks);
        self.scene = Some(Arrangement::build(
            &self.palette,
            &self.font,
            &project,
            &planned,
            self.layout,
            &self.midi,
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
        if self.view == View::PatchList {
            self.redraw_patch_list();
            return;
        }
        if self.view == View::Mixer {
            self.redraw_mixer();
            return;
        }
        if self.scene.is_none() {
            return;
        }
        // What moves the view before the frame — the edge autoscroll,
        // the playhead's page turn — goes first, so the scroll the
        // frame reads is the one it draws.
        self.before_arrange_frame();
        let scroll_bars = self.scrollbars();
        let bar_held = self.bar_drag.map(|(axis, _)| axis);
        let frame = self.frame();
        // What this frame can see. Everything outside it is skipped
        // before it reaches Vello's encoder — see `Arrangement::index`.
        // The rails are excluded, or the panel draws rows behind them
        // and pays for every one.
        let view = self.viewport();
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
        let profile = session_daw::rails::profile(
            session_daw::rails::Surface::Arrange,
            self.mode,
            self.phase,
            self.visibility.shown(),
            self.settings,
            self.visibility.audience(),
            &self.instrument(),
        );
        let Some(scene) = &self.scene else { return };
        let bars = Bars::at(scene.bpm);
        // One painter for the frame, shared with the bench and every
        // shot — see `session_daw::frame`.
        let arrange = session_daw::frame::Arrange {
            scene,
            palette: &self.palette,
            font: &self.font,
            frame,
            view,
            bars,
            grid: &self.grid,
            // The rows the SCENE was recorded from — after the preset,
            // not the session's full list. The scene's row indices are
            // indices into these, and reading the full list here is how
            // a hidden track makes every row below it name the wrong
            // track.
            rows: self.arrange_rows.as_slice(),
            tracks: self.tracks.as_slice(),
            map: &self.arrange_map,
            panel: &self.panel,
            rename: self.rename.as_ref(),
            profile: &profile,
            rail_at: (self.hovered_rail, self.pressed_rail),
            icons: &mut self.icons,
            mode: self.mode,
            play_at,
            edit: self.editor.cursor,
            hovered_item: self.hovered_item,
            in_flight: self.editor.fade_in_flight(),
            selected: &self.editor.selected,
            ghost: self.editor.ghost(),
            scroll_bars,
            bar_held,
            dock: self.expression.as_mut().filter(|_| frame.dock > 0.0),
            zoom_box: self.editor.zoom_marquee(),
        };
        let mut drawn = session_daw::profile::Counts::default();
        self.renderer.render(|painter| {
            drawn = arrange.paint(painter);
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

/// The patch list for an attached project.
///
/// Filled on the loader thread once REAPER has told us where its
/// project lives. A `OnceLock` rather than a field because the window is
/// already built by the time the answer exists, and the alternative —
/// blocking the window on a connection — is the blank screen the loader
/// thread exists to avoid.
static LIVE_PATCH_LIST: std::sync::OnceLock<session_daw::patch_list::Panel> =
    std::sync::OnceLock::new();

/// The taxonomy for an attached project, filled beside the patch list.
///
/// Read from REAPER's project file rather than through ext-state calls:
/// one file read answers for every track at once, where the service
/// would be a round trip per track on a link that is not free.
static LIVE_KINDS: std::sync::OnceLock<session_daw::plan::Kinds> = std::sync::OnceLock::new();

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "warn,session_daw=info".into()),
        )
        .init();

    let Some(source) = open::source() else {
        eprintln!(
            "session-daw vello needs a project, or a REAPER to attach to:\n  \
             cargo run --bin vello -- <song.rpp>\n  \
             cargo run --bin vello -- --reaper [socket]"
        );
        std::process::exit(2);
    };

    let theme = theme::resolve().theme;
    let palette = Palette::from_theme(&theme);
    let layout = session_daw::layout::Layout::from_env();

    // The album's plan: found by walking up from the project, resolved
    // against this machine's studio profile. Read here rather than on
    // the loader thread because it is neither the project nor the
    // facade — a few file reads that cannot fail the window.
    // When this window owns the project the album is beside it on disk,
    // so the panel is read here and cannot fail the window. When it is
    // attached, the project lives wherever REAPER has it — which we
    // only learn after connecting, so that case fills the panel on the
    // loader thread instead.
    let patch_list = match &source {
        open::Source::Own(path) => session_daw::patch_list::Panel::for_project(path),
        open::Source::Reaper(_) => session_daw::patch_list::Panel::Absent,
    };
    tracing::info!(
        patch.present = patch_list.table().is_some(),
        patch.studio = patch_list.table().map_or("", |t| t.studio.as_str()),
        patch.unresolved = patch_list.table().map_or(0, |t| t.unresolved),
        "patch list"
    );

    // The taxonomy the template wrote into this project: read once,
    // here, because every scene's selectors match against it and it
    // does not change while the window is open.
    // Owned: the file is right here. Attached: REAPER holds the
    // project, and the kinds come from the same file once it has told
    // us where that is — read on the loader thread with the patch list,
    // for the same reason.
    let kinds = match &source {
        open::Source::Own(path) => session_daw::plan::Kinds::read(path),
        open::Source::Reaper(_) => session_daw::plan::Kinds::default(),
    };
    tracing::info!(scene.taxonomy = kinds.len(), "taxonomy");

    // The window opens in Mix, so it opens on Mix's scene — follow-mode
    // from the first frame rather than from the first mode change.
    let mut opening =
        dynamic_template::scenes::Follow::new(dynamic_template::scenes::Audience::Engineer);
    opening.enter(
        dynamic_template::scenes::scenes(),
        session::modes::Mode::Mix.slug(),
        dynamic_template::scenes::follow::DEFAULT_INSTRUMENT,
    );

    // The window opens now; the project fills in behind it.
    let for_loader = theme.clone();
    let kinds_for_loader = kinds.clone();
    let opening_scene = opening.shown().map(str::to_owned);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("session-daw-load".into())
        .spawn(move || {
            tracing::info!("loader thread started");
            match source {
                open::Source::Own(path) => match open::open_and_serve(&path) {
                    Ok(opened) => tracing::info!(
                        project.name = opened.name,
                        project.tracks = opened.track_count,
                        project.owned = true,
                        "project open"
                    ),
                    Err(e) => {
                        tracing::error!(error = %e, "the project did not open");
                        return;
                    }
                },
                open::Source::Reaper(socket) => match open::attach_to_reaper(socket) {
                    Ok(live) => {
                        tracing::info!(
                            project.name = live.name,
                            project.tracks = live.track_count,
                            project.owned = false,
                            project.path =
                                live.path.as_ref().map_or("", |p| p.to_str().unwrap_or("")),
                            "attached to REAPER"
                        );
                        // The album is beside REAPER's project, which we
                        // could not know until now.
                        if let Some(path) = live.path.as_deref() {
                            let _ = LIVE_PATCH_LIST
                                .set(session_daw::patch_list::Panel::for_project(path));
                            let kinds = session_daw::plan::Kinds::read(path);
                            tracing::info!(
                                scene.taxonomy = kinds.len(),
                                "taxonomy read from the live project"
                            );
                            let _ = LIVE_KINDS.set(kinds);
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "could not attach to REAPER");
                        return;
                    }
                },
            }
            // The facade is up; read it and record the scene.
            match build_scene(
                &for_loader,
                layout,
                opening_scene.as_deref(),
                &kinds_for_loader,
                session::modes::Mode::Mix.slug(),
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
        zoom_y: 1.0,
        zoom_held: false,
        grid_held: false,
        repeat: session_daw::repeat::Repeat::default(),
        transients: Vec::new(),
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
        patch_list,
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
        editor: session_daw::arrange_edit::Editor::default(),
        expression: None,
        dock: None,
        dock_focus: false,
        dock_drag: false,
        hovered_item: None,
        keys: session_daw::mousemap::Mods::default(),
        bar_drag: None,
        follow: !std::env::var("FTS_FOLLOW")
            .is_ok_and(|v| matches!(v.trim(), "off" | "0" | "false")),
        bindings: session_daw::keys::Keys::load(),
        pressed_row: None,
        hovered_rail: None,
        pressed_rail: None,
        row_drag: None,
        watch: session_daw::engine::Watch::start(),
        refresh: session_daw::engine::Refresh::start(),
        midi: session_daw::midi::Previews::default(),
        last_dial: None,
        meters: session_daw::engine::Meters::start(),
        mode: session::modes::Mode::Mix,
        phase: session::mix_phases::MixPhase::Tone,
        visibility: opening,
        kinds,
        settings: session_daw::settings::Settings::default(),
        rename: None,
        last_ruler_click: None,
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
        tails_seen: 0,
        tone_settings: session_daw::tone::Store::default(),
        clips: session_daw::overlay::Clips::default(),
        // A starting scroll, for shots and for looking at a processor
        // that lives past the fold. `FTS_VELLO_RACK_SCROLL=600`.
        // Synced by default: the chain is read across the mixer, and
        // folds that differed per strip would put a different processor
        // at the same height on every track.
        rack_folds: session_daw::tone::Fold::rest(),
        rack_scroll: std::env::var("FTS_VELLO_RACK_SCROLL")
            .ok()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .unwrap_or(0.0)
            .max(0.0),
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
    shown: Option<&str>,
    kinds: &session_daw::plan::Kinds,
    mode: &str,
) -> Option<Loaded> {
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
    let palette = Palette::from_theme(theme);
    let font = session_daw::text::Font::embedded().ok()?;
    // The opening scene is applied HERE rather than by the window, so
    // the first arrangement the window shows is already the one the
    // scene asks for. Recording it twice — once plain, once planned —
    // is four milliseconds nobody sees and a frame of the wrong layout
    // that they do.
    let planned = daw_ui::studio::RowsRef(std::sync::Arc::new(
        match shown.and_then(dynamic_template::scenes::scene) {
            Some(scene) => session_daw::plan::apply_scene(
                rows.as_slice(),
                kinds,
                scene,
                session_daw::plan::Panel {
                    surface: session_daw::plan::Surface::Arrange,
                    mode: Some(mode),
                    settings: session_daw::settings::Settings::default(),
                    extent: 0.0,
                    active_language: None,
                },
            ),
            None => rows.as_slice().to_vec(),
        },
    ));
    // The project and its rows come back with the arrangement, because
    // the mixer is recorded against the WINDOW's height and that is not
    // known here — see `App::mixer_for`.
    Some(Loaded {
        // The loader thread has no cache to consult — the notes are
        // read after the window is up, and the first recording draws
        // MIDI items plain until they land.
        arrangement: Arrangement::build(
            &palette,
            &font,
            &project,
            &planned,
            layout,
            &session_daw::midi::Previews::default(),
        ),
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
/// A key as the expression editor's keymap names it — the browser's
/// names, which is what its bindings were written against: `"Delete"`,
/// `"ArrowLeft"`, `"F2"`, and the character itself for the rest.
fn expression_key_name(key: &winit::keyboard::Key) -> Option<String> {
    match key {
        winit::keyboard::Key::Named(named) => Some(format!("{named:?}")),
        _ => key.to_text().map(str::to_owned),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Arrangement,
    Mixer,
    /// The album's patch list — read-only here; editing and applying
    /// are session #57.
    PatchList,
}

impl View {
    /// `x` flips the two working views. From the patch list it returns
    /// to the arrangement, because the plan is a place you look at, not
    /// one of the two you work in.
    const fn toggled(self) -> Self {
        match self {
            Self::Arrangement => Self::Mixer,
            Self::Mixer | Self::PatchList => Self::Arrangement,
        }
    }

    const fn title(self) -> &'static str {
        match self {
            Self::Arrangement => "Session — arrangement (Vello)",
            Self::Mixer => "Session — mixer (Vello)",
            Self::PatchList => "Session — patch list (Vello)",
        }
    }
}

/// The dock's height when it first opens, as a share of the window.
const DOCK_SHARE: f64 = 0.4;
/// The least the dock can be dragged to, and the least the arrangement
/// keeps above it.
const DOCK_MIN: f64 = 160.0;
/// How near the dock's top edge a press takes hold of it to resize.
const DOCK_GRIP: f64 = 4.0;

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
        Edit::SetInputMonitor(_, mode) => track.input_monitor = *mode,
        Edit::SetParentSend(_, enabled) => track.parent_send = *enabled,
        Edit::SetColor(_, color) => track.color = Some(*color),
        Edit::SetAutomationMode(_, mode) => track.automation_mode = *mode,
        Edit::SetRecordInput(_, input) => track.record_input = *input,
        Edit::SetVisibility(_, tcp, mixer) => {
            track.visible_in_tcp = *tcp;
            track.visible_in_mixer = *mixer;
        }
        Edit::SetHeight(_, pixels) => track.height = Some(*pixels),
        // Not predicted. Folder depth moves every track BELOW this one
        // between levels, and grouping is a matrix the engine resolves
        // — REAPER decides what a lead in one family does to the rest.
        // Predicting either means predicting the engine's whole answer,
        // and being wrong about the shape of the session looks far
        // worse than one frame of waiting for the right one.
        Edit::SetFolderDepth(..)
        | Edit::SetGroupMembership(..)
        | Edit::SetGroupFlags(..)
        | Edit::SetGroupModifier(..) => {}
        // The ruler's, not a track's. Nothing is predicted: a marker
        // or a region is drawn from the project snapshot, which the
        // refresh re-reads the moment the engine has it. A ghost
        // during the drag is the window's own, and it is drawn by the
        // ruler rather than patched into the list here.
        Edit::AddMarker(..)
        | Edit::MoveMarker(..)
        | Edit::RenameMarker(..)
        | Edit::RemoveMarker(..)
        | Edit::AddRegion(..)
        | Edit::SetRegionBounds(..)
        | Edit::RenameRegion(..)
        | Edit::RemoveRegion(..)
        // The tempo is the project's, and the ruler redraws from the
        // snapshot the refresh brings back.
        | Edit::SetTempo(..) => {}
        // An item's, not the track's: applied to the project copy where
        // the drag ends — see `commit_fade`.
        Edit::SetFadeIn(..)
        | Edit::SetFadeOut(..)
        | Edit::SelectItem(..)
        | Edit::DeselectAllItems(_)
        | Edit::SelectAllItems(_)
        | Edit::MoveItem(..)
        | Edit::TrimItem(..)
        | Edit::SplitItem(..)
        | Edit::DeleteItem(_) => {}
        // Selection is the engine's to decide: an exclusive select
        // changes every OTHER track too, and predicting which ones
        // stop being selected would be predicting the engine's whole
        // answer. Getting that wrong looks worse than a frame of lag
        // looks slow — and the frame is one round trip in-process.
        Edit::Select(_) | Edit::AddToSelection(_) => {}
    }
}

/// The size to open at, as `FTS_VELLO_SIZE=WxH`.
///
/// The window is what the mixer is actually judged in, and a shot of it
/// is only worth reading at the resolution it will be used at — so the
/// size is a knob rather than a number compiled in. Same spelling as
/// the bench's `FTS_BENCH_SIZE`, because they are asked the same
/// question.
fn window_size() -> (f64, f64) {
    const DEFAULT: (f64, f64) = (1600.0, 900.0);
    let Ok(value) = std::env::var("FTS_VELLO_SIZE") else {
        return DEFAULT;
    };
    let Some((w, h)) = value.split_once(['x', 'X']) else {
        return DEFAULT;
    };
    match (w.trim().parse::<f64>(), h.trim().parse::<f64>()) {
        (Ok(w), Ok(h)) if w >= 320.0 && h >= 240.0 => (w, h),
        // A size the window could not show anything in is a typo, not
        // an instruction.
        _ => DEFAULT,
    }
}

/// Whether two ruler hits are the same mark.
///
/// A region is the same band wherever in it the pointer landed: the
/// zone says edge or body, and a double-click that started on the body
/// and finished a pixel into the edge is still a double-click on that
/// band.
const fn same_mark(a: session_daw::ruler::On, b: session_daw::ruler::On) -> bool {
    use session_daw::ruler::On;
    match (a, b) {
        (On::Marker { id: one }, On::Marker { id: two })
        | (On::Region { id: one, .. }, On::Region { id: two, .. }) => one == two,
        _ => false,
    }
}
