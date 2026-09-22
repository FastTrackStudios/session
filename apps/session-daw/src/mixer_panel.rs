//! The Mixer panel: the session's strips, docked under the arrangement,
//! toggled by the profile's "Toggle mixer" (`x`, REAPER's 40078), the way
//! REAPER docks its mixer.
//!
//! The painting, hit test and edits are the ones the painted window
//! (`bin/vello.rs`) has always used: [`crate::mcp::Mixer`] records the
//! strips' chrome, [`crate::overlay::controls`] draws the values over it
//! every frame (so a fader moves without re-recording anything), and
//! [`crate::engine::click`] / [`crate::engine::drag`] turn a gesture into
//! an edit. What is new is only the widget holding them, and the
//! [`Links`] it shares with the arrangement:
//!
//! - **rows**: the mixer shows the arrangement's rows, so a group the
//!   visibility manager hides leaves both;
//! - **edits** each way: a widget applies its own edits at once (a fader
//!   that waited for the engine would lag the hand), so the panel hands
//!   each widget's edits to the OTHER one to apply too. Never back to the
//!   one that made them, or a toggle would flip twice and do nothing;
//! - **open**, and a toggle either widget can ask for.
//!
//! v1 leaves out what the painted window's mixer has beyond that: the
//! routing panel, renaming, folding a folder's strip, the Tone rack.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use anyrender::{RenderContext, Scene};
use blitz_dom::node::{ComputedStyles, Widget};
use blitz_traits::events::{MouseEventButton, UiEvent};
use dioxus::prelude::*;
use vello::kurbo::Affine;

use crate::engine::Edit;
use crate::mcp::Control;
use crate::pointer::{Pointer, Spot};
use crate::studio::StudioSession;

/// How tall the docked mixer is: the strip height the mixer was designed
/// at.
pub const HEIGHT: f64 = crate::mcp::DEFAULT_HEIGHT;

type Rows = Vec<(daw_proto::Track, u32)>;
type Queue = Rc<RefCell<Vec<Edit>>>;

/// What the arrangement and the mixer share. Provided by [`DawPanels`];
/// the arrangement takes it if it is there.
#[derive(Clone)]
pub struct Links {
    /// Whether the mixer is showing.
    pub open: Signal<bool>,
    /// Asked for by a widget's key (`x`), carried out by the arrangement
    /// panel once a frame, since a widget cannot set a signal.
    pub toggle: Rc<Cell<bool>>,
    /// The arrangement's rows, with a generation that goes up on every
    /// change, so the mixer knows to re-record.
    pub rows: Rc<RefCell<(u64, Rows)>>,
    /// Edits the mixer made, for the engine and the arrangement.
    pub from_mixer: Queue,
    /// Edits the arrangement made, for the mixer to apply too.
    pub to_mixer: Queue,
    /// Edits the mixer made, for the arrangement to apply too.
    pub to_arrange: Queue,
    /// Keys pressed while the mixer has the focus, for the arrangement:
    /// the keyboard is one keymap whichever panel was clicked last, and
    /// the arrangement is where it is read.
    pub keys: Rc<RefCell<Vec<Key>>>,
    /// The arrangement's node. A click in the mixer takes the focus off
    /// it without giving it to anything that reads keys, so the mixer
    /// hands it straight back: the keyboard is the arrangement's.
    pub arrange_node: Rc<RefCell<Option<Rc<MountedData>>>>,
}

/// A key the mixer passed on.
#[derive(Clone, Debug)]
pub enum Key {
    Down(blitz_traits::events::BlitzKeyEvent),
    Up(blitz_traits::events::BlitzKeyEvent),
}

impl Links {
    fn new(rows: Rows) -> Self {
        Self {
            open: Signal::new(false),
            toggle: Rc::new(Cell::new(false)),
            rows: Rc::new(RefCell::new((0, rows))),
            from_mixer: Queue::default(),
            to_mixer: Queue::default(),
            to_arrange: Queue::default(),
            keys: Rc::default(),
            arrange_node: Rc::default(),
        }
    }
}

/// The DAW view's panels: the arrangement, and the mixer under it when
/// it is open.
#[cfg(feature = "native")]
#[component]
pub fn DawPanels(
    /// Open the mixer from the start — the Overview, where the two are one
    /// docked pair rather than a panel `x` summons.
    #[props(default)]
    docked: bool,
) -> Element {
    let session: StudioSession = use_context();
    let links = use_context_provider(|| {
        let mut links = Links::new(session.rows.as_slice().to_vec());
        links.open = Signal::new(docked);
        links
    });
    let open = (links.open)();
    let arrange_bottom = if open { HEIGHT } else { 0.0 };
    let mixer_display = if open { "block" } else { "none" };
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:{arrange_bottom}px;",
            crate::studio::Arrangement {}
        }
        div {
            style: "display:{mixer_display}; position:absolute; left:0; right:0; bottom:0; \
                    height:{HEIGHT}px; border-top:1px solid #000;",
            Mixer {}
        }
    }
}

/// The mixer panel. No props: the session and the [`Links`] come from
/// context.
#[cfg(feature = "native")]
#[component]
pub fn Mixer() -> Element {
    let links: Links = use_context();
    // Live mode: live strips (a short band) — see `strip::shape`.
    let mode: Option<Signal<session::modes::Mode>> = try_use_context();
    let live_mode = mode.is_some_and(|mode| mode() == session::modes::Mode::Live);
    let live = use_hook(|| Rc::new(Cell::new(false)));
    live.set(live_mode);
    let scroll = use_hook(|| Rc::new(Cell::new(0.0_f64)));
    let content_w = use_hook(|| Rc::new(Cell::new(0.0_f64)));
    let widget = use_hook(|| {
        dioxus_native_dom::CustomWidgetAttr::new(MixerWidget::new(
            links.clone(),
            Rc::clone(&scroll),
            Rc::clone(&content_w),
            Rc::clone(&live),
        ))
    });

    // The wheel, which Blitz does not send to the DOM: read at the window
    // and taken only over this panel's rectangle. Either direction runs
    // along the strips.
    let mut rect = use_signal(|| (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64));
    let mounted = use_hook(|| Rc::new(RefCell::new(None::<Rc<MountedData>>)));
    let measured = use_hook(|| Rc::new(Cell::new(None::<web_time::Instant>)));
    let pointer = use_hook(|| Rc::new(Cell::new((0.0_f64, 0.0_f64))));
    let measuring = Rc::clone(&mounted);
    let scrolling = Rc::clone(&scroll);
    let arrange_node = Rc::clone(&links.arrange_node);
    let refocus = use_hook(|| Rc::new(Cell::new(false)));
    dioxus_native::use_window_event(move |event, _| match event {
        // A press here: hand the keyboard back to the arrangement. Not
        // from this event: the document is still borrowed while Blitz
        // handles it, and focusing from inside that panics ("RefCell
        // already borrowed"). The next redraw does it, as the rect is
        // read.
        winit::event::WindowEvent::PointerButton { state, .. } if state.is_pressed() => {
            let r = *rect.peek();
            let (x, y) = pointer.get();
            if x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3 {
                refocus.set(true);
            }
        }
        winit::event::WindowEvent::PointerMoved { position, .. } => {
            pointer.set((position.x, position.y));
        }
        winit::event::WindowEvent::MouseWheel { delta, .. } => {
            let r = *rect.peek();
            let (x, y) = pointer.get();
            if !(x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3) {
                return;
            }
            let (dx, dy) = match delta {
                winit::event::MouseScrollDelta::LineDelta(x, y) => {
                    (f64::from(*x) * 40.0, f64::from(*y) * 40.0)
                }
                winit::event::MouseScrollDelta::PixelDelta(at) => (at.x, at.y),
            };
            let most = (content_w.get() - r.2).max(0.0);
            scrolling.set((scrolling.get() - dx - dy).clamp(0.0, most));
        }
        winit::event::WindowEvent::RedrawRequested => {
            if refocus.take()
                && let Some(node) = arrange_node.borrow().clone()
            {
                spawn(async move {
                    let _ = node.set_focus(true).await;
                });
            }
            let stale = measured
                .get()
                .is_none_or(|at| at.elapsed() > std::time::Duration::from_millis(200));
            if stale && let Some(node) = measuring.borrow().clone() {
                measured.set(Some(web_time::Instant::now()));
                spawn(async move {
                    if let Ok(got) = node.get_client_rect().await {
                        let next = (got.origin.x, got.origin.y, got.size.width, got.size.height);
                        if next != *rect.peek() {
                            rect.set(next);
                        }
                    }
                });
            }
        }
        _ => {}
    });

    let colors = daw_ui::studio::lanes::Colors::from_theme(&daw_ui::theming::Theme::dark());
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; overflow:hidden; \
                    background:{colors.surface};",
            onmounted: move |event| {
                *mounted.borrow_mut() = Some(event.data());
            },

            object {
                style: "position:absolute; left:0; top:0; width:100%; height:100%;",
                tabindex: "0",
                data: widget.clone(),
            }
        }
    }
}

/// The DAW view's panels in a browser: [`DawPanels`], with the widgets in
/// canvases.
#[cfg(feature = "web")]
#[component]
pub fn WebDawPanels(
    engine: crate::web_engine::EngineRef,
    /// Open the mixer from the start — the Overview, where the two are one
    /// docked pair rather than a panel `x` summons.
    #[props(default)]
    docked: bool,
) -> Element {
    let session: StudioSession = use_context();
    let links = use_context_provider(|| {
        let mut links = Links::new(session.rows.as_slice().to_vec());
        links.open = Signal::new(docked);
        links
    });
    let open = (links.open)();
    let arrange_bottom = if open { HEIGHT } else { 0.0 };
    // Hidden rather than removed: kept at its size, the mixer builds its
    // strips and its GPU context at load, so `x` opens it at once instead
    // of after the second a first build takes.
    let (visibility, events) = if open {
        ("visible", "auto")
    } else {
        ("hidden", "none")
    };
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:{arrange_bottom}px;",
            crate::web_host::WebArrangement { engine }
        }
        div {
            style: "visibility:{visibility}; pointer-events:{events}; position:absolute; \
                    left:0; right:0; bottom:0; height:{HEIGHT}px; border-top:1px solid #000;",
            WebMixer { hidden: !open }
        }
    }
}

/// The mixer panel in a browser. The wheel runs along the strips; a press
/// hands the keyboard back to the arrangement, as [`Mixer`] does.
#[cfg(feature = "web")]
#[component]
pub fn WebMixer(hidden: bool) -> Element {
    use crate::panel::PanelEvent;
    let links: Links = use_context();
    let mode: Option<Signal<session::modes::Mode>> = try_use_context();
    let live_mode = mode.is_some_and(|mode| mode() == session::modes::Mode::Live);
    let live = use_hook(|| Rc::new(Cell::new(false)));
    live.set(live_mode);
    let scroll = use_hook(|| Rc::new(Cell::new(0.0_f64)));
    let content_w = use_hook(|| Rc::new(Cell::new(0.0_f64)));
    let element = use_hook(|| Rc::new(RefCell::new(None::<web_sys::HtmlElement>)));
    let widget = use_hook(|| {
        crate::web_host::HostedRef(Rc::new(RefCell::new(MixerWidget::new(
            links.clone(),
            Rc::clone(&scroll),
            Rc::clone(&content_w),
            Rc::clone(&live),
        ))))
    });
    let slot = crate::web_host::ElementSlot(Rc::clone(&element));
    let arrange_node = Rc::clone(&links.arrange_node);
    let on_input = move |event: PanelEvent| match event {
        PanelEvent::Wheel { dx, dy } => {
            let width = element
                .borrow()
                .as_ref()
                .map_or(0.0, |el| el.get_bounding_client_rect().width());
            let most = (content_w.get() - width).max(0.0);
            scroll.set((scroll.get() - dx - dy).clamp(0.0, most));
        }
        PanelEvent::Button { pressed: false, .. } => {
            if let Some(node) = arrange_node.borrow().clone() {
                spawn(async move {
                    let _ = node.set_focus(true).await;
                });
            }
        }
        _ => {}
    };
    let colors = daw_ui::studio::lanes::Colors::from_theme(&daw_ui::theming::Theme::dark());
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; overflow:hidden; \
                    background:{colors.surface};",
            crate::web_host::WidgetCanvas { widget, panel: on_input, element: Some(slot), hidden }
        }
    }
}

#[cfg(feature = "web")]
impl crate::web_host::Hosted for MixerWidget {
    fn paint(&mut self, width: u32, height: u32, scale: f64) -> Scene {
        self.paint_scene(width, height, scale)
    }
    fn event(&mut self, event: &UiEvent) {
        MixerWidget::event(self, event);
    }
}

/// A fader or pan knob mid-drag.
struct Turn {
    spot: Spot,
    from_y: f64,
    /// The track as it was when the drag began: the drag is relative to
    /// where the value started, not to where it has got to.
    was: daw_proto::Track,
}

/// The widget: the recorded strips and what the pointer is doing to them.
struct MixerWidget {
    links: Links,
    palette: crate::arrangement::Palette,
    font: crate::text::Font,
    layout: crate::layout::Layout,
    /// The rows as last taken from [`Links::rows`], and its generation.
    rows: Rows,
    generation: Option<u64>,
    /// The live values the overlay draws, and which track each strip is.
    tracks: Vec<daw_proto::Track>,
    map: crate::plan::Rows,
    mixer: Option<crate::mcp::Mixer>,
    scroll: Rc<Cell<f64>>,
    content_w: Rc<Cell<f64>>,
    pointer: Pointer<Spot>,
    turning: Option<Turn>,
    /// The latest value of a drag in flight, not yet sent (see
    /// [`MixerWidget::flush_drag`]).
    dragged: Option<Edit>,
    clips: crate::overlay::Clips,
    meters: Option<crate::engine::Meters>,
    /// Live-mode strips, as the panel last said, and as the recording
    /// was built.
    live: Rc<Cell<bool>>,
    built_live: bool,
    dirty: Cell<bool>,
}

impl MixerWidget {
    fn new(
        links: Links,
        scroll: Rc<Cell<f64>>,
        content_w: Rc<Cell<f64>>,
        live: Rc<Cell<bool>>,
    ) -> Self {
        let theme = daw_ui::theming::Theme::dark();
        Self {
            links,
            palette: crate::arrangement::Palette::from_theme(&theme),
            font: crate::text::Font::embedded().expect("the embedded font"),
            layout: crate::layout::Layout::from_env(),
            rows: Vec::new(),
            generation: None,
            tracks: Vec::new(),
            map: crate::plan::Rows::of(&[], &[]),
            mixer: None,
            scroll,
            content_w,
            pointer: Pointer::default(),
            turning: None,
            dragged: None,
            clips: crate::overlay::Clips::default(),
            meters: crate::engine::Meters::start(),
            live,
            built_live: false,
            dirty: Cell::new(false),
        }
    }

    /// Take the arrangement's rows when they have changed, and the edits
    /// it has made since the last frame.
    fn catch_up(&mut self) {
        {
            let shared = self.links.rows.borrow();
            if self.generation != Some(shared.0) {
                self.generation = Some(shared.0);
                self.rows.clone_from(&shared.1);
                self.tracks = self.rows.iter().map(|(t, _)| t.clone()).collect();
                self.map = crate::plan::Rows::of(&self.rows, &self.tracks);
                self.mixer = None;
                self.pointer = Pointer::default();
                self.turning = None;
            }
        }
        let echoed: Vec<Edit> = self.links.to_mixer.borrow_mut().drain(..).collect();
        for edit in &echoed {
            predict(&mut self.tracks, edit);
        }
    }

    /// The control under a point in the widget's own coordinates.
    fn spot_at(&self, x: f64, y: f64) -> Option<Spot> {
        let mixer = self.mixer.as_ref()?;
        let content_x = x + self.scroll.get();
        let row = mixer.strip_at(content_x)?;
        let (left, _, _) = mixer.strip_box(row)?;
        let control = crate::mcp::control_at(mixer, row, content_x - left, y)?;
        Some(Spot { row, control })
    }

    /// Send the drag's latest value on, if it has moved since the last.
    fn flush_drag(&mut self) {
        if let Some(edit) = self.dragged.take() {
            self.links.from_mixer.borrow_mut().push(edit);
        }
    }

    /// Do an edit here, and send it on: to the engine, and to the
    /// arrangement.
    fn commit(&mut self, edit: Edit) {
        predict(&mut self.tracks, &edit);
        self.links.from_mixer.borrow_mut().push(edit);
    }

    fn track_at(&self, row: usize) -> Option<&daw_proto::Track> {
        self.map.live(&self.tracks, row)
    }

    fn took(&mut self, event: &UiEvent) -> bool {
        let at = |e: &blitz_traits::events::BlitzPointerEvent| {
            (f64::from(e.coords.client_x), f64::from(e.coords.client_y))
        };
        match event {
            UiEvent::PointerMove(e) => {
                let (x, y) = at(e);
                if let Some(turn) = &self.turning {
                    let travel = self
                        .mixer
                        .as_ref()
                        .and_then(|m| m.strip_box(turn.spot.row))
                        .map_or(1.0, |(_, _, h)| h * 0.4);
                    // `drag_fraction` takes the screen delta and makes up
                    // positive itself.
                    let fraction = crate::gesture::drag_fraction(y - turn.from_y, travel);
                    let edit = crate::engine::drag(turn.spot.control, &turn.was.guid, &turn.was, fraction);
                    if let Some(edit) = edit {
                        // Shown now, sent once a frame: a drag moves the
                        // pointer many times a frame, and every edit sent
                        // is an engine call and a re-record of the
                        // arrangement's controls.
                        predict(&mut self.tracks, &edit);
                        self.dragged = Some(edit);
                    }
                    return true;
                }
                let spot = self.spot_at(x, y);
                self.pointer.hover(spot)
            }
            UiEvent::PointerDown(e) if e.button == MouseEventButton::Main => {
                let (x, y) = at(e);
                let spot = self.spot_at(x, y);
                self.pointer.hover(spot);
                self.pointer.press();
                if let Some(spot) = spot.filter(|s| s.control.is_continuous())
                    && let Some(track) = self.track_at(spot.row)
                {
                    self.turning = Some(Turn {
                        spot,
                        from_y: y,
                        was: track.clone(),
                    });
                }
                true
            }
            UiEvent::PointerUp(e) if e.button == MouseEventButton::Main => {
                let (x, y) = at(e);
                if self.turning.take().is_some() {
                    self.flush_drag();
                    self.pointer.release();
                    self.pointer.hover(self.spot_at(x, y));
                    return true;
                }
                let up = self.spot_at(x, y);
                if let Some(spot) = self.pointer.pressed()
                    && up == Some(spot)
                    && let Some(track) = self.track_at(spot.row).cloned()
                {
                    // The clip latch clears on a click while it is lit;
                    // otherwise the band is the fader under it.
                    let control = match spot.control {
                        Control::Clip if self.clips.clear(&track.guid) => None,
                        Control::Clip => Some(Control::Volume),
                        other => Some(other),
                    };
                    let edit =
                        control.and_then(|c| crate::engine::click(c, &track.guid, &track, false));
                    if let Some(edit) = edit {
                        self.commit(edit);
                    }
                }
                self.pointer.release();
                self.pointer.hover(up);
                true
            }
            UiEvent::PointerCancel(_) => {
                self.flush_drag();
                self.turning = None;
                self.pointer.release();
                true
            }
            UiEvent::KeyDown(e) => {
                self.links.keys.borrow_mut().push(Key::Down(e.clone()));
                false
            }
            UiEvent::KeyUp(e) => {
                self.links.keys.borrow_mut().push(Key::Up(e.clone()));
                false
            }
            _ => false,
        }
    }
}

/// What an edit does to the track it names, predicted here rather than
/// waited for (the engine is in-process but not instant).
fn predict(tracks: &mut [daw_proto::Track], edit: &Edit) {
    let (guid, change): (&str, &dyn Fn(&mut daw_proto::Track)) = match edit {
        Edit::ToggleMute(guid) => (guid, &|t| t.muted = !t.muted),
        Edit::ToggleSolo(guid) => (guid, &|t| t.soloed = !t.soloed),
        Edit::ToggleArm(guid) => (guid, &|t| t.armed = !t.armed),
        Edit::SetPhase(guid, on) => (guid, &|t| t.phase_inverted = *on),
        Edit::SetVolume(guid, gain) => (guid, &|t| t.volume = *gain),
        Edit::SetPan(guid, pan) => (guid, &|t| t.pan = *pan),
        Edit::Rename(guid, name) => (guid, &|t| t.name.clone_from(name)),
        Edit::SetInputMonitor(guid, mode) => (guid, &|t| t.input_monitor = *mode),
        _ => return,
    };
    for track in tracks.iter_mut().filter(|t| t.guid == guid) {
        change(track);
    }
}

impl MixerWidget {
    /// One input event: what Blitz's `Widget::handle_event` calls, and what
    /// the web host calls.
    pub fn event(&mut self, event: &UiEvent) {
        let changed = self.took(event);
        self.dirty.set(changed);
    }

    /// The picture: what Blitz's `Widget::paint` returns, and what the web
    /// host draws into its canvas.
    pub fn paint_scene(&mut self, width: u32, height: u32, _scale: f64) -> Scene {
        self.dirty.set(false);
        self.flush_drag();
        self.catch_up();
        let (w, h) = (f64::from(width), f64::from(height));
        let mut out = Scene::new();
        if w < 1.0 || h < 1.0 {
            return out;
        }
        let live = self.live.get();
        let stale = self
            .mixer
            .as_ref()
            .is_none_or(|m| (m.height - h).abs() > 0.5)
            || live != self.built_live;
        if stale {
            self.built_live = live;
            let project = daw_ui::studio::ProjectRef(std::sync::Arc::new(
                daw_ui::studio::project::Project::default(),
            ));
            let rows = daw_ui::studio::RowsRef(std::sync::Arc::new(self.rows.clone()));
            self.mixer = Some(crate::mcp::Mixer::build(
                &self.palette,
                &self.font,
                &project,
                &rows,
                h,
                self.layout,
                &[],
                false,
                crate::settings::Settings {
                    live_strips: live,
                    ..crate::settings::Settings::default()
                },
                &crate::tone::Store::default(),
            ));
        }
        let Some(mixer) = self.mixer.as_ref() else {
            return out;
        };
        self.content_w.set(mixer.content_width());
        let most = (mixer.content_width() - w).max(0.0);
        let scroll = self.scroll.get().clamp(0.0, most);
        self.scroll.set(scroll);
        let levels = self
            .meters
            .as_ref()
            .map(crate::engine::Meters::levels)
            .unwrap_or_default();
        let at = Affine::translate((-scroll, 0.0));
        mixer.replay(&mut out, scroll, w, at);
        crate::overlay::controls(
            &mut out,
            &self.palette,
            &self.font,
            mixer,
            &self.tracks,
            &self.map,
            &self.pointer,
            &levels,
            &self.clips,
            0.0,
            &mut crate::overlay::Racks::none(),
            scroll,
            w,
            at,
        );
        out
    }
}

impl Widget for MixerWidget {
    fn handle_event(&mut self, event: &UiEvent) {
        self.event(event);
    }

    fn needs_redraw(&self) -> bool {
        self.dirty.get()
    }

    fn paint(
        &mut self,
        _render_ctx: &mut dyn RenderContext,
        _styles: &ComputedStyles,
        width: u32,
        height: u32,
        scale: f64,
    ) -> Scene {
        self.paint_scene(width, height, scale)
    }
}
