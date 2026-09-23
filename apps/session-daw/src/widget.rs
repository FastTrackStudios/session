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
//! both ways and gated pixel-for-pixel against each other (by a test
//! that went with the component lanes once they lost):
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

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use anyrender::{RenderContext, Scene};
use blitz_dom::node::{ComputedStyles, Widget};
use blitz_traits::events::{Modifiers, MouseEventButton, UiEvent};

use vello::kurbo::Affine;

use crate::arrangement::{Arrangement, Palette, Viewport};
use crate::mousemap::Gesture;
use crate::profile::Counts;
use crate::ruler::{self, Bars};

/// What the arrangement shares with a docked mixer — see
/// `crate::mixer_panel::Links`.
#[derive(Clone)]
pub struct MixerLinks {
    pub toggle: Rc<std::cell::Cell<bool>>,
    pub rows: Rc<RefCell<(u64, Vec<(daw_proto::Track, u32)>)>>,
    /// The mixer's edits, to apply here as well.
    pub echo: Rc<RefCell<Vec<crate::engine::Edit>>>,
    /// Keys pressed while the mixer had the focus, to handle here.
    pub keys: Rc<RefCell<Vec<crate::mixer_panel::Key>>>,
}

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

impl View {
    /// Where a freshly opened session is looked at from: the start, at
    /// zoom one, the playhead at zero.
    pub const OPENING: Self = Self {
        scroll_x: 0.0,
        scroll_y: 0.0,
        zoom_x: 1.0,
        zoom_y: 1.0,
        play_at: 0.0,
    };
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
    /// The tool the panel holds and the pointer's shape — see
    /// [`crate::tool`].
    pointing: crate::tool::Shared,
    /// What the which-key popup shows, for the panel to render.
    which: crate::which_key::Shared,
    /// Zooms the keys asked for, for the panel (which owns the view) to
    /// carry out.
    zooms: crate::zoom::Requests,
    /// A song opens fitted: every track top to bottom (`z v`), then the
    /// whole song across (`z x`) — asked once, on the first paint, when
    /// the rows have their places.
    fit_on_open: bool,
    /// How to plan the rows again, and the project as the engine has it
    /// (with the visibility this window has changed since): what a track
    /// shown or hidden re-plans from. `None` in a widget built without
    /// one, which then cannot show or hide anything.
    replan: Option<(crate::studio::Planner, daw_ui::studio::project::Project)>,
    /// The rows' height at zoom 1, for the panel's scroll range, which
    /// changes when rows are shown or hidden.
    content_h: Rc<std::cell::Cell<f64>>,
    /// The docked mixer's links (`crate::mixer_panel::Links`): asking for
    /// it to toggle, the rows it shows, and the edits it made, to apply
    /// here too. `None` with no mixer beside this arrangement.
    mixer: Option<MixerLinks>,
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
    /// A window asks; a picture or a benchmark does not, because an
    /// overlay is a difference in the one and a cost in the other.
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
    /// Where the play cursor has been, for its trail.
    trail: crate::cursor::Trail,
    /// The engine's levels, for the meters in the name fields — and
    /// whether any was lit last frame, so a falling meter keeps redrawing.
    meters: Option<crate::engine::Meters>,
    meters_lit: Cell<bool>,
    /// Whether the panel is in its compact shape — shared with the
    /// toolbar that toggles it, read each paint (a change re-cuts the
    /// recorded panel, which is drawn to the shape).
    compact: Rc<Cell<bool>>,
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
    /// What a key means here.
    ///
    /// The window used to have no answer at all: `typed` served the
    /// rename field and dropped everything else, so on the blitz path
    /// Delete, Home and every bound action did nothing — the gesture
    /// reached the widget and the widget threw it away. Loaded here
    /// rather than in the binary because the EDITOR is here, and a key
    /// that acts on the arrangement has to arrive where the arrangement
    /// is. See #124.
    keys: crate::keys::Keys,
    /// The last refused edit, until it has had its say.
    ///
    /// One at a time and the newest wins: refusals arrive in answer to
    /// a gesture, and the gesture in front of you is the one you are
    /// asking about. A queue would show you the reply to a drag you had
    /// already given up on.
    notice: Option<crate::notice::Notice>,
    /// When and where the last click on a NAME landed, so the next one
    /// can tell whether it is the second half of a double.
    last_name: Option<(usize, web_time::Instant)>,
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
            pointing: crate::tool::Pointing::shared(None),
            which: crate::which_key::Shared::default(),
            zooms: crate::zoom::Requests::default(),
            fit_on_open: true,
            replan: None,
            mixer: None,
            content_h: Rc::new(std::cell::Cell::new(
                rows.iter().map(|(track, _)| layout.height_of(track.height)).sum(),
            )),
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
            trail: crate::cursor::Trail::default(),
            meters: crate::engine::Meters::start(),
            meters_lit: Cell::new(false),
            compact: Rc::new(Cell::new(false)),
            sections: project.sections.clone(),
            markers: project.markers.clone(),
            project,
            previews,
            renaming: None,
            keys: crate::keys::Keys::load(),
            notice: None,
            last_name: None,
            turning: None,
            edits: Rc::new(RefCell::new(Vec::new())),
            controls: None,
            was: at_rest,
            stats: readout.then(crate::fps::Stats::new),
            spent: Passes::default(),
        }
    }

    /// The widget over an opened session, built the way every host builds
    /// it: the dark theme's palette, the embedded font, the row layout
    /// the environment asks for, and the recording cut in the panel's
    /// shape (`compact`).
    ///
    /// The app's panel ([`crate::panel::use_arrangement_panel`]) and the
    /// headless benchmark (`bin/bench`) both start here, so a number the
    /// bench prints is the cost of the recording the window draws.
    ///
    /// # Panics
    ///
    /// If the embedded font does not load — a build defect, not a
    /// runtime condition.
    #[must_use]
    pub fn for_session(
        project: &daw_ui::studio::ProjectRef,
        rows: &daw_ui::studio::RowsRef,
        previews: &crate::midi::Previews,
        compact: bool,
        view: Shared,
        readout: bool,
    ) -> Self {
        let palette = Palette::from_theme(&daw_ui::theming::Theme::dark());
        let font = crate::text::Font::embedded().expect("the embedded font");
        let layout = crate::layout::Layout::from_env();
        let scene = Arrangement::build(
            &palette,
            &font,
            project,
            rows,
            layout,
            previews,
            crate::tcp::Tcp { compact },
        );
        let bpm = scene.bpm;
        Self::new(
            scene,
            palette,
            font,
            bpm,
            crate::studio::PPS,
            rows.as_slice().to_vec(),
            layout,
            // The widget's own copy, which its editor moves before the
            // engine has — see `ArrangementWidget::project`.
            (*project.0).clone(),
            previews.clone(),
            view,
            readout,
        )
        .with_compact(Rc::new(Cell::new(compact)))
    }

    /// The recording the widget replays — for a caller that has to aim a
    /// gesture at something in it (the benchmark's slip drag).
    #[must_use]
    pub const fn scene(&self) -> &Arrangement {
        &self.scene
    }

    /// Share the panel's shape with whoever toggles it.
    #[must_use]
    pub fn with_compact(mut self, compact: Rc<Cell<bool>>) -> Self {
        self.compact = compact;
        self
    }

    /// Share the panel's tool state and window, so a tool can take the
    /// mouse and the pointer can show what a press would do.
    #[must_use]
    pub fn with_pointing(mut self, pointing: crate::tool::Shared) -> Self {
        self.pointing = pointing;
        self
    }

    /// Share the which-key popup's state and the zoom queue with the panel.
    #[must_use]
    pub fn with_view_links(
        mut self,
        which: crate::which_key::Shared,
        zooms: crate::zoom::Requests,
    ) -> Self {
        self.which = which;
        self.zooms = zooms;
        self
    }

    /// Let the widget plan its rows again (the visibility manager), from
    /// the session's planner, and share its content height with the panel.
    #[must_use]
    pub fn with_planner(
        mut self,
        planner: crate::studio::Planner,
        content_h: Rc<std::cell::Cell<f64>>,
    ) -> Self {
        let raw = (*planner.raw).clone();
        self.replan = Some((planner, raw));
        content_h.set(self.content_h.get());
        self.content_h = content_h;
        self
    }

    /// The visibility manager: show or hide a group of tracks, then plan
    /// the rows again and tell the engine. `false` when nothing changed
    /// (no planner, or a group no track is in).
    fn visibility(&mut self, change: &crate::keys::Visibility) -> bool {
        use crate::keys::Visibility;
        let Some((planner, raw)) = self.replan.as_mut() else {
            return false;
        };
        let names: Vec<String> = raw.tracks.iter().map(|t| t.name.clone()).collect();
        let groups = match dynamic_template::visibility::groups(names) {
            Ok(groups) => groups,
            Err(e) => {
                tracing::warn!(error = %e, "visibility: the tracks could not be grouped");
                return false;
            }
        };
        let in_group = |key: &str| -> std::collections::HashSet<String> {
            groups.get(key).map(|n| n.iter().cloned().collect()).unwrap_or_default()
        };
        let (targets, show): (std::collections::HashSet<String>, bool) = match change {
            Visibility::ShowAll => (raw.tracks.iter().map(|t| t.name.clone()).collect(), true),
            Visibility::HideAll => (groups.values().flatten().cloned().collect(), false),
            Visibility::Toggle(group) => {
                let targets = in_group(&dynamic_template::visibility::normalize_key(group));
                // Shown again only when all of it is hidden; any of it on
                // screen, and the toggle hides it — REAPER's rule.
                let any_shown = raw
                    .tracks
                    .iter()
                    .any(|t| targets.contains(&t.name) && t.visible_in_tcp);
                (targets, !any_shown)
            }
        };
        if targets.is_empty() {
            self.notice = Some(crate::notice::Notice::new("no tracks in that group", None));
            return true;
        }
        let mut changed = Vec::new();
        for track in &mut raw.tracks {
            if targets.contains(&track.name) && track.visible_in_tcp != show {
                track.visible_in_tcp = show;
                track.visible_in_mixer = show;
                changed.push(track.guid.clone());
            }
        }
        if changed.is_empty() {
            return false;
        }
        let (project, rows) = planner.plan(raw);
        for guid in changed {
            self.edits
                .borrow_mut()
                .push(crate::engine::Edit::SetVisibility(guid, show, show));
        }
        self.restructure((*project.0).clone(), rows.as_slice().to_vec());
        true
    }

    /// Read the session back from the engine and draw that: what a change
    /// made there rather than here (a toolbar insert, an edited chart laid
    /// over the song) needs. MIDI items the arrangement has not seen — a
    /// chart's new chords — get their notes read first, so they draw
    /// filled rather than empty.
    #[cfg(feature = "native")]
    fn resync(&mut self) {
        let Some(fresh) = crate::studio::fetch() else {
            tracing::warn!("the session could not be read back from the engine");
            return;
        };
        let unread: Vec<(String, f64)> = fresh
            .items
            .values()
            .flatten()
            .filter(|item| fresh.is_midi(&item.guid) && self.previews.get(&item.guid).is_none())
            .map(|item| (item.guid.clone(), item.length.as_seconds()))
            .collect();
        if !unread.is_empty() {
            self.previews.fill_blocking(unread);
        }
        let Some((planner, raw)) = self.replan.as_mut() else {
            return;
        };
        *raw = fresh;
        let (project, rows) = planner.plan(raw);
        self.restructure((*project.0).clone(), rows.as_slice().to_vec());
    }

    /// Take a new set of rows (tracks shown or hidden): everything the
    /// rows are drawn and hit-tested from, then the recording.
    fn restructure(
        &mut self,
        project: daw_ui::studio::project::Project,
        rows: Vec<(daw_proto::Track, u32)>,
    ) {
        self.tracks = rows.iter().map(|(t, _)| t.clone()).collect();
        self.map = crate::plan::Rows::of(&rows, &self.tracks);
        self.content_h.set(
            rows.iter()
                .map(|(track, _)| self.layout.height_of(track.height))
                .sum(),
        );
        self.rows = rows;
        self.project = project;
        self.hovered_item = None;
        self.turning = None;
        self.pointer = crate::pointer::Pointer::default();
        self.recut();
        self.publish_rows();
    }

    /// Link to a docked mixer: the rows go to it (now, and on every
    /// change), its edits come back here.
    #[must_use]
    pub fn with_mixer(mut self, links: MixerLinks) -> Self {
        self.mixer = Some(links);
        self.publish_rows();
        self
    }

    /// The rows, for the mixer, with the generation bumped.
    fn publish_rows(&self) {
        if let Some(links) = &self.mixer {
            let mut shared = links.rows.borrow_mut();
            shared.0 = shared.0.wrapping_add(1);
            shared.1.clone_from(&self.rows);
        }
    }

    /// The mixer's edits since the last frame, applied here too, and the
    /// keys it passed on, handled as if they had been pressed here.
    fn echoes(&mut self) {
        let Some(links) = &self.mixer else { return };
        let echoed: Vec<crate::engine::Edit> = links.echo.borrow_mut().drain(..).collect();
        let keys: Vec<crate::mixer_panel::Key> = links.keys.borrow_mut().drain(..).collect();
        for edit in &echoed {
            self.assume(edit);
        }
        for key in &keys {
            match key {
                crate::mixer_panel::Key::Down(e) => {
                    self.typed(e);
                }
                crate::mixer_panel::Key::Up(e) => {
                    self.released(e);
                }
            }
        }
    }

    /// Tell the popup what the keyboard has pending.
    fn publish_which_key(&self) {
        *self.which.borrow_mut() = self.keys.which_key();
    }

    /// A zoom the keys asked for, in session units, for the panel.
    /// `None` when there is nothing to frame (no track selected for
    /// `z t`, say).
    fn zoom_request(&self, command: crate::zoom::Command) -> Option<crate::zoom::Request> {
        use crate::zoom::{Command, Request};
        let span = |boxes: &mut dyn Iterator<Item = (f64, f64)>| {
            boxes.fold(None, |acc: Option<(f64, f64)>, (a, b)| {
                Some(acc.map_or((a, b), |(lo, hi)| (lo.min(a), hi.max(b))))
            })
        };
        let rows_of = |rows: &mut dyn Iterator<Item = usize>| {
            span(&mut rows.filter_map(|row| {
                self.scene.row_box(row).map(|(top, h)| (top, top + h))
            }))
        };
        let selected_rows =
            || rows_of(&mut (0..self.rows.len()).filter(|&row| self.rows[row].0.selected));
        let selected_items = || {
            (0..self.scene.items())
                .filter_map(|i| self.scene.item(i))
                .filter(|item| self.editor.selected.contains(&item.guid))
                .collect::<Vec<_>>()
        };
        let time_selection = self
            .editor
            .cursor
            .selection
            .filter(|s| s.is_meaningful())
            .map(|s| (s.start, s.end));
        let items_time = || span(&mut selected_items().into_iter().map(|item| (item.x0, item.x1)));
        let frame = |time: Option<(f64, f64)>, rows: Option<(f64, f64)>, toggle| {
            (time.is_some() || rows.is_some()).then_some(Request::Frame { time, rows, toggle })
        };
        match command {
            Command::ToggleTracks => frame(time_selection, selected_rows(), true),
            Command::FitTracks => frame(None, rows_of(&mut (0..self.rows.len())), false),
            Command::Project => frame(Some((0.0, self.project.length_secs.max(1.0))), None, false),
            Command::Selection => frame(time_selection.or_else(items_time), None, false),
            Command::ToggleSelection => frame(time_selection.or_else(items_time), None, true),
            Command::Items => {
                let items = selected_items();
                let rows = rows_of(&mut items.iter().map(|item| item.row));
                frame(items_time(), rows, false)
            }
            Command::Back => Some(Request::Back),
            Command::Forward => Some(Request::Forward),
            Command::Step { vertical, inward } => Some(Request::Scale {
                vertical,
                by: if inward { 1.25 } else { 0.8 },
            }),
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
            panel_w: self.scene.tcp.width(),
        }
    }

    /// The panel control under a point in the widget's own coordinates.
    ///
    /// The panel starts at the widget's left edge and the lanes below
    /// the ruler, so the only conversion is the ruler's height — the
    /// same offset `paint` puts in its transforms.
    fn spot_at(&self, x: f64, y: f64) -> Option<crate::pointer::RowSpot> {
        let view = self.viewport(self.size.0, self.size.1);
        let below = y - ruler::ruler_h();
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
            let now = web_time::Instant::now();
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
        ((x - self.scene.tcp.width() + view.scroll_x) / view.pps.max(f64::EPSILON)).max(0.0)
    }

    /// A key that is not going into a text field: what it is bound to,
    /// done.
    ///
    /// `true` when something happened, which is what tells the shell to
    /// draw again — a key that changed nothing must not cost a frame.
    fn acted(&mut self, event: &blitz_traits::events::BlitzKeyEvent) -> bool {
        use blitz_traits::events::Key;
        // `keyboard_types::Key` is flat: a character carries its text
        // and everything else displays as the W3C name the profile
        // writes ("Delete", "ArrowLeft"). `key_code` wants them apart.
        let named;
        let (named, text) = match &event.key {
            Key::Character(text) => (None, Some(text.as_str())),
            other => {
                named = other.to_string();
                (Some(named.as_str()), None)
            }
        };
        let held = mods(event.modifiers);
        // The window's own: which words the items wear. Not in the
        // keybind profile because REAPER has no such view to bind — and
        // ctrl+shift+K is free there, so it shadows nothing.
        if held.ctrl && held.shift && text.is_some_and(|t| t.eq_ignore_ascii_case("k")) {
            self.scene.lettering = self.scene.lettering.next();
            return true;
        }
        // And whether the ruler carries the CHORDS lane. Ctrl+shift+J is
        // as free in the profile as K.
        if held.ctrl && held.shift && text.is_some_and(|t| t.eq_ignore_ascii_case("j")) {
            ruler::show_chords(!ruler::chords_shown());
            return true;
        }
        let Some(code) = crate::keys::key_code(named, text) else {
            return false;
        };
        // Escape abandons a half-typed sequence, and does only that.
        if code == input::KeyCode::Escape && self.keys.is_pending() {
            self.keys.cancel();
            self.publish_which_key();
            return true;
        }
        // A held key repeats. Inside a sequence a repeat is not a second
        // press: holding `z` is the prefix (and the zoom tool), not `z z`.
        if event.is_auto_repeating && self.keys.is_pending() {
            return true;
        }
        let actions = self.keys.press(
            code,
            input::Modifiers {
                ctrl: held.ctrl,
                alt: held.alt,
                shift: held.shift,
                meta: false,
            },
        );
        self.publish_which_key();
        if actions.is_empty() {
            // Consumed, when it opened or extended a sequence: the popup
            // changed, and the key must not also do anything else.
            return self.keys.is_pending();
        }
        let mut effects = Vec::new();
        let mut handled = false;
        let mut tracks = self.tracks.clone();
        let bpm = self.scene.bpm;
        for action in actions {
            if let crate::keys::Action::View(command) = action {
                if let Some(request) = self.zoom_request(command) {
                    self.zooms.borrow_mut().push(request);
                }
                handled = true;
                continue;
            }
            if let crate::keys::Action::Visibility(change) = &action {
                handled |= self.visibility(change);
                continue;
            }
            if action == crate::keys::Action::ToggleMixer {
                if let Some(links) = &self.mixer {
                    links.toggle.set(true);
                    handled = true;
                }
                continue;
            }
            handled |= self.editor.key(
                action,
                &mut self.project,
                &self.scene,
                &self.rows,
                &mut tracks,
                &self.map,
                bpm,
                &mut effects,
            );
        }
        self.tracks = tracks;
        self.settle(effects);
        handled
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
                // Straight to the engine, fire and forget, as the
                // painted window does: dropping this is why space did
                // nothing in the Blitz window. The play cursor needs no
                // telling — the window polls it from the engine.
                Effect::Transport(command, at) => crate::engine::transport(command, at),
                Effect::Playhead(_) => {}
                // The warn line stays: a refusal is alertable, and the
                // log is where a session nobody was watching gets read
                // back. The notice is for the person who IS watching.
                Effect::Refused { why, row } => {
                    tracing::warn!(why, "the edit was refused");
                    self.notice = Some(crate::notice::Notice::new(why, row));
                }
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
        // A view setting, not the session's: an edit must not reset it.
        let lettering = self.scene.lettering;
        self.scene = Arrangement::build(
            &self.palette,
            &self.font,
            &project,
            &rows,
            self.layout,
            &self.previews,
            crate::tcp::Tcp { compact: self.compact.get() },
        );
        self.scene.lettering = lettering;
        self.sections = self.project.sections.clone();
        self.markers = self.project.markers.clone();
        self.controls = None;
    }

    /// A key came back up: the processor's held-key state (auto-repeat,
    /// the sticky prefix), and the one decision a held `z` leaves open.
    /// Held and used as the zoom tool, it was the tool, not a prefix:
    /// the popup goes and nothing is left pending. Tapped, it stays open
    /// for the next key.
    fn released(&mut self, event: &blitz_traits::events::BlitzKeyEvent) -> bool {
        use blitz_traits::events::Key;
        let named;
        let (named, text) = match &event.key {
            Key::Character(text) => (None, Some(text.as_str())),
            other => {
                named = other.to_string();
                (Some(named.as_str()), None)
            }
        };
        let Some(code) = crate::keys::key_code(named, text) else {
            return false;
        };
        let held = mods(event.modifiers);
        let was = self.keys.which_key();
        let hide = self.keys.release(
            code.clone(),
            input::Modifiers {
                ctrl: held.ctrl,
                alt: held.alt,
                shift: held.shift,
                meta: false,
            },
        );
        let tool = std::mem::take(&mut self.pointing.borrow_mut().tool_used);
        if hide || (tool && code == input::KeyCode::Character("z".into())) {
            self.keys.cancel();
        }
        self.publish_which_key();
        was != self.keys.which_key()
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
            // No field open, so the key is the session's rather than a
            // letter. While one IS open the keyboard belongs to it —
            // `d` must type a d, not delete what is selected.
            return self.acted(event);
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
        // A notice fading and a refused ghost going home both change the
        // picture with no event behind them, so neither can wait on
        // `dirty` — nothing is going to set it. Nor can a read-back the
        // engine asked for.
        self.dirty.get()
            || crate::studio::resync_pending()
            || self
                .notice
                .as_ref()
                .is_some_and(crate::notice::Notice::alive)
            || self.editor.settling()
            // A trail drawing in behind a stopped cursor changes the
            // picture every frame with nothing else moving.
            || self.trail.alive(web_time::Instant::now())
            || self.meters_lit.get()
            // The toolbar changed the panel's shape: nothing else will
            // ask for the frame that re-cuts it.
            || self.scene.tcp.compact != self.compact.get()
            // Other people's pointers glide and their play cursors move
            // with nothing happening here.
            || crate::ghosts::active()
    }

    fn handle_event(&mut self, event: &UiEvent) {
        // A press in the arrangement takes the keyboard from the chart
        // editor, so the space bar plays again.
        if matches!(event, UiEvent::PointerDown(_)) {
            crate::keys::set_editing(false);
        }
        self.event(event);
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

impl ArrangementWidget {
    /// One input event, in the widget's own coordinates: what Blitz's
    /// `Widget::handle_event` calls, and what the web host calls.
    pub fn event(&mut self, event: &UiEvent) {
        // What a press here would do, asked BEFORE the press is taken: a
        // razor starts as an area under the pointer, which would then
        // answer as the area rather than as the razor being drawn.
        let pressed = match event {
            UiEvent::PointerDown(e) if !self.stands_down(event) => {
                let (x, y) = (f64::from(e.coords.client_x), f64::from(e.coords.client_y));
                self.hit(x, y).map(|hit| {
                    let context = self.editor.context_at(hit);
                    crate::mousemap::resolve(context, Gesture::Drag, mods(e.mods)).action
                })
            }
            _ => None,
        };
        let changed = !self.stands_down(event) && self.took(event);
        self.point(event, pressed);
        self.dirty.set(changed);
    }

    /// The picture at `width` x `height`: what Blitz's `Widget::paint`
    /// returns, and what the web host draws into its canvas.
    pub fn paint_scene(&mut self, width: u32, height: u32, scale: f64) -> Scene {
        self.dirty.set(false);
        let scene = self.draw(width, height, scale);
        if std::mem::take(&mut self.fit_on_open) {
            for command in [crate::zoom::Command::FitTracks, crate::zoom::Command::Project] {
                if let Some(request) = self.zoom_request(command) {
                    self.zooms.borrow_mut().push(request);
                }
            }
        }
        scene
    }

    /// Whether this event is not the arrangement's to act on.
    ///
    /// A press with any button but the main one: the middle button is the
    /// panel's hand, and nothing here is bound to the others. And anything
    /// while a tool is up, which owns the mouse. A gesture already under
    /// way still finishes, though: the main button's release always comes
    /// through, or a drag begun before `z` went down would never end.
    fn stands_down(&self, event: &UiEvent) -> bool {
        let busy = self.holding || self.turning.is_some() || self.pointer.pressed().is_some();
        match event {
            UiEvent::PointerDown(e) => {
                e.button != MouseEventButton::Main || self.pointing.borrow().tool_active()
            }
            UiEvent::PointerUp(e) => e.button != MouseEventButton::Main,
            UiEvent::PointerMove(_) => !busy && self.pointing.borrow().tool_active(),
            _ => false,
        }
    }

    /// Tell [`crate::tool::Pointing`] what the pointer is over and what it
    /// is doing, and put the matching shape on the window.
    fn point(&self, event: &UiEvent, pressed: Option<crate::mousemap::Action>) {
        let e = match event {
            UiEvent::PointerMove(e) | UiEvent::PointerDown(e) | UiEvent::PointerUp(e) => e,
            UiEvent::PointerCancel(_) => {
                let mut pointing = self.pointing.borrow_mut();
                pointing.gesture = None;
                pointing.apply();
                return;
            }
            _ => return,
        };
        let (x, y) = (f64::from(e.coords.client_x), f64::from(e.coords.client_y));
        let over = if self.turning.is_some() {
            // A knob mid-turn keeps its shape off the knob, as it keeps
            // the pointer.
            crate::tool::Over::Knob
        } else if let Some(spot) = self.spot_at(x, y) {
            if spot.control.is_continuous() {
                crate::tool::Over::Knob
            } else {
                crate::tool::Over::Button
            }
        } else {
            self.hit(x, y).map_or(crate::tool::Over::Nothing, |hit| {
                crate::tool::Over::Map(self.editor.context_at(hit))
            })
        };
        let mut pointing = self.pointing.borrow_mut();
        pointing.mods = mods(e.mods);
        pointing.inside = true;
        pointing.over = over;
        if !self.holding {
            pointing.gesture = None;
        } else if pressed.is_some() {
            pointing.gesture = pressed;
        }
        pointing.apply();
    }

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
                    // A razor is the one drag here that is a rectangle,
                    // so it is the one that needs the row as well as
                    // the time.
                    if self.editor.razor_in_flight().is_some() {
                        let row = self
                            .scene
                            .row_at_screen(y - ruler::ruler_h() + view.scroll_y, view)
                            .unwrap_or(0);
                        return self.editor.razor_moved(seconds, row, bpm);
                    }
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
            UiEvent::KeyUp(e) => self.released(e),
            UiEvent::PointerCancel(_) => {
                self.holding = false;
                self.turning = None;
                self.pointer.release();
                true
            }
            _ => false,
        }
    }

    fn draw(&mut self, width: u32, height: u32, _scale: f64) -> Scene {
        self.echoes();
        // A rename open is a text field holding the keyboard: the window's
        // transport keys stand aside while it is (a space in a name).
        crate::keys::set_typing(self.renaming.is_some());
        // Where the edit cursor and selection are, for the toolbar's inserts.
        crate::cursor::publish(self.editor.cursor);
        // The engine changed the session under us (a toolbar insert, an
        // edited chart): read it back before drawing it.
        #[cfg(feature = "native")]
        if crate::studio::take_resync() {
            self.resync();
        }
        // The panel's shape, if the toolbar has changed it: the rows are
        // RECORDED to it, so this re-cuts rather than re-scales.
        if self.scene.tcp.compact != self.compact.get() {
            self.recut();
        }
        let began = web_time::Instant::now();
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
        let below = ruler::ruler_h() - view.scroll_y;
        // Timed pass by pass, by a mark between each. Inline rather
        // than wrapped in a closure because every pass wants `&mut out`
        // and a closure that also holds it is a borrow fight for no
        // gain — five `mark()` calls say the same thing and read as the
        // list of passes they are measuring.
        let mut spent = Passes::default();
        let mut mark = web_time::Instant::now();
        let since = |mark: &mut web_time::Instant| {
            let spent = mark.elapsed().as_micros();
            *mark = web_time::Instant::now();
            spent
        };

        let lanes = self.scene.replay_lanes(
            &mut out,
            view,
            Affine::translate((self.scene.tcp.width() - view.scroll_x, below))
                * Affine::scale_non_uniform(view.pps, view.zoom_y),
        );
        spent.lanes = since(&mut mark);
        // An item being slipped, redrawn at the offset the drag has
        // reached. Immediately after the lanes and BEFORE the titles:
        // it covers the recorded item to hide the old waveform, and a
        // cover drawn after the titles takes the item's name with it —
        // which reads as the label vanishing for the length of a drag.
        crate::arrangement::slip_overlay(
            &mut out,
            &self.palette,
            &self.scene,
            view,
            (self.scene.tcp.width() - view.scroll_x, below),
            self.editor.slip_in_flight(),
        );
        crate::arrangement::titles(
            &mut out,
            &self.palette,
            &self.font,
            &self.scene,
            view,
            (self.scene.tcp.width() - view.scroll_x, below),
        );
        // What the editor has in flight, over the recorded items: the
        // fade handles on the item under the pointer and a fade being
        // dragged, then the selection's outlines and the ghost of an
        // item being moved or trimmed.
        let lanes_at = (self.scene.tcp.width() - view.scroll_x, below);
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
        // The razor's rectangles over the items they cut, and under
        // nothing: they are the thing being aimed, so they go last of
        // the lane passes.
        crate::arrangement::razor_overlay(
            &mut out,
            &self.palette,
            &self.scene,
            view,
            lanes_at,
            &self.editor.razor,
            self.editor.razor_in_flight(),
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
        ruler::chord_lane(
            &mut out,
            &self.palette,
            &self.font,
            view,
            (0.0, 0.0),
            self.scene.chart(),
            self.scene.lettering,
        );
        ruler::lane_lines(
            &mut out,
            &self.palette,
            view,
            (0.0, 0.0),
            self.scene.sections(),
            self.scene.markers(),
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
        // The levels, in the name fields.
        if let Some(meters) = &self.meters {
            let lit = crate::overlay::row_meters(
                &mut out,
                &self.palette,
                &self.scene,
                &self.rows,
                &self.tracks,
                &self.map,
                view,
                &meters.levels(),
                at,
            );
            self.meters_lit.set(lit);
        }
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
        let now = web_time::Instant::now();
        self.trail.record(now, at_now.play_at);
        crate::cursor::paint(
            &mut out,
            {
                let look = crate::cursor::Look::default();
                look.trailing(self.trail.length(now, view.pps, look.trail))
            },
            at_now.play_at.mul_add(view.pps, self.scene.tcp.width() - view.scroll_x),
            0.0,
            view.height,
            self.scene.tcp.width(),
        );
        // Everyone else in the session, faintly: their selections,
        // cursors and mouse. Over your cursors so a peer on the same spot
        // is still seen, but drawn thin and pale so theirs never reads as
        // yours.
        crate::ghosts::paint(
            &mut out,
            &self.palette,
            &self.font,
            &self.scene,
            &self.rows,
            view,
            lanes_at,
            view.height,
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
                self.scene.tcp,
            );
            if let Some(field) = shape.rect(crate::row::Control::Name) {
                crate::rename::paint(&mut out, &self.palette, &self.font, rename, field, at);
            }
        }
        // A refused edit, saying why, on the row it was refused on.
        //
        // Over every other pass because it is a reply and not part of
        // the picture: it has to be legible against whatever was
        // already there, including an open rename.
        let showing = self.notice.as_ref().and_then(|notice| {
            let alpha = notice.alpha()?;
            Some((
                notice.why,
                crate::notice::area(&self.font, &self.scene, &self.rows, notice, view),
                alpha,
            ))
        });
        match showing {
            Some((why, area, alpha)) => {
                crate::notice::paint(&mut out, &self.palette, &self.font, why, area, alpha, at);
            }
            // Dropped on the first frame after its time rather than on a
            // timer, so `needs_redraw` and the paint agree about when it
            // is gone.
            None => self.notice = None,
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
            crate::tcp::Tcp::FULL,
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
            widget.scene.tcp,
        );
        let r = shape.rect(control).expect("a control with somewhere to be");
        (
            r.x0 + r.width() / 2.0,
            r.y0 + r.height() / 2.0 + crate::ruler::ruler_h(),
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

    fn button(
        mut event: blitz_traits::events::BlitzPointerEvent,
        which: blitz_traits::events::MouseEventButton,
    ) -> blitz_traits::events::BlitzPointerEvent {
        event.button = which;
        event
    }

    /// The middle button is the panel's hand; nothing here acts on it.
    #[test]
    fn a_middle_click_does_nothing_here() {
        use blitz_traits::events::MouseEventButton::Auxiliary;
        let mut widget = widget();
        let (x, y) = at(&widget, 1, C::Mute);
        widget.handle_event(&UiEvent::PointerDown(button(pointer(x, y), Auxiliary)));
        widget.handle_event(&UiEvent::PointerUp(button(pointer(x, y), Auxiliary)));
        assert!(widget.edits.borrow().is_empty(), "{:?}", widget.edits.borrow());
        assert!(!widget.rows[1].0.muted);
    }

    /// A tool owns the mouse: a click under the zoom spring is not a click.
    #[test]
    fn a_click_under_a_tool_is_the_tools() {
        let mut widget = widget();
        widget.pointing.borrow_mut().tool = crate::tool::Tool::Zoom;
        let (x, y) = at(&widget, 1, C::Mute);
        widget.handle_event(&UiEvent::PointerDown(pointer(x, y)));
        widget.handle_event(&UiEvent::PointerUp(pointer(x, y)));
        assert!(widget.edits.borrow().is_empty(), "{:?}", widget.edits.borrow());
        assert_eq!(widget.pointing.borrow().icon(), cursor_icon::CursorIcon::ZoomIn);
    }

    /// A press made before the tool went up still ends: its release comes
    /// through, or the gesture would never finish.
    #[test]
    fn a_gesture_begun_before_a_tool_still_finishes() {
        let mut widget = widget();
        let (x, y) = at(&widget, 1, C::Mute);
        widget.handle_event(&UiEvent::PointerDown(pointer(x, y)));
        widget.pointing.borrow_mut().tool = crate::tool::Tool::Zoom;
        widget.handle_event(&UiEvent::PointerUp(pointer(x, y)));
        let queued = widget.edits.borrow().clone();
        assert!(matches!(queued.as_slice(), [Edit::ToggleMute(_)]), "{queued:?}");
    }

    /// The pointer's shape follows what is under it.
    #[test]
    fn the_pointer_says_what_is_under_it() {
        use crate::tool::Over;
        use cursor_icon::CursorIcon;
        let mut widget = widget();
        let (x, y) = at(&widget, 1, C::Volume);
        widget.handle_event(&UiEvent::PointerMove(pointer(x, y)));
        assert_eq!(widget.pointing.borrow().over, Over::Knob);
        assert_eq!(widget.pointing.borrow().icon(), CursorIcon::NsResize);

        let (x, y) = at(&widget, 1, C::Mute);
        widget.handle_event(&UiEvent::PointerMove(pointer(x, y)));
        assert_eq!(widget.pointing.borrow().over, Over::Button);

        // The empty lanes, with Ctrl held: a drag there is a razor.
        let mut lane = pointer(widget.scene.tcp.width() + 400.0, y);
        lane.mods = blitz_traits::events::Modifiers::CONTROL;
        widget.handle_event(&UiEvent::PointerMove(lane));
        assert!(
            matches!(widget.pointing.borrow().over, Over::Map(_)),
            "{:?}",
            widget.pointing.borrow().over
        );
        assert_eq!(widget.pointing.borrow().icon(), CursorIcon::Crosshair);
    }

    /// `z` then `t`, with a track selected: the popup shows, then goes,
    /// and the panel is asked to frame that track's row.
    #[test]
    fn z_t_asks_the_panel_to_frame_the_selected_track() {
        let mut widget = widget();
        widget.rows[2].0.selected = true;
        widget.handle_event(&UiEvent::KeyDown(key(blitz_traits::events::Key::Character("z".into()))));
        assert!(widget.which.borrow().is_some(), "the popup is up");
        let mut up = key(blitz_traits::events::Key::Character("z".into()));
        up.state = blitz_traits::events::KeyState::Released;
        widget.handle_event(&UiEvent::KeyUp(up));
        widget.handle_event(&UiEvent::KeyDown(key(blitz_traits::events::Key::Character("t".into()))));
        assert!(widget.which.borrow().is_none(), "and down again");
        let (top, h) = widget.scene.row_box(2).unwrap();
        let asked = widget.zooms.borrow().clone();
        assert_eq!(
            asked,
            vec![crate::zoom::Request::Frame {
                time: None,
                rows: Some((top, top + h)),
                toggle: true,
            }]
        );
    }

    /// A held `z` used as the zoom tool closes the tree on release.
    #[test]
    fn z_used_as_the_tool_closes_the_tree_on_release() {
        let mut widget = widget();
        widget.handle_event(&UiEvent::KeyDown(key(blitz_traits::events::Key::Character("z".into()))));
        widget.pointing.borrow_mut().tool_used = true;
        let mut up = key(blitz_traits::events::Key::Character("z".into()));
        up.state = blitz_traits::events::KeyState::Released;
        widget.handle_event(&UiEvent::KeyUp(up));
        assert!(widget.which.borrow().is_none());
        assert!(!widget.keys.is_pending());
    }

    /// The visibility manager: toggling a group hides its tracks' rows
    /// and tells the engine; toggling again brings them back.
    #[test]
    fn a_visibility_toggle_hides_the_group_and_shows_it_again() {
        use crate::keys::Visibility;
        let names = ["Kick", "Snare", "Bass", "Piano"];
        let tracks: Vec<daw_proto::Track> = names
            .iter()
            .enumerate()
            .map(|(i, name)| daw_proto::Track {
                guid: format!("g{i}"),
                name: (*name).to_owned(),
                index: u32::try_from(i).unwrap(),
                ..daw_proto::Track::default()
            })
            .collect();
        let raw = daw_ui::studio::project::Project {
            tracks: tracks.clone(),
            ..daw_ui::studio::project::Project::default()
        };
        let planner = crate::studio::Planner {
            raw: std::sync::Arc::new(raw),
            scene: None,
            kinds: std::sync::Arc::new(crate::plan::Kinds::default()),
        };
        let content_h = std::rc::Rc::new(std::cell::Cell::new(0.0));
        let mut widget = widget().with_planner(planner.clone(), std::rc::Rc::clone(&content_h));
        let (project, rows) = planner.plan(&planner.raw);
        widget.restructure((*project.0).clone(), rows.as_slice().to_vec());
        let before = content_h.get();
        let names_shown = |w: &ArrangementWidget| {
            w.rows.iter().map(|(t, _)| t.name.clone()).collect::<Vec<_>>()
        };
        assert_eq!(names_shown(&widget), names);

        assert!(widget.visibility(&Visibility::Toggle("DRUMS".into())));
        let shown = names_shown(&widget);
        assert!(!shown.contains(&"Kick".to_owned()) && !shown.contains(&"Snare".to_owned()), "{shown:?}");
        assert!(shown.contains(&"Bass".to_owned()), "{shown:?}");
        assert!(content_h.get() < before, "the scroll range shrinks");
        let hidden = widget
            .edits
            .borrow()
            .iter()
            .filter(|e| matches!(e, Edit::SetVisibility(_, false, false)))
            .count();
        assert_eq!(hidden, 2, "kick and snare, told to the engine");

        assert!(widget.visibility(&Visibility::Toggle("DRUMS".into())));
        assert_eq!(names_shown(&widget), names, "and back");

        assert!(widget.visibility(&Visibility::HideAll));
        assert!(widget.visibility(&Visibility::ShowAll));
        assert_eq!(names_shown(&widget), names);
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

    /// A bound key acts on the session, with no rename field open.
    ///
    /// The window used to drop every key that was not going into a
    /// rename: `typed` returned false the moment `renaming` was `None`,
    /// so Delete, Home and every other binding reached the widget and
    /// died there. It looked exactly like a window that was not
    /// receiving keyboard events at all, which is what made it take a
    /// day to find — see #124.
    #[test]
    fn a_bound_key_reaches_the_session() {
        let mut widget = widget();
        // Delete, which the built-in profile binds to 40697. With
        // nothing selected it has nothing to remove and still answers
        // for itself, which is the whole point: the question here is
        // whether the key ARRIVES, not what it does when it does.
        let acted = widget.took(&UiEvent::KeyDown(key(blitz_traits::events::Key::Delete)));
        assert!(acted, "a bound key did nothing at all");
    }

    /// And a key with nothing behind it still does nothing, so the
    /// window does not spend a frame on every keystroke.
    #[test]
    fn an_unbound_key_costs_nothing() {
        let mut widget = widget();
        let acted = widget.took(&UiEvent::KeyDown(key(
            blitz_traits::events::Key::Character("§".into()),
        )));
        assert!(!acted, "an unbound key asked for a frame");
    }

    /// While a rename IS open the keyboard belongs to it: `d` types a
    /// d rather than deleting what is selected.
    #[test]
    fn a_rename_keeps_the_keyboard() {
        let mut widget = widget();
        let (x, y) = at(&widget, 1, C::Name);
        widget.handle_event(&UiEvent::PointerDown(pointer(x, y)));
        widget.handle_event(&UiEvent::PointerUp(pointer(x, y)));
        widget.handle_event(&UiEvent::PointerDown(pointer(x, y)));
        widget.handle_event(&UiEvent::PointerUp(pointer(x, y)));
        assert!(
            widget.renaming.is_some(),
            "the double click did not open one"
        );
        widget.handle_event(&UiEvent::KeyDown(key(
            blitz_traits::events::Key::Character("d".into()),
        )));
        let typed = widget
            .renaming
            .as_ref()
            .map(|r| r.text().to_owned())
            .unwrap_or_default();
        assert!(
            typed.ends_with('d'),
            "the letter went to the session instead of the field: {typed:?}"
        );
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

    /// The lanes start where the panel ends, in either shape: a point
    /// just past the compact panel is lane, not track, and reads the
    /// same time a point the same distance past the full one does.
    #[test]
    fn the_lanes_start_where_the_compact_panel_ends() {
        use crate::hit::Target;
        let mut widget = widget();
        let (_, y) = at(&widget, 0, C::Name);
        let lane_at = |widget: &ArrangementWidget, past: f64| {
            match widget.hit(widget.scene.tcp.width() + past, y).expect("a hit").target {
                Target::Lane { row, seconds } => (row, seconds),
                other => panic!("{past} px past the panel is {other:?}"),
            }
        };
        let full = lane_at(&widget, 50.0);

        widget.compact.set(true);
        widget.recut();
        assert!(widget.scene.tcp.compact, "the toggle re-cuts the panel");
        assert!(widget.scene.tcp.width() < crate::arrangement::TCP_WIDTH);
        let compact = lane_at(&widget, 50.0);
        assert_eq!(full.0, compact.0);
        assert!((full.1 - compact.1).abs() < 1e-9, "{full:?} vs {compact:?}");
        // The width the panel gave up is lane now, not panel.
        let _ = lane_at(&widget, 1.0);
    }

    #[test]
    fn a_click_outside_the_panel_asks_for_nothing() {
        let mut widget = widget();
        let past = widget.scene.tcp.width() + 200.0;
        widget.handle_event(&UiEvent::PointerDown(pointer(past, 300.0)));
        widget.handle_event(&UiEvent::PointerUp(pointer(past, 300.0)));
        assert!(widget.edits.borrow().is_empty());
        assert_eq!(widget.pointer.hovered(), None);
    }
}
