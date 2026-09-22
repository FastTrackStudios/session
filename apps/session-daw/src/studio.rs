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

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use dioxus::prelude::*;

use daw_ui::studio::{ProjectRef, RowsRef};


/// Pixels a second at zoom 1 — the studio's base scale.
pub const PPS: f64 = 40.0;

use crate::panel::BAR;

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
/// plan folds anything), the scene it was laid out by, and the track kinds
/// the template wrote into the file (read once, at open).
#[derive(Clone)]
pub struct Planner {
    pub raw: Arc<daw_ui::studio::project::Project>,
    pub scene: Option<&'static str>,
    pub kinds: Arc<crate::plan::Kinds>,
}

impl Planner {
    /// Plan the rows for `raw`: [`plan_rows`] with this planner's scene
    /// and file.
    #[must_use]
    pub fn plan(&self, raw: &daw_ui::studio::project::Project) -> (ProjectRef, RowsRef) {
        plan_rows_with(raw, self.scene, &self.kinds)
    }
}

impl PartialEq for StudioSession {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.project.0, &other.project.0) && Arc::ptr_eq(&self.rows.0, &other.rows.0)
    }
}

#[cfg(feature = "native")]
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
            kinds: Arc::new(crate::plan::Kinds::read(path)),
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
#[cfg(feature = "native")]
#[must_use]
pub fn read_back(
    scene: Option<&str>,
    project_path: &std::path::Path,
) -> Option<(ProjectRef, RowsRef)> {
    Some(plan_rows(&fetch()?, scene, project_path))
}

/// The open project, as the engine has it.
#[cfg(feature = "native")]
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
    plan_rows_with(raw, scene, &crate::plan::Kinds::read(project_path))
}

/// [`plan_rows`], with the track kinds already read.
#[must_use]
pub fn plan_rows_with(
    raw: &daw_ui::studio::project::Project,
    scene: Option<&str>,
    kinds: &crate::plan::Kinds,
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
        planned = crate::plan::apply_scene(
            &planned,
            kinds,
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

/// The arrangement panel, on dioxus-native: the widget as a Blitz custom
/// widget, winit's window events translated into [`crate::panel`]'s.
///
/// No props: the session comes from context ([`StudioSession`]), so the
/// same panel renders in any tile of any window.
#[cfg(feature = "native")]
#[component]
pub fn Arrangement() -> Element {
    use crate::panel::{Button, PanelEvent, WHEEL_LINE};
    let window = dioxus_native::use_window();
    let sink: crate::tool::Sink =
        Rc::new(move |icon| window.set_cursor(winit::cursor::Cursor::Icon(icon)));
    let applier = use_hook(|| Rc::new(crate::engine::Applier::start()));
    let engine: crate::panel::Engine = {
        let applier = Rc::clone(&applier);
        Rc::new(move |edit| match applier.as_ref() {
            Some(applier) => applier.send(edit),
            None => tracing::debug!(?edit, "no engine to carry out the edit"),
        })
    };
    let (panel, widget) = crate::panel::use_arrangement_panel(
        Some(sink),
        engine,
        dioxus_native_dom::CustomWidgetAttr::new,
    );
    let transport = crate::engine::Transport::shared();
    let events = panel.clone();
    dioxus_native::use_window_event(move |event, _| {
        let panel = &events;
        match event {
            winit::event::WindowEvent::KeyboardInput { event, .. } => {
                // By the physical key, not the character: with Shift held
                // the character is "Z", and Shift+z is a zoom of its own.
                if event.physical_key
                    == winit::keyboard::PhysicalKey::Code(winit::keyboard::KeyCode::KeyZ)
                {
                    panel.handle(PanelEvent::ZKey {
                        pressed: event.state.is_pressed(),
                        repeat: event.repeat,
                    });
                }
            }
            winit::event::WindowEvent::ModifiersChanged(state) => {
                let keys = state.state();
                panel.handle(PanelEvent::Modifiers(crate::mousemap::Mods {
                    shift: keys.shift_key(),
                    ctrl: keys.control_key(),
                    alt: keys.alt_key(),
                }));
            }
            winit::event::WindowEvent::PointerMoved { position, .. } => {
                panel.handle(PanelEvent::Pointer {
                    x: position.x,
                    y: position.y,
                });
            }
            winit::event::WindowEvent::PointerButton { state, button, .. } => {
                let button = match button {
                    winit::event::ButtonSource::Mouse(winit::event::MouseButton::Middle) => {
                        Button::Middle
                    }
                    winit::event::ButtonSource::Mouse(winit::event::MouseButton::Left) => {
                        Button::Left
                    }
                    winit::event::ButtonSource::Mouse(_) => Button::Other,
                    _ => Button::Left,
                };
                panel.handle(PanelEvent::Button {
                    button,
                    pressed: state.is_pressed(),
                });
            }
            winit::event::WindowEvent::MouseWheel { delta, .. } => {
                let (dx, dy) = match delta {
                    winit::event::MouseScrollDelta::LineDelta(x, y) => {
                        (f64::from(*x) * WHEEL_LINE, f64::from(*y) * WHEEL_LINE)
                    }
                    winit::event::MouseScrollDelta::PixelDelta(at) => (at.x, at.y),
                };
                panel.handle(PanelEvent::Wheel { dx, dy });
            }
            winit::event::WindowEvent::RedrawRequested => {
                let play_at = transport.map_or(0.0, |t| t.read().0);
                panel.handle(PanelEvent::Frame { play_at });
            }
            _ => {}
        }
    });

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
            crate::panel::PanelChrome { panel: panel.clone() }
        }
    }
}

/// One scrollbar: a track, and a thumb saying where in the session the
/// view is and how much of it is on screen. Dragged: the thumb IS the view.
#[component]
pub fn ScrollBar(
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
