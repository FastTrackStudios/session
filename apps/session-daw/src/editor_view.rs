//! The Editor view: the expression editor, up close, on the selected
//! track's item.
//!
//! [`crate::expression::Expression`] is the editor mounted as a box in
//! a window — its picture a scene, its gestures calls into the core.
//! This is the view around it: which item it is on (the selected
//! track's first MIDI item, as a drum kit's lanes when the track is a
//! drum), the window's events turned into the editor's, and a word on
//! screen when there is nothing to edit rather than an empty roll that
//! reads as a broken load.

#[cfg(feature = "native")]
use anyrender::Scene;
#[cfg(feature = "native")]
use blitz_traits::events::{BlitzWheelDelta, MouseEventButton, UiEvent};
use dioxus::prelude::*;

use crate::expression::Expression;

/// What the view is on: a track's MIDI item, by guid, with what the
/// editor needs to read it.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub track: String,
    pub item: String,
    pub drums: bool,
    pub bpm: f64,
    pub length: f64,
}

/// The item the Editor opens on in `project`: the selected track's
/// first MIDI item, else the first MIDI item of any track. `None` when
/// the session has no MIDI to edit.
#[must_use]
pub fn target(project: &daw_ui::studio::project::Project) -> Option<Target> {
    let first_midi = |track: &daw_proto::Track| {
        project
            .items
            .get(&track.guid)
            .and_then(|items| items.iter().find(|i| project.midi.contains(&i.guid)))
            .map(|item| Target {
                track: track.guid.clone(),
                item: item.guid.clone(),
                drums: crate::expression::is_drum_track(&track.name),
                bpm: project.bpm,
                length: item.length.as_seconds(),
            })
    };
    project
        .tracks
        .iter()
        .filter(|t| t.selected)
        .find_map(first_midi)
        .or_else(|| project.tracks.iter().find_map(first_midi))
}

/// The Editor view, on the session's selected track.
#[component]
pub fn EditorPanel() -> Element {
    let Some(session) = try_use_context::<crate::studio::StudioSession>() else {
        return rsx! { crate::shell::EditorComing {} };
    };
    let target = target(&session.project.0);
    #[cfg(feature = "native")]
    {
        let widget = use_hook(|| dioxus_native_dom::CustomWidgetAttr::new(EditorView::new(target)));
        rsx! {
            object {
                style: "position:absolute; left:0; top:0; width:100%; height:100%; background:#0b0c0e;",
                data: widget,
            }
        }
    }
    #[cfg(not(feature = "native"))]
    {
        let _ = target;
        rsx! { crate::shell::EditorComing {} }
    }
}

/// The widget: the editor on its item, or why there is none.
#[cfg(feature = "native")]
pub struct EditorView {
    target: Option<Target>,
    view: Option<Expression>,
    look: expression_editor_paint::paint::Look,
    /// The theme the look was taken from: a new one (a saved edit to the
    /// theme file) and the look is taken again.
    skin: u64,
    font: Option<crate::text::Font>,
    /// Loaded yet: the take is read on the first paint, when the box's
    /// size is known.
    tried: bool,
    dirty: bool,
}

#[cfg(feature = "native")]
impl EditorView {
    #[must_use]
    pub fn new(target: Option<Target>) -> Self {
        Self {
            target,
            view: None,
            look: look(),
            skin: crate::skin::version(),
            font: crate::text::Font::embedded().ok(),
            tried: false,
            dirty: true,
        }
    }

    /// Read the target's take into the editor, `size` big. A take with
    /// no notes, or no engine to read it from, leaves no editor.
    fn load(&mut self, size: (f64, f64)) {
        self.tried = true;
        let Some(t) = &self.target else {
            return;
        };
        let Some(snapshot) = crate::expression::load_take(&t.item, t.bpm, t.length) else {
            tracing::warn!(
                editor.item = %t.item,
                "the editor's take could not be read, or has no notes"
            );
            return;
        };
        let mut view = Expression::from_take(&snapshot, t.item.clone(), t.drums, (0.0, 0.0), size);
        view.set_look(self.look);
        self.view = Some(view);
    }

    fn event(&mut self, event: &UiEvent) {
        let Some(view) = self.view.as_mut() else {
            return;
        };
        let changed = match event {
            UiEvent::PointerDown(e) => {
                let button = match e.button {
                    MouseEventButton::Secondary => 2,
                    MouseEventButton::Auxiliary => 1,
                    _ => 0,
                };
                view.press(
                    f64::from(e.coords.client_x),
                    f64::from(e.coords.client_y),
                    crate::widget::mods(e.mods),
                    button,
                )
            }
            UiEvent::PointerMove(e) => view.moved(
                f64::from(e.coords.client_x),
                f64::from(e.coords.client_y),
                crate::widget::mods(e.mods),
            ),
            UiEvent::PointerUp(e) | UiEvent::PointerCancel(e) => view.release(
                f64::from(e.coords.client_x),
                f64::from(e.coords.client_y),
                crate::widget::mods(e.mods),
            ),
            UiEvent::Wheel(e) => {
                let (dx, dy) = match e.delta {
                    BlitzWheelDelta::Lines(x, y) => (x * 40.0, y * 40.0),
                    BlitzWheelDelta::Pixels(x, y) => (x, y),
                };
                view.wheel(
                    f64::from(e.coords.client_x),
                    f64::from(e.coords.client_y),
                    dx,
                    dy,
                    crate::widget::mods(e.mods),
                )
            }
            UiEvent::KeyDown(e) => view.key(&key_name(&e.key), crate::widget::mods(e.modifiers)),
            UiEvent::KeyUp(e) => view.key_up(&key_name(&e.key), crate::widget::mods(e.modifiers)),
            _ => false,
        };
        self.dirty |= changed;
    }

    fn paint_scene(&mut self, width: u32, height: u32, scale: f64) -> Scene {
        let scale = scale.max(f64::EPSILON);
        let size = (f64::from(width) / scale, f64::from(height) / scale);
        if !self.tried {
            self.load(size);
        }
        if self.skin != crate::skin::version() {
            self.skin = crate::skin::version();
            self.look = look();
            if let Some(view) = self.view.as_mut() {
                view.set_look(self.look);
            }
        }
        self.dirty = false;
        let mut scene = Scene::new();
        let mut inner = Scene::new();
        if let Some(view) = self.view.as_mut() {
            view.layout((0.0, 0.0), size);
            view.paint(&mut inner);
        } else if let Some(font) = &self.font {
            let why = if self.target.is_some() {
                "This item's notes could not be read."
            } else {
                "Select a track with a MIDI item to edit it here."
            };
            crate::tcp::glyphs(
                &mut inner,
                font,
                self.look.text_dim,
                why,
                24.0,
                size.1 / 2.0,
                14.0,
            );
        }
        anyrender::PaintScene::append_scene(&mut scene, inner, vello::kurbo::Affine::scale(scale));
        scene
    }
}

/// The editor's colours from the loaded theme: the window's palette, and
/// the theme's `editor.*` tokens over it.
#[cfg(feature = "native")]
fn look() -> expression_editor_paint::paint::Look {
    crate::expression::look_of(&crate::skin::palette(
        &crate::arrangement::Palette::from_theme(&daw_ui::theming::Theme::dark()),
    ))
}

/// A key as the editor names it: a character's text, else the W3C name
/// (`"Delete"`, `"ArrowLeft"`).
#[cfg(feature = "native")]
fn key_name(key: &blitz_traits::events::Key) -> String {
    match key {
        blitz_traits::events::Key::Character(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(feature = "native")]
impl blitz_dom::node::Widget for EditorView {
    fn handle_event(&mut self, event: &UiEvent) {
        self.event(event);
    }

    fn needs_redraw(&self) -> bool {
        self.dirty
            || self.skin != crate::skin::version()
            || self.view.as_ref().is_some_and(Expression::dragging)
    }

    fn paint(
        &mut self,
        _render_ctx: &mut dyn anyrender::RenderContext,
        _styles: &blitz_dom::node::ComputedStyles,
        width: u32,
        height: u32,
        scale: f64,
    ) -> Scene {
        self.paint_scene(width, height, scale)
    }
}
