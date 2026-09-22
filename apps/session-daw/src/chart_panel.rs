//! The Chart panel: the song's keyflow chart, engraved and painted with a
//! live playback cursor, pannable and zoomable — the native counterpart to
//! the web chart panes.
//!
//! Session's own `apps/desktop/src/session_chart_pane.rs` and, further
//! along the same design, `task`'s
//! `crates/player-ui/src/session_chart_pane.rs` (paginated A4 pages,
//! drag-to-pan, wheel-to-zoom, auto-follow to the playhead's page) render
//! fontless SVG through a browser engine — no wgpu, no canvas — because
//! that is what a wasm build can do. Blitz is not a browser, so this
//! paints straight into the widget's own scene instead, through
//! `keyflow::engraver::renderer::ChartView`: the SAME layout engine, the
//! same [`keyflow::engraver::layout::chart::cursor::ChartCursor`] playhead
//! math, and the same embedded fonts those SVG panes call — only the last
//! step, painting, differs from theirs. `ChartView` also paints the page
//! white, behind the notation, the way sheet music reads.
//!
//! **Interaction** matches the keyflow site (a browser: a trackpad's
//! two-finger scroll pans a page, an actual mouse wheel or ctrl+scroll
//! zooms it — Safari's and Chrome's own split): a trackpad scroll (winit
//! delivers it as `PixelDelta`, smooth and on both axes at once) pans;
//! ctrl+scroll, or a plain wheel on an actual mouse (`LineDelta`, discrete
//! single-axis steps — winit's own way of telling the two apart on
//! macOS), zooms. A middle-drag pans too, for a mouse that has one.
//!
//! **The count-in offset.** The DAW's transport and the chart's own layout
//! do not share a zero: the chart's timeline starts at its first REAL
//! measure (the count-in is a header snippet at negative chart time), while
//! the transport's seconds run from the project's own start, before any
//! count-in. This panel reads the SONGSTART marker once at construction and
//! subtracts it from every transport read, so a playhead over the count-in
//! lands on the count-in header rather than nowhere — the same subtraction
//! task's pane docs as its own playhead model.
//!
//! **v1**, scoped down from task's pane: one continuous column rather than
//! paginated A4 pages, and no auto-follow yet (manual pan takes priority —
//! the two would fight without a "the user just touched it" timeout, which
//! is the thing to add before bringing auto-follow back). Bringing
//! pagination here is the next step; port `DocRender`'s page-box math from
//! task's pane rather than rewriting it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use anyrender::{RenderContext, Scene};
use blitz_dom::node::{ComputedStyles, Widget};
use dioxus::prelude::*;

use keyflow::engraver::renderer::{ChartView, points_to_px};

use crate::studio::StudioSession;

/// How far one wheel notch zooms, as a factor.
const ZOOM_STEP: f64 = 1.12;
const ZOOM_MIN: f64 = 0.2;
const ZOOM_MAX: f64 = 6.0;

/// What the widget reads (and partly writes) every paint, shared with the
/// panel's own window-level input handling — the same shape
/// `crate::widget::Shared` gives the arrangement.
struct Live {
    /// Pan, in chart points — zoom-independent, see [`ChartView::paint`].
    scroll_pt: (f64, f64),
    zoom: f64,
    /// The chart's own content size in points, and the device-pixels-per-
    /// point factor (at the CURRENT `zoom`) — both written by the widget
    /// after each paint, so the input handling can turn a screen-pixel
    /// drag or a wheel notch into points without knowing keyflow's DPI
    /// constant itself. One frame behind a size or a zoom change, which
    /// self-corrects the next frame — the same trade-off `ChartView`'s own
    /// `cursor_y_pt` makes.
    content_pt: (f64, f64),
    px_per_pt: f64,
}

type Shared = Rc<RefCell<Live>>;

/// The widget: owns the layout cache; the pan and zoom live in [`Live`].
struct ChartWidget {
    chart: std::sync::Arc<keyflow::Chart>,
    /// Stable for the session's lifetime — [`Rc::as_ptr`], not a content
    /// hash: the chart this widget holds never changes after
    /// [`StudioSession::open`], so identity is all [`ChartView`] needs to
    /// know when to keep its cached layout.
    key: u64,
    view: ChartView,
    live: Shared,
    /// The SONGSTART marker's position, in project seconds — subtracted
    /// from the transport's read to get chart time. `None` with no such
    /// marker (an unprepared session, or one opened without a chart).
    songstart: Option<f64>,
    /// One page at a time, filled to the panel and following the song
    /// ([`Live::paged`]) — the Overview's chart, and any pane too small
    /// to be read by scrolling.
    paged: bool,
    /// The page a paged panel is showing (1-indexed), kept between frames
    /// so a playhead outside the chart leaves the page where it was.
    page: u32,
}

impl ChartWidget {
    /// The picture: what Blitz's `Widget::paint` returns, and what the web
    /// host draws into its canvas.
    pub fn paint_scene(&mut self, width: u32, height: u32, scale: f64) -> Scene {
        let (w, h) = (f64::from(width), f64::from(height));
        let chart_secs = self
            .songstart
            .zip(crate::engine::Transport::shared())
            .map(|(songstart, t)| t.read().0 - songstart);
        if self.paged {
            self.fit_page(w, h, scale, chart_secs);
        }
        let (zoom, scroll_pt) = {
            let live = self.live.borrow();
            (live.zoom, live.scroll_pt)
        };

        let mut out = Scene::new();
        let content_pt = self.view.paint(
            &mut out, &self.chart, self.key, w, scale, zoom, scroll_pt, chart_secs,
        );
        let mut live = self.live.borrow_mut();
        live.content_pt = content_pt;
        live.px_per_pt = points_to_px(scale) * zoom;
        out
    }

    /// Put ONE page on the panel: the page the playhead is on, its corners
    /// on the panel's corners.
    ///
    /// The zoom is whatever makes the page fit — by its width or its
    /// height, whichever runs out first — and the scroll is the page's own
    /// corner, so there is no paper showing beside it and no gap above it.
    /// A page is A4-ish and a panel rarely is, so one axis has room left
    /// over; nothing is cropped to take it.
    ///
    /// Before the downbeat (the count-in, or stopped at zero) that is the
    /// first page; past the chart's end, the page it ended on stays up.
    fn fit_page(&mut self, w: f64, h: f64, scale: f64, chart_secs: Option<f64>) {
        match chart_secs {
            Some(secs) if secs < 0.0 => self.page = 1,
            Some(secs) => {
                if let Some(number) = self.view.page_number_at_time(secs) {
                    self.page = number;
                }
            }
            None => {}
        }
        let Some((x, y, page_w, page_h)) = self.view.page(self.page) else {
            return;
        };
        let per_pt = points_to_px(scale);
        if page_w <= 0.0 || page_h <= 0.0 || per_pt <= 0.0 {
            return;
        }
        let zoom = (w / (page_w * per_pt)).min(h / (page_h * per_pt));
        let mut live = self.live.borrow_mut();
        live.zoom = zoom;
        live.scroll_pt = (x, y);
    }
}

impl Widget for ChartWidget {
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

/// The panel. Reads [`StudioSession::chart`] from context; when the session
/// has none it says so rather than showing an empty page.
/// The chart widget for the session's chart, and the view state its input
/// drives. `None` without a chart (or with the fonts failing to build).
fn build(session: &StudioSession, paged: bool) -> Option<(ChartWidget, Shared)> {
    let chart = session.chart.clone()?;
    let view = match ChartView::new() {
        Ok(view) => view,
        Err(e) => {
            tracing::error!(error = %e, "chart panel: font bundle failed");
            return None;
        }
    };
    let key = std::sync::Arc::as_ptr(&chart) as usize as u64;
    let live: Shared = Rc::new(RefCell::new(Live {
        scroll_pt: (0.0, 0.0),
        zoom: 1.0,
        content_pt: (1.0, 1.0),
        px_per_pt: 1.0,
    }));
    let widget = ChartWidget {
        chart,
        key,
        view,
        live: Rc::clone(&live),
        songstart: songstart_secs(&session.project),
        paged,
        page: 1,
    };
    Some((widget, live))
}

/// Move the view by a screen-pixel delta, in POINTS (so it stays
/// physically anchored to the content as the zoom changes).
fn pan_by(live: &Shared, dx_px: f64, dy_px: f64) {
    let mut live = live.borrow_mut();
    let k = live.px_per_pt.max(f64::EPSILON);
    let (cw, ch) = live.content_pt;
    // A page and a half of slack past either edge — panning clean off the
    // content is normal while looking for something, and the clamp is only
    // there to stop a drag from running away into the thousands.
    live.scroll_pt.0 = (live.scroll_pt.0 - dx_px / k).clamp(-cw * 0.5, cw * 1.5);
    live.scroll_pt.1 = (live.scroll_pt.1 - dy_px / k).clamp(-ch * 0.5, ch * 1.5);
}

/// Zoom by `notches` steps (positive in).
fn zoom_by(live: &Shared, notches: f64) {
    if notches.abs() < f64::EPSILON {
        return;
    }
    let mut live = live.borrow_mut();
    live.zoom = (live.zoom * ZOOM_STEP.powf(notches)).clamp(ZOOM_MIN, ZOOM_MAX);
}

/// No chart: a quiet line where it would be.
#[component]
fn NoChart() -> Element {
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; width:100%; height:100%; \
                    display:flex; align-items:center; justify-content:center; \
                    color:#8b9099; font-size:12px; background:#1a1b1e;",
            "No chart for this session."
        }
    }
}

/// The chart panel in a browser: the widget in a canvas; a wheel or a
/// trackpad pans, ctrl+wheel (and a pinch, which browsers deliver as one)
/// zooms, a middle-drag pans.
#[cfg(feature = "web")]
#[component]
pub fn WebChart(
    /// One page at a time, filled to the panel and following the song.
    #[props(default)]
    paged: bool,
) -> Element {
    use crate::panel::{Button, PanelEvent};
    let session: StudioSession = use_context();
    let built = use_hook(|| {
        build(&session, paged).map(|(widget, live)| {
            (
                crate::web_host::HostedRef(Rc::new(RefCell::new(widget))),
                live,
            )
        })
    });
    let Some((widget, live)) = built else {
        return rsx! { NoChart {} };
    };
    let input = use_hook(|| Rc::new(Cell::new((false, None::<(f64, f64)>))));
    let on_input = move |event: PanelEvent| {
        let (ctrl, drag) = input.get();
        match event {
            PanelEvent::Modifiers(mods) => input.set((mods.ctrl, drag)),
            PanelEvent::Button { button: Button::Middle, pressed } => {
                input.set((ctrl, pressed.then_some((f64::NAN, f64::NAN))));
            }
            PanelEvent::Pointer { x, y } => {
                if let Some(from) = drag {
                    if !from.0.is_nan() {
                        pan_by(&live, x - from.0, y - from.1);
                    }
                    input.set((ctrl, Some((x, y))));
                }
            }
            PanelEvent::Wheel { dx, dy } => {
                if ctrl {
                    zoom_by(&live, dy / 40.0);
                } else {
                    pan_by(&live, dx, dy);
                }
            }
            _ => {}
        }
    };
    rsx! {
        div {
            style: "position:absolute; top:0; left:0; width:100%; height:100%; \
                    overflow:hidden; background:#1a1b1e;",
            crate::web_host::WidgetCanvas { widget, panel: on_input }
        }
    }
}

#[cfg(feature = "web")]
impl crate::web_host::Hosted for ChartWidget {
    fn paint(&mut self, width: u32, height: u32, scale: f64) -> Scene {
        self.paint_scene(width, height, scale)
    }
    fn event(&mut self, _event: &blitz_traits::events::UiEvent) {}
}

#[cfg(feature = "native")]
#[component]
pub fn Chart(
    /// One page at a time, filled to the panel and following the song.
    #[props(default)]
    paged: bool,
) -> Element {
    let session: StudioSession = use_context();
    let built = use_hook(|| {
        build(&session, paged)
            .map(|(widget, live)| (dioxus_native_dom::CustomWidgetAttr::new(widget), live))
    });
    let Some((widget, live)) = built else {
        return rsx! { NoChart {} };
    };

    // This panel's rectangle in the window — needed to turn a window-space
    // drag or wheel into a panel-local one, and to filter a middle-drag to
    // presses that started inside it. Blitz does not route the wheel
    // through the DOM at all, so this happens at the winit level, the same
    // idiom `crate::studio::Arrangement` uses for its own wheel and drag.
    let mut rect = use_signal(|| (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64));
    let mounted = use_hook(|| Rc::new(RefCell::new(None::<Rc<MountedData>>)));
    let dragging = use_hook(|| Rc::new(Cell::new(Option::<(f64, f64)>::None)));
    // The pointer's last known window position, tracked on every move —
    // not only during a drag — so a bare wheel notch can be told whether
    // it landed over this panel at all.
    let pointer = use_hook(|| Rc::new(Cell::new((0.0_f64, 0.0_f64))));
    let ctrl_held = use_hook(|| Rc::new(Cell::new(false)));
    let measured = use_hook(|| Rc::new(Cell::new(None::<web_time::Instant>)));

    let measuring = Rc::clone(&mounted);
    let drag_state = Rc::clone(&dragging);
    let tracking = Rc::clone(&pointer);
    let ctrl = Rc::clone(&ctrl_held);
    let driving = Rc::clone(&live);
    dioxus_native::use_window_event(move |event, _| {
        let r = *rect.peek();
        let inside = |p: (f64, f64)| p.0 >= r.0 && p.0 < r.0 + r.2 && p.1 >= r.1 && p.1 < r.1 + r.3;
        // Move the view by a screen-pixel delta, in POINTS (so it stays
        // physically anchored to the content as `zoom` changes) — shared by
        // a middle-drag and a trackpad's two-finger scroll.
        let pan_by = |dx_px: f64, dy_px: f64| pan_by(&driving, dx_px, dy_px);
        let zoom_by = |dy: f64| zoom_by(&driving, dy);
        match event {
            winit::event::WindowEvent::ModifiersChanged(state) => {
                ctrl.set(state.state().control_key());
            }
            winit::event::WindowEvent::PointerButton { state, button, position, .. } => {
                let which = match button {
                    winit::event::ButtonSource::Mouse(button) => *button,
                    _ => winit::event::MouseButton::Left,
                };
                let at = (position.x, position.y);
                if which == winit::event::MouseButton::Middle {
                    drag_state.set(state.is_pressed().then_some(at).filter(|&p| inside(p)));
                }
            }
            winit::event::WindowEvent::PointerMoved { position, .. } => {
                let at = (position.x, position.y);
                tracking.set(at);
                let Some(from) = drag_state.get() else { return };
                drag_state.set(Some(at));
                pan_by(at.0 - from.0, at.1 - from.1);
            }
            winit::event::WindowEvent::MouseWheel { delta, .. } => {
                if !inside(tracking.get()) {
                    return;
                }
                // A trackpad's two-finger scroll arrives as `PixelDelta`
                // (smooth, both axes) on macOS; an actual mouse wheel
                // arrives as `LineDelta` (discrete, one axis) — winit's own
                // way of telling the two apart, and what this uses to pick
                // pan (a trackpad scroll, the natural gesture) from zoom (a
                // real wheel, or either device with ctrl held — Safari's
                // and Chrome's own split).
                match delta {
                    // A real wheel always zooms — it has no second axis to
                    // pan with.
                    winit::event::MouseScrollDelta::LineDelta(_, y) => zoom_by(f64::from(*y)),
                    // A trackpad scroll pans, unless ctrl is held (Safari's
                    // and Chrome's own zoom modifier).
                    winit::event::MouseScrollDelta::PixelDelta(at) if ctrl.get() => {
                        zoom_by(at.y / 40.0);
                    }
                    winit::event::MouseScrollDelta::PixelDelta(at) => pan_by(at.x, at.y),
                }
            }
            winit::event::WindowEvent::RedrawRequested => {
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
        }
    });

    rsx! {
        div {
            style: "position:absolute; top:0; left:0; width:100%; height:100%; \
                    overflow:hidden; background:#1a1b1e;",
            onmounted: move |event| {
                *mounted.borrow_mut() = Some(event.data());
            },
            object {
                style: "position:absolute; top:0; left:0; width:100%; height:100%;",
                data: widget.clone(),
            }
        }
    }
}

/// Where the MARKS lane's SONGSTART marker sits, in project seconds — the
/// chart's own time zero. `None` when the session has no such marker.
fn songstart_secs(project: &daw_ui::studio::project::Project) -> Option<f64> {
    project.markers.iter().find(|m| m.name == "SONGSTART").map(|m| m.at)
}

#[cfg(test)]
mod tests {
    use keyflow::engraver::renderer::ChartView;

    /// A chart long enough for several pages: the page the playhead is on
    /// moves with it. The pages sit side by side — every one at the same
    /// `y` — which is why the page is found by the cursor's own number;
    /// found by position, it was always the last.
    #[test]
    fn the_page_follows_the_playhead_across_pages() {
        let chart = keyflow::parse("Song - Artist\n120bpm 4/4 #C\n\nvs 160\nC G Am F x40\n")
            .expect("parses");
        let mut view = ChartView::new().expect("the fonts load");
        let mut scene = anyrender::Scene::new();
        view.paint(&mut scene, &chart, 1, 800.0, 1.0, 1.0, (0.0, 0.0), None);
        assert!(view.pages() > 1, "{} page(s): not long enough", view.pages());

        assert_eq!(view.page_number_at_time(0.5), Some(1));
        let last = view
            .page_number_at_time(310.0)
            .expect("310 s is inside 160 bars at 120");
        assert!(last > 1, "the end of the chart is still on page {last}");

        let (first_x, first_y, ..) = view.page(1).expect("page one");
        let (last_x, last_y, ..) = view.page(last).expect("the last page");
        assert!(last_x > first_x, "pages sit side by side");
        assert!((last_y - first_y).abs() < f64::EPSILON);
    }
}
