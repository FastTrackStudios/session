//! The Arrangement panel's own behaviour, host-free.
//!
//! What the panel does with input that is not the widget's: the zoom
//! spring (`z`) and its drag and wheel, the middle-button hand, the wheel
//! scroll, the zoom requests the keys make, the which-key popup, the
//! mixer's toggle and the edits each way. It used to be written directly
//! against winit's events in the dioxus-native component; it is the same
//! logic in the browser, so it lives here and each host translates its own
//! events into a [`PanelEvent`]:
//!
//! - native (`studio::Arrangement`, dioxus-native): winit window events;
//! - web (`web_host`, dioxus-web): DOM events on the panel's canvas.
//!
//! The chrome drawn over the widget — the main toolbar, the popup, the
//! scrollbars — is ordinary DOM and shared too ([`PanelChrome`]).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use dioxus::prelude::*;

use crate::studio::{PPS, StudioSession};

/// The scrollbar's thickness.
pub const BAR: f64 = 12.0;
/// One wheel notch, for a wheel that counts in lines.
pub const WHEEL_LINE: f64 = 40.0;
/// How far in each zoom may go.
const ZOOM_IN: (f64, f64) = (32.0, 6.0);

/// A mouse button, as the panel cares about it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Left,
    Middle,
    Other,
}

/// What a host tells the panel. Positions are in the host's window (or
/// page) coordinates, the same ones the panel's rectangle is read in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PanelEvent {
    /// The `z` key, by its physical position (Shift+z is a zoom too).
    ZKey { pressed: bool, repeat: bool },
    Modifiers(crate::mousemap::Mods),
    Pointer { x: f64, y: f64 },
    Button { button: Button, pressed: bool },
    /// A wheel, already in pixels (a line-counting wheel times
    /// [`WHEEL_LINE`]).
    Wheel { dx: f64, dy: f64 },
    /// Once a frame, with where the play cursor is.
    Frame { play_at: f64 },
}

/// What the pointer and the keyboard are doing to the view.
#[derive(Default)]
struct Input {
    /// Whether the zoom spring (`z`) is held.
    zooming: bool,
    shift: bool,
    pointer: (f64, f64),
    drag: Option<Drag>,
    /// Where a zoom-drag started, which it zooms about: the pointer moves
    /// while it zooms, and a view that chased it would slide as it grew.
    anchor: (f64, f64),
    /// The middle button, held down over this panel: the hand, whether
    /// or not the pointer has moved yet.
    panning: bool,
}

impl Input {
    /// The tool up, if any: the hand while the middle button is held (it
    /// was pressed on purpose, mid-zoom or not), else the zoom spring.
    fn tool(&self) -> crate::tool::Tool {
        if self.panning {
            crate::tool::Tool::Pan
        } else if self.zooming {
            crate::tool::Tool::Zoom
        } else {
            crate::tool::Tool::Map
        }
    }
}

/// A drag in flight.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Drag {
    /// The hand: the session moves under the pointer.
    Pan,
    /// The zoom tool, sprung from a held `z`.
    Zoom,
}

/// The scroll that keeps the point under the pointer where it is across a
/// zoom from `was` to `to`: zooming goes where the pointer points, rather
/// than toward the top-left corner.
///
/// `at` is the pointer along the axis, measured from the frame's own
/// origin (the lanes' left edge, or the top below the ruler); `scroll` is
/// the scroll in zoomed pixels before the zoom. The session point under
/// the pointer is `(scroll + at) / was` either side of the zoom, so the
/// new scroll is that point times `to`, less `at`.
#[must_use]
pub fn zoom_about(at: f64, scroll: f64, was: f64, to: f64) -> f64 {
    if was <= 0.0 {
        return scroll;
    }
    (scroll + at) / was * to - at
}

/// A wheel or drag distance as a zoom factor: the zoom tool's own, so a
/// benchmark that sweeps a drag zooms by what a hand would.
#[must_use]
pub fn factor(pixels: f64) -> f64 {
    (pixels / 200.0).exp()
}

/// Where edits go: the engine, however this host reaches it.
pub type Engine = Rc<dyn Fn(crate::engine::Edit)>;

/// The panel: its view (signals the chrome renders from), and what it
/// shares with the widget.
#[derive(Clone)]
pub struct ArrangementPanel {
    pub scroll: Signal<f64>,
    pub down: Signal<f64>,
    pub zoom: Signal<(f64, f64)>,
    /// The panel's rectangle in the window, read back from the layout.
    pub rect: Signal<(f64, f64, f64, f64)>,
    span_x: f64,
    span_y: Signal<f64>,
    pub which_shown: Signal<Option<crate::which_key::WhichKey>>,
    content_h: Rc<Cell<f64>>,
    pointing: crate::tool::Shared,
    which: crate::which_key::Shared,
    zooms: crate::zoom::Requests,
    history: Rc<RefCell<crate::zoom::History>>,
    view: crate::widget::Shared,
    edits: Rc<RefCell<Vec<crate::engine::Edit>>>,
    input: Rc<RefCell<Input>>,
    engine: Engine,
    mixer: Option<crate::mixer_panel::Links>,
    /// The panel's shape, toggled by the toolbar and read by the widget.
    pub compact: Rc<Cell<bool>>,
    /// The same, as a signal: what re-renders the chrome over the panel
    /// (the toolbar's width, the scrollbar's left end) when it changes.
    pub shape: Signal<bool>,
    /// The panel's outer node (measured) and the widget's (focused).
    pub mounted: Rc<RefCell<Option<Rc<MountedData>>>>,
    pub focus_node: Rc<RefCell<Option<Rc<MountedData>>>>,
    measured: Rc<Cell<Option<web_time::Instant>>>,
    /// The generated Click track, for the toolbar's metronome.
    click: (Option<String>, bool),
}

impl PartialEq for ArrangementPanel {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.input, &other.input)
    }
}

/// A handle on the panel for a host that drives its view itself — a
/// benchmark sweeping a gesture, a window animating one — rather than
/// through input. Provided above the panel; [`use_arrangement_panel`]
/// fills it with the panel it builds. The app provides none.
#[derive(Clone, Default)]
pub struct Slot(pub Rc<RefCell<Option<ArrangementPanel>>>);

impl PartialEq for Slot {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// Build the panel and its widget, once. `host` wraps the widget the way
/// the host mounts it (dioxus-native's `CustomWidgetAttr`, or the web
/// canvas's handle), and what it returns comes back with the panel.
pub fn use_arrangement_panel<H: Clone + 'static>(
    cursor: Option<crate::tool::Sink>,
    engine: Engine,
    host: impl FnOnce(crate::widget::ArrangementWidget) -> H,
) -> (ArrangementPanel, H) {
    let session: StudioSession = use_context();
    // The docked mixer's, when the view has one beside this panel.
    let mixer: Option<crate::mixer_panel::Links> = try_use_context();
    // The rows' height at zoom 1: showing or hiding tracks (the visibility
    // manager) changes it, and the widget reports the new one here.
    let content_h = use_hook(|| {
        let layout = crate::layout::Layout::from_env();
        Rc::new(Cell::new(
            session
                .rows
                .iter()
                .map(|(track, _)| layout.height_of(track.height))
                .sum::<f64>(),
        ))
    });
    let pointing = use_hook(|| crate::tool::Pointing::shared(cursor));
    let which = use_hook(crate::which_key::Shared::default);
    let zooms = use_hook(crate::zoom::Requests::default);
    // The panel's shape, shared with the toolbar that toggles it. The
    // view decides where it starts: compact where the arrangement is
    // docked beside more (the Overview), full in the DAW view.
    let docked = mixer.as_ref().is_some_and(|links| links.docked);
    let compact = use_hook(|| Rc::new(Cell::new(docked)));
    let shape = use_signal(|| compact.get());
    let history = use_hook(|| Rc::new(RefCell::new(crate::zoom::History::default())));
    let which_shown = use_signal(|| None::<crate::which_key::WhichKey>);
    let focus_node = use_hook(|| {
        mixer.as_ref().map_or_else(
            || Rc::new(RefCell::new(None::<Rc<MountedData>>)),
            |links| Rc::clone(&links.arrange_node),
        )
    });

    // The widget, built once. Its view — scroll, zoom, where the play
    // cursor is — is a plain cell it reads every paint, because the paint
    // runs outside the Dioxus runtime.
    let (hosted, view, edits) = use_hook(|| {
        let view: crate::widget::Shared =
            Rc::new(RefCell::new(crate::widget::View::OPENING));
        let built = crate::widget::ArrangementWidget::for_session(
            &session.project,
            &session.rows,
            &session.previews,
            compact.get(),
            Rc::clone(&view),
            std::env::var("FTS_BLITZ_FPS").is_ok_and(|v| v != "0"),
        )
        .with_pointing(Rc::clone(&pointing))
        .with_view_links(Rc::clone(&which), Rc::clone(&zooms))
        .with_planner(session.planner.clone(), Rc::clone(&content_h))
        .with_compact(Rc::clone(&compact));
        let built = match &mixer {
            Some(links) => built.with_mixer(crate::widget::MixerLinks {
                toggle: Rc::clone(&links.toggle),
                rows: Rc::clone(&links.rows),
                echo: Rc::clone(&links.to_arrange),
                keys: Rc::clone(&links.keys),
            }),
            None => built,
        };
        let edits = built.edits();
        (host(built), view, edits)
    });

    let scroll = use_signal(|| 0.0_f64);
    let down = use_signal(|| 0.0_f64);
    let zoom = use_signal(|| (1.0_f64, 1.0_f64));
    let rect = use_signal(|| (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64));
    let span_y = use_signal(|| content_h.get());
    let input = use_hook(|| Rc::new(RefCell::new(Input::default())));
    let mounted = use_hook(|| Rc::new(RefCell::new(None::<Rc<MountedData>>)));
    let measured = use_hook(|| Rc::new(Cell::new(None::<web_time::Instant>)));
    let click = session
        .project
        .tracks
        .iter()
        .find(|t| t.name.trim() == "Click")
        .map_or((None, true), |t| (Some(t.guid.clone()), t.muted));

    let panel = ArrangementPanel {
        scroll,
        down,
        zoom,
        rect,
        span_x: (session.project.length_secs * PPS).max(1.0),
        span_y,
        which_shown,
        content_h,
        pointing,
        which,
        zooms,
        history,
        view,
        edits,
        input,
        engine,
        mixer,
        compact,
        shape,
        mounted,
        focus_node,
        measured,
        click,
    };
    let slot: Option<Slot> = try_use_context();
    if let Some(slot) = slot
        && slot.0.borrow().is_none()
    {
        *slot.0.borrow_mut() = Some(panel.clone());
    }
    (panel, hosted)
}

/// The arrangement panel's tree on dioxus-native: the widget's node, and
/// the chrome over it, filling the positioned tile it is in.
///
/// What `studio::Arrangement` renders around the widget once it has
/// wired winit's events to the panel — and what a document with no winit
/// window renders (`bin/blitz_shot`'s pictures and its benchmark), which
/// has no events to wire. One tree, whoever hosts it.
#[cfg(feature = "native")]
#[component]
pub fn NativeTree(panel: ArrangementPanel, widget: dioxus_native_dom::CustomWidgetAttr) -> Element {
    let colors = daw_ui::studio::lanes::Colors::from_theme(&daw_ui::theming::Theme::dark());
    let surface = colors.surface.clone();
    let mounted = Rc::clone(&panel.mounted);
    let focus_node = Rc::clone(&panel.focus_node);
    rsx! {
        div {
            // Filling the positioned tile it is in. Absolute rather than
            // `height:100%`: a percentage of a flex item's height does not
            // resolve in Blitz, and the panel came out zero tall.
            style: "position:absolute; top:0; left:0; right:0; bottom:0; overflow:hidden; \
                    background:{surface};",
            onmounted: move |event| {
                *mounted.borrow_mut() = Some(event.data());
            },
            object {
                style: "position:absolute; left:0; top:0; width:100%; height:100%;",
                // `<object>` is not in Blitz's default-focusable list, and
                // the arrangement takes the keyboard (a rename is a field
                // inside it).
                tabindex: "0",
                data: widget.clone(),
                // The keyboard is the arrangement's from the start: with
                // nothing focused, every shortcut waited for a first click.
                onmounted: move |event| {
                    let node = event.data();
                    *focus_node.borrow_mut() = Some(Rc::clone(&node));
                    async move {
                        let _ = node.set_focus(true).await;
                    }
                },
            }
            PanelChrome { panel: panel.clone() }
        }
    }
}

impl ArrangementPanel {
    /// The lanes' frame: the panel less the track column and the ruler on
    /// one side, and the scrollbars on the other.
    #[must_use]
    pub fn frame(&self, r: (f64, f64, f64, f64)) -> (f64, f64) {
        (
            (r.2 - self.tcp().width() - BAR).max(1.0),
            (r.3 - crate::ruler::ruler_h() - BAR).max(1.0),
        )
    }

    /// How far the view can scroll, at a zoom.
    #[must_use]
    pub fn extent(&self, r: (f64, f64, f64, f64), zx: f64, zy: f64) -> (f64, f64) {
        let (fw, fh) = self.frame(r);
        let span_y = *self.span_y.peek();
        ((self.span_x * zx - fw).max(0.0), (span_y * zy - fh).max(0.0))
    }

    /// Out no further than the whole session filling the frame, and never
    /// above [`ZOOM_IN`].
    fn limits(&self, r: (f64, f64, f64, f64)) -> ((f64, f64), (f64, f64)) {
        let (fw, fh) = self.frame(r);
        let floor = |f: f64, s: f64| {
            if s > 0.0 && f > 0.0 {
                (f / s).min(1.0)
            } else {
                1.0
            }
        };
        (
            (floor(fw, self.span_x), ZOOM_IN.0),
            (floor(fh, *self.span_y.peek()), ZOOM_IN.1),
        )
    }

    /// Whether a window point is inside the panel.
    fn inside(&self, p: (f64, f64)) -> bool {
        let r = *self.rect.peek();
        p.0 >= r.0 && p.0 < r.0 + r.2 && p.1 >= r.1 && p.1 < r.1 + r.3
    }

    /// A tool going up or down, or a modifier changing what a drag would
    /// do, changes the pointer's shape with the pointer standing still,
    /// when the widget hears nothing. So the panel re-applies it.
    fn reshape(&self, input: &Input) {
        let mut pointing = self.pointing.borrow_mut();
        pointing.tool = input.tool();
        pointing.inside = self.inside(input.pointer);
        pointing.apply();
    }

    /// Set the zoom, keeping the session point under `about` (a window
    /// position) where it is on screen.
    fn rezoom(&self, to: (f64, f64), about: (f64, f64)) {
        let r = *self.rect.peek();
        let (mut zoom, mut scroll, mut down) = (self.zoom, self.scroll, self.down);
        let (was_x, was_y) = *zoom.peek();
        let (fw, fh) = self.frame(r);
        let ax = (about.0 - r.0 - self.tcp().width()).clamp(0.0, fw);
        let ay = (about.1 - r.1 - crate::ruler::ruler_h()).clamp(0.0, fh);
        let (ex, ey) = self.extent(r, to.0, to.1);
        let across = zoom_about(ax, *scroll.peek(), was_x, to.0);
        let deep = zoom_about(ay, *down.peek(), was_y, to.1);
        zoom.set(to);
        scroll.set(across.clamp(0.0, ex));
        down.set(deep.clamp(0.0, ey));
    }

    /// One event from the host.
    pub fn handle(&self, event: PanelEvent) {
        let r = *self.rect.peek();
        let (mut scroll, mut down) = (self.scroll, self.down);
        match event {
            PanelEvent::ZKey { pressed, repeat } => {
                let mut input = self.input.borrow_mut();
                if pressed && !repeat {
                    // A fresh hold: not yet used as the tool.
                    self.pointing.borrow_mut().tool_used = false;
                }
                input.zooming = pressed;
                self.reshape(&input);
            }
            PanelEvent::Modifiers(mods) => {
                let mut input = self.input.borrow_mut();
                input.shift = mods.shift;
                self.pointing.borrow_mut().mods = mods;
                self.reshape(&input);
            }
            PanelEvent::Pointer { x, y } => {
                let at = (x, y);
                let mut input = self.input.borrow_mut();
                let moved = (at.0 - input.pointer.0, at.1 - input.pointer.1);
                input.pointer = at;
                let (zx, zy) = *self.zoom.peek();
                match input.drag {
                    Some(Drag::Pan) => {
                        let (ex, ey) = self.extent(r, zx, zy);
                        let across = *scroll.peek();
                        scroll.set((across - moved.0).clamp(0.0, ex));
                        let deep = *down.peek();
                        down.set((deep - moved.1).clamp(0.0, ey));
                    }
                    Some(Drag::Zoom) => {
                        self.pointing.borrow_mut().tool_used = true;
                        let ((x0, x1), (y0, y1)) = self.limits(r);
                        self.rezoom(
                            (
                                (zx * factor(moved.0)).clamp(x0, x1),
                                // Down zooms in: the rows come toward you as
                                // you pull, as in REAPER's zoom tool.
                                (zy * factor(moved.1)).clamp(y0, y1),
                            ),
                            input.anchor,
                        );
                    }
                    None => {}
                }
                // Over the toolbar or a scrollbar the widget hears nothing,
                // and a tool's shape still has to hold there.
                if input.tool() != crate::tool::Tool::Map || !self.inside(at) {
                    self.reshape(&input);
                }
            }
            PanelEvent::Button { button, pressed } => {
                let mut input = self.input.borrow_mut();
                let over = self.inside(input.pointer);
                if button == Button::Middle {
                    input.panning = pressed && over;
                }
                // A drag starts only over this panel; it ends wherever the
                // button comes up.
                input.drag = match (pressed, button) {
                    (true, Button::Middle) if over => Some(Drag::Pan),
                    (true, Button::Left) if input.zooming && over => {
                        input.anchor = input.pointer;
                        Some(Drag::Zoom)
                    }
                    _ => None,
                };
                self.reshape(&input);
            }
            PanelEvent::Wheel { dx, dy } => {
                let input = self.input.borrow();
                if !self.inside(input.pointer) {
                    return;
                }
                let (zx, zy) = *self.zoom.peek();
                let ((x0, x1), (y0, y1)) = self.limits(r);
                if input.zooming {
                    self.pointing.borrow_mut().tool_used = true;
                    // `z` zooms time, the common one; with shift, the rows.
                    // Either axis of the wheel counts: macOS turns a shifted
                    // wheel sideways, so the notch arrives in `dx`.
                    let notch = (dx + dy) * 2.0;
                    let to = if input.shift {
                        (zx, (zy * factor(notch)).clamp(y0, y1))
                    } else {
                        ((zx * factor(notch)).clamp(x0, x1), zy)
                    };
                    self.rezoom(to, input.pointer);
                } else {
                    let (ex, ey) = self.extent(r, zx, zy);
                    if input.shift {
                        let across = *scroll.peek();
                        scroll.set((across - dx - dy).clamp(0.0, ex));
                    } else {
                        let across = *scroll.peek();
                        scroll.set((across - dx).clamp(0.0, ex));
                        let deep = *down.peek();
                        down.set((deep - dy).clamp(0.0, ey));
                    }
                }
            }
            PanelEvent::Frame { play_at } => self.frame_tick(play_at),
        }
    }

    /// Once a frame: the widget's view, the zooms the keys asked for, the
    /// popup, the edits each way, and the panel's rectangle.
    fn frame_tick(&self, play_at: f64) {
        let r = *self.rect.peek();
        let (mut scroll, mut down, mut zoom) = (self.scroll, self.down, self.zoom);
        // Four numbers, written every time because a missed write is a
        // frame drawn in the wrong place.
        *self.view.borrow_mut() = crate::widget::View {
            scroll_x: *scroll.peek(),
            scroll_y: *down.peek(),
            zoom_x: zoom.peek().0,
            zoom_y: zoom.peek().1,
            play_at,
        };
        // The zooms the keys asked for: the widget has no way to set a
        // signal, so it leaves them here.
        let asked: Vec<_> = self.zooms.borrow_mut().drain(..).collect();
        for request in asked {
            let (zx, zy) = *zoom.peek();
            let now = crate::zoom::Target {
                zoom_x: zx,
                zoom_y: zy,
                scroll_x: *scroll.peek(),
                scroll_y: *down.peek(),
            };
            let (fw, fh) = self.frame(r);
            let ((x0, x1), (y0, y1)) = self.limits(r);
            let at = crate::zoom::Frame {
                width: fw,
                height: fh,
                pps: PPS,
                limits_x: (x0, x1),
                limits_y: (y0, y1),
            };
            let framed = || match request {
                crate::zoom::Request::Frame { time, rows, .. } => {
                    crate::zoom::frame(now, at, time, rows)
                }
                crate::zoom::Request::Scale { vertical, by } => {
                    // About the middle of the lanes.
                    let mut to = now;
                    if vertical {
                        to.zoom_y = (zy * by).clamp(y0, y1);
                        to.scroll_y = zoom_about(fh / 2.0, now.scroll_y, zy, to.zoom_y);
                    } else {
                        to.zoom_x = (zx * by).clamp(x0, x1);
                        to.scroll_x = zoom_about(fw / 2.0, now.scroll_x, zx, to.zoom_x);
                    }
                    to
                }
                _ => now,
            };
            let Some(to) = self.history.borrow_mut().go(now, request, framed) else {
                continue;
            };
            let (ex, ey) = self.extent(r, to.zoom_x, to.zoom_y);
            zoom.set((to.zoom_x, to.zoom_y));
            scroll.set(to.scroll_x.clamp(0.0, ex));
            down.set(to.scroll_y.clamp(0.0, ey));
        }
        // Rows shown or hidden: the scroll range follows.
        let mut span_y = self.span_y;
        if self.content_h.get() != *span_y.peek() {
            span_y.set(self.content_h.get());
        }
        // A held `z` used as the tool is the tool, not a prefix: no popup
        // over the zoom.
        let wanted = if self.pointing.borrow().tool_used {
            None
        } else {
            self.which.borrow().clone()
        };
        let mut which_shown = self.which_shown;
        if *which_shown.peek() != wanted {
            which_shown.set(wanted);
        }
        // Whatever the widget asked to have done, to the engine.
        let mut pending: Vec<_> = self.edits.borrow_mut().drain(..).collect();
        if let Some(links) = &self.mixer {
            // `x`, from either widget.
            if links.toggle.take() {
                let mut open = links.open;
                let was = *open.peek();
                open.set(!was);
                // Closing it: the keyboard comes back here, not to a panel
                // that is no longer on screen.
                if was && let Some(node) = self.focus_node.borrow().clone() {
                    spawn(async move {
                        let _ = node.set_focus(true).await;
                    });
                }
            }
            // Each widget's edits to the other one, never back to the one
            // that made them (a toggle would flip twice).
            links.to_mixer.borrow_mut().extend(pending.iter().cloned());
            let from_mixer: Vec<_> = links.from_mixer.borrow_mut().drain(..).collect();
            links.to_arrange.borrow_mut().extend(from_mixer.iter().cloned());
            pending.extend(from_mixer);
        }
        for edit in pending {
            (self.engine)(edit);
        }
        // Where this panel is, re-read a few times a second: a divider or a
        // window resize moves it without telling anything here.
        let stale = self
            .measured
            .get()
            .is_none_or(|at| at.elapsed() > std::time::Duration::from_millis(200));
        if stale && let Some(node) = self.mounted.borrow().clone() {
            self.measured.set(Some(web_time::Instant::now()));
            let mut rect = self.rect;
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

    /// The track panel's shape, as the widget is drawing it.
    #[must_use]
    pub fn tcp(&self) -> crate::tcp::Tcp {
        crate::tcp::Tcp { compact: self.compact.get() }
    }

    /// The toolbar's edits: the same queue the widget's go on.
    #[must_use]
    pub fn edits(&self) -> crate::studio::Edits {
        crate::studio::Edits(Rc::clone(&self.edits))
    }
}

/// What is drawn over the widget: the main toolbar, the which-key popup,
/// and the two scrollbars. Ordinary DOM, the same under either host.
#[component]
pub fn PanelChrome(panel: ArrangementPanel) -> Element {
    let r = (panel.rect)();
    let (fw, fh) = panel.frame(r);
    let (zx, zy) = (panel.zoom)();
    let travel = panel.extent(r, zx, zy);
    let colors = daw_ui::studio::lanes::Colors::from_theme(&daw_ui::theming::Theme::dark());
    let ruler = crate::ruler::ruler_h();
    let (mut scroll, mut down) = (panel.scroll, panel.down);
    let (click, click_muted) = panel.click.clone();
    // Read as a signal so a toggle re-renders what is sized to the panel.
    let tcp_w = crate::tcp::Tcp { compact: (panel.shape)() }.width();
    rsx! {
        // The main toolbar, in the corner left of the ruler's lane names.
        crate::toolbar::MainToolbar {
            click,
            click_muted,
            edits: panel.edits(),
            // The toolbar sits over the panel, so it is as wide as
            // whichever shape the panel is in.
            width: tcp_w - crate::ruler::LABEL_W,
            compact: Rc::clone(&panel.compact),
            shape: panel.shape,
        }
        crate::which_key::Panel { showing: (panel.which_shown)(), colors: colors.clone() }
        crate::studio::ScrollBar {
            across: true,
            at: scroll(),
            travel: travel.0,
            window: fw,
            left: tcp_w,
            top: (r.3 - BAR).max(0.0),
            length: fw,
            colors: colors.clone(),
            on_move: move |to: f64| scroll.set(to.clamp(0.0, travel.0)),
        }
        crate::studio::ScrollBar {
            across: false,
            at: down(),
            travel: travel.1,
            window: fh,
            left: (r.2 - BAR).max(0.0),
            top: ruler,
            length: fh,
            colors,
            on_move: move |to: f64| down.set(to.clamp(0.0, travel.1)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::zoom_about;

    /// The session point under the pointer stays under it.
    #[test]
    fn a_zoom_keeps_the_point_under_the_pointer() {
        // 300px into the frame, scrolled 500: session point 800 at zoom 1.
        let scroll = zoom_about(300.0, 500.0, 1.0, 2.0);
        // At zoom 2 that point is at 1600; on screen at 1600 - scroll.
        assert!((1600.0 - scroll - 300.0).abs() < 1e-9, "{scroll}");
        // And back out lands where it began.
        assert!((zoom_about(300.0, scroll, 2.0, 1.0) - 500.0).abs() < 1e-9);
    }

    /// With the pointer at the frame's origin it is the old behaviour:
    /// the top-left stays put.
    #[test]
    fn at_the_origin_it_zooms_about_the_corner() {
        assert!((zoom_about(0.0, 100.0, 1.0, 3.0) - 300.0).abs() < 1e-9);
    }
}
