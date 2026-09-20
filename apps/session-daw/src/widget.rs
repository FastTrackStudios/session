//! The arrangement as ONE node: ruler, track panel and lanes, painted
//! rather than built.
//!
//! # Why this one surface is not a DOM
//!
//! Everything else in the window is better as components — a button
//! wants `:hover`, focus, a name a screen reader can say, and a layout
//! somebody can change without recompiling. The arrangement wants none
//! of that and pays dearly for all of it.
//!
//! Measured on the golden session at 5120x1440, the same picture drawn
//! both ways, gated pixel-for-pixel against each other by
//! `tests/component_lanes.rs`:
//!
//! | gesture | painted | as a component tree |
//! |---|---|---|
//! | scroll | 1.06 ms | 15.5 ms |
//! | zoom | 0.71 ms | 21.8 ms |
//! | the whole session on screen | 3.77 ms | — |
//!
//! Fifteen to thirty times, and it is not the GPU: that is 0.3–1.0 ms
//! either way. It is ten thousand nodes going through Stylo's cascade,
//! Taffy's layout, damage propagation and a per-node paint walk, every
//! frame, to produce a picture that a recorded command list replays in
//! one. No amount of tuning closes a gap of that shape — a month of it
//! took the component tree from 124 ms to 15, and 15 is still not 4.17.
//!
//! So the arrangement becomes a [`Widget`]: one element in the DOM, laid
//! out and positioned by CSS like any other, whose contents are drawn by
//! the renderer this window already had. The toolbars, the transport and
//! the rails stay components, because for them the DOM is the point.
//!
//! # What this costs, honestly
//!
//! Hit testing, keyboard focus and accessibility inside this rectangle
//! are ours to write. The first two we already wrote — the wheel is
//! handled at the winit level because Blitz dispatches no wheel event,
//! and `arrange_edit` has done item hit testing from the start. The
//! third is the real debt: a screen reader sees one element here. The
//! answer is that every piece of session state is reachable through the
//! CLI and the RPC surface, so the window is not the only way in — but
//! that is an argument, not an implementation, and it should be written
//! down as one.
//!
//! # Why it still runs in a browser
//!
//! [`Widget::paint`] returns an `anyrender::Scene` — a list of
//! commands, not a wgpu call. Whatever can replay that list can draw
//! this: the CPU backend, the WebGL-class one, and a canvas in a browser
//! tab. The escape hatch that takes a `Device` and `Queue` in
//! `can_create_surfaces` is the one that would tie us to native, and
//! this does not use it.

use std::cell::RefCell;
use std::rc::Rc;

use anyrender::{RenderContext, Scene};
use blitz_dom::node::{ComputedStyles, Widget};
use blitz_traits::events::{Modifiers, UiEvent};

use vello::kurbo::Affine;

use crate::arrangement::{Arrangement, Palette, TCP_WIDTH, Viewport};
use crate::profile::Counts;
use crate::ruler::{self, Bars};

/// The finest grid division the ruler will draw.
const FINEST: f64 = 1.0 / 16.0;

/// Where the view is, shared between the window and the widget.
///
/// A plain cell rather than a signal. The paint happens inside Blitz's
/// own traversal, which is not the Dioxus runtime — reading a `Signal`
/// there is a panic waiting for the first frame that takes a different
/// path. What the widget needs is four numbers, and four numbers do not
/// need a reactive graph.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct View {
    pub scroll_x: f64,
    pub scroll_y: f64,
    pub zoom_x: f64,
    pub zoom_y: f64,
    /// Where the transport is, in seconds.
    ///
    /// The window's to know and the widget's to draw. A position is a
    /// LEVEL and not an event — see `engine::Transport` — so it is
    /// written every frame like the scroll rather than sent as a
    /// change, and a missed one is covered by the next.
    pub play_at: f64,
}

/// Blitz's modifier set, as the mouse map's.
fn mods(from: Modifiers) -> crate::mousemap::Mods {
    crate::mousemap::Mods {
        shift: from.contains(Modifiers::SHIFT),
        ctrl: from.contains(Modifiers::CONTROL),
        alt: from.contains(Modifiers::ALT),
    }
}

/// A knob under the pointer, mid-turn.
struct Turn {
    spot: crate::pointer::RowSpot,
    /// Where the press landed, in the widget's own coordinates.
    from: f64,
    /// The track as it was when the press landed.
    was: daw_proto::Track,
}

/// How far a knob turns through its whole range, in pixels.
///
/// REAPER's own, so a full sweep takes about the same movement here as
/// it does there.
const KNOB_TRAVEL: f64 = 150.0;

/// A handle on that, for the window to write and the widget to read.
pub type Shared = Rc<RefCell<View>>;

/// How much the last frame drew, for the benchmark to report.
#[derive(Clone, Copy, Debug, Default)]
pub struct Drawn {
    pub replayed: u64,
    pub submitted: u64,
}

/// The arrangement, as a widget.
pub struct ArrangementWidget {
    scene: Arrangement,
    palette: Palette,
    font: crate::text::Font,
    bars: Bars,
    grid: adaptive_grid::Adaptive,
    /// Pixels per second before the zoom — the base the view scales.
    pps: f64,
    /// What the panel's live controls are drawn from.
    ///
    /// They are NOT in the recording, and cannot be: a knob shows a
    /// value, and a value changes without the session changing. The
    /// recording holds the row — its tint, its rail, its number, its
    /// name — and the controls go over it every frame, which is what
    /// the painted window has always done.
    rows: Vec<(daw_proto::Track, u32)>,
    tracks: Vec<daw_proto::Track>,
    map: crate::plan::Rows,
    /// What row heights come from, for re-cutting the panel.
    layout: crate::layout::Layout,
    view: Shared,
    drawn: Rc<RefCell<Drawn>>,
    /// The frame-time graph, when the window asked for one.
    ///
    /// A window asks; a comparison shot does not, and that is not a
    /// taste: `tests/component_lanes.rs` holds this widget to the
    /// painted window pixel for pixel, and an overlay is a difference.
    /// A readout that made the gate looser would be measuring the thing
    /// it broke.
    stats: Option<crate::fps::Stats>,
    /// The size the last paint was given, so a hit arriving between
    /// two paints is measured against the frame the picture was drawn
    /// in and not against whatever the window has become since.
    size: (f64, f64),
    /// What the pointer is on, and what it is doing to it.
    ///
    /// Cheap precisely because the panel is a recording: a control
    /// cannot change appearance without re-cutting the row, and it does
    /// not have to. The one control under the pointer is drawn AGAIN,
    /// over the recording, in its hover cell — one control a frame
    /// instead of a panel a mouse move.
    pointer: crate::pointer::Pointer<crate::pointer::RowSpot>,
    /// Items, fades, the ruler and the time selection.
    ///
    /// The whole arrangement-editing state machine, which the direct
    /// window has had from the start — moves, trims, fades, marker
    /// drags, region edges, the zoom tool. Not reimplemented here: the
    /// widget hands it hits and paints what it says is in flight.
    editor: crate::arrange_edit::Editor,
    /// The widget's own copy of the project, which the editor moves
    /// before the engine has.
    ///
    /// The same bargain as the tracks — see
    /// [`ArrangementWidget::assume`]. An item that jumps back to where
    /// it was for two frames after you drop it reads as a failed drag.
    project: daw_ui::studio::project::Project,
    /// What is needed to cut the recording again when an edit changes
    /// what it holds.
    previews: crate::midi::Previews,
    /// The ruler's own furniture, for hit testing.
    sections: Vec<daw_ui::studio::project::Section>,
    markers: Vec<daw_ui::studio::project::Marker>,
    /// Whether the last event changed the picture, for
    /// `Widget::needs_redraw`. A `Cell` because that question is asked
    /// through `&self`.
    dirty: std::cell::Cell<bool>,
    /// Whether the editor took the last press and has not been let go
    /// of yet.
    ///
    /// Tracked here rather than asked of the editor, because
    /// `Editor::dragging` answers a different question: it is false
    /// until a ghost exists, and the ghost is created INSIDE `moved`.
    /// Gating the moves on it means the first move never arrives, so
    /// the ghost is never made and nothing can be dragged at all — and
    /// a press on the ruler is not in its answer under any
    /// circumstances.
    holding: bool,
    /// The item under the pointer, so its fade handles are drawn —
    /// REAPER puts them in the top corners and only shows them on the
    /// item you are over.
    hovered_item: Option<usize>,
    /// An open rename, and the row it belongs to.
    ///
    /// The field draws over the name it replaces and takes the keyboard
    /// until it is committed or abandoned. Blitz sends key events to
    /// the focused node, and a pointer-down on a widget focuses it, so
    /// the double-click that opens this has already done the focusing.
    renaming: Option<crate::rename::Rename>,
    /// When and where the last click on a NAME landed, so the next one
    /// can tell whether it is the second half of a double.
    last_name: Option<(usize, std::time::Instant)>,
    /// A knob being turned: where the press landed, and the track as it
    /// was at that moment.
    ///
    /// The value at PRESS, not the value now. A drag is an absolute
    /// gesture — this far up from where you grabbed it is this much
    /// louder — and reading the current value each move would compound
    /// the deltas, so a slow drag and a fast one over the same distance
    /// would land somewhere different.
    turning: Option<Turn>,
    /// What the widget wants done to the session, for the window to
    /// pick up and hand to the engine.
    ///
    /// A queue rather than a call, because a widget is a painter: it
    /// knows a click landed on the mute button of row 12, and it must
    /// not know what a mute is, how to reach the engine, or whether
    /// doing it is allowed. See `crate::engine::Edit`.
    edits: Rc<RefCell<Vec<crate::engine::Edit>>>,
    /// The live controls, recorded, and the zoom they were cut at.
    ///
    /// The single most expensive thing the paint used to do: forty rows
    /// of vector art — arms, knobs, meters, buttons, names — generated
    /// from scratch every frame, while a pan or a scroll changes none
    /// of it. Recorded per row like the panel is, so a scroll replays a
    /// different span rather than rebuilding anything.
    ///
    /// Re-cut when the vertical zoom moves, because that is what
    /// decides which tier of controls a row shows. A value changing —
    /// somebody turning a knob — needs the same, and the hook for that
    /// is [`ArrangementWidget::values_changed`].
    controls: Option<(crate::overlay::Controls, f64)>,
    /// Last frame's vertical zoom, so this one can tell whether a
    /// gesture is still in flight.
    ///
    /// Seeded from the view rather than left empty, so the FIRST frame
    /// counts as settled and takes the cut. Otherwise a window that is
    /// never touched draws live forever, and — worse — the one frame a
    /// comparison shot renders is the one frame that does not use the
    /// cache, so the gates would be checking a path the window does not
    /// take.
    was: f64,
    /// What the last paint's passes cost, in microseconds.
    ///
    /// Kept across frames so the readout has something to say on the
    /// frame it is drawn on, rather than reporting the pass timings of
    /// a frame that has not finished yet.
    spent: Passes,
}

/// What one paint spent, pass by pass.
///
/// The frame graph says a frame costs ten milliseconds; this says which
/// part of the widget it went to. Without it the only honest thing to
/// do about a slow frame is guess, and a guess costs a whole build to
/// disprove.
#[derive(Clone, Copy, Debug, Default)]
struct Passes {
    lanes: u128,
    titles: u128,
    panel: u128,
    ruler: u128,
    controls: u128,
}

impl ArrangementWidget {
    /// Build the recording once, from the session.
    ///
    /// Once, and that is the whole point: the commands for a lane do not
    /// change when the view moves over them, so a pan replays what is on
    /// screen and records nothing.
    #[must_use]
    pub fn new(
        scene: Arrangement,
        palette: Palette,
        font: crate::text::Font,
        bpm: f64,
        pps: f64,
        rows: Vec<(daw_proto::Track, u32)>,
        layout: crate::layout::Layout,
        project: daw_ui::studio::project::Project,
        previews: crate::midi::Previews,
        view: Shared,
        readout: bool,
    ) -> Self {
        let tracks: Vec<daw_proto::Track> = rows.iter().map(|(t, _)| t.clone()).collect();
        let at_rest = view.borrow().zoom_y;
        let map = crate::plan::Rows::of(&rows, &tracks);
        Self {
            scene,
            palette,
            font,
            bars: Bars::at(bpm),
            grid: adaptive_grid::Adaptive::default(),
            pps,
            rows,
            tracks,
            map,
            layout,
            view,
            drawn: Rc::new(RefCell::new(Drawn::default())),
            size: (0.0, 0.0),
            pointer: crate::pointer::Pointer::default(),
            editor: crate::arrange_edit::Editor::default(),
            hovered_item: None,
            holding: false,
            dirty: std::cell::Cell::new(false),
            sections: project.sections.clone(),
            markers: project.markers.clone(),
            project,
            previews,
            renaming: None,
            last_name: None,
            turning: None,
            edits: Rc::new(RefCell::new(Vec::new())),
            controls: None,
            was: at_rest,
            stats: readout.then(crate::fps::Stats::new),
            spent: Passes::default(),
        }
    }

    /// What the widget wants done, for the window to drain.
    #[must_use]
    pub fn edits(&self) -> Rc<RefCell<Vec<crate::engine::Edit>>> {
        Rc::clone(&self.edits)
    }

    /// Throw the recorded controls away, because a value they draw has
    /// changed.
    ///
    /// Not worked out from the data: finding the one knob that moved
    /// would mean diffing forty tracks every frame, which costs more
    /// than the redraw it saves. Whoever changed the value knows, and
    /// says so.
    pub fn values_changed(&mut self) {
        self.controls = None;
    }

    /// What the last frame replayed and submitted.
    #[must_use]
    pub fn drawn(&self) -> Rc<RefCell<Drawn>> {
        Rc::clone(&self.drawn)
    }
}

impl ArrangementWidget {
    /// The viewport the widget is showing, at a size.
    ///
    /// The same four numbers `paint` works from, so a hit and a draw
    /// cannot disagree about where anything is.
    fn viewport(&self, width: f64, height: f64) -> Viewport {
        let at = *self.view.borrow();
        Viewport {
            scroll_x: at.scroll_x,
            scroll_y: at.scroll_y,
            pps: self.pps * at.zoom_x,
            zoom_y: at.zoom_y,
            width,
            height,
        }
    }

    /// The panel control under a point in the widget's own coordinates.
    ///
    /// The panel starts at the widget's left edge and the lanes below
    /// the ruler, so the only conversion is the ruler's height — the
    /// same offset `paint` puts in its transforms.
    fn spot_at(&self, x: f64, y: f64) -> Option<crate::pointer::RowSpot> {
        let view = self.viewport(self.size.0, self.size.1);
        let below = y - ruler::RULER_H;
        if below < 0.0 {
            return None;
        }
        self.scene
            .row_spot_at(view, &self.rows, x, below + view.scroll_y)
            .map(|(row, control)| crate::pointer::RowSpot { row, control })
    }

    /// What a click on a control means, as an edit to the session.
    ///
    /// The widget knows a click landed on the mute button of row 12. It
    /// deliberately does not know what a mute IS — the queue goes to
    /// the window, which owns the engine.
    fn act(&mut self, spot: crate::pointer::RowSpot) {
        use crate::engine::Edit;
        use crate::row::Control as C;
        let Some((track, _)) = self.rows.get(spot.row) else {
            return;
        };
        let guid = track.guid.clone();
        // A double-click on a name edits it. The single click that
        // preceded it selected the track, which is what you wanted on
        // the way here anyway.
        if spot.control == C::Name {
            let now = std::time::Instant::now();
            let again = self.last_name.is_some_and(|(row, when)| {
                row == spot.row && now.saturating_duration_since(when) <= crate::gesture::DOUBLE
            });
            if again {
                self.last_name = None;
                self.renaming = Some(crate::rename::Rename::new(
                    crate::rename::Surface::Arrange,
                    spot.row,
                    crate::rename::What::Track(guid),
                    &track.name,
                ));
                return;
            }
            self.last_name = Some((spot.row, now));
        }
        let edit = match spot.control {
            C::Mute => Edit::ToggleMute(guid),
            C::Solo => Edit::ToggleSolo(guid),
            C::RecArm => Edit::ToggleArm(guid),
            C::Phase => Edit::SetPhase(guid, !track.phase_inverted),
            C::Name => Edit::Select(guid),
            // A fold is an edit to the VIEW and not to a track, and the
            // rack and the routing panel are surfaces this widget does
            // not own yet. Left alone rather than guessed at.
            C::Folder | C::Fx | C::Routing | C::Volume | C::Pan => return,
        };
        // Shown before it is true.
        //
        // The engine is a channel and a worker thread, and the state
        // comes back through a subscription some frames later. Waiting
        // for that would make every button feel broken — a mute that
        // lights up two frames after the click reads as a missed click,
        // and the second click un-does the first. So the widget moves
        // its own copy now and the engine makes it so; when the real
        // event arrives it agrees, and if the engine refuses, the next
        // update corrects it.
        self.assume(&edit);
        self.edits.borrow_mut().push(edit);
    }

    /// Where a point in the widget falls, as the window's hit test
    /// sees it.
    ///
    /// The widget's coordinates start where the rails end, which is
    /// exactly the offset `hit::arrangement` expects to subtract — so
    /// the tested hit test is reused rather than a second one written
    /// that could disagree with it.
    fn hit(&self, x: f64, y: f64) -> Option<crate::hit::Hit> {
        let view = self.viewport(self.size.0, self.size.1);
        Some(crate::hit::arrangement(
            &self.scene,
            view,
            0,
            &self.sections,
            &self.markers,
            x + crate::rails::SIDE,
            y + crate::rails::TOP,
        ))
    }

    /// The item under a point, for the fade handles.
    fn item_under(&self, x: f64, y: f64) -> Option<usize> {
        match self.hit(x, y)?.target {
            crate::hit::Target::Item { index, .. } => Some(index),
            _ => None,
        }
    }

    /// The time under an x in the widget's coordinates.
    fn seconds_at(&self, x: f64, view: Viewport) -> f64 {
        ((x - TCP_WIDTH + view.scroll_x) / view.pps.max(f64::EPSILON)).max(0.0)
    }

    /// What the editor asked for, carried out.
    ///
    /// `ReRecord` is the expensive one and the reason edits are
    /// predicted rather than awaited: the recording is cut again from
    /// the widget's own project, so the picture is right on the next
    /// frame instead of whenever the engine gets round to it.
    fn settle(&mut self, effects: Vec<crate::arrange_edit::Effect>) {
        use crate::arrange_edit::Effect;
        let mut recut = false;
        for effect in effects {
            match effect {
                Effect::Send(edit) => self.edits.borrow_mut().push(edit),
                Effect::ReRecord => recut = true,
                // The transport and the playhead are the window's, and
                // a notice has nowhere to go yet — see `Effect`.
                Effect::Transport(..) | Effect::Playhead(_) => {}
                Effect::Refused(why) => tracing::warn!(why, "the edit was refused"),
            }
        }
        if recut {
            self.recut();
        }
    }

    /// Cut the whole recording again, from the widget's own project.
    fn recut(&mut self) {
        let rows = daw_ui::studio::RowsRef(std::sync::Arc::new(self.rows.clone()));
        let project = daw_ui::studio::ProjectRef(std::sync::Arc::new(self.project.clone()));
        self.scene = Arrangement::build(
            &self.palette,
            &self.font,
            &project,
            &rows,
            self.layout,
            &self.previews,
        );
        self.sections = self.project.sections.clone();
        self.markers = self.project.markers.clone();
        self.controls = None;
    }

    /// One key, into an open rename.
    ///
    /// `false` when there is no rename open, which is how a key that is
    /// not for the field falls through to whatever else the window does
    /// with it.
    fn typed(&mut self, event: &blitz_traits::events::BlitzKeyEvent) -> bool {
        // Blitz is on `keyboard_types` 0.7, whose `Key` is flat — the
        // named keys are variants of it rather than of a `NamedKey`
        // beside it.
        use blitz_traits::events::Key;
        let Some(rename) = self.renaming.as_mut() else {
            return false;
        };
        match &event.key {
            Key::Enter => {
                let Some(rename) = self.renaming.take() else {
                    return false;
                };
                // An empty name is refused, and refusing it by closing
                // the field is kinder than leaving it open with no way
                // to tell why Enter did nothing.
                let Some(name) = rename.commit().map(str::to_owned) else {
                    return true;
                };
                if let crate::rename::What::Track(guid) = rename.what {
                    let edit = crate::engine::Edit::Rename(guid, name);
                    self.assume(&edit);
                    // A name is RECORDED chrome and not a live value,
                    // so moving the track is not enough — the row it is
                    // written into has to be cut again or the field
                    // closes on the old name.
                    self.scene.forget_panel();
                    self.edits.borrow_mut().push(edit);
                }
            }
            Key::Escape => self.renaming = None,
            Key::Backspace => rename.backspace(),
            Key::Delete => rename.delete(),
            Key::ArrowLeft => rename.left(),
            Key::ArrowRight => rename.right(),
            Key::Home => rename.home(),
            Key::End => rename.end(),
            Key::Character(text) => {
                for c in text.chars() {
                    rename.insert(c);
                }
            }
            _ => return false,
        }
        true
    }

    /// A knob, turned to wherever the pointer has got to.
    ///
    /// Measured from the press and against the value the track had
    /// THEN, so the same movement always means the same change however
    /// it was made. `fine` is the modified drag: the same distance
    /// worth a fraction as much.
    fn turn(&mut self, y: f64, fine: bool) {
        let Some(turn) = self.turning.as_ref() else {
            return;
        };
        let control = match turn.spot.control {
            crate::row::Control::Volume => crate::mcp::Control::Volume,
            crate::row::Control::Pan => crate::mcp::Control::Pan,
            _ => return,
        };
        let dy = y - turn.from;
        let dy = if fine { dy * crate::gesture::FINE } else { dy };
        let fraction = crate::gesture::drag_fraction(dy, KNOB_TRAVEL);
        let Some(edit) = crate::engine::drag(control, &turn.was.guid, &turn.was, fraction) else {
            return;
        };
        self.assume(&edit);
        self.edits.borrow_mut().push(edit);
    }

    /// Apply an edit to the widget's own copy of the tracks.
    ///
    /// Only the ones a control can make, and only the fields a control
    /// DRAWS. This is a picture being kept honest, not a second source
    /// of truth — see [`ArrangementWidget::act`].
    fn assume(&mut self, edit: &crate::engine::Edit) {
        use crate::engine::Edit;
        // Selection is not a field on one track: selecting one
        // DESELECTS the rest, which is a walk over all of them and not
        // a change to the one named. Handled before the rest.
        if let Edit::Select(guid) | Edit::AddToSelection(guid) = edit {
            let only = matches!(edit, Edit::Select(_));
            for track in self
                .tracks
                .iter_mut()
                .chain(self.rows.iter_mut().map(|(track, _)| track))
            {
                if track.guid == *guid {
                    track.selected = true;
                } else if only {
                    track.selected = false;
                }
            }
            // A row's tint carries its selection, and a tint is
            // recorded — the same reason a rename has to forget the
            // cut.
            self.scene.forget_panel();
            self.values_changed();
            return;
        }
        // A toggle flips what it finds; the others carry the value they
        // mean. Both shapes, one walk.
        let (guid, change): (&str, &dyn Fn(&mut daw_proto::Track)) = match edit {
            Edit::ToggleMute(guid) => (guid, &|t| t.muted = !t.muted),
            Edit::ToggleSolo(guid) => (guid, &|t| t.soloed = !t.soloed),
            Edit::ToggleArm(guid) => (guid, &|t| t.armed = !t.armed),
            Edit::SetPhase(guid, on) => (guid, &|t| t.phase_inverted = *on),
            Edit::SetVolume(guid, gain) => (guid, &|t| t.volume = *gain),
            Edit::SetPan(guid, pan) => (guid, &|t| t.pan = *pan),
            Edit::Rename(guid, name) => (guid, &|t| t.name.clone_from(name)),
            _ => return,
        };
        for track in self
            .tracks
            .iter_mut()
            .chain(self.rows.iter_mut().map(|(track, _)| track))
            .filter(|track| track.guid == guid)
        {
            change(track);
        }
        self.values_changed();
    }
}

impl Widget for ArrangementWidget {
    /// Whether the last event changed the picture.
    ///
    /// Most moves do not: a pointer crossing the panel raises an event
    /// per pixel and changes which control it is on perhaps twice.
    fn needs_redraw(&self) -> bool {
        self.dirty.get()
    }

    fn handle_event(&mut self, event: &UiEvent) {
        let changed = self.took(event);
        self.dirty.set(changed);
    }

    fn paint(
        &mut self,
        render_ctx: &mut dyn RenderContext,
        styles: &ComputedStyles,
        width: u32,
        height: u32,
        scale: f64,
    ) -> Scene {
        self.dirty.set(false);
        self.draw(render_ctx, styles, width, height, scale)
    }
}

impl ArrangementWidget {
    /// Pointer events, already in the widget's own coordinates.
    ///
    /// Blitz makes them relative to the node before handing them over,
    /// which is what lets this be a hit test against the arrangement
    /// rather than against the window: nothing here has to know where
    /// in the window the arrangement was put.
    ///
    /// The answer is whether the picture changed. Most moves do not: a
    /// pointer crossing the panel raises an event per pixel and changes
    /// which control it is on perhaps twice.
    fn took(&mut self, event: &UiEvent) -> bool {
        let at = |e: &blitz_traits::events::BlitzPointerEvent| {
            (f64::from(e.coords.client_x), f64::from(e.coords.client_y))
        };
        match event {
            UiEvent::PointerMove(e) => {
                let (x, y) = at(e);
                // The editor owns the gesture once it has taken a
                // press: an item being dragged follows the pointer off
                // the lane it started on, which is what dragging is.
                if self.holding {
                    let view = self.viewport(self.size.0, self.size.1);
                    let seconds = self.seconds_at(x, view);
                    let bpm = self.scene.bpm;
                    return self
                        .editor
                        .moved(Some(seconds), view.pps, bpm, mods(e.mods));
                }
                // A knob mid-turn keeps the pointer: the hand can leave
                // the control and the drag goes on, which is what makes
                // a fine adjustment possible at all.
                if self.turning.is_some() {
                    self.turn(y, e.mods.contains(Modifiers::CONTROL));
                    return true;
                }
                let spot = self.spot_at(x, y);
                let over = self.item_under(x, y);
                let moved = self.hovered_item != over;
                self.hovered_item = over;
                self.pointer.hover(spot) || moved
            }
            UiEvent::PointerDown(e) => {
                let (x, y) = at(e);
                let spot = self.spot_at(x, y);
                self.pointer.hover(spot);
                self.pointer.press();
                // Nothing in the panel: the arrangement and the ruler
                // are the editor's.
                if spot.is_none() {
                    let hit = self.hit(x, y);
                    let mut effects = Vec::new();
                    let took = self
                        .editor
                        .press(hit, mods(e.mods), &self.scene, &mut effects);
                    self.settle(effects);
                    if took {
                        self.holding = true;
                        return true;
                    }
                }
                self.turning = spot
                    .filter(|spot| {
                        matches!(
                            spot.control,
                            crate::row::Control::Volume | crate::row::Control::Pan
                        )
                    })
                    .and_then(|spot| {
                        let (was, _) = self.rows.get(spot.row)?;
                        Some(Turn {
                            spot,
                            from: y,
                            was: was.clone(),
                        })
                    });
                true
            }
            UiEvent::PointerUp(e) => {
                let (x, y) = at(e);
                // A turn ends where it ends. It has already been sent,
                // every move of it, so there is nothing to do on
                // release but stop — and NOT to treat it as a click,
                // which would re-fire the control it was turning.
                if self.turning.take().is_some() {
                    self.pointer.release();
                    self.pointer.hover(self.spot_at(x, y));
                    return true;
                }
                if self.holding {
                    self.holding = false;
                    let view = self.viewport(self.size.0, self.size.1);
                    let seconds = self.seconds_at(x, view);
                    let mut effects = Vec::new();
                    let mut project = core::mem::take(&mut self.project);
                    self.editor
                        .release(Some(seconds), mods(e.mods), &mut project, &mut effects);
                    self.project = project;
                    self.settle(effects);
                    return true;
                }
                // Acted on only if the release lands on the control the
                // press did. Dragging off a button and letting go is
                // how every toolkit says "no, cancel that", and the
                // pressed look is already drawn to promise it.
                let up = self.spot_at(x, y);
                if let Some(spot) = self.pointer.active().map(|(spot, _)| spot)
                    && up == Some(spot)
                {
                    self.act(spot);
                }
                self.pointer.release();
                self.pointer.hover(up);
                true
            }
            UiEvent::KeyDown(e) => self.typed(e),
            UiEvent::PointerCancel(_) => {
                self.holding = false;
                self.turning = None;
                self.pointer.release();
                true
            }
            _ => false,
        }
    }

    fn draw(
        &mut self,
        _render_ctx: &mut dyn RenderContext,
        _styles: &ComputedStyles,
        width: u32,
        height: u32,
        _scale: f64,
    ) -> Scene {
        let began = std::time::Instant::now();
        let at_now = *self.view.borrow();
        let view = self.viewport(f64::from(width), f64::from(height));
        self.size = (view.width, view.height);

        // The panel is cut at a zoom, not scaled to one. Asking every
        // frame is free when the zoom has not moved, and on the frame
        // it has, re-cutting is the difference between a taller row and
        // a row with the lettering pulled out of shape.
        self.scene.repanel(
            &self.palette,
            &self.font,
            &self.rows,
            self.layout,
            view.zoom_y,
        );

        let mut out = Scene::new();
        // The same five calls, in the same order, as the painted window
        // — including the ruler AFTER the lanes, because the lane
        // backgrounds are opaque and would paint the grid straight out
        // of the frame.
        // Everything under the ruler starts below it. The recording is
        // in session coordinates and knows nothing about the strip over
        // the top of it, so the offset belongs in every transform that
        // places recorded content — which is what the painted window has
        // always done, and leaving it out of one of them is how the
        // controls ended up half a row above their own names.
        let below = ruler::RULER_H - view.scroll_y;
        // Timed pass by pass, by a mark between each. Inline rather
        // than wrapped in a closure because every pass wants `&mut out`
        // and a closure that also holds it is a borrow fight for no
        // gain — five `mark()` calls say the same thing and read as the
        // list of passes they are measuring.
        let mut spent = Passes::default();
        let mut mark = std::time::Instant::now();
        let since = |mark: &mut std::time::Instant| {
            let spent = mark.elapsed().as_micros();
            *mark = std::time::Instant::now();
            spent
        };

        let lanes = self.scene.replay_lanes(
            &mut out,
            view,
            Affine::translate((TCP_WIDTH - view.scroll_x, below))
                * Affine::scale_non_uniform(view.pps, view.zoom_y),
        );
        spent.lanes = since(&mut mark);
        crate::arrangement::titles(
            &mut out,
            &self.palette,
            &self.font,
            &self.scene,
            view,
            (TCP_WIDTH - view.scroll_x, below),
        );
        // What the editor has in flight, over the recorded items: the
        // fade handles on the item under the pointer and a fade being
        // dragged, then the selection's outlines and the ghost of an
        // item being moved or trimmed.
        let lanes_at = (TCP_WIDTH - view.scroll_x, below);
        crate::arrangement::fade_overlay(
            &mut out,
            &self.palette,
            &self.scene,
            view,
            lanes_at,
            self.hovered_item,
            self.editor.fade_in_flight(),
        );
        crate::arrangement::selection_overlay(
            &mut out,
            &self.palette,
            &self.scene,
            view,
            lanes_at,
            &self.editor.selected,
            self.editor.ghost(),
        );
        spent.titles = since(&mut mark);
        let panel = self
            .scene
            .replay_panel(&mut out, view, Affine::translate((0.0, below)));
        spent.panel = since(&mut mark);
        ruler::grid(
            &mut out,
            &self.palette,
            view,
            self.bars,
            &self.grid,
            FINEST,
            (0.0, 0.0),
        );
        ruler::ruler(
            &mut out,
            &self.palette,
            &self.font,
            view,
            self.scene.tempo(),
            (0.0, 0.0),
        );
        ruler::tempo(
            &mut out,
            &self.palette,
            &self.font,
            view,
            (0.0, 0.0),
            self.scene.tempo(),
        );
        // The song's own shape over the timeline: the section bands and
        // the marker flags, and the lines they drop through the lanes.
        ruler::lanes(
            &mut out,
            &self.palette,
            &self.font,
            view,
            (0.0, 0.0),
            self.scene.sections(),
            self.scene.markers(),
        );
        ruler::lane_lines(
            &mut out,
            &self.palette,
            view,
            (0.0, 0.0),
            self.scene.sections(),
            self.scene.markers(),
            ruler::RULER_H,
            view.height,
        );
        spent.ruler = since(&mut mark);
        // And the controls over the recorded rows.
        //
        // Recorded for every row, which is what makes a scroll free —
        // it replays a different span of the same cut. The cost of that
        // is that a cut is the whole panel, so while the zoom is MOVING
        // the cache would be rebuilt every frame for rows that are
        // about to change again. Measured: it took zoom-y from 5.1ms to
        // 8.5.
        //
        // So the cut is taken only once the zoom has settled — the
        // frame after it stopped moving. While it moves, the controls
        // are drawn live, exactly as they were before there was a
        // cache. A gesture is no worse and everything after it is free.
        let at = Affine::translate((0.0, below));
        let settled = (self.was - view.zoom_y).abs() < f64::EPSILON;
        self.was = view.zoom_y;
        if !settled {
            self.controls = None;
        }
        let stale = self
            .controls
            .as_ref()
            .is_none_or(|(_, zoom)| (zoom - view.zoom_y).abs() >= f64::EPSILON);
        if settled && stale {
            let recorded = crate::overlay::record_controls(
                &self.palette,
                &self.font,
                &self.scene,
                &self.rows,
                &self.tracks,
                &self.map,
                view,
            );
            self.controls = Some((recorded, view.zoom_y));
        }
        let controls = match self.controls.as_ref() {
            Some((controls, _)) => {
                let counts = controls.replay(&mut out, &self.scene, view, at);
                // The recording is at rest, so the row the pointer is
                // on is drawn again over it. One row of forty, and only
                // while the pointer is on one.
                crate::overlay::hovered_row(
                    &mut out,
                    &self.palette,
                    &self.font,
                    &self.scene,
                    &self.rows,
                    &self.tracks,
                    &self.map,
                    view,
                    &self.pointer,
                    at,
                );
                counts
            }
            None => crate::overlay::panel_controls(
                &mut out,
                &self.palette,
                &self.font,
                &self.scene,
                &self.rows,
                &self.tracks,
                &self.map,
                view,
                &self.pointer,
                at,
            ),
        };
        // The edit cursor and the time selection, over the lanes and
        // up through the ruler. Drawn from the editor's own state,
        // which is what a click on the ruler moves.
        crate::cursor::paint_edit(
            &mut out,
            &self.palette,
            &self.editor.cursor,
            view,
            (0.0, 0.0),
            0.0,
            view.height,
        );
        // And the play cursor over it, which is the transport's and not
        // the editor's — the window polls it and writes it into the
        // view like the scroll.
        crate::cursor::paint(
            &mut out,
            crate::cursor::Look::default(),
            at_now.play_at.mul_add(view.pps, TCP_WIDTH - view.scroll_x),
            0.0,
            view.height,
            TCP_WIDTH,
        );
        // An open rename, over the name it replaces. Last of the panel
        // passes, because it is a field ON one and has to cover it.
        if let Some(rename) = self.renaming.as_ref()
            && let Some((top, height)) = self.scene.row_band(rename.row, view)
            && let Some((track, depth)) = self.rows.get(rename.row)
        {
            let shape = crate::row::Row::new(
                top,
                height,
                i32::try_from(*depth).unwrap_or(0),
                track.is_folder,
            );
            if let Some(field) = shape.rect(crate::row::Control::Name) {
                crate::rename::paint(&mut out, &self.palette, &self.font, rename, field, at);
            }
        }
        spent.controls = since(&mut mark);
        let _ = controls;
        self.spent = spent;

        let total = |a: Counts, b: Counts| Drawn {
            replayed: a.replayed.saturating_add(b.replayed),
            submitted: a.submitted.saturating_add(b.submitted),
        };
        let drawn = total(lanes, panel);
        *self.drawn.borrow_mut() = drawn;

        // Last, so it is over the picture rather than under it. The
        // sample is the PREVIOUS frame — this one is not finished, and
        // will not be until the shell has encoded and presented what
        // this call returns — which is the only honest number a paint
        // can read about itself.
        if let Some(stats) = self.stats.as_mut() {
            // The shell's number when there is a shell. The headless
            // renderer draws frames and presents none, so there it
            // never writes one — and a graph that stays empty in the
            // one mode that can dump a PNG of itself is a graph nobody
            // can look at. So fall back to what this paint cost, and
            // say which is being shown rather than letting the two be
            // mistaken for each other.
            let shell = blitz_traits::LAST_FRAME_MICROS.load(core::sync::atomic::Ordering::Relaxed);
            // The same frame split at the hand-off to the GPU. A window
            // that draws too much and a window that draws little and
            // waits on the compositor to take it are the same frame
            // time and want opposite fixes; these say which one this is.
            let encode =
                blitz_traits::LAST_ENCODE_MICROS.load(core::sync::atomic::Ordering::Relaxed);
            let present =
                blitz_traits::LAST_PRESENT_MICROS.load(core::sync::atomic::Ordering::Relaxed);
            let source = if shell > 0 { "frame" } else { "paint" };
            let micros = if shell > 0 {
                shell
            } else {
                u64::try_from(began.elapsed().as_micros()).unwrap_or(u64::MAX)
            };
            stats.add(micros);
            crate::fps::draw(
                &mut out,
                &self.font,
                stats,
                (view.width, view.height),
                &[
                    format!("{} of {} commands", drawn.submitted, drawn.replayed),
                    format!(
                        "lanes {:.1}  panel {:.1}  ruler {:.1}",
                        ms(spent.lanes),
                        ms(spent.panel),
                        ms(spent.ruler)
                    ),
                    format!(
                        "titles {:.1}  controls {:.1}  ({source})",
                        ms(spent.titles),
                        ms(spent.controls)
                    ),
                    format!(
                        "encode {:.1}  present {:.1}",
                        ms(u128::from(encode)),
                        ms(u128::from(present))
                    ),
                ],
            );
        }
        out
    }
}

/// Microseconds as milliseconds, for the readout's lines.
fn ms(micros: u128) -> f64 {
    u32::try_from(micros).map_or(f64::from(u32::MAX), f64::from) / 1000.0
}

#[cfg(test)]
mod tests {
    use super::{ArrangementWidget, Shared, View};
    use crate::engine::Edit;
    use crate::row::Control as C;
    use blitz_dom::node::Widget as _;
    use blitz_traits::events::UiEvent;

    /// Four rows tall enough that every control has somewhere to be.
    /// The narrow tiers drop mute and solo on purpose, and a test that
    /// used them would be asserting they are missing.
    fn widget() -> ArrangementWidget {
        let palette = crate::arrangement::Palette::from_theme(&daw_ui::theming::Theme::dark());
        let font = crate::text::Font::embedded().expect("the embedded font");
        let tracks: Vec<daw_proto::Track> = (0..4)
            .map(|i| daw_proto::Track {
                guid: format!("t{i}"),
                name: format!("Track {i}"),
                height: Some(90),
                ..daw_proto::Track::default()
            })
            .collect();
        let rows: Vec<(daw_proto::Track, u32)> = tracks.iter().cloned().map(|t| (t, 0)).collect();
        let refs = daw_ui::studio::RowsRef(std::sync::Arc::new(rows.clone()));
        let project =
            daw_ui::studio::ProjectRef(std::sync::Arc::new(daw_ui::studio::Project::default()));
        let layout = crate::layout::Layout::default();
        let scene = crate::arrangement::Arrangement::build(
            &palette,
            &font,
            &project,
            &refs,
            layout,
            &crate::midi::Previews::default(),
        );
        let view: Shared = std::rc::Rc::new(std::cell::RefCell::new(View {
            scroll_x: 0.0,
            scroll_y: 0.0,
            zoom_x: 1.0,
            zoom_y: 1.0,
            play_at: 0.0,
        }));
        let mut widget = ArrangementWidget::new(
            scene,
            palette,
            font,
            120.0,
            100.0,
            rows,
            layout,
            daw_ui::studio::project::Project::default(),
            crate::midi::Previews::default(),
            view,
            false,
        );
        widget.size = (1600.0, 900.0);
        widget
    }

    /// Where a control is, in the widget's own coordinates — which is
    /// what an event arrives in.
    fn at(widget: &ArrangementWidget, row: usize, control: C) -> (f64, f64) {
        let view = widget.viewport(widget.size.0, widget.size.1);
        let (top, height) = widget.scene.row_band(row, view).expect("a row");
        let (track, depth) = &widget.rows[row];
        let shape = crate::row::Row::new(
            top,
            height,
            i32::try_from(*depth).unwrap_or(0),
            track.is_folder,
        );
        let r = shape.rect(control).expect("a control with somewhere to be");
        (
            r.x0 + r.width() / 2.0,
            r.y0 + r.height() / 2.0 + crate::ruler::RULER_H,
        )
    }

    /// A mouse at a point, with nothing else going on — which is what
    /// the widget reads: the coordinates and nothing more.
    fn pointer(x: f64, y: f64) -> blitz_traits::events::BlitzPointerEvent {
        use blitz_traits::events::{
            BlitzPointerEvent, BlitzPointerId, Modifiers, MouseEventButton, MouseEventButtons,
            Point, PointerCoords, PointerDetails,
        };
        #[expect(
            clippy::cast_possible_truncation,
            reason = "a coordinate inside a test window"
        )]
        let (x, y) = (x as f32, y as f32);
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
            button: MouseEventButton::Main,
            buttons: MouseEventButtons::Primary,
            mods: Modifiers::default(),
            details: PointerDetails::default(),
            element: Point { x, y },
            active_pointers: std::sync::Arc::default(),
        }
    }

    #[test]
    fn a_move_finds_the_control_under_it() {
        let mut widget = widget();
        let (x, y) = at(&widget, 2, C::Mute);
        assert!(
            widget.took(&UiEvent::PointerMove(pointer(x, y))),
            "moving onto a control is a change worth redrawing"
        );
        assert_eq!(
            widget
                .pointer
                .hovered()
                .map(|spot| (spot.row, spot.control)),
            Some((2, C::Mute))
        );
        // And the second move onto the SAME control is not: a pointer
        // crossing a panel raises an event per pixel, and redrawing for
        // each of them is the thing hover state exists to avoid.
        assert!(
            !widget.took(&UiEvent::PointerMove(pointer(x + 1.0, y))),
            "staying on one control changes nothing"
        );
    }

    #[test]
    fn a_click_on_mute_asks_for_a_mute() {
        let mut widget = widget();
        let (x, y) = at(&widget, 1, C::Mute);
        widget.handle_event(&UiEvent::PointerDown(pointer(x, y)));
        widget.handle_event(&UiEvent::PointerUp(pointer(x, y)));
        let queued = widget.edits.borrow().clone();
        assert!(
            matches!(queued.as_slice(), [Edit::ToggleMute(guid)] if guid == "t1"),
            "{queued:?}"
        );
        // And the widget shows it without waiting for the engine.
        assert!(widget.rows[1].0.muted, "the row it drew from");
        assert!(widget.tracks[1].muted, "the track the controls read");
    }

    #[test]
    fn dragging_off_a_button_cancels_it() {
        let mut widget = widget();
        let (x, y) = at(&widget, 1, C::Solo);
        let (ax, ay) = at(&widget, 3, C::Mute);
        widget.handle_event(&UiEvent::PointerDown(pointer(x, y)));
        widget.handle_event(&UiEvent::PointerUp(pointer(ax, ay)));
        assert!(
            widget.edits.borrow().is_empty(),
            "releasing somewhere else is how every toolkit says 'no'"
        );
        assert!(!widget.rows[1].0.soloed);
    }

    fn key(named: blitz_traits::events::Key) -> blitz_traits::events::BlitzKeyEvent {
        use blitz_traits::events::{BlitzKeyEvent, Code, KeyState, Location, Modifiers};
        BlitzKeyEvent {
            key: named,
            code: Code::Unidentified,
            modifiers: Modifiers::default(),
            location: Location::Standard,
            is_auto_repeating: false,
            is_composing: false,
            state: KeyState::Pressed,
            text: None,
        }
    }

    #[test]
    fn dragging_the_pan_knob_turns_it() {
        use blitz_traits::events::Key;
        let _ = Key::Escape;
        let mut widget = widget();
        let (x, y) = at(&widget, 2, C::Pan);
        let before = widget.rows[2].0.pan;
        widget.handle_event(&UiEvent::PointerDown(pointer(x, y)));
        // Up is more, so a drag UP raises the value.
        widget.handle_event(&UiEvent::PointerMove(pointer(x, y - 30.0)));
        let after = widget.rows[2].0.pan;
        assert!(after > before, "{before} -> {after}");
        assert!(matches!(
            widget.edits.borrow().last(),
            Some(Edit::SetPan(guid, _)) if guid == "t2"
        ));

        // Measured from the PRESS, not from the last frame: a second
        // move to the same place must mean the same value, or a slow
        // drag and a fast one over the same distance end up apart.
        widget.handle_event(&UiEvent::PointerMove(pointer(x, y - 30.0)));
        assert!((widget.rows[2].0.pan - after).abs() < f64::EPSILON);

        // And the release is not also a click on the knob.
        widget.handle_event(&UiEvent::PointerUp(pointer(x, y - 30.0)));
        assert!(widget.turning.is_none());
    }

    #[test]
    fn a_double_click_on_a_name_opens_a_rename_and_enter_commits_it() {
        use blitz_traits::events::Key;
        let mut widget = widget();
        let (x, y) = at(&widget, 0, C::Name);
        let click = |w: &mut ArrangementWidget| {
            w.handle_event(&UiEvent::PointerDown(pointer(x, y)));
            w.handle_event(&UiEvent::PointerUp(pointer(x, y)));
        };
        click(&mut widget);
        assert!(
            widget.renaming.is_none(),
            "one click selects, it does not rename"
        );
        click(&mut widget);
        assert!(
            widget.renaming.is_some(),
            "the second click inside the window renames"
        );

        widget.handle_event(&UiEvent::KeyDown(key(Key::Backspace)));
        widget.handle_event(&UiEvent::KeyDown(key(Key::Character("!".into()))));
        widget.handle_event(&UiEvent::KeyDown(key(Key::Enter)));
        assert!(widget.renaming.is_none(), "Enter closes the field");
        // "Track 0" with the 0 backspaced away and a ! typed in.
        assert_eq!(widget.rows[0].0.name, "Track !");
        assert!(matches!(
            widget.edits.borrow().last(),
            Some(Edit::Rename(guid, name)) if guid == "t0" && name == "Track !"
        ));
        // The name is recorded chrome, so the row has to be cut again
        // or the field closes on the old one.
        assert!(widget.scene.panel_zoom.is_nan(), "the panel was forgotten");
    }

    #[test]
    fn escape_abandons_a_rename() {
        use blitz_traits::events::Key;
        let mut widget = widget();
        let (x, y) = at(&widget, 0, C::Name);
        for _ in 0..2 {
            widget.handle_event(&UiEvent::PointerDown(pointer(x, y)));
            widget.handle_event(&UiEvent::PointerUp(pointer(x, y)));
        }
        widget.handle_event(&UiEvent::KeyDown(key(Key::Character("z".into()))));
        widget.handle_event(&UiEvent::KeyDown(key(Key::Escape)));
        assert!(widget.renaming.is_none());
        assert_eq!(widget.rows[0].0.name, "Track 0", "unchanged");
        assert!(
            !widget
                .edits
                .borrow()
                .iter()
                .any(|edit| matches!(edit, Edit::Rename(..)))
        );
    }

    #[test]
    fn a_click_outside_the_panel_asks_for_nothing() {
        let mut widget = widget();
        let past = crate::arrangement::TCP_WIDTH + 200.0;
        widget.handle_event(&UiEvent::PointerDown(pointer(past, 300.0)));
        widget.handle_event(&UiEvent::PointerUp(pointer(past, 300.0)));
        assert!(widget.edits.borrow().is_empty());
        assert_eq!(widget.pointer.hovered(), None);
    }
}
