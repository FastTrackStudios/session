//! The studio as a panel: the painted arrangement, its ruler, its track
//! panel and its scrollbars, in whatever rectangle it is given.
//!
//! Lifted out of `bin/blitz_shot.rs`, where it was one whole window, so the
//! Session app can dock it beside the other panels (`docs/app-on-blitz.md`).
//! What changed on the way:
//!
//! - **Sized by its container**, not by the window. The window may hold
//!   other panels; this one fills the tile it is put in and reads its own
//!   rectangle back from the layout.
//! - **Input only over itself.** Blitz does not send wheel events to the
//!   DOM, so the wheel, the middle-drag hand and the zoom spring are read
//!   at the winit level — and with other panels in the window, only while
//!   the pointer is inside this panel's rectangle.
//! - **No side rails and no modes.** The modes belong to the app's top bar
//!   now, visible whichever view is up.
//!
//! [`StudioSession`] is what the panel stands on: the opened, prepared
//! session and the reads the widget is built from. The app opens it once
//! and provides it as context; every Arrangement panel reads that.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use dioxus::prelude::*;

use daw_ui::studio::{ProjectRef, RowsRef};

use crate::arrangement::TCP_WIDTH;

/// Pixels a second at zoom 1 — the studio's base scale.
pub const PPS: f64 = 40.0;

/// How thick a scrollbar is.
const BAR: f64 = 12.0;

/// One wheel "line", in pixels.
const WHEEL_LINE: f64 = 40.0;

/// How far in each zoom may go.
const ZOOM_IN: (f64, f64) = (32.0, 6.0);

/// The arrangement's queue of edits for the engine, shared with what sits on
/// the panel beside the widget (the main toolbar). Equal by identity.
#[derive(Clone)]
pub struct Edits(pub Rc<RefCell<Vec<crate::engine::Edit>>>);

impl PartialEq for Edits {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// The opened session, as the panels read it.
///
/// Cheap to clone: everything in it is shared.
#[derive(Clone)]
pub struct StudioSession {
    pub project: ProjectRef,
    pub rows: RowsRef,
    pub previews: crate::midi::Previews,
    /// The song's chart, parsed once at [`StudioSession::open`] — what the
    /// Chart panel paints (`crate::chart_panel`). `None` when the session
    /// was opened with no `--chart`, or the file failed to parse: the DAW
    /// and Performance views still work without it.
    pub chart: Option<Arc<keyflow::Chart>>,
    /// How the rows were planned, kept so they can be planned again after
    /// a change the plan depends on (a track shown or hidden) without
    /// reading the whole session back from the engine.
    pub planner: Planner,
}

/// The row plan's inputs: the project as the engine had it (before the
/// plan folds anything), the scene it was laid out by, and the file (for
/// the track kinds the template wrote).
#[derive(Clone)]
pub struct Planner {
    pub raw: Arc<daw_ui::studio::project::Project>,
    pub scene: Option<&'static str>,
    pub path: Arc<std::path::PathBuf>,
}

impl Planner {
    /// Plan the rows for `raw`: [`plan_rows`] with this planner's scene
    /// and file.
    #[must_use]
    pub fn plan(&self, raw: &daw_ui::studio::project::Project) -> (ProjectRef, RowsRef) {
        plan_rows(raw, self.scene, &self.path)
    }
}

impl PartialEq for StudioSession {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.project.0, &other.project.0) && Arc::ptr_eq(&self.rows.0, &other.rows.0)
    }
}

impl StudioSession {
    /// Open `path` on the process's engine with audio, run `prepare` on it
    /// (organize / chart / guide — see [`crate::prepare`]), and read it back
    /// the way the studio lays it out.
    ///
    /// # Errors
    ///
    /// The project could not be opened or read back. A prepare step that
    /// fails is logged and the session opens as it was.
    pub fn open(path: &std::path::Path, prepare: &crate::prepare::Prepare) -> eyre::Result<Self> {
        let opened = crate::open::open_and_serve(path)?;
        if !prepare.is_empty()
            && let Err(e) = prepare.run(&opened)
        {
            tracing::error!(error = %e, "preparing the session failed; opening it as it was");
        }
        const SCENE: &str = "drum-mixing";
        let raw = fetch().ok_or_else(|| eyre::eyre!("could not read {} back", path.display()))?;
        let planner = Planner {
            raw: Arc::new(raw),
            scene: Some(SCENE),
            path: Arc::new(path.to_path_buf()),
        };
        let (project, rows) = planner.plan(&planner.raw);
        let previews = previews_of(&project);
        let chart = prepare.chart.as_deref().and_then(|chart_path| {
            let text = std::fs::read_to_string(chart_path)
                .inspect_err(|e| tracing::error!(error = %e, path = %chart_path.display(), "chart: could not read"))
                .ok()?;
            keyflow::parse(text.as_str())
                .inspect_err(|e| tracing::error!(error = %e, path = %chart_path.display(), "chart: could not parse"))
                .ok()
                .map(Arc::new)
        });
        Ok(Self {
            project,
            rows,
            previews,
            chart,
            planner,
        })
    }
}

/// The open project, as the studio's own refs: every visible row, laid out
/// by `scene` (the visual track manager's scene slug) when one is given.
#[must_use]
pub fn read_back(
    scene: Option<&str>,
    project_path: &std::path::Path,
) -> Option<(ProjectRef, RowsRef)> {
    Some(plan_rows(&fetch()?, scene, project_path))
}

/// The open project, as the engine has it.
#[must_use]
pub fn fetch() -> Option<daw_ui::studio::project::Project> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    rt.block_on(daw_ui::studio::project::fetch())
}

/// The rows of `raw`, laid out by `scene`: the visible tracks (a hidden
/// folder hides what it holds), planned and folded the way the studio
/// shows them.
#[must_use]
pub fn plan_rows(
    raw: &daw_ui::studio::project::Project,
    scene: Option<&str>,
    project_path: &std::path::Path,
) -> (ProjectRef, RowsRef) {
    let project = ProjectRef(Arc::new(raw.clone()));
    let (visible, depths) =
        daw_ui::components::folders::FolderState::default().visible(&project.tracks);
    let mut planned: Vec<(daw_proto::Track, u32)> = visible.into_iter().zip(depths).collect();
    // The session file carries no per-track height; the heights are a
    // layout the scene decides.
    let settings = crate::settings::Settings {
        folded_takes: std::env::var("FTS_BLITZ_FOLDED_TAKES").as_deref() != Ok("0"),
        ..crate::settings::Settings::default()
    };
    if let Some(slug) = scene.and_then(dynamic_template::scenes::scene) {
        let kinds = crate::plan::Kinds::read(project_path);
        planned = crate::plan::apply_scene(
            &planned,
            &kinds,
            slug,
            crate::plan::Panel {
                surface: crate::plan::Surface::Arrange,
                mode: None,
                settings,
                extent: 1440.0,
                active_language: None,
            },
        );
    }
    // A folder the scene shut keeps its row and gets its children's items,
    // folded: the mix happens on the folder, so its name, fader and colour
    // stay, and so does what is in it.
    let mut project = project;
    {
        let shown: Vec<daw_proto::Track> = planned.iter().map(|(t, _)| t.clone()).collect();
        daw_ui::studio::folded::refold(
            Arc::make_mut(&mut project.0),
            &shown,
            settings.folded_takes,
        );
    }
    (project, RowsRef(Arc::new(planned)))
}

/// Every MIDI item's notes, read before anything is drawn — an item drawn
/// from a waveform it does not have is why a chord track once looked like
/// a shaker.
#[must_use]
pub fn previews_of(project: &ProjectRef) -> crate::midi::Previews {
    let previews = crate::midi::Previews::default();
    previews.fill_blocking(
        project
            .0
            .items
            .values()
            .flatten()
            .filter(|item| project.0.is_midi(&item.guid))
            .map(|item| (item.guid.clone(), item.length.as_seconds()))
            .collect(),
    );
    previews
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
fn zoom_about(at: f64, scroll: f64, was: f64, to: f64) -> f64 {
    if was <= 0.0 {
        return scroll;
    }
    (scroll + at) / was * to - at
}

/// A wheel or drag distance as a zoom factor.
fn factor(pixels: f64) -> f64 {
    (pixels / 200.0).exp()
}

/// The arrangement panel.
///
/// No props: the session comes from context ([`StudioSession`]), so the
/// same panel renders in any tile of any window.
#[component]
pub fn Arrangement() -> Element {
    let session: StudioSession = use_context();
    // The docked mixer's, when the view has one beside this panel.
    let mixer: Option<crate::mixer_panel::Links> = try_use_context();
    // The rows' height at zoom 1. A signal, not a number worked out
    // once: showing or hiding tracks (the visibility manager) changes it,
    // and the widget reports the new one through `content_h`.
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
    let window = dioxus_native::use_window();
    // The tool the panel holds and the pointer's shape, shared with the
    // widget — see `crate::tool`.
    let pointing = use_hook(|| crate::tool::Pointing::shared(Some(window)));
    // The which-key popup and the zooms the keys ask for, both written by
    // the widget (which owns the keyboard) and read here once a frame.
    let which = use_hook(crate::which_key::Shared::default);
    let zooms = use_hook(crate::zoom::Requests::default);
    let history = use_hook(|| Rc::new(RefCell::new(crate::zoom::History::default())));
    let mut which_shown = use_signal(|| None::<crate::which_key::WhichKey>);

    // The widget, built once. Its view — scroll, zoom, where the play
    // cursor is — is a plain cell it reads every paint, because the paint
    // runs outside the Dioxus runtime.
    let (widget, view, edits) = use_hook(|| {
        let theme = daw_ui::theming::Theme::dark();
        let palette = crate::arrangement::Palette::from_theme(&theme);
        let font = crate::text::Font::embedded().expect("the embedded font");
        let layout = crate::layout::Layout::from_env();
        let scene = crate::arrangement::Arrangement::build(
            &palette,
            &font,
            &session.project,
            &session.rows,
            layout,
            &session.previews,
        );
        let bpm = scene.bpm;
        let view: crate::widget::Shared = Rc::new(RefCell::new(crate::widget::View {
            scroll_x: 0.0,
            scroll_y: 0.0,
            zoom_x: 1.0,
            zoom_y: 1.0,
            play_at: 0.0,
        }));
        let built = crate::widget::ArrangementWidget::new(
            scene,
            palette,
            font,
            bpm,
            PPS,
            session.rows.as_slice().to_vec(),
            layout,
            (*session.project.0).clone(),
            session.previews.clone(),
            Rc::clone(&view),
            std::env::var("FTS_BLITZ_FPS").is_ok_and(|v| v != "0"),
        )
        .with_pointing(Rc::clone(&pointing))
        .with_view_links(Rc::clone(&which), Rc::clone(&zooms))
        .with_planner(session.planner.clone(), Rc::clone(&content_h));
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
        (dioxus_native_dom::CustomWidgetAttr::new(built), view, edits)
    });

    // Where the view is. Read by the scrollbars and written by the input;
    // the widget hears about it through `view`, once a frame.
    let mut scroll = use_signal(|| 0.0_f64);
    let mut down = use_signal(|| 0.0_f64);
    let zoom = use_signal(|| (1.0_f64, 1.0_f64));
    // This panel's rectangle in the window, read back from the layout.
    let mut rect = use_signal(|| (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64));
    let mounted = use_hook(|| Rc::new(RefCell::new(None::<Rc<MountedData>>)));

    let span_x = (session.project.length_secs * PPS).max(1.0);
    let mut span_y_signal = use_signal(|| content_h.get());
    let span_y_now = span_y_signal;

    // The lanes' frame: the panel less the track column and the ruler on
    // one side, and the scrollbars on the other.
    let frame = move |r: (f64, f64, f64, f64)| {
        (
            (r.2 - TCP_WIDTH - BAR).max(1.0),
            (r.3 - crate::ruler::ruler_h() - BAR).max(1.0),
        )
    };
    let extent = move |r: (f64, f64, f64, f64), zx: f64, zy: f64| {
        let (fw, fh) = frame(r);
        let span_y = *span_y_now.peek();
        ((span_x * zx - fw).max(0.0), (span_y * zy - fh).max(0.0))
    };
    // Out no further than the whole session filling the frame, and never
    // above one.
    let limits = move |r: (f64, f64, f64, f64)| {
        let (fw, fh) = frame(r);
        let floor = |f: f64, s: f64| {
            if s > 0.0 && f > 0.0 {
                (f / s).min(1.0)
            } else {
                1.0
            }
        };
        (
            (floor(fw, span_x), ZOOM_IN.0),
            (floor(fh, *span_y_now.peek()), ZOOM_IN.1),
        )
    };

    let applier = use_hook(|| Rc::new(crate::engine::Applier::start()));
    let transport = crate::engine::Transport::shared();
    let input = use_hook(|| Rc::new(RefCell::new(Input::default())));
    // When the rectangle was last read, so it is read a few times a
    // second rather than every frame.
    let measured = use_hook(|| Rc::new(Cell::new(None::<std::time::Instant>)));

    // The toolbar pushes onto the same queue the window drains.
    let queue = Edits(Rc::clone(&edits));
    let driving = Rc::clone(&input);
    let measuring = Rc::clone(&mounted);
    let showing = Rc::clone(&pointing);
    // The arrangement's own node, to give the keyboard back to.
    let focus_node = use_hook(|| {
        mixer.as_ref().map_or_else(
            || Rc::new(RefCell::new(None::<Rc<MountedData>>)),
            |links| Rc::clone(&links.arrange_node),
        )
    });
    let focusing = Rc::clone(&focus_node);
    dioxus_native::use_window_event(move |event, _| {
        let r = *rect.peek();
        let inside = |p: (f64, f64)| p.0 >= r.0 && p.0 < r.0 + r.2 && p.1 >= r.1 && p.1 < r.1 + r.3;
        // A tool going up or down, or a modifier changing what a drag
        // would do, changes the pointer's shape with the pointer standing
        // still, when the widget hears nothing. So the panel re-applies
        // it here. Before Blitz sees the event, which is fine: Blitz only
        // sets the shape itself when the pointer crosses into a new node.
        let reshape = |input: &Input| {
            let mut pointing = showing.borrow_mut();
            pointing.tool = input.tool();
            pointing.inside = inside(input.pointer);
            pointing.apply();
        };
        // Set the zoom, keeping the session point under `about` (a window
        // position) where it is on screen.
        let rezoom = move |to: (f64, f64), about: (f64, f64)| {
            let mut zoom = zoom;
            let mut scroll = scroll;
            let mut down = down;
            let (was_x, was_y) = *zoom.peek();
            let (fw, fh) = frame(r);
            let ax = (about.0 - r.0 - TCP_WIDTH).clamp(0.0, fw);
            let ay = (about.1 - r.1 - crate::ruler::ruler_h()).clamp(0.0, fh);
            let (ex, ey) = extent(r, to.0, to.1);
            let across = zoom_about(ax, *scroll.peek(), was_x, to.0);
            let deep = zoom_about(ay, *down.peek(), was_y, to.1);
            zoom.set(to);
            scroll.set(across.clamp(0.0, ex));
            down.set(deep.clamp(0.0, ey));
        };
        match event {
            winit::event::WindowEvent::KeyboardInput { event, .. } => {
                // By the physical key, not the character: with Shift held
                // the character is "Z", and Shift+z is a zoom of its own.
                if event.physical_key
                    == winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyZ)
                {
                    let mut input = driving.borrow_mut();
                    if event.state.is_pressed() && !event.repeat {
                        // A fresh hold: not yet used as the tool.
                        showing.borrow_mut().tool_used = false;
                    }
                    input.zooming = event.state.is_pressed();
                    reshape(&input);
                }
            }
            winit::event::WindowEvent::ModifiersChanged(state) => {
                let keys = state.state();
                let mut input = driving.borrow_mut();
                input.shift = keys.shift_key();
                showing.borrow_mut().mods = crate::mousemap::Mods {
                    shift: keys.shift_key(),
                    ctrl: keys.control_key(),
                    alt: keys.alt_key(),
                };
                reshape(&input);
            }
            winit::event::WindowEvent::PointerMoved { position, .. } => {
                let at = (position.x, position.y);
                let mut input = driving.borrow_mut();
                let moved = (at.0 - input.pointer.0, at.1 - input.pointer.1);
                input.pointer = at;
                let (zx, zy) = *zoom.peek();
                match input.drag {
                    Some(Drag::Pan) => {
                        let (ex, ey) = extent(r, zx, zy);
                        let across = *scroll.peek();
                        scroll.set((across - moved.0).clamp(0.0, ex));
                        let deep = *down.peek();
                        down.set((deep - moved.1).clamp(0.0, ey));
                    }
                    Some(Drag::Zoom) => {
                        showing.borrow_mut().tool_used = true;
                        let ((x0, x1), (y0, y1)) = limits(r);
                        rezoom(
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
                if input.tool() != crate::tool::Tool::Map || !inside(at) {
                    reshape(&input);
                }
            }
            winit::event::WindowEvent::PointerButton { state, button, .. } => {
                let mut input = driving.borrow_mut();
                let which = match button {
                    winit::event::ButtonSource::Mouse(button) => *button,
                    _ => winit::event::MouseButton::Left,
                };
                if which == winit::event::MouseButton::Middle {
                    input.panning = state.is_pressed() && inside(input.pointer);
                }
                // A drag starts only over this panel; it ends wherever
                // the button comes up.
                input.drag = match (state.is_pressed(), which) {
                    (true, winit::event::MouseButton::Middle) if inside(input.pointer) => {
                        Some(Drag::Pan)
                    }
                    (true, winit::event::MouseButton::Left)
                        if input.zooming && inside(input.pointer) =>
                    {
                        input.anchor = input.pointer;
                        Some(Drag::Zoom)
                    }
                    _ => None,
                };
                reshape(&input);
            }
            winit::event::WindowEvent::MouseWheel { delta, .. } => {
                let input = driving.borrow();
                if !inside(input.pointer) {
                    return;
                }
                let (dx, dy) = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => {
                        (f64::from(*x) * WHEEL_LINE, f64::from(*y) * WHEEL_LINE)
                    }
                    winit::event::MouseScrollDelta::PixelDelta(at) => (at.x, at.y),
                };
                let (zx, zy) = *zoom.peek();
                let ((x0, x1), (y0, y1)) = limits(r);
                if input.zooming {
                    showing.borrow_mut().tool_used = true;
                    // `z` zooms time, the common one; with shift, the rows.
                    // Either axis of the wheel counts: macOS turns a
                    // shifted wheel sideways, so the notch arrives in `dx`.
                    let notch = (dx + dy) * 2.0;
                    let to = if input.shift {
                        (zx, (zy * factor(notch)).clamp(y0, y1))
                    } else {
                        ((zx * factor(notch)).clamp(x0, x1), zy)
                    };
                    rezoom(to, input.pointer);
                } else {
                    let (ex, ey) = extent(r, zx, zy);
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
            winit::event::WindowEvent::RedrawRequested => {
                // The widget's view, once a frame: four numbers, written
                // every time because a missed write is a frame drawn in
                // the wrong place.
                let play_at = transport.map_or(0.0, |t| t.read().0);
                *view.borrow_mut() = crate::widget::View {
                    scroll_x: *scroll.peek(),
                    scroll_y: *down.peek(),
                    zoom_x: zoom.peek().0,
                    zoom_y: zoom.peek().1,
                    play_at,
                };
                // The zooms the keys asked for, and the popup: the widget
                // has no way to set a signal, so it leaves them here.
                let asked: Vec<_> = zooms.borrow_mut().drain(..).collect();
                for request in asked {
                    let (zx, zy) = *zoom.peek();
                    let now = crate::zoom::Target {
                        zoom_x: zx,
                        zoom_y: zy,
                        scroll_x: *scroll.peek(),
                        scroll_y: *down.peek(),
                    };
                    let (fw, fh) = frame(r);
                    let ((x0, x1), (y0, y1)) = limits(r);
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
                    let Some(to) = history.borrow_mut().go(now, request, framed) else {
                        continue;
                    };
                    let (ex, ey) = extent(r, to.zoom_x, to.zoom_y);
                    let mut zoom = zoom;
                    zoom.set((to.zoom_x, to.zoom_y));
                    scroll.set(to.scroll_x.clamp(0.0, ex));
                    down.set(to.scroll_y.clamp(0.0, ey));
                }
                // Rows shown or hidden: the scroll range follows.
                if content_h.get() != *span_y_signal.peek() {
                    span_y_signal.set(content_h.get());
                }
                // A held `z` used as the tool is the tool, not a prefix:
                // no popup over the zoom.
                let wanted = if showing.borrow().tool_used {
                    None
                } else {
                    which.borrow().clone()
                };
                if *which_shown.peek() != wanted {
                    which_shown.set(wanted);
                }
                // Whatever the widget asked to have done, carried out on
                // this thread, where the engine's sender is.
                let mut pending: Vec<_> = edits.borrow_mut().drain(..).collect();
                if let Some(links) = &mixer {
                    // `x`, from either widget.
                    if links.toggle.take() {
                        let mut open = links.open;
                        let was = *open.peek();
                        open.set(!was);
                        // Closing it: the keyboard comes back here, not to a
                        // panel that is no longer on screen.
                        if was && let Some(node) = focusing.borrow().clone() {
                            spawn(async move {
                                let _ = node.set_focus(true).await;
                            });
                        }
                    }
                    // Each widget's edits to the other one, never back to
                    // the one that made them (a toggle would flip twice).
                    links.to_mixer.borrow_mut().extend(pending.iter().cloned());
                    let from_mixer: Vec<_> = links.from_mixer.borrow_mut().drain(..).collect();
                    links.to_arrange.borrow_mut().extend(from_mixer.iter().cloned());
                    pending.extend(from_mixer);
                }
                for edit in pending {
                    match applier.as_ref() {
                        Some(applier) => applier.send(edit),
                        None => tracing::debug!(?edit, "no engine to carry out the edit"),
                    }
                }
                // Where this panel is, re-read a few times a second: a
                // dock divider or a window resize moves it without telling
                // anything here.
                let stale = measured
                    .get()
                    .is_none_or(|at| at.elapsed() > std::time::Duration::from_millis(200));
                if stale && let Some(node) = measuring.borrow().clone() {
                    measured.set(Some(std::time::Instant::now()));
                    spawn(async move {
                        if let Ok(got) = node.get_client_rect().await {
                            let next =
                                (got.origin.x, got.origin.y, got.size.width, got.size.height);
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

    // The generated Click track — what the toolbar's metronome mutes.
    let (click, click_muted) = session
        .project
        .tracks
        .iter()
        .find(|t| t.name.trim() == "Click")
        .map_or((None, true), |t| (Some(t.guid.clone()), t.muted));

    let r = rect();
    let (fw, fh) = frame(r);
    let (zx, zy) = zoom();
    let travel = extent(r, zx, zy);
    let colors = daw_ui::studio::lanes::Colors::from_theme(&daw_ui::theming::Theme::dark());
    let surface = colors.surface.clone();
    let ruler = crate::ruler::ruler_h();
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
            // The main toolbar, in the corner left of the ruler's lane
            // names.
            crate::toolbar::MainToolbar {
                click,
                click_muted,
                edits: queue,
                width: TCP_WIDTH - crate::ruler::LABEL_W,
            }
            crate::which_key::Panel { showing: which_shown(), colors: colors.clone() }
            ScrollBar {
                across: true,
                at: scroll(),
                travel: travel.0,
                window: fw,
                left: TCP_WIDTH,
                top: (r.3 - BAR).max(0.0),
                length: fw,
                colors: colors.clone(),
                on_move: move |to: f64| scroll.set(to.clamp(0.0, travel.0)),
            }
            ScrollBar {
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
}

/// One scrollbar: a track, and a thumb saying where in the session the
/// view is and how much of it is on screen. Dragged: the thumb IS the view.
#[component]
fn ScrollBar(
    across: bool,
    at: f64,
    travel: f64,
    window: f64,
    left: f64,
    top: f64,
    length: f64,
    colors: daw_ui::studio::lanes::Colors,
    on_move: EventHandler<f64>,
) -> Element {
    let whole = (travel + window).max(1.0);
    // Never smaller than a thumb you can hit, however long the session.
    let thumb = (window / whole * length).max(24.0);
    let along = (at / travel.max(1.0)) * (length - thumb);
    let mut held = use_signal(|| Option::<f64>::None);
    let (size, place) = if across {
        (
            format!("left:{left}px; top:{top}px; width:{length}px; height:{BAR}px;"),
            format!(
                "left:{along:.1}px; top:2px; width:{thumb:.1}px; height:{}px;",
                BAR - 4.0
            ),
        )
    } else {
        (
            format!("left:{left}px; top:{top}px; width:{BAR}px; height:{length}px;"),
            format!(
                "left:2px; top:{along:.1}px; width:{}px; height:{thumb:.1}px;",
                BAR - 4.0
            ),
        )
    };
    rsx! {
        div {
            style: "position:absolute; {size} background:{colors.tcp_column};",
            onmousedown: move |event| {
                let at = event.data().element_coordinates();
                held.set(Some(if across { at.x } else { at.y }));
            },
            onmouseup: move |_| held.set(None),
            onmouseleave: move |_| held.set(None),
            onmousemove: move |event| {
                if held().is_none() {
                    return;
                }
                let at = event.data().element_coordinates();
                let along = if across { at.x } else { at.y };
                let room = (length - thumb).max(1.0);
                let fraction = ((along - thumb / 2.0) / room).clamp(0.0, 1.0);
                on_move.call(fraction * travel.max(0.0));
            },
            div {
                style: "position:absolute; {place} background:{colors.text_dim}; border-radius:2px;",
            }
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
