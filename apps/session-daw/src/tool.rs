//! Tools and the mouse pointer.
//!
//! Two things decide what a press over the arrangement does, and the
//! pointer's shape has to say which one is in charge:
//!
//! - a **tool**: a mode the panel holds (the zoom spring on a held `z`,
//!   the hand on a held middle button). While one is up it owns the
//!   mouse outright. The arrangement's own gestures stand down, or a
//!   zoom-drag would also drag out a time selection underneath it.
//! - otherwise, the **mouse map** ([`crate::mousemap`]): the context
//!   under the pointer and the keys held pick an action, and the pointer
//!   shows what a drag from here would do (a trim arrow on an item's
//!   edge, a crosshair once Ctrl turns the drag into a razor, the copy
//!   badge on Alt), so a modifier's meaning is visible before it is used.
//!
//! [`Pointing`] is shared between the panel (which knows about tools and
//! the keyboard) and the widget (which knows what is under the pointer).
//! The shape is set on the window directly, not through CSS: Blitz reads
//! a node's `cursor` only when the pointer crosses into another node, and
//! the whole arrangement is one node.

use cursor_icon::CursorIcon;
use std::cell::RefCell;
use std::rc::Rc;

use input_config_proto::MouseModifierContext as Context;

use crate::mousemap::{self, Action, Gesture, Mods};

/// A mode that owns the mouse while it is up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    /// None: the mouse map decides.
    #[default]
    Map,
    /// The zoom spring, held on `z`: drag or wheel zooms.
    Zoom,
    /// The hand, held on the middle button: drag pans.
    Pan,
}

/// What the pointer is over, as the widget last saw it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Over {
    /// Nothing the widget answers for (or the pointer is elsewhere).
    #[default]
    Nothing,
    /// A continuous control in the track panel: a knob, turned by
    /// dragging up and down.
    Knob,
    /// A button in the track panel.
    Button,
    /// The lanes or the ruler, in a mouse-map context.
    Map(Context),
}

/// Where a pointer shape goes: the native window's cursor, or the web
/// canvas's CSS `cursor`.
pub type Sink = Rc<dyn Fn(CursorIcon)>;

/// The shared state, and where it sets the pointer.
#[derive(Default)]
pub struct Pointing {
    window: Option<Sink>,
    /// Written by the panel.
    pub tool: Tool,
    /// The keys held, from the panel's own modifier events (the widget
    /// only hears about keys when an event of its own carries them).
    pub mods: Mods,
    /// Whether the pointer is inside the panel.
    pub inside: bool,
    /// Written by the widget on every move.
    pub over: Over,
    /// The action of a drag the widget has taken and not yet let go of.
    pub gesture: Option<Action>,
    /// A tool was used while its key was held: a held `z` zoomed, so its
    /// release closes the which-key tree it also opened. Set by the
    /// panel, taken by the widget on the key-up.
    pub tool_used: bool,
}

pub type Shared = Rc<RefCell<Pointing>>;

impl Pointing {
    #[must_use]
    pub fn shared(window: Option<Sink>) -> Shared {
        Rc::new(RefCell::new(Self {
            window,
            ..Self::default()
        }))
    }

    /// Whether a tool has the mouse, so the arrangement's own gestures
    /// must not start.
    #[must_use]
    pub fn tool_active(&self) -> bool {
        self.tool != Tool::Map
    }

    /// The shape for the state as it stands.
    #[must_use]
    pub fn icon(&self) -> CursorIcon {
        icon_for(self.tool, self.gesture, self.over, self.mods)
    }

    /// Put the shape on the window. Every time, not only on a change:
    /// Blitz resets it whenever the pointer crosses into another node,
    /// so a cached "already showing that" would be wrong after a
    /// crossing and nothing would say so.
    pub fn apply(&self) {
        if self.inside
            && let Some(window) = &self.window
        {
            window(self.icon());
        }
    }
}

/// The pointer shape: a tool's own, then a drag in flight, then what a
/// drag from here would do.
#[must_use]
pub fn icon_for(tool: Tool, gesture: Option<Action>, over: Over, mods: Mods) -> CursorIcon {
    match tool {
        Tool::Zoom => return CursorIcon::ZoomIn,
        Tool::Pan => return CursorIcon::Grabbing,
        Tool::Map => {}
    }
    if let Some(action) = gesture {
        return dragging(action);
    }
    match over {
        Over::Nothing | Over::Button => CursorIcon::Default,
        Over::Knob => CursorIcon::NsResize,
        Over::Map(context) => hovering(mousemap::resolve(context, Gesture::Drag, mods).action),
    }
}

/// What a drag from here would do, before it starts.
fn hovering(action: Action) -> CursorIcon {
    match action {
        // The item's body stays the arrow until it is actually moving:
        // a whole lane of items under a four-way arrow is noise, and a
        // click there selects as often as a drag moves.
        Action::MoveItem => CursorIcon::Default,
        other => dragging(other),
    }
}

/// The shape of a drag, once it is one.
fn dragging(action: Action) -> CursorIcon {
    match action {
        Action::MoveItem | Action::MoveRazorArea => CursorIcon::Move,
        Action::CopyItem => CursorIcon::Copy,
        Action::SlipItem => CursorIcon::EwResize,
        Action::TrimLeft => CursorIcon::WResize,
        Action::TrimRight => CursorIcon::EResize,
        Action::FadeIn | Action::FadeOut | Action::FadeShape => CursorIcon::ColResize,
        Action::RazorArea => CursorIcon::Crosshair,
        Action::SetEditCursor
        | Action::TimeSelection
        | Action::SelectItem
        | Action::ToggleItemSelection
        | Action::RemoveRazorArea
        | Action::Nothing => CursorIcon::Default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods {
        shift: false,
        ctrl: false,
        alt: false,
    };
    const CTRL: Mods = Mods {
        shift: false,
        ctrl: true,
        alt: false,
    };
    const ALT: Mods = Mods {
        shift: false,
        ctrl: false,
        alt: true,
    };

    /// A tool's shape wins over everything under it, including a drag
    /// the map would have started there.
    #[test]
    fn a_tool_shows_its_own_shape_wherever_it_is() {
        let edge = Over::Map(Context::MediaItemLeftEdge);
        assert_eq!(icon_for(Tool::Zoom, None, edge, NONE), CursorIcon::ZoomIn);
        assert_eq!(icon_for(Tool::Pan, None, edge, CTRL), CursorIcon::Grabbing);
        assert_eq!(
            icon_for(Tool::Zoom, Some(Action::MoveItem), Over::Knob, NONE),
            CursorIcon::ZoomIn
        );
    }

    /// Each modifier's meaning on an item shows before the press.
    #[test]
    fn the_modifiers_show_what_a_drag_would_do() {
        let body = Over::Map(Context::MediaItemBottomHalf);
        assert_eq!(icon_for(Tool::Map, None, body, NONE), CursorIcon::Default);
        assert_eq!(icon_for(Tool::Map, None, body, ALT), CursorIcon::Copy);
        assert_eq!(icon_for(Tool::Map, None, body, CTRL), CursorIcon::Crosshair);
        let chord = Mods {
            shift: false,
            ctrl: true,
            alt: true,
        };
        assert_eq!(icon_for(Tool::Map, None, body, chord), CursorIcon::EwResize);
        let empty = Over::Map(Context::ArrangeView);
        assert_eq!(
            icon_for(Tool::Map, None, empty, CTRL),
            CursorIcon::Crosshair
        );
    }

    #[test]
    fn edges_and_fades_show_which_way_they_drag() {
        let at = |c| icon_for(Tool::Map, None, Over::Map(c), NONE);
        assert_eq!(at(Context::MediaItemLeftEdge), CursorIcon::WResize);
        assert_eq!(at(Context::MediaItemRightEdge), CursorIcon::EResize);
        assert_eq!(at(Context::MediaItemFade), CursorIcon::ColResize);
        assert_eq!(at(mousemap::RAZOR_AREA), CursorIcon::Move);
    }

    /// A drag in flight keeps its shape wherever the pointer wanders.
    #[test]
    fn a_drag_keeps_its_shape_off_the_thing_it_started_on() {
        assert_eq!(
            icon_for(Tool::Map, Some(Action::MoveItem), Over::Nothing, NONE),
            CursorIcon::Move
        );
        assert_eq!(
            icon_for(Tool::Map, Some(Action::TrimRight), Over::Knob, NONE),
            CursorIcon::EResize
        );
    }

    #[test]
    fn a_knob_turns_up_and_down() {
        assert_eq!(
            icon_for(Tool::Map, None, Over::Knob, NONE),
            CursorIcon::NsResize
        );
        assert_eq!(
            icon_for(Tool::Map, None, Over::Button, NONE),
            CursorIcon::Default
        );
    }
}
