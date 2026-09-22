//! The panels in a browser: dioxus-web, with each widget in a `<canvas>`.
//!
//! dioxus-web has no custom-widget mechanism of its own (that is
//! dioxus-native's `CustomWidgetAttr`), so this is one: a canvas per
//! widget, drawn every animation frame with vello_hybrid's WebGL2 renderer
//! (keyflow-ui's `wasm-graphics` pattern), and the canvas's DOM events
//! turned into the same `UiEvent`s Blitz hands a widget. The widgets
//! themselves (the arrangement, the mixer, the chart) are unchanged: they
//! paint a scene and take events, and do not know which host they are in.
//!
//! Sizes: a widget paints at the canvas's CSS size (events arrive in CSS
//! pixels, so the hit test agrees with the picture), and the scene is
//! scaled by `devicePixelRatio` into the canvas's backing store, so it is
//! sharp on a high-density display.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::str::FromStr as _;

use anyrender::PaintScene as _;
use blitz_traits::events::{
    BlitzKeyEvent, BlitzPointerEvent, BlitzPointerId, Code, Key, KeyState, Location,
    Modifiers, MouseEventButton, MouseEventButtons, PointerCoords, PointerDetails, UiEvent,
};
use dioxus::prelude::*;
use vello::kurbo::Affine;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::closure::Closure;

use crate::panel::{Button, PanelEvent, WHEEL_LINE};

/// What a canvas hosts: something that paints a scene and takes events.
pub trait Hosted {
    fn paint(&mut self, width: u32, height: u32, scale: f64) -> anyrender::Scene;
    fn event(&mut self, event: &UiEvent);
}

impl Hosted for crate::widget::ArrangementWidget {
    fn paint(&mut self, width: u32, height: u32, scale: f64) -> anyrender::Scene {
        self.paint_scene(width, height, scale)
    }
    fn event(&mut self, event: &UiEvent) {
        crate::widget::ArrangementWidget::event(self, event);
    }
}

/// A hosted widget, shared between its canvas and whoever built it.
#[derive(Clone)]
pub struct HostedRef(pub Rc<RefCell<dyn Hosted>>);

impl PartialEq for HostedRef {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// Where a canvas's node goes once mounted, for whoever gives it the
/// keyboard (the panel's `focus_node`).
#[derive(Clone)]
pub struct FocusSlot(pub Rc<RefCell<Option<Rc<MountedData>>>>);

impl FocusSlot {
    fn set(&self, node: Rc<MountedData>) {
        *self.0.borrow_mut() = Some(node);
    }
}

impl PartialEq for FocusSlot {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// A canvas element slot, equal by identity.
#[derive(Clone)]
pub struct ElementSlot(pub Rc<RefCell<Option<web_sys::HtmlElement>>>);

impl PartialEq for ElementSlot {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// vello_hybrid's WebGL2 renderer over one canvas.
struct Gpu {
    renderer: vello_hybrid::WebGlRenderer,
    resources: vello_hybrid::Resources,
    scene: vello_hybrid::Scene,
    images: rustc_hash::FxHashMap<u64, vello_common::paint::ImageId>,
    size: (u32, u32),
}

impl Gpu {
    fn new(canvas: &web_sys::HtmlCanvasElement, width: u32, height: u32) -> Self {
        Self {
            renderer: vello_hybrid::WebGlRenderer::new(canvas),
            resources: vello_hybrid::Resources::new(),
            scene: Self::scene_at(width, height),
            images: rustc_hash::FxHashMap::default(),
            size: (width, height),
        }
    }

    fn scene_at(width: u32, height: u32) -> vello_hybrid::Scene {
        #[expect(clippy::cast_possible_truncation, reason = "a canvas is well under 65k px")]
        let (w, h) = (width.min(u32::from(u16::MAX)) as u16, height.min(u32::from(u16::MAX)) as u16);
        vello_hybrid::Scene::new_with(w, h, vello_hybrid::RenderSettings::default())
    }

    fn resize(&mut self, width: u32, height: u32) {
        if self.size != (width, height) {
            self.size = (width, height);
            self.scene = Self::scene_at(width, height);
        }
    }

    /// Draw `scene` (in CSS pixels) scaled by `dpr` into the canvas.
    fn draw(&mut self, scene: anyrender::Scene, dpr: f64) {
        {
            let images = anyrender_vello_hybrid::WebGlImageManager::new(
                &mut self.renderer,
                &mut self.resources,
                &mut self.images,
            );
            let mut painter = anyrender_vello_hybrid::WebGlScenePainter::new(&mut self.scene, images);
            painter.append_scene(scene, Affine::scale(dpr));
        }
        let size = vello_hybrid::RenderSize {
            width: self.size.0,
            height: self.size.1,
        };
        if let Err(e) = self.renderer.render(&self.scene, &mut self.resources, &size) {
            tracing::error!(error = ?e, "canvas render failed");
        }
        self.scene.reset();
    }
}

/// The DOM's modifier keys, as Blitz's.
fn modifiers(m: dioxus::prelude::Modifiers) -> Modifiers {
    let mut out = Modifiers::empty();
    if m.shift() {
        out |= Modifiers::SHIFT;
    }
    if m.ctrl() {
        out |= Modifiers::CONTROL;
    }
    if m.alt() {
        out |= Modifiers::ALT;
    }
    if m.meta() {
        out |= Modifiers::META;
    }
    out
}

fn mods(m: dioxus::prelude::Modifiers) -> crate::mousemap::Mods {
    crate::mousemap::Mods {
        shift: m.shift(),
        ctrl: m.ctrl(),
        alt: m.alt(),
    }
}

/// A DOM pointer event, as the widget reads one: coordinates relative to
/// the canvas (Blitz makes them relative to the node the same way).
fn pointer(e: &PointerData) -> BlitzPointerEvent {
    #[expect(clippy::cast_possible_truncation, reason = "a coordinate on a page")]
    let (x, y) = {
        let at = e.element_coordinates();
        (at.x as f32, at.y as f32)
    };
    let button = match e.trigger_button() {
        Some(dioxus::html::input_data::MouseButton::Auxiliary) => MouseEventButton::Auxiliary,
        Some(dioxus::html::input_data::MouseButton::Secondary) => MouseEventButton::Secondary,
        _ => MouseEventButton::Main,
    };
    BlitzPointerEvent {
        id: BlitzPointerId::Mouse,
        is_primary: true,
        coords: PointerCoords {
            page_x: x,
            page_y: y,
            screen_x: x,
            screen_y: y,
            client_x: x,
            client_y: y,
        },
        button,
        buttons: MouseEventButtons::empty(),
        mods: modifiers(e.modifiers()),
        details: PointerDetails::default(),
        element: blitz_traits::events::Point { x, y },
        active_pointers: std::sync::Arc::default(),
    }
}

/// A DOM key event, as the widget reads one: by its W3C names (the DOM
/// and Blitz are on different `keyboard-types` versions).
fn key(e: &KeyboardData, state: KeyState) -> BlitzKeyEvent {
    BlitzKeyEvent {
        key: Key::from_str(&e.key().to_string()).unwrap_or(Key::Unidentified),
        code: Code::from_str(&e.code().to_string()).unwrap_or(Code::Unidentified),
        modifiers: modifiers(e.modifiers()),
        location: Location::Standard,
        is_auto_repeating: e.is_auto_repeating(),
        is_composing: false,
        state,
        text: None,
    }
}

fn panel_button(e: &PointerData) -> Button {
    match e.trigger_button() {
        Some(dioxus::html::input_data::MouseButton::Primary) => Button::Left,
        Some(dioxus::html::input_data::MouseButton::Auxiliary) => Button::Middle,
        _ => Button::Other,
    }
}

/// One widget in a canvas: painted every animation frame, fed the canvas's
/// events. `panel`, when given, hears the panel-level events too (the
/// zoom spring, the hand, the wheel), and `frame` runs once a frame before
/// the paint.
#[component]
pub fn WidgetCanvas(
    widget: HostedRef,
    panel: Option<EventHandler<PanelEvent>>,
    frame: Option<EventHandler<()>>,
    /// Where the canvas's node goes, for whoever gives it the focus.
    focus: Option<FocusSlot>,
    /// Where the canvas element goes, for the pointer shape.
    element: Option<ElementSlot>,
) -> Element {
    let alive = use_hook(|| Rc::new(Cell::new(true)));
    {
        let alive = Rc::clone(&alive);
        use_drop(move || alive.set(false));
    }
    let to_panel = move |event: PanelEvent| {
        if let Some(panel) = panel {
            panel.call(event);
        }
    };
    let (w1, w2, w3, w4, w5) = (
        widget.clone(),
        widget.clone(),
        widget.clone(),
        widget.clone(),
        widget.clone(),
    );
    rsx! {
        canvas {
            style: "position:absolute; left:0; top:0; width:100%; height:100%; \
                    display:block; outline:none; touch-action:none;",
            tabindex: "0",
            onmounted: move |event| {
                let data = event.data();
                // Only the canvas that reads the keyboard takes the focus.
                let takes_focus = focus.is_some();
                if let Some(slot) = &focus {
                    slot.set(Rc::clone(&data));
                }
                let Some(canvas) = data
                    .downcast::<web_sys::Element>()
                    .and_then(|el| el.clone().dyn_into::<web_sys::HtmlCanvasElement>().ok())
                else {
                    tracing::error!("the widget's node is not a canvas");
                    return;
                };
                if let Some(slot) = &element {
                    *slot.0.borrow_mut() = Some(canvas.clone().unchecked_into());
                }
                if takes_focus {
                    let _ = canvas.focus();
                }
                start_painting(canvas, widget.clone(), frame, Rc::clone(&alive));
            },
            onpointermove: move |e| {
                to_panel(PanelEvent::Pointer { x: e.client_coordinates().x, y: e.client_coordinates().y });
                w1.0.borrow_mut().event(&UiEvent::PointerMove(pointer(&e)));
            },
            onpointerdown: move |e| {
                // Keep the pointer through a drag that leaves the canvas.
                if let Some(target) = e.data().downcast::<web_sys::PointerEvent>()
                    .and_then(|p| p.target())
                    .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                {
                    let _ = target.set_pointer_capture(e.pointer_id());
                }
                to_panel(PanelEvent::Button { button: panel_button(&e), pressed: true });
                w2.0.borrow_mut().event(&UiEvent::PointerDown(pointer(&e)));
            },
            onpointerup: move |e| {
                to_panel(PanelEvent::Button { button: panel_button(&e), pressed: false });
                w3.0.borrow_mut().event(&UiEvent::PointerUp(pointer(&e)));
            },
            oncontextmenu: move |e| e.prevent_default(),
            onwheel: move |e| {
                e.prevent_default();
                let delta = e.delta();
                let (dx, dy) = match delta {
                    dioxus::html::geometry::WheelDelta::Pixels(v) => (v.x, v.y),
                    dioxus::html::geometry::WheelDelta::Lines(v) => (v.x * WHEEL_LINE, v.y * WHEEL_LINE),
                    dioxus::html::geometry::WheelDelta::Pages(v) => (v.x * 400.0, v.y * 400.0),
                };
                // The DOM's positive delta scrolls down; the panel's (winit's)
                // positive delta moves the content down.
                to_panel(PanelEvent::Modifiers(mods(e.modifiers())));
                to_panel(PanelEvent::Wheel { dx: -dx, dy: -dy });
            },
            onkeydown: move |e| {
                if e.code() == dioxus::prelude::Code::KeyZ {
                    to_panel(PanelEvent::ZKey { pressed: true, repeat: e.is_auto_repeating() });
                }
                to_panel(PanelEvent::Modifiers(mods(e.modifiers())));
                // The page must not scroll on space or the arrows, nor take
                // a shortcut the session means.
                if !matches!(e.key(), dioxus::prelude::Key::Tab) {
                    e.prevent_default();
                }
                w4.0.borrow_mut().event(&UiEvent::KeyDown(key(&e, KeyState::Pressed)));
            },
            onkeyup: move |e| {
                if e.code() == dioxus::prelude::Code::KeyZ {
                    to_panel(PanelEvent::ZKey { pressed: false, repeat: false });
                }
                to_panel(PanelEvent::Modifiers(mods(e.modifiers())));
                w5.0.borrow_mut().event(&UiEvent::KeyUp(key(&e, KeyState::Released)));
            },
        }
    }
}

/// The animation-frame loop for one canvas: size the backing store to the
/// canvas's CSS size times `devicePixelRatio`, run `frame`, paint.
fn start_painting(
    canvas: web_sys::HtmlCanvasElement,
    widget: HostedRef,
    frame: Option<EventHandler<()>>,
    alive: Rc<Cell<bool>>,
) {
    let runtime = dioxus::dioxus_core::Runtime::current();
    let scope = dioxus::dioxus_core::current_scope_id();
    let gpu = Rc::new(RefCell::new(None::<Gpu>));
    let tick: Rc<RefCell<Option<Closure<dyn FnMut()>>>> = Rc::new(RefCell::new(None));
    let again = Rc::clone(&tick);
    *tick.borrow_mut() = Some(Closure::new(move || {
        if !alive.get() {
            // Unmounted: stop, and let the closure go.
            again.borrow_mut().take();
            return;
        }
        let Some(window) = web_sys::window() else { return };
        let dpr = window.device_pixel_ratio().max(1.0);
        let rect = canvas.get_bounding_client_rect();
        let (css_w, css_h) = (rect.width().max(1.0), rect.height().max(1.0));
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a canvas size")]
        let (px_w, px_h) = ((css_w * dpr).round() as u32, (css_h * dpr).round() as u32);
        if canvas.width() != px_w || canvas.height() != px_h {
            canvas.set_width(px_w);
            canvas.set_height(px_h);
        }
        if let Some(frame) = frame {
            runtime.in_scope(scope, || frame.call(()));
        }
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a canvas size")]
        let scene = widget.0.borrow_mut().paint(css_w as u32, css_h as u32, 1.0);
        let mut gpu = gpu.borrow_mut();
        let gpu = gpu.get_or_insert_with(|| Gpu::new(&canvas, px_w, px_h));
        gpu.resize(px_w, px_h);
        gpu.draw(scene, dpr);
        if let Some(next) = again.borrow().as_ref() {
            let _ = window.request_animation_frame(next.as_ref().unchecked_ref());
        }
    }));
    if let (Some(window), Some(first)) = (web_sys::window(), tick.borrow().as_ref()) {
        let _ = window.request_animation_frame(first.as_ref().unchecked_ref());
    }
}

/// The arrangement panel in a browser: [`crate::panel`]'s behaviour, the
/// widget in a canvas, the shared chrome over it.
#[component]
pub fn WebArrangement(engine: crate::web_engine::EngineRef) -> Element {
    let element = use_hook(|| Rc::new(RefCell::new(None::<web_sys::HtmlElement>)));
    let sink: crate::tool::Sink = {
        let element = Rc::clone(&element);
        Rc::new(move |icon: cursor_icon::CursorIcon| {
            if let Some(el) = element.borrow().as_ref() {
                let _ = el.style().set_property("cursor", icon.name());
            }
        })
    };
    let to_engine: crate::panel::Engine = {
        let engine = engine.clone();
        Rc::new(move |edit| engine.send(edit))
    };
    let (panel, widget) = crate::panel::use_arrangement_panel(Some(sink), to_engine, |w| {
        HostedRef(Rc::new(RefCell::new(w)))
    });
    let colors = daw_ui::studio::lanes::Colors::from_theme(&daw_ui::theming::Theme::dark());
    let surface = colors.surface.clone();
    let mounted = Rc::clone(&panel.mounted);
    let (on_panel, on_frame) = (panel.clone(), panel.clone());
    let focus = FocusSlot(Rc::clone(&panel.focus_node));
    let playhead = engine.clone();
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; right:0; bottom:0; overflow:hidden; \
                    background:{surface};",
            onmounted: move |event| {
                *mounted.borrow_mut() = Some(event.data());
            },
            WidgetCanvas {
                widget,
                panel: move |e| on_panel.handle(e),
                frame: move |()| on_frame.handle(PanelEvent::Frame { play_at: playhead.position() }),
                focus,
                element: Some(ElementSlot(Rc::clone(&element))),
            }
            crate::panel::PanelChrome { panel: panel.clone() }
        }
    }
}

/// Fetch a URL's text.
async fn fetch_text(url: &str) -> Result<String, String> {
    let window = web_sys::window().ok_or("no window")?;
    let response = wasm_bindgen_futures::JsFuture::from(window.fetch_with_str(url))
        .await
        .map_err(|e| format!("{url}: {e:?}"))?;
    let response: web_sys::Response = response.dyn_into().map_err(|_| "not a response".to_owned())?;
    if !response.ok() {
        return Err(format!("{url}: HTTP {}", response.status()));
    }
    let text = response.text().map_err(|e| format!("{e:?}"))?;
    wasm_bindgen_futures::JsFuture::from(text)
        .await
        .map_err(|e| format!("{e:?}"))?
        .as_string()
        .ok_or_else(|| format!("{url}: not text"))
}

/// The web demo: fetch a session's project (and chart), open it in the
/// page, and show it.
#[component]
pub fn WebDemo(name: String, rpp_url: String, chart_url: Option<String>) -> Element {
    let opened = use_resource(move || {
        let (name, rpp_url, chart_url) = (name.clone(), rpp_url.clone(), chart_url.clone());
        async move {
            let rpp = fetch_text(&rpp_url).await?;
            let chart = match &chart_url {
                Some(url) => Some(fetch_text(url).await?),
                None => None,
            };
            crate::web_engine::open(&name, &rpp, chart.as_deref())
                .await
                .map_err(|e| e.to_string())
        }
    });
    let state = opened.read();
    match &*state {
        None => rsx! {
            div { style: "color:#9aa0a6; font:14px system-ui; padding:24px;", "Opening the session…" }
        },
        Some(Err(e)) => rsx! {
            div { style: "color:#f87171; font:14px system-ui; padding:24px;", "Could not open the session: {e}" }
        },
        Some(Ok((engine, session))) => rsx! {
            DemoView { engine: engine.clone(), session: session.clone() }
        },
    }
}

const BAR_H: f64 = 40.0;
const BAR_BG: &str = "#17181b";
const RULE: &str = "#2a2c31";
const TEXT: &str = "#e5e7eb";
const DIM: &str = "#8b9099";
const ACCENT: &str = "#3aa0ff";

/// The views the demo's top bar switches between (the desktop app's).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum View {
    Performance,
    Daw,
}

impl View {
    const ALL: [Self; 2] = [Self::Performance, Self::Daw];

    const fn name(self) -> &'static str {
        match self {
            Self::Performance => "Performance",
            Self::Daw => "DAW",
        }
    }
}

/// The opened demo: the desktop app's frame — a top bar with the views,
/// the transport and the mode — over the view that is up.
#[component]
fn DemoView(engine: crate::web_engine::EngineRef, session: crate::studio::StudioSession) -> Element {
    use session::modes::Mode;
    use_context_provider(|| session.clone());
    let mut view = use_signal(|| View::Daw);
    let mut mode = use_signal(|| Mode::Live);
    use_context_provider(|| mode);
    let mut picking = use_signal(|| false);
    let segment = |on: bool| {
        let (bg, fg) = if on { (ACCENT, "#0b0c0e") } else { ("transparent", DIM) };
        format!(
            "height:24px; padding:0 12px; border:none; border-radius:5px; cursor:pointer; \
             background:{bg}; color:{fg}; font-size:12px; font-weight:600;"
        )
    };
    let option = |on: bool| {
        let (bg, fg) = if on { ("#23262c", TEXT) } else { ("transparent", DIM) };
        format!(
            "padding:6px 10px; border-radius:5px; cursor:pointer; background:{bg}; \
             color:{fg}; font-size:12px;"
        )
    };
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; width:100vw; height:100vh; display:flex; \
                    flex-direction:column; background:#0f1012; color:{TEXT}; \
                    font-family:system-ui, sans-serif;",
            div {
                style: "position:relative; height:{BAR_H}px; flex:none; display:flex; \
                        align-items:center; gap:8px; padding:0 10px 0 12px; \
                        background:{BAR_BG}; border-bottom:1px solid {RULE};",
                span {
                    style: "font-size:13px; font-weight:600; color:{TEXT}; margin-right:8px;",
                    "Session"
                }
                div {
                    style: "display:flex; gap:2px; padding:2px; background:#0f1012; \
                            border:1px solid {RULE}; border-radius:7px;",
                    for each in View::ALL {
                        button {
                            style: segment(view() == each),
                            onclick: move |_| view.set(each),
                            "{each.name()}"
                        }
                    }
                }
                div { style: "flex:1;" }
                crate::transport_bar::WebTransportBar {}
                div {
                    style: "position:relative;",
                    button {
                        style: "display:flex; align-items:center; gap:6px; height:26px; \
                                padding:0 10px; border-radius:6px; border:1px solid {RULE}; \
                                background:#0f1012; color:{TEXT}; font-size:12px; cursor:pointer;",
                        onclick: move |_| picking.toggle(),
                        span { style: "color:{DIM};", "Mode" }
                        span { style: "font-weight:600;", "{mode().display_name()}" }
                    }
                    if picking() {
                        div {
                            style: "position:absolute; right:0; top:30px; z-index:10; \
                                    min-width:160px; padding:4px; background:{BAR_BG}; \
                                    border:1px solid {RULE}; border-radius:8px; \
                                    box-shadow:0 8px 24px rgba(0,0,0,0.5);",
                            for each in Mode::ALL {
                                div {
                                    style: option(mode() == each),
                                    onclick: move |_| {
                                        mode.set(each);
                                        picking.set(false);
                                    },
                                    "{each.display_name()}"
                                }
                            }
                        }
                    }
                }
            }
            div {
                style: "position:relative; flex:1; min-height:0;",
                match view() {
                    View::Daw => rsx! { crate::mixer_panel::WebDawPanels { engine } },
                    View::Performance => rsx! {
                        div {
                            style: "position:absolute; top:0; left:0; right:0; bottom:0; display:flex; \
                                    flex-direction:column; gap:16px; padding:16px;",
                            crate::progress::ProgressBar {}
                            div {
                                style: "position:relative; flex:1; min-height:0; border-radius:8px; \
                                        overflow:hidden; border:1px solid {RULE};",
                                crate::chart_panel::WebChart {}
                            }
                            crate::progress::TransportButtons {}
                        }
                    },
                }
            }
        }
    }
}
