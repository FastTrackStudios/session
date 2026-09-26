//! The Mixer panel: the session's strips, docked under the arrangement,
//! toggled by the profile's "Toggle mixer" (`x`, REAPER's 40078), the way
//! REAPER docks its mixer.
//!
//! The painting, hit test and edits are the mixer's own:
//! [`crate::mcp::Mixer`] records the
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
//! v1 leaves out what the retired winit window's mixer had beyond that:
//! the routing panel, renaming, folding a folder's strip, the Tone rack,
//! and balance groups (`crate::balance`, whose rule is implemented and
//! tested but not wired to a fader here yet).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use anyrender::{RenderContext, Scene};
use blitz_dom::node::{ComputedStyles, Widget};
use blitz_traits::events::{BlitzPointerId, MouseEventButton, UiEvent};
use dioxus::prelude::*;
use vello::kurbo::Affine;

use crate::engine::Edit;
use crate::mcp::Control;
use crate::pointer::{Pointer, Spot};
use crate::studio::StudioSession;

/// How tall the docked mixer is: the strip height the mixer was designed
/// at.
pub const HEIGHT: f64 = crate::mcp::DEFAULT_HEIGHT;

/// Where the arrangement stops and how tall the mixer under it is, as CSS
/// lengths.
///
/// Alone, the mixer fills the panel. On a finger's screen the two share
/// it — evenly in the Overview, a little more to the mixer in the DAW
/// view: an iPad's pane is some six hundred pixels, and a fixed mixer at
/// touch size left the arrangement a strip a third of a row tall.
/// Otherwise the mixer is its own height, in pixels.
fn split(open: bool, mixer_only: bool, docked: bool, touch: bool) -> (String, String) {
    if mixer_only {
        return ("0px".to_owned(), "100%".to_owned());
    }
    let mixer = if touch {
        // The Overview's pane is shared with the chart beside it, so the
        // arrangement keeps half; the DAW view's is the arrangement's own,
        // and the mixer, opened there, gets the larger share.
        if docked { "50%" } else { "55%" }.to_owned()
    } else {
        format!("{HEIGHT}px")
    };
    let bottom = if open {
        mixer.clone()
    } else {
        "0px".to_owned()
    };
    (bottom, mixer)
}

type Rows = Vec<(daw_proto::Track, u32)>;
type Queue = Rc<RefCell<Vec<Edit>>>;

/// What the arrangement and the mixer share. Provided by [`DawPanels`];
/// the arrangement takes it if it is there.
#[derive(Clone)]
pub struct Links {
    /// Whether the mixer is showing.
    pub open: Signal<bool>,
    /// Docked: the arrangement and the mixer are one pair in a view that
    /// shares its window with more (the Overview), so the arrangement
    /// opens with the compact track panel. The DAW view's is full.
    pub docked: bool,
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
    /// No arrangement beside it — the phone's Mixer view, the record view
    /// — so nothing else carries the mixer's edits to the engine: it
    /// sends them itself.
    pub alone: bool,
    /// Strips whose record arm arms another track, by the strip's guid.
    /// The record view's groups are folders, and a folder's arm is the
    /// track under it that records; the strip shows that track's arm.
    pub arm_of: Rc<std::collections::HashMap<String, daw_proto::Track>>,
    /// A few strips given more room than they need (the record view's
    /// five): drawn as big as fills it, rather than leaving it empty.
    pub fill: bool,
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
            docked: false,
            toggle: Rc::new(Cell::new(false)),
            rows: Rc::new(RefCell::new((0, rows))),
            from_mixer: Queue::default(),
            to_mixer: Queue::default(),
            to_arrange: Queue::default(),
            keys: Rc::default(),
            arrange_node: Rc::default(),
            alone: false,
            arm_of: Rc::default(),
            fill: false,
        }
    }

    /// A mixer on its own: `rows` its strips, open, sending its edits to
    /// the engine itself, with `arm_of`'s strips arming other tracks.
    #[must_use]
    pub fn alone(rows: Rows, arm_of: std::collections::HashMap<String, daw_proto::Track>) -> Self {
        let mut links = Self::new(rows);
        links.open = Signal::new(true);
        links.alone = true;
        links.arm_of = Rc::new(arm_of);
        links
    }
}

/// Whether the mixer is open, remembered per mode and per view (docked in
/// the Overview or summoned in the DAW view) — a context the shell
/// provides above the songs, so picking another song keeps it.
///
/// Unremembered, the Overview's docked mixer is open except in Organize,
/// where the room goes to the arrangement; the DAW view's is closed.
#[derive(Clone, Copy, PartialEq)]
pub struct MixerMemory(pub Signal<std::collections::HashMap<(bool, session::modes::Mode), bool>>);

impl MixerMemory {
    #[must_use]
    pub fn new() -> Self {
        Self(Signal::new(std::collections::HashMap::new()))
    }

    /// Open or closed, for panels `docked` or not, shown in `mode`.
    fn wanted(memory: Option<Self>, docked: bool, mode: Option<session::modes::Mode>) -> bool {
        let fallback = docked && mode != Some(session::modes::Mode::Organize);
        match (memory, mode) {
            (Some(memory), Some(mode)) => memory
                .0
                .peek()
                .get(&(docked, mode))
                .copied()
                .unwrap_or(fallback),
            _ => fallback,
        }
    }

    fn remember(mut self, docked: bool, mode: session::modes::Mode, open: bool) {
        if self.0.peek().get(&(docked, mode)) != Some(&open) {
            self.0.write().insert((docked, mode), open);
        }
    }
}

impl Default for MixerMemory {
    fn default() -> Self {
        Self::new()
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
    /// The mode the panels are shown in, for [`MixerMemory`]: whether the
    /// mixer is open is remembered per mode, and Organize starts closed.
    #[props(default)]
    mode: Option<session::modes::Mode>,
    /// The mixer alone, filling the panel — the small-screen layout's
    /// Mixer view.
    #[props(default)]
    mixer_only: bool,
) -> Element {
    let session: StudioSession = use_context();
    let memory = try_use_context::<MixerMemory>();
    let links = use_context_provider(|| {
        let mut links = Links::new(session.rows.as_slice().to_vec());
        links.open = Signal::new(MixerMemory::wanted(memory, docked, mode));
        links.docked = docked;
        links.alone = mixer_only;
        links
    });
    // Into another mode: the mixer as that mode last had it.
    let mut open_sig = links.open;
    let mut shown_in = use_signal(|| mode);
    use_effect(use_reactive!(|mode| {
        if *shown_in.peek() != mode {
            shown_in.set(mode);
            let want = MixerMemory::wanted(memory, docked, mode);
            if *open_sig.peek() != want {
                open_sig.set(want);
            }
        }
    }));
    // And whatever it is now (`x`), remembered for this mode.
    use_effect(move || {
        let open = open_sig();
        if let (Some(memory), Some(mode)) = (memory, *shown_in.peek()) {
            memory.remember(docked, mode, open);
        }
    });
    let open = (links.open)() || mixer_only;
    let (arrange_bottom, mixer_height) = split(open, mixer_only, docked, crate::touch::use_touch());
    let mixer_display = if open { "block" } else { "none" };
    rsx! {
        if !mixer_only {
            div {
                style: "position:absolute; top:0; left:0; right:0; bottom:{arrange_bottom};",
                crate::studio::Arrangement {}
            }
        }
        div {
            style: "display:{mixer_display}; position:absolute; left:0; right:0; bottom:0; \
                    height:{mixer_height}; border-top:1px solid #000;",
            // Where others' pointers over the mixer are placed.
            onmounted: move |e| crate::ghosts::region_mounted("mixer", e.data()),
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
    let touch = use_hook(|| Rc::new(Cell::new(false)));
    touch.set(crate::touch::use_touch());
    let scroll = use_hook(|| Rc::new(Cell::new(0.0_f64)));
    let content_w = use_hook(|| Rc::new(Cell::new(0.0_f64)));
    let strips = use_hook(|| {
        let strips: Rc<RefCell<Strips>> = Rc::default();
        crate::ghosts::register_anchor(
            "mixer",
            Rc::new(MixerAnchor {
                strips: Rc::clone(&strips),
                scroll: Rc::clone(&scroll),
            }),
        );
        strips
    });
    let widget = use_hook(|| {
        let mut widget = MixerWidget::new(
            links.clone(),
            Rc::clone(&scroll),
            Rc::clone(&content_w),
            Rc::clone(&live),
            Rc::clone(&touch),
        );
        widget.strips = Rc::clone(&strips);
        widget.redraw = crate::touch::redraw_hook();
        dioxus_native_dom::CustomWidgetAttr::new(widget)
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
    let window = dioxus_native::use_window();
    dioxus_native::use_window_event(move |event, _| match event {
        // A press here: hand the keyboard back to the arrangement. Not
        // from this event: the document is still borrowed while Blitz
        // handles it, and focusing from inside that panics ("RefCell
        // already borrowed"). The next redraw does it, as the rect is
        // read.
        winit::event::WindowEvent::PointerButton {
            state, position, ..
        } if state.is_pressed() => {
            pointer.set(crate::studio::css_point(&*window, position.x, position.y));
            let r = *rect.peek();
            let (x, y) = pointer.get();
            if x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3 {
                refocus.set(true);
            }
        }
        // In CSS pixels, as the rect is.
        winit::event::WindowEvent::PointerMoved { position, .. } => {
            pointer.set(crate::studio::css_point(&*window, position.x, position.y));
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
                winit::event::MouseScrollDelta::PixelDelta(at) => {
                    crate::studio::css_delta(&*window, at.x, at.y)
                }
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
    /// The mixer alone, filling the panel — the small-screen layout's
    /// Mixer view.
    #[props(default)]
    mixer_only: bool,
) -> Element {
    let session: StudioSession = use_context();
    let links = use_context_provider(|| {
        let mut links = Links::new(session.rows.as_slice().to_vec());
        links.open = Signal::new(docked || mixer_only);
        links.docked = docked;
        links.alone = mixer_only;
        links
    });
    let open = (links.open)() || mixer_only;
    let (arrange_bottom, mixer_height) = split(open, mixer_only, docked, crate::touch::use_touch());
    // Hidden rather than removed: kept at its size, the mixer builds its
    // strips and its GPU context at load, so `x` opens it at once instead
    // of after the second a first build takes.
    let (visibility, events) = if open {
        ("visible", "auto")
    } else {
        ("hidden", "none")
    };
    rsx! {
        if !mixer_only {
            div {
                style: "position:absolute; top:0; left:0; right:0; bottom:{arrange_bottom};",
                crate::web_host::WebArrangement { engine }
            }
        }
        div {
            style: "visibility:{visibility}; pointer-events:{events}; position:absolute; \
                    left:0; right:0; bottom:0; height:{mixer_height}; border-top:1px solid #000;",
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
    let touch = use_hook(|| Rc::new(Cell::new(false)));
    touch.set(crate::touch::use_touch());
    let scroll = use_hook(|| Rc::new(Cell::new(0.0_f64)));
    let content_w = use_hook(|| Rc::new(Cell::new(0.0_f64)));
    let element = use_hook(|| Rc::new(RefCell::new(None::<web_sys::HtmlElement>)));
    let widget = use_hook(|| {
        crate::web_host::HostedRef(Rc::new(RefCell::new(MixerWidget::new(
            links.clone(),
            Rc::clone(&scroll),
            Rc::clone(&content_w),
            Rc::clone(&live),
            Rc::clone(&touch),
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
    /// How far, in content pixels, takes the value end to end: the
    /// fader's own travel for a finger, so the cap stays under it.
    travel: f64,
    /// The track as it was when the drag began: the drag is relative to
    /// where the value started, not to where it has got to.
    was: daw_proto::Track,
}

/// What one pointer is doing, from its press to its release. One each, so
/// two fingers move two faders.
enum Hold {
    /// A fader or a knob, being turned.
    Turn(Turn),
    /// A press waiting to be a click: one if it lets go where it landed.
    /// A finger that wanders past [`crate::touch::SLOP`] first is
    /// scrolling instead.
    Press {
        spot: Option<Spot>,
        from: (f64, f64),
    },
    /// A press in the overview band ([`OVERVIEW_H`]): the strips go
    /// wherever it points, as it moves.
    Overview,
    /// A finger scrolling the strips, from where it and the scroll were,
    /// and how fast it is going (to throw the strips when it lets go).
    Pan {
        from_x: f64,
        was: f64,
        speed: crate::touch::Speed,
    },
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
    /// Each pointer's gesture in flight.
    holds: Vec<(BlitzPointerId, Hold)>,
    /// The latest value of each drag in flight, not yet sent (see
    /// [`MixerWidget::flush_drag`]).
    dragged: Vec<(Spot, Edit)>,
    /// Touch mode, as the panel last said: bigger strips (drawn at
    /// [`crate::touch::ZOOM`]).
    touch: Rc<Cell<bool>>,
    /// The engine's track events: what changed anywhere else — another
    /// client, REAPER itself — so the strips do not silently disagree.
    watch: Option<crate::engine::Watch>,
    /// The engine, for a mixer with no arrangement beside it to hand its
    /// edits to ([`Links::alone`]).
    applier: Option<crate::engine::Applier>,
    /// The tracks [`Links::arm_of`] arms in place of their strips'.
    targets: Vec<daw_proto::Track>,
    /// The strips still moving after a finger threw them.
    fling: Option<crate::touch::Fling>,
    /// Ask the host for another frame: Blitz paints a widget after an
    /// event, and a fling has none. (The page paints every frame.)
    redraw: Option<Rc<dyn Fn()>>,
    clips: crate::overlay::Clips,
    meters: Option<crate::engine::Meters>,
    /// Live-mode strips, as the panel last said, and as the recording
    /// was built.
    live: Rc<Cell<bool>>,
    built_live: bool,
    dirty: Cell<bool>,
    /// The widget's width as last painted, in CSS pixels: how far a
    /// finger can scroll.
    width: Cell<f64>,
    /// The zoom the last paint drew at ([`MixerWidget::zoom_for`]).
    drawn_zoom: Cell<f64>,
    /// How tall the overview band over the strips was drawn, in CSS
    /// pixels: 0 where there is none.
    band: Cell<f64>,
    /// Each strip as last laid out, for anchoring others' pointers.
    strips: Rc<RefCell<Strips>>,
}

/// How tall the overview band over the strips is, in CSS pixels
/// ([`MixerWidget::overview`]).
const OVERVIEW_H: f64 = 30.0;

/// The most a filling mixer ([`Links::fill`]) zooms its strips to fill
/// its width: past this, four strips are four slabs.
const FILL_MAX: f64 = 2.2;

/// The shortest a strip is laid out when zoomed: where the name plate
/// still clears the routing and the buttons over it.
const MIN_STRIP_H: f64 = 230.0;

/// Each strip's track, left edge and width, in content pixels.
type Strips = Vec<(String, f64, f64)>;

/// A pointer over the mixer, anchored to a strip: the track's guid, how
/// far across the strip, how far down — the same fader on the same track
/// in every window, whatever each has scrolled to.
struct MixerAnchor {
    strips: Rc<RefCell<Strips>>,
    scroll: Rc<Cell<f64>>,
}

impl crate::ghosts::Anchor for MixerAnchor {
    fn anchor(&self, x: f64, y: f64, (_, h): (f64, f64)) -> Option<(String, f64, f64)> {
        let cx = x + self.scroll.get();
        let strips = self.strips.borrow();
        let (guid, left, width) = strips.iter().find(|(_, l, w)| cx >= *l && cx < l + w)?;
        Some((guid.clone(), (cx - left) / width.max(1.0), y / h.max(1.0)))
    }

    fn place(&self, key: &str, u: f64, v: f64, (_, h): (f64, f64)) -> Option<(f64, f64)> {
        let strips = self.strips.borrow();
        let (_, left, width) = strips.iter().find(|(g, _, _)| g == key)?;
        Some((u.mul_add(*width, *left) - self.scroll.get(), v * h))
    }
}

impl MixerWidget {
    fn new(
        links: Links,
        scroll: Rc<Cell<f64>>,
        content_w: Rc<Cell<f64>>,
        live: Rc<Cell<bool>>,
        touch: Rc<Cell<bool>>,
    ) -> Self {
        let theme = daw_ui::theming::Theme::dark();
        let applier = if links.alone {
            crate::engine::Applier::start()
        } else {
            None
        };
        let targets = links.arm_of.values().cloned().collect();
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
            holds: Vec::new(),
            dragged: Vec::new(),
            touch,
            watch: crate::engine::Watch::start(),
            applier,
            targets,
            fling: None,
            redraw: None,
            clips: crate::overlay::Clips::default(),
            meters: crate::engine::Meters::start(),
            live,
            strips: Rc::default(),
            built_live: false,
            dirty: Cell::new(false),
            width: Cell::new(0.0),
            drawn_zoom: Cell::new(1.0),
            band: Cell::new(0.0),
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
                self.holds.clear();
            }
        }
        let echoed: Vec<Edit> = self.links.to_mixer.borrow_mut().drain(..).collect();
        for edit in &echoed {
            predict(&mut self.tracks, edit);
        }
        self.heard();
    }

    /// What the engine says changed, applied to the strips. A fader or a
    /// knob in a hand keeps the hand's value: the engine's answer to an
    /// earlier step of the same drag would pull it back.
    fn heard(&mut self) {
        let Some(watch) = &self.watch else { return };
        let events: Vec<_> = watch.drain().collect();
        for event in &events {
            use daw_proto::track::TrackEvent as E;
            // The rows are the arrangement's (or the panel's) to change;
            // an event that adds, removes or reorders tracks would
            // misalign them here.
            if matches!(event, E::Added(_) | E::Removed(_) | E::Moved { .. }) {
                continue;
            }
            if let Some(guid) = crate::engine::continuous_for(event)
                && self
                    .holds
                    .iter()
                    .any(|(_, hold)| matches!(hold, Hold::Turn(turn) if turn.was.guid == guid))
            {
                continue;
            }
            crate::engine::apply_event(&mut self.tracks, event);
            crate::engine::apply_event(&mut self.targets, event);
        }
        self.show_targets();
    }

    /// A strip that arms another track shows that track's arm.
    fn show_targets(&mut self) {
        if self.targets.is_empty() {
            return;
        }
        for track in &mut self.tracks {
            if let Some(target) = self.links.arm_of.get(&track.guid)
                && let Some(now) = self.targets.iter().find(|t| t.guid == target.guid)
            {
                track.armed = now.armed;
                track.input_monitor = now.input_monitor;
            }
        }
    }

    /// The track a control on `track`'s strip acts on: the strip's own,
    /// except an arm (and the monitoring under it) that [`Links::arm_of`]
    /// sends elsewhere.
    fn acts_on(&self, control: Control, track: &daw_proto::Track) -> daw_proto::Track {
        if matches!(control, Control::RecArm | Control::Monitor)
            && let Some(target) = self.links.arm_of.get(&track.guid)
        {
            return self
                .targets
                .iter()
                .find(|t| t.guid == target.guid)
                .unwrap_or(target)
                .clone();
        }
        track.clone()
    }

    /// The control under a point in the widget's own coordinates.
    fn spot_at(&self, x: f64, y: f64) -> Option<Spot> {
        let (content_x, content_y) = self.content(x, y);
        let mixer = self.mixer.as_ref()?;
        let row = mixer.strip_at(content_x)?;
        let (left, _, _) = mixer.strip_box(row)?;
        let control = crate::mcp::control_at(mixer, row, content_x - left, content_y)?;
        Some(Spot { row, control })
    }

    /// How much bigger the strips are drawn than they are laid out, as the
    /// last paint drew them: what a press is read back through.
    fn zoom(&self) -> f64 {
        self.drawn_zoom.get()
    }

    /// How much bigger to draw strips in a `width` by `height` panel (CSS
    /// pixels): touch mode's zoom for that width; for a mixer that
    /// [`Links::fill`]s, as big as fills the width, up to [`FILL_MAX`];
    /// and never so big that a strip is laid out shorter than
    /// [`MIN_STRIP_H`] — below that its name runs into the buttons above
    /// it, so a short dock (the Overview's, on a tablet) gets strips
    /// somewhat less big instead.
    fn zoom_for(&self, width: f64, height: f64) -> f64 {
        let mut zoom = crate::touch::mixer_zoom(self.touch.get(), width);
        if self.links.fill && !self.rows.is_empty() {
            #[expect(clippy::cast_precision_loss, reason = "a strip count")]
            let across = self.rows.len() as f64 * (crate::mcp::STRIP_W + crate::mcp::STRIP_GAP);
            zoom = zoom.max((width / across).min(FILL_MAX));
        }
        zoom.min((height / MIN_STRIP_H).max(1.0))
    }

    /// A point in the widget, in the strips' own (content) coordinates.
    /// The scroll is kept in CSS pixels, so the wheel and the ghosts'
    /// anchors need not know about the zoom.
    fn content(&self, x: f64, y: f64) -> (f64, f64) {
        let zoom = self.zoom();
        ((x + self.scroll.get()) / zoom, (y - self.band.get()) / zoom)
    }

    /// Whether a finger at `(x, y)` is on `row`'s fader cap, give or take
    /// a fingertip's worth: in touch mode the fader moves only by its
    /// cap, so a finger landing on the groove can scroll instead.
    fn on_cap(&self, row: usize, x: f64, y: f64) -> bool {
        let (content_x, content_y) = self.content(x, y);
        let (Some(mixer), Some(track)) = (self.mixer.as_ref(), self.track_at(row)) else {
            return false;
        };
        let (Some((left, _, _)), Some(strip)) = (mixer.strip_box(row), mixer.strip(row)) else {
            return false;
        };
        strip.cap(track.volume).is_some_and(|cap| {
            cap.inflate(6.0, 8.0)
                .contains((content_x - left, content_y))
        })
    }

    /// Send each drag's latest value on, if it has moved since the last.
    fn flush_drag(&mut self) {
        for (_, edit) in std::mem::take(&mut self.dragged) {
            self.send(edit);
        }
    }

    /// Do an edit here, and send it on: to the engine, and to the
    /// arrangement.
    fn commit(&mut self, edit: Edit) {
        predict(&mut self.tracks, &edit);
        predict(&mut self.targets, &edit);
        self.show_targets();
        self.send(edit);
    }

    /// Ask for another frame, where the host needs asking.
    fn redraw(&self) {
        if let Some(redraw) = &self.redraw {
            redraw();
        }
    }

    /// A thrown scroll's next step, if one is moving: carried along and
    /// stopped at either end.
    fn carry_fling(&mut self) {
        let Some(fling) = self.fling.as_mut() else {
            return;
        };
        let ((dx, _), going) = fling.step();
        let most = (self.content_w.get() - self.width.get()).max(0.0);
        let to = (self.scroll.get() + dx).clamp(0.0, most);
        let stopped = !going || to <= 0.0 || to >= most;
        self.scroll.set(to);
        if stopped {
            self.fling = None;
        } else {
            self.dirty.set(true);
            self.redraw();
        }
    }

    /// Scroll so the strips under `x` in the overview band are in the
    /// middle of the panel.
    fn overview_to(&mut self, x: f64) {
        let (width, content) = (self.width.get().max(1.0), self.content_w.get());
        let most = (content - width).max(0.0);
        let at = (x / width).clamp(0.0, 1.0) * content - width / 2.0;
        self.scroll.set(at.clamp(0.0, most));
        self.fling = None;
    }

    /// The overview band: every strip at once, each a live meter over its
    /// track's number on its track's colour, and the part of the mixer on screen
    /// outlined — where a mixer wider than the screen is found, and, with
    /// a press, gone to. In CSS pixels, `width` by `band`.
    fn overview(&self, levels: &[daw_proto::TrackLevels], width: f64, band: f64) -> Scene {
        use anyrender::PaintScene as _;
        use vello::kurbo::Rect;
        use vello::peniko::{Color, Fill};
        let mut out = Scene::new();
        let fill = |out: &mut Scene, rect: Rect, color: Color| {
            out.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
        };
        fill(
            &mut out,
            Rect::new(0.0, 0.0, width, band),
            Color::from_rgb8(0x0c, 0x0d, 0x0f),
        );
        let count = self.rows.len();
        if count == 0 {
            return out;
        }
        #[expect(clippy::cast_precision_loss, reason = "a strip count")]
        let cell = width / count as f64;
        let numbered = cell >= 14.0;
        let meter_bottom = if numbered { band - 12.0 } else { band - 4.0 };
        for row in 0..count {
            #[expect(clippy::cast_precision_loss, reason = "a strip index")]
            let x = row as f64 * cell;
            let Some(track) = self.track_at(row) else {
                continue;
            };
            // The track's colour, along the foot of its cell.
            fill(
                &mut out,
                Rect::new(x + 1.0, band - 2.0, x + cell - 1.0, band),
                crate::mcp::strip_ground(&self.palette, track),
            );
            // Its level, a sliver up the cell.
            let level = usize::try_from(track.index)
                .ok()
                .and_then(|i| levels.get(i))
                .map_or(0.0, |l| {
                    crate::engine::meter_fraction(l.peak_left.max(l.peak_right))
                });
            let meter_w = (cell * 0.3).clamp(2.0, 5.0);
            let mx = x + (cell - meter_w) / 2.0;
            let top = 3.0;
            fill(
                &mut out,
                Rect::new(mx, top, mx + meter_w, meter_bottom),
                Color::from_rgb8(0x1c, 0x1e, 0x22),
            );
            if level > 0.0 {
                let lit_top = meter_bottom - (meter_bottom - top) * level.clamp(0.0, 1.0);
                fill(
                    &mut out,
                    Rect::new(mx, lit_top, mx + meter_w, meter_bottom),
                    Color::from_rgb8(0x4a, 0xde, 0x80),
                );
            }
            if numbered {
                // The track's own number, as its strip shows it: a mixer
                // with tracks hidden skips numbers.
                let number = (track.index + 1).to_string();
                let size = 8.0_f32;
                let text_w = self.font.width(&number, size);
                crate::tcp::glyphs(
                    &mut out,
                    &self.font,
                    self.palette.text_dim,
                    &number,
                    x + (cell - text_w) / 2.0,
                    band - 4.0,
                    size,
                );
            }
        }
        // What is on screen.
        let content = self.content_w.get().max(1.0);
        let left = self.scroll.get() / content * width;
        let shown = (width / content * width).min(width);
        let window = Rect::new(left, 0.5, (left + shown).min(width), band - 0.5);
        fill(&mut out, window, Color::from_rgba8(0xff, 0xff, 0xff, 0x14));
        out.stroke(
            &vello::kurbo::Stroke::new(1.0),
            Affine::IDENTITY,
            Color::from_rgba8(0xff, 0xff, 0xff, 0x66),
            None,
            &window,
        );
        out
    }

    /// An edit, to the engine: straight there when the mixer stands alone,
    /// otherwise by way of the arrangement, which applies it too.
    fn send(&self, edit: Edit) {
        match &self.applier {
            Some(applier) => applier.send(edit),
            None => self.links.from_mixer.borrow_mut().push(edit),
        }
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
                // A mouse let go outside the panel: its release went
                // elsewhere, so whatever it was holding is over. Without
                // this the next move over the mixer carried on a drag
                // nobody was making.
                if e.is_mouse() && e.buttons.is_empty() && self.hold_of(e.id).is_some() {
                    self.let_go(e.id);
                }
                if self.hold_of(e.id).is_some() {
                    return self.moved(e.id, x, y);
                }
                // A finger has no hover: it is only there while it
                // presses.
                if e.is_finger() {
                    return false;
                }
                let spot = self.spot_at(x, y);
                self.pointer.hover(spot)
            }
            UiEvent::PointerDown(e) if e.button == MouseEventButton::Main => {
                let (x, y) = at(e);
                self.pressed(e.id, e.is_finger(), x, y);
                true
            }
            UiEvent::PointerUp(e) if e.button == MouseEventButton::Main => {
                let (x, y) = at(e);
                self.released(e.id, e.is_finger(), x, y);
                true
            }
            UiEvent::PointerCancel(e) => {
                self.let_go(e.id);
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

    fn hold_of(&self, id: BlitzPointerId) -> Option<usize> {
        self.holds.iter().position(|(held, _)| *held == id)
    }

    /// A press: a turn if it lands on a fader or a knob, otherwise a
    /// press waiting to be a click (or, by a finger, a scroll).
    ///
    /// A finger turns a fader only by its cap, and then one to one, so
    /// the cap stays under it. A mouse takes the whole column, as REAPER
    /// does, at the gearing the mixer always had.
    fn pressed(&mut self, id: BlitzPointerId, finger: bool, x: f64, y: f64) {
        // A press catches a thrown scroll, as it does on a phone.
        self.fling = None;
        // A second press by the same pointer means its release was lost.
        self.let_go(id);
        if y < self.band.get() {
            self.overview_to(x);
            self.holds.push((id, Hold::Overview));
            return;
        }
        let spot = self.spot_at(x, y);
        let turn = spot
            .filter(|spot| spot.control.is_continuous())
            .filter(|spot| {
                !finger || spot.control != Control::Volume || self.on_cap(spot.row, x, y)
            })
            .and_then(|spot| {
                let track = self.track_at(spot.row)?.clone();
                let mixer = self.mixer.as_ref()?;
                let height = mixer.strip_box(spot.row)?.2;
                let travel = match mixer.strip(spot.row) {
                    Some(strip) if finger && spot.control == Control::Volume => strip.travel(),
                    _ => height * 0.4,
                };
                Some(Turn {
                    spot,
                    from_y: y,
                    travel,
                    was: track,
                })
            });
        let hold = match turn {
            Some(turn) => Hold::Turn(turn),
            // A finger on a groove is not a decision about the level.
            None => Hold::Press {
                spot: spot.filter(|s| !finger || s.control != Control::Volume),
                from: (x, y),
            },
        };
        let shown = match &hold {
            Hold::Turn(turn) => Some(turn.spot),
            Hold::Press { spot, .. } => *spot,
            Hold::Pan { .. } | Hold::Overview => None,
        };
        self.pointer.hover(shown);
        self.pointer.press();
        self.holds.push((id, hold));
    }

    fn moved(&mut self, id: BlitzPointerId, x: f64, y: f64) -> bool {
        let zoom = self.zoom();
        let Some(i) = self.hold_of(id) else {
            return false;
        };
        match &mut self.holds[i].1 {
            Hold::Overview => {
                self.overview_to(x);
                true
            }
            Hold::Turn(turn) => {
                let fraction = crate::gesture::drag_fraction((y - turn.from_y) / zoom, turn.travel);
                let spot = turn.spot;
                if let Some(edit) =
                    crate::engine::drag(spot.control, &turn.was.guid, &turn.was, fraction)
                {
                    // Shown now, sent once a frame: a drag moves the
                    // pointer many times a frame, and every edit sent is
                    // an engine call and a re-record of the arrangement's
                    // controls.
                    predict(&mut self.tracks, &edit);
                    self.dragged.retain(|(held, _)| *held != spot);
                    self.dragged.push((spot, edit));
                }
                true
            }
            Hold::Press { from, .. } => {
                let (dx, dy) = (x - from.0, y - from.1);
                if id != BlitzPointerId::Mouse && dx.hypot(dy) > crate::touch::SLOP {
                    let from_x = from.0;
                    self.holds[i].1 = Hold::Pan {
                        from_x,
                        was: self.scroll.get(),
                        speed: crate::touch::Speed::default(),
                    };
                    self.pointer.release();
                    self.pointer.hover(None);
                    return self.moved(id, x, y);
                }
                false
            }
            Hold::Pan { from_x, was, speed } => {
                speed.moved((x, y));
                let most = (self.content_w.get() - self.width.get()).max(0.0);
                self.scroll.set((*was - (x - *from_x)).clamp(0.0, most));
                true
            }
        }
    }

    fn released(&mut self, id: BlitzPointerId, finger: bool, x: f64, y: f64) {
        let Some(i) = self.hold_of(id) else {
            return;
        };
        let (_, hold) = self.holds.remove(i);
        let up = self.spot_at(x, y);
        match hold {
            Hold::Turn(_) => self.flush_drag(),
            // Thrown: the strips carry on the way the finger was going.
            Hold::Pan { speed, .. } => {
                let (vx, _) = speed.velocity();
                self.fling = crate::touch::Fling::thrown((-vx, 0.0));
                self.redraw();
            }
            Hold::Press {
                spot: Some(spot), ..
            } if up == Some(spot) => {
                if let Some(track) = self.track_at(spot.row).cloned() {
                    // The clip latch clears on a click while it is lit;
                    // otherwise the band is the fader under it.
                    let control = match spot.control {
                        Control::Clip if self.clips.clear(&track.guid) => None,
                        Control::Clip => Some(Control::Volume),
                        other => Some(other),
                    };
                    let edit = control.and_then(|c| {
                        let target = self.acts_on(c, &track);
                        crate::engine::click(c, &target.guid, &target, false)
                    });
                    if let Some(edit) = edit {
                        self.commit(edit);
                    }
                }
            }
            Hold::Press { .. } | Hold::Overview => {}
        }
        self.pointer.release();
        self.pointer.hover(if finger { None } else { up });
    }

    /// A pointer's gesture ended without a release here: cancelled, or
    /// its release lost. What it had moved stays moved.
    fn let_go(&mut self, id: BlitzPointerId) {
        if let Some(i) = self.hold_of(id) {
            self.holds.remove(i);
            self.flush_drag();
            self.pointer.release();
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
    pub fn paint_scene(&mut self, width: u32, height: u32, scale: f64) -> Scene {
        self.dirty.set(false);
        self.flush_drag();
        self.catch_up();
        self.carry_fling();
        // Blitz hands a widget its size in device pixels, and the pointer
        // in CSS pixels; the page hands both in CSS pixels (and `scale`
        // 1). Laid out in CSS pixels, so the hit test agrees with the
        // picture, then drawn at the device's scale. Drawing device pixels
        // as if they were CSS put a 2x screen's strips at half size and
        // every tap twice as far along as what it pressed.
        let scale = scale.max(f64::EPSILON);
        let (css_w, full_h) = (f64::from(width) / scale, f64::from(height) / scale);
        self.width.set(css_w);
        // The overview band over the strips: on a touchscreen, and
        // wherever the strips run past the panel.
        #[expect(clippy::cast_precision_loss, reason = "a strip count")]
        let across = self.rows.len() as f64 * (crate::mcp::STRIP_W + crate::mcp::STRIP_GAP);
        let band = if !self.links.fill && (self.touch.get() || across > css_w) {
            OVERVIEW_H
        } else {
            0.0
        };
        self.band.set(band);
        let css_h = (full_h - band).max(0.0);
        let zoom = self.zoom_for(css_w, css_h);
        self.drawn_zoom.set(zoom);
        // Laid out at the size it has once zoomed, and drawn bigger.
        let (w, h) = (css_w / zoom, css_h / zoom);
        let mut out = Scene::new();
        if w < 1.0 || h < 1.0 {
            return out;
        }
        let live = self.live.get();
        let stale = self
            .mixer
            .as_ref()
            .is_none_or(|m| (m.height - h).abs() > 0.5 || m.touch != self.touch.get())
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
                    touch_strips: self.touch.get(),
                    ..crate::settings::Settings::default()
                },
                &crate::tone::Store::default(),
            ));
        }
        let Some(mixer) = self.mixer.as_ref() else {
            return out;
        };
        // CSS pixels, as the scroll is.
        self.content_w.set(mixer.content_width() * zoom);
        {
            let mut strips = self.strips.borrow_mut();
            strips.clear();
            for (row, (track, _)) in self.rows.iter().enumerate() {
                if let Some((left, width, _)) = mixer.strip_box(row) {
                    strips.push((track.guid.clone(), left * zoom, width * zoom));
                }
            }
        }
        let most = (mixer.content_width() - w).max(0.0) * zoom;
        let screen_scroll = self.scroll.get().clamp(0.0, most);
        self.scroll.set(screen_scroll);
        let scroll = screen_scroll / zoom;
        let levels = self
            .meters
            .as_ref()
            .map(crate::engine::Meters::levels)
            .unwrap_or_default();
        let at = Affine::scale(scale)
            * Affine::translate((0.0, band))
            * Affine::scale(zoom)
            * Affine::translate((-scroll, 0.0));
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
        if band > 0.0 {
            let overview = self.overview(&levels, css_w, band);
            anyrender::PaintScene::append_scene(&mut out, overview, Affine::scale(scale));
        }
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
