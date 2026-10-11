//! Session's Action List: everything the window can do, in the one
//! registry its keys, its menus and a model talking MCP all run.
//!
//! [`list`] is the list (`architect::action::ActionList`), filled on first
//! use. Two kinds of action are in it:
//!
//! - **The window's** — transport, selection, the edit cursor, zooms,
//!   panels, comping, gain staging: what a key does. Running one queues
//!   it ([`take_window`]); the window applies the queue on its next pass,
//!   through the same code a key press always went through. A key press
//!   itself runs here first ([`through_list`]), so a key and a model are
//!   one path.
//! - **The session's data** — its tracks, a drum's hits, a drum's
//!   fundamental, a track's fader: run against the engine directly, typed
//!   in and out, so a model can read the song and change it.
//!
//! Every engine edit the window applies is reported as a change
//! (`crate::engine`), so a model's `watch` follows what happens in the
//! app as it happens.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use architect::action::{ActionList, ActionMeta, Effect};
use daw::service::{ProjectContext, TempoMap as _, TrackRef, Tracks};
use facet::Facet;

use crate::keys::Action;

static LIST: OnceLock<ActionList> = OnceLock::new();

/// The window's actions waiting for it.
static WINDOW: Mutex<VecDeque<Action>> = Mutex::new(VecDeque::new());

/// The Action List, every action registered.
pub fn list() -> &'static ActionList {
    LIST.get_or_init(|| {
        let list = ActionList::new();
        register_window(&list);
        register_data(&list);
        list
    })
}

/// The window's actions run since it last looked, in order.
pub fn take_window() -> Vec<Action> {
    let taken: Vec<Action> = WINDOW
        .lock()
        .map(|mut q| q.drain(..).collect())
        .unwrap_or_default();
    if !taken.is_empty() {
        FOLLOW_UP.store(true, std::sync::atomic::Ordering::Release);
    }
    taken
}

/// Whether any are waiting.
pub fn window_pending() -> bool {
    WINDOW.lock().is_ok_and(|q| !q.is_empty())
}

/// Whether the window owes a frame for actions it just carried out —
/// taken, so one frame is asked for each time. For the shell's frame
/// loop alone.
///
/// The window sends its edits to the engine at the start of a frame and
/// carries out these actions while painting one, so what they edited
/// goes on the frame AFTER. No event is coming to ask for it.
pub fn frame_owed() -> bool {
    FOLLOW_UP.swap(false, std::sync::atomic::Ordering::AcqRel)
}

/// A frame owed for actions just carried out.
static FOLLOW_UP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn queue(action: Action) {
    if let Ok(mut q) = WINDOW.lock() {
        q.push_back(action);
    }
}

/// A key's actions run through the list, and what the window is to do
/// about them.
///
/// A key is one more caller of the same actions a model runs. An action
/// the list does not have (a binding the window cannot do yet) goes
/// straight through, to be logged as it always was.
#[must_use]
pub fn through_list(actions: Vec<Action>) -> Vec<Action> {
    let list = list();
    for action in actions {
        match id_and_args(&action) {
            Some((id, args)) if behind_the_window(id) => off_the_window(id, args),
            Some((id, args)) => {
                if let Err(e) = list.run_json(id, args.as_bytes()) {
                    tracing::warn!(action.id = id, error = %e, "key action refused");
                }
            }
            None => queue(action),
        }
    }
    take_window()
}

/// An action's id in the list and its arguments, as a key produced it.
fn id_and_args(action: &Action) -> Option<(&'static str, String)> {
    use crate::zoom::Command as Z;
    let by = |by: i32| format!(r#"{{"by":{by}}}"#);
    Some(match action {
        Action::PlayStop => (PLAY_STOP.id, String::new()),
        Action::PlayPause => (PLAY_PAUSE.id, String::new()),
        Action::GoToStart => (GO_TO_START.id, String::new()),
        Action::ToggleRecord => (TOGGLE_RECORD.id, String::new()),
        Action::SplitAtCursor => (SPLIT.id, String::new()),
        Action::QuantizeDrums => (QUANTIZE_DRUMS.id, String::new()),
        Action::GainStage { clear: false } => (GAIN_STAGE.id, String::new()),
        Action::GainStage { clear: true } => (GAIN_STAGE_CLEAR.id, String::new()),
        Action::CompTake(take) => (COMP_TAKE.id, format!(r#"{{"take":{take}}}"#)),
        Action::DeleteSelectedItems => (DELETE_ITEMS.id, String::new()),
        Action::SelectAllItems => (SELECT_ALL.id, String::new()),
        Action::ClearSelection => (CLEAR_SELECTION.id, String::new()),
        Action::CursorBar(n) => (CURSOR_BAR.id, by(*n)),
        Action::CursorBeat(n) => (CURSOR_BEAT.id, by(*n)),
        Action::Marker(n) => (MARKER.id, by(*n)),
        Action::TrackStep { by, extend } => {
            (TRACK_STEP.id, format!(r#"{{"by":{by},"extend":{extend}}}"#))
        }
        Action::ToggleMixer => (TOGGLE_MIXER.id, String::new()),
        #[cfg(feature = "native")]
        Action::Save => (SAVE.id, String::new()),
        #[cfg(not(feature = "native"))]
        Action::Save => return None,
        #[cfg(feature = "native")]
        Action::Undo => (UNDO.id, String::new()),
        #[cfg(feature = "native")]
        Action::Redo => (REDO.id, String::new()),
        #[cfg(not(feature = "native"))]
        Action::Undo | Action::Redo => return None,
        Action::ToggleEditor => (TOGGLE_EDITOR.id, String::new()),
        Action::ToggleFx => (TOGGLE_FX.id, String::new()),
        Action::FxFull => (FX_FULL.id, String::new()),
        Action::ToggleScene(slug) => (SCENE.id, format!(r#"{{"scene":"{slug}"}}"#)),
        Action::Visibility(crate::keys::Visibility::Toggle(group)) => (
            SHOW_GROUP.id,
            format!(r#"{{"group":{}}}"#, serde_json::Value::from(group.as_str())),
        ),
        Action::Visibility(crate::keys::Visibility::ShowAll) => (SHOW_ALL.id, String::new()),
        Action::Visibility(crate::keys::Visibility::HideAll) => (HIDE_ALL.id, String::new()),
        Action::View(command) => {
            let zoom = match command {
                Z::ToggleTracks => "ToggleTracks",
                Z::FitTracks => "FitTracks",
                Z::Project => "Project",
                Z::Selection => "Selection",
                Z::ToggleSelection => "ToggleSelection",
                Z::Items => "Items",
                Z::Back => "Back",
                Z::Forward => "Forward",
                Z::Step {
                    vertical: false,
                    inward: true,
                } => "InHorizontal",
                Z::Step {
                    vertical: false,
                    inward: false,
                } => "OutHorizontal",
                Z::Step {
                    vertical: true,
                    inward: true,
                } => "InVertical",
                Z::Step {
                    vertical: true,
                    inward: false,
                } => "OutVertical",
            };
            (ZOOM.id, format!(r#"{{"zoom":"{zoom}"}}"#))
        }
        // Already the list's own answer (`DRUMS_SLIP`), or a binding the
        // window does not do: straight to the window.
        Action::Unbound(_)
        | Action::SlipDrums { .. }
        | Action::Frame { .. }
        | Action::RemoveRange { .. } => {
            return None;
        }
    })
}

/// Whether a key's action runs behind the window rather than on its
/// thread: one that reads the whole song or writes its file (an undo, a
/// save) would hold the window still while it did.
#[cfg(feature = "native")]
fn behind_the_window(id: &str) -> bool {
    [UNDO.id, REDO.id, SAVE.id].contains(&id)
}

#[cfg(not(feature = "native"))]
const fn behind_the_window(_id: &str) -> bool {
    false
}

/// Run a key's action on the one thread that runs them behind the window,
/// in the order they were pressed; the window redraws when each is done,
/// as it does for a model's.
fn off_the_window(id: &'static str, args: String) {
    static WORKER: std::sync::OnceLock<
        std::sync::Mutex<std::sync::mpsc::Sender<(&'static str, String)>>,
    > = std::sync::OnceLock::new();
    let worker = WORKER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<(&'static str, String)>();
        let _ = std::thread::Builder::new()
            .name("key-actions".into())
            .spawn(move || {
                for (id, args) in rx {
                    if let Err(e) = list().run_json(id, args.as_bytes()) {
                        tracing::warn!(action.id = id, error = %e, "key action refused");
                    }
                }
            });
        std::sync::Mutex::new(tx)
    });
    if let Ok(tx) = worker.lock() {
        let _ = tx.send((id, args));
    }
}

/// An action's description, as the list and its menus show it.
pub(crate) const fn meta(
    id: &'static str,
    method_name: &'static str,
    display_name: &'static str,
    description: &'static str,
    group: &'static str,
    shortcut: &'static str,
) -> ActionMeta {
    ActionMeta {
        id,
        trait_name: "Session",
        method_name,
        display_name,
        description,
        category: "Session",
        group,
        shortcut,
        toggleable: false,
        undo: false,
    }
}

static PLAY_STOP: ActionMeta = meta(
    "SESSION_PLAY_STOP",
    "play_stop",
    "Play/stop",
    "Start playback, or stop it.",
    "Transport",
    "Space",
);
static PLAY_PAUSE: ActionMeta = meta(
    "SESSION_PLAY_PAUSE",
    "play_pause",
    "Play/pause",
    "Start playback, or pause it where it is.",
    "Transport",
    "",
);
static GO_TO_START: ActionMeta = meta(
    "SESSION_GO_TO_START",
    "go_to_start",
    "Go to start",
    "The edit cursor to the start of the song.",
    "Transport",
    "Home",
);
static TOGGLE_RECORD: ActionMeta = meta(
    "SESSION_TOGGLE_RECORD",
    "toggle_record",
    "Record",
    "Start recording the armed tracks, or stop.",
    "Transport",
    "",
);
static SPLIT: ActionMeta = meta(
    "SESSION_SPLIT",
    "split",
    "Split at cursor",
    "Split the selected items at the edit cursor — or, with nothing selected, every item under it.",
    "Edit",
    "S",
);
static QUANTIZE_DRUMS: ActionMeta = meta(
    "SESSION_QUANTIZE_DRUMS",
    "quantize_drums",
    "Quantize drums",
    "Quantize the kit under the selected folded rows: every mic cut before each hit, the slices moved onto the grid together.",
    "Drums",
    "",
);
static GAIN_STAGE: ActionMeta = meta(
    "SESSION_GAIN_STAGE",
    "gain_stage",
    "Gain stage",
    "Set every recorded track's item volume to the reference kit's levels — non-destructive, the audio untouched.",
    "Mix",
    "Shift+G",
);
static GAIN_STAGE_CLEAR: ActionMeta = meta(
    "SESSION_GAIN_STAGE_CLEAR",
    "gain_stage_clear",
    "Clear gain staging",
    "Put every staged item back at an item volume of 1.",
    "Mix",
    "Alt+G",
);
static COMP_TAKE: ActionMeta = meta(
    "SESSION_COMP_TAKE",
    "comp_take",
    "Comp from take",
    "Comp the razor areas from take N — on a folded row, every mic in the folder at once.",
    "Edit",
    "Ctrl+N",
);
static DELETE_ITEMS: ActionMeta = meta(
    "SESSION_DELETE_ITEMS",
    "delete_items",
    "Delete selected items",
    "Delete the selected items.",
    "Edit",
    "Delete",
);
static SELECT_ALL: ActionMeta = meta(
    "SESSION_SELECT_ALL",
    "select_all",
    "Select all items",
    "Select every item.",
    "Edit",
    "Ctrl+A",
);
static CLEAR_SELECTION: ActionMeta = meta(
    "SESSION_CLEAR_SELECTION",
    "clear_selection",
    "Clear selection",
    "No time selection, nothing selected.",
    "Edit",
    "Escape",
);
static CURSOR_BAR: ActionMeta = meta(
    "SESSION_CURSOR_BAR",
    "cursor_bar",
    "Cursor by bars",
    "Move the edit cursor by bars (negative: back).",
    "Navigate",
    "",
);
static CURSOR_BEAT: ActionMeta = meta(
    "SESSION_CURSOR_BEAT",
    "cursor_beat",
    "Cursor by beats",
    "Move the edit cursor by beats (negative: back).",
    "Navigate",
    "",
);
static MARKER: ActionMeta = meta(
    "SESSION_MARKER",
    "marker",
    "Cursor by markers",
    "Move the edit cursor to the previous (negative) or next marker.",
    "Navigate",
    "",
);
static TRACK_STEP: ActionMeta = meta(
    "SESSION_TRACK_STEP",
    "track_step",
    "Step track selection",
    "Move the track selection up (negative) or down the panel; `extend` keeps the rest selected.",
    "Navigate",
    "",
);
static TOGGLE_MIXER: ActionMeta = meta(
    "SESSION_TOGGLE_MIXER",
    "toggle_mixer",
    "Mixer",
    "Show or hide the docked mixer.",
    "View",
    "",
);
static TOGGLE_EDITOR: ActionMeta = meta(
    "SESSION_TOGGLE_EDITOR",
    "toggle_editor",
    "Editor",
    "Show or hide the Editor under the arrangement.",
    "View",
    "E",
);
static TOGGLE_FX: ActionMeta = meta(
    "SESSION_TOGGLE_FX",
    "toggle_fx",
    "FX",
    "Show or hide the selected track's effects.",
    "View",
    "F",
);
static FX_FULL: ActionMeta = meta(
    "SESSION_FX_FULL",
    "fx_full",
    "FX, full size",
    "The FX view, full size: several tracks' chains.",
    "View",
    "Shift+F",
);
static SCENE: ActionMeta = meta(
    "SESSION_SCENE",
    "scene",
    "Scene",
    "Show the session as a scene (`overview`, `overview-top`), or, in it already, go back.",
    "View",
    "",
);
static SHOW_GROUP: ActionMeta = meta(
    "SESSION_SHOW_GROUP",
    "show_group",
    "Show/hide group",
    "Show a group of tracks if all of it is hidden, else hide it (`DRUMS`, matched loosely).",
    "View",
    "",
);
static SHOW_ALL: ActionMeta = meta(
    "SESSION_SHOW_ALL",
    "show_all",
    "Show all tracks",
    "Show every track.",
    "View",
    "",
);
static HIDE_ALL: ActionMeta = meta(
    "SESSION_HIDE_ALL",
    "hide_all",
    "Hide all tracks",
    "Hide every track.",
    "View",
    "",
);
static ZOOM: ActionMeta = meta(
    "SESSION_ZOOM",
    "zoom",
    "Zoom",
    "Zoom the arrangement: the selected tracks, every track, the whole song, the selection, the items, back/forward, or a step in or out.",
    "View",
    "",
);

/// By how many.
#[derive(Facet)]
struct By {
    /// Negative: back, up.
    by: i32,
}

/// A track selection step.
#[derive(Facet)]
struct Step {
    by: i32,
    /// Keep the tracks already selected.
    extend: Option<bool>,
}

/// Which take.
#[derive(Facet)]
struct Take {
    /// 1-based, as Ctrl+1 … Ctrl+9.
    take: u32,
}

/// Which scene.
#[derive(Facet)]
struct SceneArg {
    scene: String,
}

/// Which group of tracks.
#[derive(Facet)]
struct GroupArg {
    group: String,
}

/// A zoom.
#[derive(Facet)]
struct ZoomArg {
    zoom: Zoom,
}

#[derive(Facet, Clone, Copy)]
#[repr(u8)]
enum Zoom {
    ToggleTracks,
    FitTracks,
    Project,
    Selection,
    ToggleSelection,
    Items,
    Back,
    Forward,
    InHorizontal,
    OutHorizontal,
    InVertical,
    OutVertical,
}

/// The scenes a scene action can name: the window's `&'static` slugs.
const SCENES: &[&str] = &["overview", "overview-top"];

fn register_window(list: &ActionList) {
    let plain: [(&'static ActionMeta, Effect, Action); 16] = [
        (&PLAY_STOP, Effect::Mutating, Action::PlayStop),
        (&PLAY_PAUSE, Effect::Mutating, Action::PlayPause),
        (&GO_TO_START, Effect::Mutating, Action::GoToStart),
        (&TOGGLE_RECORD, Effect::Destructive, Action::ToggleRecord),
        (&SPLIT, Effect::Mutating, Action::SplitAtCursor),
        (&QUANTIZE_DRUMS, Effect::Mutating, Action::QuantizeDrums),
        (
            &GAIN_STAGE,
            Effect::Mutating,
            Action::GainStage { clear: false },
        ),
        (
            &GAIN_STAGE_CLEAR,
            Effect::Mutating,
            Action::GainStage { clear: true },
        ),
        (
            &DELETE_ITEMS,
            Effect::Destructive,
            Action::DeleteSelectedItems,
        ),
        (&SELECT_ALL, Effect::Mutating, Action::SelectAllItems),
        (&CLEAR_SELECTION, Effect::Mutating, Action::ClearSelection),
        (&TOGGLE_MIXER, Effect::Mutating, Action::ToggleMixer),
        (&TOGGLE_EDITOR, Effect::Mutating, Action::ToggleEditor),
        (&TOGGLE_FX, Effect::Mutating, Action::ToggleFx),
        (&FX_FULL, Effect::Mutating, Action::FxFull),
        (
            &SHOW_ALL,
            Effect::Mutating,
            Action::Visibility(crate::keys::Visibility::ShowAll),
        ),
    ];
    for (meta, effect, action) in plain {
        list.register(meta, effect, move |(): ()| {
            queue(action.clone());
            Ok(())
        });
    }
    list.register(&HIDE_ALL, Effect::Mutating, |(): ()| {
        queue(Action::Visibility(crate::keys::Visibility::HideAll));
        Ok(())
    });
    list.register(&COMP_TAKE, Effect::Mutating, |t: Take| {
        queue(Action::CompTake(t.take));
        Ok(())
    });
    list.register(&CURSOR_BAR, Effect::Mutating, |b: By| {
        queue(Action::CursorBar(b.by));
        Ok(())
    });
    list.register(&CURSOR_BEAT, Effect::Mutating, |b: By| {
        queue(Action::CursorBeat(b.by));
        Ok(())
    });
    list.register(&MARKER, Effect::Mutating, |b: By| {
        queue(Action::Marker(b.by));
        Ok(())
    });
    list.register(&TRACK_STEP, Effect::Mutating, |s: Step| {
        queue(Action::TrackStep {
            by: s.by,
            extend: s.extend.unwrap_or(false),
        });
        Ok(())
    });
    list.register(&SCENE, Effect::Mutating, |s: SceneArg| {
        let slug = SCENES
            .iter()
            .find(|known| known.eq_ignore_ascii_case(&s.scene))
            .ok_or_else(|| format!("no scene {}: one of {SCENES:?}", s.scene))?;
        queue(Action::ToggleScene(slug));
        Ok(())
    });
    list.register(&SHOW_GROUP, Effect::Mutating, |g: GroupArg| {
        queue(Action::Visibility(crate::keys::Visibility::Toggle(g.group)));
        Ok(())
    });
    list.register(&ZOOM, Effect::Mutating, |z: ZoomArg| {
        use crate::zoom::Command as Z;
        let step = |vertical, inward| Z::Step { vertical, inward };
        queue(Action::View(match z.zoom {
            Zoom::ToggleTracks => Z::ToggleTracks,
            Zoom::FitTracks => Z::FitTracks,
            Zoom::Project => Z::Project,
            Zoom::Selection => Z::Selection,
            Zoom::ToggleSelection => Z::ToggleSelection,
            Zoom::Items => Z::Items,
            Zoom::Back => Z::Back,
            Zoom::Forward => Z::Forward,
            Zoom::InHorizontal => step(false, true),
            Zoom::OutHorizontal => step(false, false),
            Zoom::InVertical => step(true, true),
            Zoom::OutVertical => step(true, false),
        }));
        Ok(())
    });
}

static TRACKS: ActionMeta = ActionMeta {
    category: "Session",
    ..meta(
        "SESSION_TRACKS",
        "tracks",
        "Tracks",
        "Every track in the open song: GUID, name, its folder path, folder or not, mute, solo, fader (dB), pan, how many items, its FX. `filter` keeps the tracks whose path contains it.",
        "Data",
        "",
    )
};
static DRUM_HITS: ActionMeta = meta(
    "DRUMS_HITS",
    "drum_hits",
    "Drum hits",
    "Every hit on a track — a drum's trigger, ideally, one spike a hit — with its time (seconds), bar and beat, and peak (dBFS). `under_db` is how far under the loudest a hit still counts (24: ghost notes too). Paged: `offset`, `limit`; `from`/`to` (seconds) narrow it.",
    "Drums",
    "",
);
static DRUM_FUNDAMENTAL: ActionMeta = meta(
    "DRUMS_FUNDAMENTAL",
    "drum_fundamental",
    "Drum fundamental",
    "A drum's fundamental (Hz), read off its mics over the ring after each of its trigger's hits — what its Fund/Sub band-pass is tuned to. `range`: kick, snare or tom.",
    "Drums",
    "",
);
static TRACK_SET: ActionMeta = meta(
    "TRACK_SET",
    "track_set",
    "Set track",
    "Set a track's fader (dB), pan (-1 left … 1 right), mute or solo — any of them.",
    "Mix",
    "",
);

/// Which tracks.
#[derive(Facet)]
struct TracksArg {
    /// Keep the tracks whose folder path contains this (case aside).
    filter: Option<String>,
}

/// A track, as the list describes it.
#[derive(Facet)]
struct TrackRow {
    guid: String,
    name: String,
    /// The folders it is in and its name: `Drums/Drum Kit/Kick/Sum/In`.
    path: String,
    folder: bool,
    muted: bool,
    soloed: bool,
    volume_db: f64,
    pan: f64,
    items: u32,
    fx: Vec<String>,
    /// Its fixed lanes, when it has them, named lanes and the playing
    /// one first — then the rest only by count.
    lanes: Vec<LaneRow>,
}

/// One fixed lane.
#[derive(Facet)]
struct LaneRow {
    lane: u32,
    name: String,
    items: u32,
    plays: bool,
}

/// `track`'s lanes worth listing: those named something other than their
/// number, and those playing.
fn lane_rows(track: &daw_proto::Track, items: &[daw_proto::Item]) -> Vec<LaneRow> {
    (0..track.lane_count)
        .filter_map(|lane| {
            let name = usize::try_from(lane)
                .ok()
                .and_then(|i| track.lane_names.get(i))
                .cloned()
                .unwrap_or_default();
            let plays = lane < 64 && track.lane_play_mask & (1_u64 << lane) != 0;
            let numbered = name.trim().parse::<u32>().is_ok() || name.trim().is_empty();
            (plays || !numbered).then(|| LaneRow {
                lane,
                items: u32::try_from(
                    items
                        .iter()
                        .filter(|i| i.fixed_lane.unwrap_or(0) == lane)
                        .count(),
                )
                .unwrap_or(u32::MAX),
                name,
                plays,
            })
        })
        .collect()
}

/// Which hits.
#[derive(Facet)]
struct HitsArg {
    /// The track: its GUID, its path (`Kick/Sum/Trig`) or its name.
    track: String,
    under_db: Option<f64>,
    from: Option<f64>,
    to: Option<f64>,
    offset: Option<u32>,
    limit: Option<u32>,
}

/// One hit.
#[derive(Facet)]
struct HitRow {
    time: f64,
    /// 1-based, as the ruler counts.
    bar: i32,
    /// 1-based, fractional.
    beat: f64,
    level_db: f64,
}

/// A page of hits.
#[derive(Facet)]
struct HitsPage {
    /// How many there are in all (in `from`..`to`).
    total: u32,
    offset: u32,
    hits: Vec<HitRow>,
}

/// Which drum.
#[derive(Facet)]
struct FundamentalArg {
    /// Its trigger: GUID, path or name.
    trigger: String,
    /// Its mics: GUIDs, paths or names.
    mics: Vec<String>,
    range: DrumRange,
}

#[derive(Facet, Clone, Copy)]
#[repr(u8)]
enum DrumRange {
    Kick,
    Snare,
    Tom,
}

/// What to set on a track.
#[derive(Facet)]
struct TrackSetArg {
    track: String,
    volume_db: Option<f64>,
    pan: Option<f64>,
    mute: Option<bool>,
    solo: Option<bool>,
}

/// How many hits a page holds unless asked.
const PAGE: u32 = 200;

static MODE: ActionMeta = meta(
    "SESSION_MODE",
    "mode",
    "Mode",
    "Put the window in a mode: Organize, Write, Produce, Record, Edit, Mix, Master, Live, Video or Scoring — what the panels show and how (Edit shows every cut on a folder's row; Live and Mix a folder's row whole).",
    "View",
    "",
);

/// Which mode.
#[derive(Facet)]
struct ModeArg {
    mode: String,
}

/// The mode a caller asked for, until the window takes it
/// ([`take_mode_request`]).
static MODE_REQUEST: Mutex<Option<session::modes::Mode>> = Mutex::new(None);

/// The mode asked for since the window last looked, if any.
pub fn take_mode_request() -> Option<session::modes::Mode> {
    MODE_REQUEST.lock().ok()?.take()
}

static VIEW: ActionMeta = meta(
    "SESSION_VIEW",
    "view",
    "View",
    "Fill the arrangement with part of the song and/or some tracks: `from`/`to` (seconds), or `bars` ([first, last], inclusive), or `section` (a section's name, `nth` for the 2nd of its name — SESSION_SECTIONS lists them); and `tracks` (GUIDs, paths or path tails — a track in a shut folder frames the folder's row). Either axis left out stays as it is. For the window's own zooms (fit every track, the whole song, back) see SESSION_ZOOM.",
    "View",
    "",
);
static SECTIONS: ActionMeta = meta(
    "SESSION_SECTIONS",
    "sections",
    "Sections",
    "The song's sections (its regions), in order: name, start and end seconds, and the bars they span — what SESSION_VIEW's `section` names.",
    "View",
    "",
);

/// What to look at.
#[derive(Facet)]
struct ViewArg {
    from: Option<f64>,
    to: Option<f64>,
    bars: Option<Vec<i32>>,
    section: Option<String>,
    /// Which of the sections of that name, from 1 (1).
    nth: Option<u32>,
    tracks: Option<Vec<String>>,
}

/// One section.
#[derive(Facet)]
pub(crate) struct SectionOut {
    pub(crate) name: String,
    pub(crate) start: f64,
    pub(crate) end: f64,
    first_bar: i32,
    last_bar: i32,
}

/// The song's sections, in order.
pub(crate) fn sections(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
) -> Vec<SectionOut> {
    let mut all: Vec<SectionOut> =
        <daw::standalone::Standalone as daw::service::region::Regions>::all(daw, project.clone())
            .into_iter()
            // The sections lane's (a region on no lane is on it): not the
            // song's own region, nor an edit zone on the Drums lane.
            .filter(|r| {
                r.lane
                    .is_none_or(|l| l == session::ruler_lanes::CoreLane::Sections.lane_index())
            })
            .map(|r| {
                let (start, end) = (r.time_range.start_seconds(), r.time_range.end_seconds());
                SectionOut {
                    name: r.name,
                    start,
                    end,
                    first_bar: crate::song_bars::musical(daw, &project, start + 1e-6).0,
                    last_bar: crate::song_bars::musical(daw, &project, (end - 1e-6).max(start)).0,
                }
            })
            .collect();
    all.sort_by(|a, b| a.start.total_cmp(&b.start));
    all
}

/// The span `a` asks for, if it asks for one.
fn view_span(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    a: &ViewArg,
) -> Result<Option<(f64, f64)>, String> {
    if let Some(name) = &a.section {
        let all = sections(daw, project);
        let named: Vec<&SectionOut> = all
            .iter()
            .filter(|s| s.name.trim().eq_ignore_ascii_case(name.trim()))
            .collect();
        let nth = usize::try_from(a.nth.unwrap_or(1).max(1)).unwrap_or(1);
        return named
            .get(nth.saturating_sub(1))
            .map(|s| Some((s.start, s.end)))
            .ok_or_else(|| {
                let mut known: Vec<&str> = all.iter().map(|s| s.name.as_str()).collect();
                known.dedup();
                format!("no section {name} (#{nth}): the song has {known:?}")
            });
    }
    if let Some(bars) = &a.bars {
        let (Some(&first), Some(&last)) = (bars.first(), bars.last()) else {
            return Err("`bars` is [first, last]".to_owned());
        };
        // Bars as the song counts them; the bar after the last found in
        // the engine's, where there is no bar 0 to step over.
        let from = crate::song_bars::time(daw, &project, first.min(last), 1, 0.0);
        let after = crate::song_bars::measure(daw, &project, first.max(last)).saturating_add(1);
        let to = daw.musical_to_time(project.clone(), after, 1, 0.0);
        return Ok(Some((from, to)));
    }
    match (a.from, a.to) {
        (Some(from), Some(to)) if to > from => Ok(Some((from.max(0.0), to))),
        (Some(_), Some(_)) => Err("`to` must be after `from`".to_owned()),
        (None, None) => Ok(None),
        _ => Err("a span is `from` and `to` together".to_owned()),
    }
}

fn register_view(list: &ActionList) {
    list.register(&VIEW, Effect::Mutating, |a: ViewArg| {
        let (daw, project) = song()?;
        let span = view_span(daw, &project, &a)?;
        let tracks = a
            .tracks
            .iter()
            .flatten()
            .map(|t| resolve(daw, &project, t))
            .collect::<Result<Vec<String>, String>>()?;
        if span.is_none() && tracks.is_empty() {
            return Err(
                "nothing to view: give a span (from/to, bars, section) or tracks".to_owned(),
            );
        }
        let us = |s: f64| num_traits::ToPrimitive::to_i64(&(s * 1_000_000.0).round()).unwrap_or(0);
        queue(Action::Frame {
            time: span.map(|(from, to)| (us(from), us(to))),
            tracks,
        });
        Ok(())
    });
    list.register(&SECTIONS, Effect::ReadOnly, |(): ()| {
        let (daw, project) = song()?;
        Ok(sections(daw, &project))
    });
}

static PHASE: ActionMeta = meta(
    "SESSION_PHASE",
    "phase",
    "Phase",
    "Put a mode in one of its phases — Mix: rescue, balance, tone, polish, relational, depth, creative, overview; Organize: import, tempo-map. `mode` defaults to Edit.",
    "View",
    "",
);

/// Which phase.
#[derive(Facet)]
struct PhaseArg {
    phase: String,
    mode: Option<String>,
}

fn register_phase(list: &ActionList) {
    list.register(&PHASE, Effect::Mutating, |a: PhaseArg| {
        let name = a.mode.as_deref().unwrap_or("Edit");
        let mode = session::modes::Mode::ALL
            .into_iter()
            .find(|m| m.display_name().eq_ignore_ascii_case(name.trim()))
            .ok_or_else(|| format!("no mode {name}"))?;
        let phases = session::mode_phases::of(mode);
        let phase = phases
            .iter()
            .find(|p| {
                p.slug.eq_ignore_ascii_case(a.phase.trim())
                    || p.name.eq_ignore_ascii_case(a.phase.trim())
            })
            .ok_or_else(|| {
                format!(
                    "{} has no phase {}: one of {:?}",
                    mode.display_name(),
                    a.phase,
                    phases.iter().map(|p| p.slug).collect::<Vec<_>>()
                )
            })?;
        session::mode_phases::choose(mode, phase.slug);
        Ok(())
    });
}

fn register_mode(list: &ActionList) {
    list.register(&MODE, Effect::Mutating, |a: ModeArg| {
        let mode = session::modes::Mode::ALL
            .into_iter()
            .find(|m| m.display_name().eq_ignore_ascii_case(a.mode.trim()))
            .ok_or_else(|| {
                let names: Vec<&str> = session::modes::Mode::ALL
                    .iter()
                    .map(|m| m.display_name())
                    .collect();
                format!("no mode {}: one of {names:?}", a.mode)
            })?;
        if let Ok(mut slot) = MODE_REQUEST.lock() {
            *slot = Some(mode);
        }
        Ok(())
    });
}

fn register_data(list: &ActionList) {
    register_mode(list);
    register_phase(list);
    register_view(list);
    register_tracks(list);
    register_hits(list);
    register_fundamental(list);
    crate::hits::register(list);
    register_track_set(list);
    register_reference(list);
    #[cfg(feature = "native")]
    crate::fx_actions::register(list);
}

static REFERENCE_MAP: ActionMeta = meta(
    "DRUMS_REFERENCE_MAP",
    "drum_reference_map",
    "Drum MIDI map",
    "The song's drum MIDI (the part as written) mapped to the kit: each note number, how many notes it has, and the piece whose trigger hits its notes land on — learned from the recording, since a drum library's map is its own. Unmapped: a cymbal, an articulation, a piece with no trigger.",
    "Drums",
    "",
);
static REFERENCE: ActionMeta = meta(
    "DRUMS_REFERENCE",
    "drum_reference",
    "Drum MIDI notes",
    "The drum part as written — the song's drum MIDI — for a piece (`Kick`, `Snare`, `Tom 1`, through the learned map) or a note number: each note's time, bar, beat and velocity. Paged; `from`/`to` (seconds) narrow it.",
    "Drums",
    "",
);
static COMPARE: ActionMeta = meta(
    "DRUMS_COMPARE",
    "drum_compare",
    "Drums against the MIDI",
    "A piece's recorded hits (its trigger) held up to its notes in the drum MIDI: how many were played, missing (written, not played) and extra (played, not written), the hits' offsets from their notes (ms, late positive: median, worst), then each note — worst first — with its bar and beat. Not every fill is as written: read missing and extra as where the take departs from the plan.",
    "Drums",
    "",
);

/// Which drum MIDI.
#[derive(Facet)]
struct MapArg {
    /// The drum MIDI track; found by name (`DRUM MIDI`) when left out.
    track: Option<String>,
}

/// One note number's place.
#[derive(Facet)]
struct MapRow {
    pitch: u8,
    notes: u32,
    piece: Option<String>,
    /// The share of its notes on that piece's hits (0..1).
    agreement: f64,
}

/// Which written notes.
#[derive(Facet)]
struct ReferenceArg {
    track: Option<String>,
    /// A piece, through the learned map.
    piece: Option<String>,
    /// Or a note number.
    pitch: Option<u8>,
    from: Option<f64>,
    to: Option<f64>,
    offset: Option<u32>,
    limit: Option<u32>,
}

/// One written note.
#[derive(Facet)]
struct NoteRow {
    time: f64,
    bar: i32,
    beat: f64,
    pitch: u8,
    velocity: u8,
}

/// A page of written notes.
#[derive(Facet)]
struct NotesPage {
    total: u32,
    offset: u32,
    notes: Vec<NoteRow>,
}

/// Which piece to compare.
#[derive(Facet)]
struct CompareArg {
    /// The piece: `Kick`, `Snare`, `Tom 1`.
    piece: String,
    track: Option<String>,
    /// How far a hit may be from its note and still be it (ms; 60).
    window_ms: Option<f64>,
    from: Option<f64>,
    to: Option<f64>,
    /// How many notes to detail (worst first; 50).
    limit: Option<u32>,
}

/// A note held up to its hit.
#[derive(Facet)]
struct NoteVsHit {
    time: f64,
    bar: i32,
    beat: f64,
    /// The hit's offset from the note, ms (late positive); `None`: not
    /// played.
    offset_ms: Option<f64>,
}

/// A piece against its part.
#[derive(Facet)]
struct CompareResult {
    piece: String,
    pitches: Vec<u8>,
    notes: u32,
    hits: u32,
    matched: u32,
    missing: u32,
    extra: u32,
    median_offset_ms: Option<f64>,
    worst_offset_ms: Option<f64>,
    /// The worst-placed and the missing notes first.
    detail: Vec<NoteVsHit>,
    /// Hits with no note, their times.
    extra_hits: Vec<f64>,
}

/// A trigger whose loudest hit is under this (dBFS) is dead.
const DEAD_TRIGGER_DB: f64 = -45.0;

/// How far a written note and its hit may be, unless asked (seconds).
const NOTE_WINDOW: f64 = 0.06;

/// The drum MIDI's notes, each live piece's name, trigger and hits, and
/// the learned map.
type Reference = (
    Vec<crate::drum_reference::RefNote>,
    Vec<(String, String, Vec<f64>)>,
    Vec<crate::drum_reference::Mapped>,
);

/// The kit's pieces' hits and the learned map, for the drum MIDI `track`.
fn reference(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    track: Option<&str>,
) -> Result<Reference, String> {
    let midi = match track {
        Some(t) => resolve(daw, project, t)?,
        None => crate::drum_reference::find_track(daw, project)
            .ok_or("no drum MIDI track — name it with `track`")?,
    };
    let notes = crate::drum_reference::notes(daw, project, &midi);
    // A trigger whose loudest is under the floor is dead — crosstalk on
    // a piezo nobody hit — and has no hits to hold anything up to.
    let pieces: Vec<(String, String, Vec<f64>)> = crate::drum_reference::pieces(daw, project)
        .into_iter()
        .filter_map(|(name, trigger)| {
            let found = crate::fundamental::hits_of(daw, project, &trigger, 24.0);
            let loudest = found
                .iter()
                .map(|h| h.level_db)
                .fold(f64::NEG_INFINITY, f64::max);
            (loudest >= DEAD_TRIGGER_DB)
                .then(|| (name, trigger, found.into_iter().map(|h| h.time).collect()))
        })
        .collect();
    let named: Vec<(String, Vec<f64>)> = pieces
        .iter()
        .map(|(n, _, h)| (n.clone(), h.clone()))
        .collect();
    let map = crate::drum_reference::learn_map(&notes, &named, NOTE_WINDOW);
    Ok((notes, pieces, map))
}

fn register_reference(list: &ActionList) {
    #[cfg(feature = "signal")]
    register_samples(list);
    #[cfg(feature = "signal")]
    register_bass_rig(list);
    register_items(list);
    register_lanes(list);
    register_remove(list);
    #[cfg(feature = "native")]
    register_verbs(list);
    #[cfg(feature = "native")]
    register_save(list);
    #[cfg(feature = "native")]
    register_undo(list);
    register_style(list);
    register_slip(list);
    register_peaks(list);
    register_reference_map(list);
    register_reference_notes(list);
    register_compare(list);
}

static EDIT_LANES: ActionMeta = meta(
    "DRUMS_EDIT_LANES",
    "drums_edit_lanes",
    "Comp and edit lanes",
    "Set the kit up to be edited, every drum track alike: its comping lane (the comp that plays) named COMP, and a new lane EDIT holding a copy of it, playing — so edits land on EDIT and COMP stays the comp they were made from (DRUMS_PLAY_LANE to hear either). The takes and their comp areas are untouched: EDIT is added after the last lane. A track already set up is left — unless `reset`, which starts the edit over: each EDIT lane's slices dropped and its COMP copied onto it again (one undo step). `dry_run` says what it would do.",
    "Drums",
    "",
);
static PLAY_LANE: ActionMeta = meta(
    "DRUMS_PLAY_LANE",
    "drums_play_lane",
    "Play a lane",
    "Play the kit's lane named `lane` (COMP, EDIT, or any lane's name) on every drum track at once — the edited drums against the comp they were made from.",
    "Drums",
    "",
);

/// Whether to only say.
#[derive(Facet)]
struct LanesArg {
    dry_run: Option<bool>,
    /// Start the edit over: each EDIT lane's slices dropped and its COMP
    /// copied onto it again.
    reset: Option<bool>,
}

/// What setting the lanes up did, track by track.
#[derive(Facet)]
struct LanesOut {
    /// `path: comp lane → EDIT lane (n items copied)`, or why it was left.
    tracks: Vec<String>,
    applied: bool,
}

/// A lane to play.
#[derive(Facet)]
struct PlayLaneArg {
    lane: String,
}

#[cfg(feature = "native")]
static VERBS: ActionMeta = meta(
    "DRUMS_VERBS",
    "drums_verbs",
    "Drum verbs, in time",
    "Set the kit's reverb returns (each piece's Verb folder: Kick/Verb/Room, Snare/Verb/Plate, Hall and Nonlin, Toms/Verb/Room and Nonlin) as a drum mix wants them — every tail a time the reverb takes itself (seconds, or a note value it reads against the song's tempo), every pre-delay too, low and high cuts on the snare's and toms', and each fader far down (kick −23 dB, snare −20, toms −26: felt more than heard). Their FTS Reverb is added if missing, set if there. Prepare does the same. `dry_run`.",
    "Drums",
    "",
);

/// Whether to only say.
#[cfg(feature = "native")]
#[derive(Facet)]
struct VerbsArg {
    dry_run: Option<bool>,
}

#[cfg(feature = "native")]
fn register_verbs(list: &ActionList) {
    list.register(&VERBS, Effect::Mutating, |a: VerbsArg| {
        let (daw, project) = song()?;
        let dry = a.dry_run.unwrap_or(false);
        let said =
            crate::mix_chains::drum_returns(daw, &project, dry).map_err(|e| e.to_string())?;
        if !dry {
            crate::studio::request_resync();
            crate::actions::list().changed("drums.verbs", "the drum verbs were set");
        }
        Ok(said)
    });
    list.register(&GROUP, Effect::Mutating, |a: VerbsArg| {
        let (daw, project) = song()?;
        let dry = a.dry_run.unwrap_or(false);
        let grouped = crate::drum_group::group(daw, &project, dry)?;
        if !dry {
            crate::studio::request_resync();
            crate::actions::list().changed("drums.group", "the kit was grouped");
        }
        Ok(grouped)
    });
    list.register(&TRIGGERS, Effect::Mutating, |_: NoArg| {
        let (daw, project) = song()?;
        let mut target = dynamic_template::apply::DawTarget::on(daw.clone(), project);
        let muted = target.mute_triggers().map_err(|e| e.to_string())?;
        crate::studio::request_resync();
        crate::actions::list().changed("drums.triggers", "the triggers were muted");
        Ok(muted)
    });
}

#[cfg(feature = "native")]
static GROUP: ActionMeta = meta(
    "DRUMS_GROUP",
    "drums_group",
    "Group the kit",
    "Put every recorded drum track (each mic and trigger under the top-level Drums folder; not a Sample track, not the Process or a Verb folder) in one track group, `Drums`, for record arm and media edit only: armed together, so the kit is never recorded part by part while grouping is on (the toolbar's Grouping); edited together, so a slip, cut or comp on one track is on all. Faders, mutes and solos stay each track's own. Returns the tracks grouped; `dry_run` names them and changes nothing.",
    "Drums",
    "",
);

#[cfg(feature = "native")]
static TRIGGERS: ActionMeta = meta(
    "DRUMS_TRIGGERS",
    "drums_triggers",
    "Mute the triggers",
    "Mute every drum trigger track (`Trig`, `T1 Trig`, …) — its clicks are never heard — and make each of its sends pre-fader, so the gates it keys and whatever it feeds still hear it. Prepare does the same. Returns the tracks it muted (none when all already were).",
    "Drums",
    "",
);

/// No arguments.
#[cfg(feature = "native")]
#[derive(Facet)]
struct NoArg {}

#[cfg(feature = "native")]
static SAVE: ActionMeta = meta(
    "SESSION_SAVE",
    "session_save",
    "Save the song",
    "Save the song as it is now — its edits, its tracks, every FX with what it holds (a sampler's kit, a rig's capture, a reverb's time) — into its project file: `Song.session` beside the `.RPP` it was opened from (what opens next time), or the alternative it is. What changed since the last save goes in as one step of the file's history. `to` saves to another file instead and leaves the song's own alone. Says where.",
    "Session",
    "",
);

/// Where to save, if not the song's own `.session`. Only `to`: a
/// misnamed argument (`dir`) would otherwise save over the song's own.
#[cfg(feature = "native")]
#[derive(Facet)]
#[facet(deny_unknown_fields)]
struct SaveArg {
    to: Option<String>,
}

#[cfg(feature = "native")]
static CHECK_SAVED: ActionMeta = meta(
    "SESSION_CHECK_SAVED",
    "session_check_saved",
    "Check a saved song",
    "Reopen a saved project file (`dir`: a `Song.session`, or an alternative `Song.session/Organized`) as a second song, as a reopen would, and set it beside the song open now as the project doc reads both: every track with its FX and what they hold, sends, automation, lanes and groups; every item with its take, stretch markers and notes; markers, regions and tempo. Says `the same` or what differs. The second song is closed again.",
    "Session",
    "",
);

/// The folder to check.
#[cfg(feature = "native")]
#[derive(Facet)]
struct CheckSavedArg {
    dir: String,
}

static FOLD: ActionMeta = meta(
    "SESSION_FOLD",
    "session_fold",
    "Open or shut a folder",
    "Open (`open: true`) the folder `track` (its GUID, path or name) and every folder it is in, so its tracks show; or shut it (`open: false`). Returns the folders changed.",
    "View",
    "",
);

/// A folder to open or shut.
#[derive(Facet)]
struct FoldArg {
    track: String,
    open: bool,
}

#[cfg(feature = "native")]
static UNDO: ActionMeta = meta(
    "SESSION_UNDO",
    "session_undo",
    "Undo",
    "Take back the last step — a move, a cut, a slip edit, whatever one gesture or action did, as one. Says what it undid, or that there is nothing to.",
    "Session",
    "",
);

#[cfg(feature = "native")]
static REDO: ActionMeta = meta(
    "SESSION_REDO",
    "session_redo",
    "Redo",
    "Do again the last step undone. Says what it redid, or that there is nothing to.",
    "Session",
    "",
);

#[cfg(feature = "native")]
static MARK_KINDS: ActionMeta = meta(
    "SESSION_MARK_KINDS",
    "session_mark_kinds",
    "Mark the kit's kinds",
    "Write on every track of the kit what the golden template says it is — piece, Sum, mic, trigger, sub, sample, reverb — matched by name, for a song prepared before kinds were kept on the track. The scenes (Edit mode's rows above all) read them there. One undo step. Says how many tracks were marked.",
    "Session",
    "",
);

static CHECKPOINT: ActionMeta = meta(
    "SESSION_CHECKPOINT",
    "session_checkpoint",
    "Save a checkpoint",
    "Save the song and fork a named alternative from it as it is now (`name`: \"Organized\", \"Mix A\") — a branch of the song's own project file (`Song.session`), sharing everything it had and keeping only what is done to it after, the way Logic keeps alternatives. The song stays open, under its own name. Open one later from the start screen's Recent or SESSION_OPEN.",
    "Session",
    "",
);

/// The checkpoint's name.
#[cfg(feature = "native")]
#[derive(Facet)]
struct CheckpointArg {
    name: String,
}

#[cfg(feature = "native")]
static ALTERNATIVES: ActionMeta = meta(
    "SESSION_ALTERNATIVES",
    "session_alternatives",
    "The song's alternatives",
    "The open song's own version and its alternatives (checkpoints saved with SESSION_CHECKPOINT), the newest first: name, folder, when saved. Read-only.",
    "Session",
    "",
);

#[cfg(feature = "native")]
static OPEN: ActionMeta = meta(
    "SESSION_OPEN",
    "session_open",
    "Open a song or version",
    "Leave the song open now and open another: `name` a version of this song (\"The song\", or a checkpoint's name — SESSION_ALTERNATIVES lists them), or `path` any song, setlist or `.session`. What is open is not saved first — SESSION_SAVE it if it should be. Through the start screen, as Back and a pick there would.",
    "Session",
    "",
);

/// Which to open: a version of this song by name, or anything by path.
#[cfg(feature = "native")]
#[derive(Facet)]
struct OpenArg {
    name: Option<String>,
    path: Option<String>,
}

#[cfg(feature = "native")]
static RECENT: ActionMeta = meta(
    "SESSION_RECENT",
    "session_recent",
    "Songs opened lately",
    "The songs and setlists opened lately, the newest first: name, the version that was open (the song, or which alternative), when, and the song's alternatives. Read-only.",
    "Session",
    "",
);

/// Whether the open song has changes it was not saved with; `None` when
/// that is not known.
#[cfg(feature = "native")]
#[must_use]
pub fn open_song_unsaved() -> Option<bool> {
    let (daw, ProjectContext::Project(guid)) = song().ok()? else {
        return None;
    };
    crate::projects::unsaved(daw, &guid)
}

/// The open song's versions, for the top bar's Versions.
///
/// The song's own (`"The song"`) and its checkpoints, the newest first —
/// each its name, when it was saved, where it is, and whether it is the
/// one open.
#[cfg(feature = "native")]
#[must_use]
pub fn open_song_versions() -> Vec<crate::versions::Version> {
    let Ok((daw, ProjectContext::Project(guid))) = song() else {
        return Vec::new();
    };
    let Some(open) = daw.read_project(&guid, |p| std::path::PathBuf::from(&p.info.path)) else {
        return Vec::new();
    };
    let Ok(base) = song_base(daw, &guid) else {
        return Vec::new();
    };
    let same = |a: &std::path::Path, b: &std::path::Path| {
        std::path::absolute(a).ok() == std::path::absolute(b).ok()
    };
    let mut out: Vec<crate::versions::Version> = crate::projects::main_of(&base)
        .map(|path| crate::versions::Version {
            name: "The song".to_owned(),
            when: String::new(),
            open: crate::projects::alternative_name(&open).is_none(),
            path,
        })
        .into_iter()
        .collect();
    out.extend(crate::projects::alternatives(&base).into_iter().map(|alt| {
        crate::versions::Version {
            open: same(&alt.path, &open),
            name: alt.name,
            when: crate::projects::ago(alt.saved),
            path: alt.path,
        }
    }));
    out
}

/// The open song's base ([`crate::projects::base_of`]): what its
/// alternatives are kept beside.
#[cfg(feature = "native")]
fn song_base(daw: &daw::standalone::Standalone, guid: &str) -> Result<std::path::PathBuf, String> {
    let path = daw
        .read_project(guid, |p| p.info.path.clone())
        .ok_or("no song open")?;
    crate::projects::base_of(std::path::Path::new(&path))
        .ok_or_else(|| format!("{path} is not a song's file: nowhere to keep alternatives"))
}

#[cfg(feature = "native")]
static SELECT_TRACK: ActionMeta = meta(
    "SESSION_SELECT_TRACK",
    "session_select_track",
    "Select a track",
    "Select `track` (its GUID, its path — `Snare/Verb/Plate`, any tail of it — or its name) and nothing else: what the FX row, the mixer and the Editor follow.",
    "Session",
    "",
);

/// Which track.
#[cfg(feature = "native")]
#[derive(Facet)]
struct SelectTrackArg {
    track: String,
}

static JOBS: ActionMeta = meta(
    "SESSION_JOBS",
    "session_jobs",
    "Work in the background",
    "What is being done off the window's thread right now — reading waveforms, slicing them, opening or saving a project file — each with how far along (`done` of `total`; a total of 0 is work that does not know its length). What the bottom bar shows. Empty when nothing is running. Read-only.",
    "View",
    "",
);

/// One job, as listed.
#[derive(Facet)]
struct JobRow {
    label: String,
    done: u64,
    total: u64,
}

static ITEM_STYLE: ActionMeta = meta(
    "SESSION_ITEM_STYLE",
    "session_item_style",
    "Item style",
    "How items are drawn: `outlined: true` (the default) a faint body in the item's colour with the waveform standing out over it in the full colour and a line round each item; `false` the half-strength block of colour the waveform is read against.",
    "View",
    "",
);

/// Outlined or not.
#[derive(Facet)]
struct ItemStyleArg {
    outlined: bool,
}

fn register_style(list: &ActionList) {
    list.register(&JOBS, Effect::ReadOnly, |(): ()| {
        Ok(crate::jobs::running()
            .into_iter()
            .map(|(label, done, total)| JobRow {
                label,
                done: u64::try_from(done).unwrap_or(u64::MAX),
                total: u64::try_from(total).unwrap_or(u64::MAX),
            })
            .collect::<Vec<_>>())
    });
    list.register(&ITEM_STYLE, Effect::Mutating, |a: ItemStyleArg| {
        crate::options::OUTLINED.set(a.outlined);
        // The lanes are recorded with their bodies: read back, drawn anew.
        crate::studio::request_resync();
        Ok(if a.outlined { "outlined" } else { "filled" })
    });
}

#[cfg(feature = "native")]
fn register_undo(list: &ActionList) {
    list.register(&FOLD, Effect::Mutating, |a: FoldArg| {
        let (daw, project) = song()?;
        let guid = resolve(daw, &project, &a.track)?;
        let tracks = daw::service::Tracks::all(daw, project);
        let by_guid: std::collections::HashMap<&str, &daw_proto::Track> =
            tracks.iter().map(|t| (t.guid.as_str(), t)).collect();
        // Opening a folder opens the ones it is in: a folder open inside a
        // shut one shows nothing.
        let mut opened = Vec::new();
        let mut at = by_guid.get(guid.as_str()).copied();
        while let Some(track) = at {
            if track.is_folder && (a.open || track.guid == guid) {
                crate::folds::toggle(&track.guid, !a.open);
                opened.push(track.name.clone());
            }
            if !a.open {
                break;
            }
            at = track
                .parent_guid
                .as_deref()
                .and_then(|p| by_guid.get(p).copied());
        }
        Ok(opened)
    });
    list.register(&UNDO, Effect::Mutating, |_: NoArg| {
        let (daw, project) = song()?;
        // A song open from its file undoes through its doc: only this
        // peer's steps, a collaborator's edits left standing.
        if let Some(said) = undo_through_doc(daw, &project, true)? {
            return Ok(said);
        }
        let label = daw::service::Projects::last_undo_label(daw, project.clone());
        if !daw::service::Projects::undo(daw, project) {
            return Ok("nothing to undo".to_owned());
        }
        crate::studio::request_resync();
        crate::actions::list().changed("items", "undo");
        Ok(format!(
            "undid {}",
            label.unwrap_or_else(|| "a step".to_owned())
        ))
    });
    list.register(&REDO, Effect::Mutating, |_: NoArg| {
        let (daw, project) = song()?;
        if let Some(said) = undo_through_doc(daw, &project, false)? {
            return Ok(said);
        }
        let label = daw::service::Projects::last_redo_label(daw, project.clone());
        if !daw::service::Projects::redo(daw, project) {
            return Ok("nothing to redo".to_owned());
        }
        crate::studio::request_resync();
        crate::actions::list().changed("items", "redo");
        Ok(format!(
            "redid {}",
            label.unwrap_or_else(|| "a step".to_owned())
        ))
    });
}

/// Undo (`back`) or redo through the song's doc, when it is open from its
/// project file: what to say, or `None` for the engine's own undo.
#[cfg(feature = "native")]
fn undo_through_doc(
    daw: &daw::standalone::Standalone,
    project: &daw::service::ProjectContext,
    back: bool,
) -> Result<Option<String>, String> {
    use crate::project_file::Undone;
    let daw::service::ProjectContext::Project(guid) = project else {
        return Ok(None);
    };
    let done = if back {
        crate::project_file::undo(daw, guid)?
    } else {
        crate::project_file::redo(daw, guid)?
    };
    let (verb, none) = if back {
        ("undid", "nothing to undo")
    } else {
        ("redid", "nothing to redo")
    };
    Ok(match done {
        Undone::NotOpen => None,
        Undone::Nothing => Some(none.to_owned()),
        Undone::Step(name) => {
            crate::studio::request_resync();
            crate::actions::list().changed("items", if back { "undo" } else { "redo" });
            Some(format!("{verb} {name}"))
        }
    })
}

#[cfg(feature = "native")]
fn register_save(list: &ActionList) {
    list.register(&SELECT_TRACK, Effect::Mutating, |a: SelectTrackArg| {
        let (daw, project) = song()?;
        let guid = resolve(daw, &project, &a.track)?;
        daw::service::Tracks::select_exclusive(
            daw,
            project,
            daw::service::TrackRef::Guid(guid.clone()),
        )
        .map_err(|e| e.to_string())?;
        Ok(guid)
    });
    list.register(&MARK_KINDS, Effect::Mutating, |_: NoArg| {
        let (daw, project) = song()?;
        daw::service::Projects::begin_undo_block(daw, project.clone(), "Mark the kit's kinds");
        let marked =
            dynamic_template::apply::DawTarget::on(daw.clone(), project.clone()).mark_kit_kinds();
        daw::service::Projects::end_undo_block(daw, project, "Mark the kit's kinds", None);
        let marked = marked.map_err(|e| e.to_string())?;
        crate::studio::request_resync();
        Ok(format!("{marked} tracks marked"))
    });
    list.register(&CHECKPOINT, Effect::Mutating, |a: CheckpointArg| {
        let (daw, project) = song()?;
        let ProjectContext::Project(guid) = &project else {
            return Err("no song open".to_owned());
        };
        let from = std::path::PathBuf::from(
            daw.read_project(guid, |p| p.info.path.clone())
                .ok_or("no song open")?,
        );
        let file = crate::project_file::fork(daw, guid, &from, &a.name)?;
        // The song was saved on the way: nothing unsaved now.
        crate::projects::mark_saved(daw, guid);
        Ok(format!(
            "saved the song and forked {} from it in {}; the song stays open",
            a.name.trim(),
            file.display()
        ))
    });
    list.register(&OPEN, Effect::Mutating, |a: OpenArg| {
        let path = match (a.name, a.path) {
            (_, Some(path)) => std::path::PathBuf::from(path),
            (Some(name), None) => open_song_versions()
                .into_iter()
                .find(|v| v.name.eq_ignore_ascii_case(name.trim()))
                .map(|v| v.path)
                .ok_or_else(|| {
                    format!("no version {name:?} of this song (SESSION_ALTERNATIVES)")
                })?,
            (None, None) => return Err("`name` (a version of this song) or `path`".to_owned()),
        };
        if !path.exists() {
            return Err(format!("{} is not there", path.display()));
        }
        crate::projects::open_from_outside(path.clone());
        Ok(format!("opening {}", path.display()))
    });
    list.register(&ALTERNATIVES, Effect::ReadOnly, |(): ()| {
        let (daw, project) = song()?;
        let ProjectContext::Project(guid) = &project else {
            return Err("no song open".to_owned());
        };
        let base = song_base(daw, guid)?;
        let unsaved = match crate::projects::unsaved(daw, guid) {
            Some(true) => " (unsaved changes)",
            Some(false) => " (saved)",
            None => "",
        };
        let mut out = vec![format!(
            "the song{unsaved}: {}",
            crate::projects::main_of(&base)
                .map_or_else(|| "(not saved yet)".to_owned(), |p| p.display().to_string())
        )];
        out.extend(crate::projects::alternatives(&base).into_iter().map(|alt| {
            format!(
                "{}: {} (saved {})",
                alt.name,
                alt.path.display(),
                crate::projects::ago(alt.saved)
            )
        }));
        Ok(out)
    });
    list.register(&RECENT, Effect::ReadOnly, |(): ()| {
        Ok(crate::projects::recent()
            .into_iter()
            .map(|r| {
                let version = r
                    .opened_alternative()
                    .map_or_else(String::new, |alt| format!(" as {alt}"));
                let alternatives: Vec<String> = crate::projects::alternatives(&r.base)
                    .into_iter()
                    .map(|a| a.name)
                    .collect();
                format!(
                    "{}{version}, {} — {}{}",
                    r.name(),
                    crate::projects::ago(r.at),
                    r.opened.display(),
                    if alternatives.is_empty() {
                        String::new()
                    } else {
                        format!(" (alternatives: {})", alternatives.join(", "))
                    }
                )
            })
            .collect::<Vec<_>>())
    });
    list.register(&SAVE, Effect::Mutating, |a: SaveArg| {
        let (daw, project) = song()?;
        let ProjectContext::Project(guid) = &project else {
            return Err("no song open".to_owned());
        };
        let dir = if let Some(to) = a.to.clone() {
            std::path::PathBuf::from(to)
        } else {
            let from = std::path::PathBuf::from(
                daw.read_project(guid, |p| p.info.path.clone())
                    .ok_or("no song open")?,
            );
            if from.extension().is_some_and(|e| e == "session") {
                from
            } else {
                from.with_extension("session")
            }
        };
        let own = a.to.is_none();
        let at = crate::session_file::save_session(daw, guid, &dir)?;
        if own {
            // The song itself, saved: nothing unsaved now. (A copy saved
            // elsewhere leaves the song as unsaved as it was.)
            crate::projects::mark_saved(daw, guid);
        }
        Ok(format!("saved to {}", at.display()))
    });
    list.register(&CHECK_SAVED, Effect::ReadOnly, |a: CheckSavedArg| {
        let (daw, project) = song()?;
        let ProjectContext::Project(guid) = &project else {
            return Err("no song open".to_owned());
        };
        let apart = crate::project_file::check(daw, guid, std::path::Path::new(&a.dir))?;
        Ok(if apart.is_empty() {
            vec!["the same".to_owned()]
        } else {
            apart
        })
    });
}

static REMOVE: ActionMeta = meta(
    "DRUMS_REMOVE",
    "drums_remove",
    "Remove from the drums",
    "Take `from`..`to` (seconds) out of the kit's edited takes (its EDIT lane, once it has one): cut at both ends, the stretch between deleted, a 2 ms fade on each edge. `tracks` (paths or GUIDs) names which — a Trig, to take a stray or doubled trigger spike out so it fires no gate and no sample; left out, the whole kit, for a hit that should not be there at all. One undo step. Look first: AUDIO_PEAKS on the Trig and its mics, and the drum MIDI (DRUMS_REFERENCE).",
    "Drums",
    "",
);

/// What to take out.
#[derive(Facet)]
struct RemoveArg {
    from: f64,
    to: f64,
    tracks: Option<Vec<String>>,
}

fn register_remove(list: &ActionList) {
    list.register(&REMOVE, Effect::Destructive, |a: RemoveArg| {
        if a.to <= a.from {
            return Err("`to` must be after `from`".to_owned());
        }
        if a.to - a.from > 2.0 {
            return Err("more than two seconds: this takes out a hit, not a passage".to_owned());
        }
        let (daw, project) = song()?;
        let tracks = a
            .tracks
            .iter()
            .flatten()
            .map(|t| resolve(daw, &project, t))
            .collect::<Result<Vec<String>, String>>()?;
        let us = |s: f64| num_traits::ToPrimitive::to_i64(&(s * 1_000_000.0).round()).unwrap_or(0);
        queue(Action::RemoveRange {
            tracks,
            from_us: us(a.from),
            to_us: us(a.to),
        });
        Ok(())
    });
}

#[cfg(feature = "signal")]
static SAMPLES: ActionMeta = meta(
    "DRUMS_SAMPLES",
    "drums_samples",
    "Drum samples under the kit",
    "Blend signal's sampled kit under the recorded drums, from the hits the triggers caught (the drum-edit detector, on the EDIT lane once there is one): each piece's hits written as notes on its Sample track (the kit's GM note, velocity from the hit's level) with an FTS Kit Mic there playing that piece's close mics; an OH Sample and a Room Sample track playing every piece's overhead and room mics, sent into the recorded overheads and rooms. A piece's voices get the same notes, so each hit is one sample on all of them. `preset` (a `.signalpreset`; GGD Modern and Massive 2's Heavy Thwacky unless given), `dry_run`.",
    "Drums",
    "",
);

/// Which kit.
#[cfg(feature = "signal")]
#[derive(Facet)]
struct SamplesArg {
    preset: Option<String>,
    dry_run: Option<bool>,
}

/// What it did.
#[cfg(feature = "signal")]
#[derive(Facet)]
struct SamplesOut {
    tracks: Vec<String>,
    notes: u32,
    applied: bool,
}

/// The kit samples blend in unless told otherwise.
#[cfg(feature = "signal")]
const DEFAULT_KIT: &str = "/run/media/AudioHaven/Signal/Libraries/Drum Kits/GGD Modern and Massive 2/Presets/Heavy Thwacky.signalpreset";

#[cfg(feature = "signal")]
static BASS_RIG: ActionMeta = meta(
    "BASS_RIG",
    "bass_rig",
    "Bass rig on the bass",
    "signal's bass rig (FTS Bass Rig: drive capture, amp capture, cab, trims) on each track under Bass, first in its chain, with the patch its name holds: Clean (the DP3X preamp fed low, barely breaking up), Harmonics (the DP3X as captured), Crunch (an HM2 drive into it). A track no patch fits is left, and said. Reran, the rig there takes the patch again. `dry_run`.",
    "Bass",
    "",
);

/// Whether to only say.
#[cfg(feature = "signal")]
#[derive(Facet)]
struct BassRigArg {
    dry_run: Option<bool>,
}

#[cfg(feature = "signal")]
fn register_bass_rig(list: &ActionList) {
    list.register(&BASS_RIG, Effect::Mutating, |a: BassRigArg| {
        let (daw, project) = song()?;
        crate::bass_rig::set_up(daw, &project, a.dry_run.unwrap_or(false))
    });
}

#[cfg(feature = "signal")]
static CLOSE_MIC_CHECK: ActionMeta = meta(
    "DRUMS_CLOSE_MIC_CHECK",
    "drums_close_mic_check",
    "How a close mic stands in for its trigger",
    "For a piece with a working trigger (`piece`: `Tom 1`), find its hits from its close mic alone — as DRUMS_SAMPLES does for a piece whose trigger is dead — and hold them up to the trigger's: how many each has, how many the mic found that the trigger has (within 15 ms), missed, and found that are not there. What the close-mic hits of a dead trigger's piece can be trusted to. Read-only.",
    "Drums",
    "",
);

/// The piece to check.
#[cfg(feature = "signal")]
#[derive(Facet)]
struct CloseMicArg {
    piece: String,
}

#[cfg(feature = "signal")]
fn register_samples(list: &ActionList) {
    list.register(&CLOSE_MIC_CHECK, Effect::ReadOnly, |a: CloseMicArg| {
        const WITHIN: f64 = 0.015;
        let (daw, project) = song()?;
        let settings = crate::drum_settings::current();
        let trigger = crate::drum_reference::pieces(daw, &project)
            .into_iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(a.piece.trim()))
            .map(|(_, t)| t)
            .ok_or_else(|| format!("no piece {:?}", a.piece))?;
        let pieces = kit(daw, &project, &settings);
        let found: Vec<f64> = crate::close_mic::hits(daw, &project, &trigger, &settings)
            .into_iter()
            .map(|h| h.time)
            .collect();
        let near = |t: f64, among: &[f64]| among.iter().any(|x| (x - t).abs() <= WITHIN);
        let Some(truth) = pieces
            .iter()
            .find(|(n, _, _)| n.eq_ignore_ascii_case(a.piece.trim()))
            .map(|(_, _, hits)| hits.iter().map(|h| h.time).collect::<Vec<f64>>())
        else {
            // Its trigger is dead: nothing to hold the mic up to. What it
            // found, and how much of it falls with another piece's hits —
            // a bleed would fall with the piece it is the bleed of.
            let with: Vec<String> = pieces
                .iter()
                .map(|(name, _, hits)| {
                    let times: Vec<f64> = hits.iter().map(|h| h.time).collect();
                    let same = found.iter().filter(|t| near(**t, &times)).count();
                    format!("{same} with {name}")
                })
                .collect();
            let alone = found
                .iter()
                .filter(|t| pieces.iter().all(|(_, _, hits)| !hits.iter().any(|h| (h.time - **t).abs() <= WITHIN)))
                .count();
            // Where it fell with each piece, by bar and beat: a part
            // (beats 2 and 4 through a section) or a scatter.
            let at: Vec<String> = pieces
                .iter()
                .filter_map(|(name, _, hits)| {
                    let times: Vec<f64> = hits.iter().map(|h| h.time).collect();
                    let beats: Vec<String> = found
                        .iter()
                        .filter(|t| near(**t, &times))
                        .take(24)
                        .map(|t| {
                            let (bar, beat, _) = crate::song_bars::musical(daw, &project, *t + 1e-6);
                            format!("{bar}.{beat:.1}")
                        })
                        .collect();
                    (!beats.is_empty()).then(|| format!("with {name} at {}", beats.join(" ")))
                })
                .collect();
            return Ok(format!(
                "{}: its trigger is dead; its close mic found {} hits — {alone} alone, {}. {}",
                a.piece.trim(),
                found.len(),
                with.join(", "),
                at.join("; ")
            ));
        };
        let matched = found.iter().filter(|t| near(**t, &truth)).count();
        let missed: Vec<f64> = truth.iter().copied().filter(|t| !near(*t, &found)).collect();
        let extra: Vec<f64> = found.iter().copied().filter(|t| !near(*t, &truth)).collect();
        Ok(format!(
            "{}: trigger {} hits, close mic {} — {matched} the same, {} missed (first at {:?}), {} not the trigger's (first at {:?})",
            a.piece.trim(),
            truth.len(),
            found.len(),
            missed.len(),
            missed.iter().take(5).map(|t| (t * 100.0).round() / 100.0).collect::<Vec<_>>(),
            extra.len(),
            extra.iter().take(5).map(|t| (t * 100.0).round() / 100.0).collect::<Vec<_>>(),
        ))
    });
    list.register(&SAMPLES, Effect::Mutating, |a: SamplesArg| {
        let (daw, project) = song()?;
        let preset = std::path::PathBuf::from(a.preset.as_deref().unwrap_or(DEFAULT_KIT));
        if !preset.exists() {
            return Err(format!("no kit preset at {}", preset.display()));
        }
        let dry = a.dry_run.unwrap_or(false);
        let report = crate::drum_samples::set_up(daw, &project, &preset, dry)?;
        Ok(SamplesOut {
            tracks: report.tracks,
            notes: u32::try_from(report.notes).unwrap_or(u32::MAX),
            applied: !dry,
        })
    });
}

static ITEMS: ActionMeta = meta(
    "TRACK_ITEMS",
    "track_items",
    "A track's items",
    "A track's items from `from` to `to` (seconds; all, unless given): each one's GUID, position, length and end, how far into its source it starts, its lane (and whether that plays), fades — to see what an edit made: the slices, where they sit, what they play. 500 at most.",
    "Data",
    "",
);

/// Which items.
#[derive(Facet)]
struct ItemsArg {
    track: String,
    from: Option<f64>,
    to: Option<f64>,
}

/// One item.
#[derive(Facet)]
struct ItemRow {
    guid: String,
    position: f64,
    length: f64,
    end: f64,
    source_offset: f64,
    lane: u32,
    plays: bool,
    fade_in_ms: f64,
    fade_out_ms: f64,
}

fn register_items(list: &ActionList) {
    list.register(&ITEMS, Effect::ReadOnly, |a: ItemsArg| {
        let (daw, project) = song()?;
        let guid = resolve(daw, &project, &a.track)?;
        let track = Tracks::get(daw, project.clone(), TrackRef::Guid(guid.clone()))
            .ok_or("the track went")?;
        let (from, to) = (
            a.from.unwrap_or(f64::NEG_INFINITY),
            a.to.unwrap_or(f64::INFINITY),
        );
        let mut rows: Vec<ItemRow> =
            daw::service::Items::get_items(daw, project, TrackRef::Guid(guid))
                .into_iter()
                .filter_map(|i| {
                    let position = i.position.as_seconds();
                    let end = position + i.length.as_seconds();
                    (end >= from && position <= to).then(|| ItemRow {
                        lane: i.fixed_lane.unwrap_or(0),
                        plays: daw_ui::studio::project::on_playing_lane(&track, &i),
                        guid: i.guid.clone(),
                        position,
                        length: i.length.as_seconds(),
                        end,
                        source_offset: i.start_offset.as_seconds(),
                        fade_in_ms: i.fade_in_length.as_seconds() * 1_000.0,
                        fade_out_ms: i.fade_out_length.as_seconds() * 1_000.0,
                    })
                })
                .collect();
        rows.sort_by(|a, b| a.position.total_cmp(&b.position));
        rows.truncate(500);
        Ok(rows)
    });
}

fn register_lanes(list: &ActionList) {
    list.register(&EDIT_LANES, Effect::Mutating, |a: LanesArg| {
        let (daw, project) = song()?;
        let dry = a.dry_run.unwrap_or(false);
        let tracks = if a.reset == Some(true) {
            crate::drum_lanes::reset_edit(daw, &project, dry)?
        } else {
            crate::drum_lanes::set_up(daw, &project, dry)?
        };
        Ok(LanesOut {
            tracks,
            applied: !dry,
        })
    });
    list.register(&PLAY_LANE, Effect::Mutating, |a: PlayLaneArg| {
        let (daw, project) = song()?;
        crate::drum_lanes::play(daw, &project, &a.lane)
    });
}

fn register_reference_map(list: &ActionList) {
    list.register(&REFERENCE_MAP, Effect::ReadOnly, |a: MapArg| {
        let (daw, project) = song()?;
        let (_, _, map) = reference(daw, &project, a.track.as_deref())?;
        Ok(map
            .into_iter()
            .map(|m| MapRow {
                pitch: m.pitch,
                notes: u32::try_from(m.notes).unwrap_or(u32::MAX),
                piece: m.piece,
                agreement: m.agreement,
            })
            .collect::<Vec<_>>())
    });
}

fn register_reference_notes(list: &ActionList) {
    list.register(&REFERENCE, Effect::ReadOnly, |a: ReferenceArg| {
        let (daw, project) = song()?;
        let (notes, _, map) = reference(daw, &project, a.track.as_deref())?;
        let pitches: Vec<u8> = match (&a.piece, a.pitch) {
            (_, Some(p)) => vec![p],
            (Some(piece), None) => map
                .iter()
                .filter(|m| {
                    m.piece
                        .as_deref()
                        .is_some_and(|p| p.eq_ignore_ascii_case(piece))
                })
                .map(|m| m.pitch)
                .collect(),
            (None, None) => map.iter().map(|m| m.pitch).collect(),
        };
        let from = a.from.unwrap_or(f64::NEG_INFINITY);
        let to = a.to.unwrap_or(f64::INFINITY);
        let all: Vec<_> = notes
            .into_iter()
            .filter(|n| pitches.contains(&n.pitch) && n.time >= from && n.time <= to)
            .collect();
        let offset = a.offset.unwrap_or(0);
        let page = all
            .iter()
            .skip(usize::try_from(offset).unwrap_or(usize::MAX))
            .take(usize::try_from(a.limit.unwrap_or(PAGE)).unwrap_or(usize::MAX))
            .map(|n| {
                let (bar, beat, frac) = crate::song_bars::musical(daw, &project, n.time);
                NoteRow {
                    time: n.time,
                    bar,
                    beat: f64::from(beat) + frac,
                    pitch: n.pitch,
                    velocity: n.velocity,
                }
            })
            .collect();
        Ok(NotesPage {
            total: u32::try_from(all.len()).unwrap_or(u32::MAX),
            offset,
            notes: page,
        })
    });
}

fn register_compare(list: &ActionList) {
    list.register(&COMPARE, Effect::ReadOnly, |a: CompareArg| {
        let (daw, project) = song()?;
        let (notes, pieces, map) = reference(daw, &project, a.track.as_deref())?;
        let (name, _, hits) = pieces
            .iter()
            .find(|(n, _, _)| n.eq_ignore_ascii_case(&a.piece))
            .ok_or_else(|| {
                format!(
                    "no piece {} with a trigger: one of {:?}",
                    a.piece,
                    pieces
                        .iter()
                        .map(|(n, _, _)| n.as_str())
                        .collect::<Vec<_>>()
                )
            })?;
        let pitches: Vec<u8> = map
            .iter()
            .filter(|m| m.piece.as_deref() == Some(name.as_str()))
            .map(|m| m.pitch)
            .collect();
        let from = a.from.unwrap_or(f64::NEG_INFINITY);
        let to = a.to.unwrap_or(f64::INFINITY);
        let written: Vec<f64> = notes
            .iter()
            .filter(|n| pitches.contains(&n.pitch) && n.time >= from && n.time <= to)
            .map(|n| n.time)
            .collect();
        let played: Vec<f64> = hits
            .iter()
            .copied()
            .filter(|t| *t >= from && *t <= to)
            .collect();
        let window = a.window_ms.map_or(NOTE_WINDOW, |ms| ms / 1_000.0);
        let c = crate::drum_reference::compare(&written, &played, window);
        let mut offsets: Vec<f64> = c.matched.iter().map(|m| m.2).collect();
        offsets.sort_by(f64::total_cmp);
        let median = offsets.get(offsets.len() / 2).copied();
        let worst = offsets
            .iter()
            .copied()
            .max_by(|x, y| x.abs().total_cmp(&y.abs()));
        let at = |t: f64| {
            let (bar, beat, frac) = crate::song_bars::musical(daw, &project, t);
            (bar, f64::from(beat) + frac)
        };
        let mut detail: Vec<NoteVsHit> = c
            .missing
            .iter()
            .map(|t| {
                let (bar, beat) = at(*t);
                NoteVsHit {
                    time: *t,
                    bar,
                    beat,
                    offset_ms: None,
                }
            })
            .chain(c.matched.iter().map(|(t, _, off)| {
                let (bar, beat) = at(*t);
                NoteVsHit {
                    time: *t,
                    bar,
                    beat,
                    offset_ms: Some(*off),
                }
            }))
            .collect();
        detail.sort_by(|x, y| {
            let key = |n: &NoteVsHit| n.offset_ms.map_or(f64::INFINITY, f64::abs);
            key(y).total_cmp(&key(x))
        });
        detail.truncate(usize::try_from(a.limit.unwrap_or(50)).unwrap_or(usize::MAX));
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        Ok(CompareResult {
            piece: name.clone(),
            pitches,
            notes: count(written.len()),
            hits: count(played.len()),
            matched: count(c.matched.len()),
            missing: count(c.missing.len()),
            extra: count(c.extra.len()),
            median_offset_ms: median,
            worst_offset_ms: worst,
            detail,
            extra_hits: c.extra.into_iter().take(50).collect(),
        })
    });
}

fn register_tracks(list: &ActionList) {
    list.register(&TRACKS, Effect::ReadOnly, |a: TracksArg| {
        let (daw, project) = song()?;
        let tracks = Tracks::all(daw, project.clone());
        let by_guid: std::collections::HashMap<&str, &daw_proto::Track> =
            tracks.iter().map(|t| (t.guid.as_str(), t)).collect();
        let filter = a.filter.map(|f| f.to_lowercase());
        Ok(tracks
            .iter()
            .map(|t| (t, crate::gain_stage::path_of(t, &by_guid).join("/")))
            .filter(|(_, path)| {
                filter
                    .as_deref()
                    .is_none_or(|f| path.to_lowercase().contains(f))
            })
            .map(|(t, path)| {
                let items = daw::service::Items::get_items(
                    daw,
                    project.clone(),
                    TrackRef::Guid(t.guid.clone()),
                );
                (t, path, items)
            })
            .map(|(t, path, items)| TrackRow {
                lanes: lane_rows(t, &items),
                guid: t.guid.clone(),
                name: t.name.clone(),
                path,
                folder: t.is_folder,
                muted: t.muted,
                soloed: t.soloed,
                volume_db: 20.0 * t.volume.max(1e-9).log10(),
                pan: t.pan,
                items: u32::try_from(items.len()).unwrap_or(u32::MAX),
                fx: daw::service::Effects::list(
                    daw,
                    project.clone(),
                    daw::service::FxChainContext::Track(t.guid.clone()),
                )
                .into_iter()
                .map(|f| {
                    if f.enabled {
                        f.plugin_name
                    } else {
                        format!("{} (bypassed)", f.plugin_name)
                    }
                })
                .collect(),
            })
            .collect::<Vec<_>>())
    });
}

fn register_hits(list: &ActionList) {
    list.register(&DRUM_HITS, Effect::ReadOnly, |a: HitsArg| {
        let (daw, project) = song()?;
        let guid = resolve(daw, &project, &a.track)?;
        let from = a.from.unwrap_or(f64::NEG_INFINITY);
        let to = a.to.unwrap_or(f64::INFINITY);
        let all: Vec<_> =
            crate::fundamental::hits_of(daw, &project, &guid, a.under_db.unwrap_or(24.0))
                .into_iter()
                .filter(|h| h.time >= from && h.time <= to)
                .collect();
        let offset = a.offset.unwrap_or(0);
        let limit = a.limit.unwrap_or(PAGE);
        let hits = all
            .iter()
            .skip(usize::try_from(offset).unwrap_or(usize::MAX))
            .take(usize::try_from(limit).unwrap_or(usize::MAX))
            .map(|h| {
                let (bar, beat, frac) = crate::song_bars::musical(daw, &project, h.time);
                HitRow {
                    time: h.time,
                    bar,
                    beat: f64::from(beat) + frac,
                    level_db: h.level_db,
                }
            })
            .collect();
        Ok(HitsPage {
            total: u32::try_from(all.len()).unwrap_or(u32::MAX),
            offset,
            hits,
        })
    });
}

fn register_fundamental(list: &ActionList) {
    list.register(&DRUM_FUNDAMENTAL, Effect::ReadOnly, |a: FundamentalArg| {
        let (daw, project) = song()?;
        let trigger = resolve(daw, &project, &a.trigger)?;
        let mics = a
            .mics
            .iter()
            .map(|m| resolve(daw, &project, m))
            .collect::<Result<Vec<_>, _>>()?;
        let range = match a.range {
            DrumRange::Kick => crate::fundamental::KICK,
            DrumRange::Snare => crate::fundamental::SNARE,
            DrumRange::Tom => crate::fundamental::TOM,
        };
        crate::fundamental::find(daw, &project, &trigger, &mics, range).ok_or_else(|| {
            "no fundamental: no hits on the trigger, or nothing on the mics at them".to_owned()
        })
    });
}

fn register_track_set(list: &ActionList) {
    list.register(&TRACK_SET, Effect::Mutating, |a: TrackSetArg| {
        let (daw, project) = song()?;
        let guid = resolve(daw, &project, &a.track)?;
        let at = || TrackRef::Guid(guid.clone());
        let failed = |e: daw_proto::DawError| e.to_string();
        if let Some(db) = a.volume_db {
            Tracks::set_volume(daw, project.clone(), at(), 10_f64.powf(db / 20.0))
                .map_err(failed)?;
        }
        if let Some(pan) = a.pan {
            Tracks::set_pan(daw, project.clone(), at(), pan.clamp(-1.0, 1.0)).map_err(failed)?;
        }
        if let Some(mute) = a.mute {
            Tracks::set_muted(daw, project.clone(), at(), mute).map_err(failed)?;
        }
        if let Some(solo) = a.solo {
            Tracks::set_soloed(daw, project, at(), solo).map_err(failed)?;
        }
        crate::actions::list().changed("tracks", &guid);
        Ok(())
    });
}

/// The engine and the open song.
pub(crate) fn song() -> Result<(&'static daw::standalone::Standalone, ProjectContext), String> {
    let daw = crate::open::local_engine().ok_or("no song open in this window's engine")?;
    let guid = crate::open::current_song().ok_or("no song open")?;
    Ok((daw, ProjectContext::Project(guid)))
}

/// A track by its GUID, its path (`Kick/Sum/Trig`, any tail of it) or its
/// name — refused, naming the candidates, when more than one fits.
pub(crate) fn resolve(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    what: &str,
) -> Result<String, String> {
    let tracks = Tracks::all(daw, project.clone());
    if let Some(t) = tracks.iter().find(|t| t.guid.eq_ignore_ascii_case(what)) {
        return Ok(t.guid.clone());
    }
    let by_guid: std::collections::HashMap<&str, &daw_proto::Track> =
        tracks.iter().map(|t| (t.guid.as_str(), t)).collect();
    let want = what.trim().trim_matches('/').to_lowercase();
    let fits: Vec<(&daw_proto::Track, String)> = tracks
        .iter()
        .map(|t| (t, crate::gain_stage::path_of(t, &by_guid).join("/")))
        .filter(|(_, path)| {
            let path = path.to_lowercase();
            path == want || path.ends_with(&format!("/{want}"))
        })
        .collect();
    // A whole path is that track, however many others end the same way:
    // `Drums` is the top-level folder, not `REFERENCES/…/Drums` too.
    let exact: Vec<&(&daw_proto::Track, String)> = fits
        .iter()
        .filter(|(_, p)| p.to_lowercase() == want)
        .collect();
    if let [(only, _)] = exact.as_slice() {
        return Ok(only.guid.clone());
    }
    match fits.as_slice() {
        [(only, _)] => Ok(only.guid.clone()),
        [] => Err(format!("no track {what} — list them with SESSION_TRACKS")),
        many => Err(format!(
            "{what} is more than one track: {:?} — give more of its path",
            many.iter().map(|(_, p)| p.as_str()).collect::<Vec<_>>()
        )),
    }
}

#[cfg(test)]
mod tests {

    /// Ctrl+S — REAPER's "Save project", as the profile binds it — saves
    /// the song: the key's action is the list's `SESSION_SAVE`.
    #[cfg(feature = "native")]
    #[test]
    fn ctrl_s_saves_the_song() {
        let action = crate::keys::action_of("40026");
        assert_eq!(action, crate::keys::Action::Save);
        assert_eq!(
            super::id_and_args(&action).map(|(id, _)| id),
            Some(super::SAVE.id)
        );
    }

    use super::{id_and_args, list, take_window, through_list};
    use crate::keys::{Action, Visibility};

    /// The window's queue is one for the process: these take turns.
    static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Every action a key can produce has an id in the list, and running
    /// it through the list hands the window the same action back — the
    /// key path and a model's are one path.
    #[test]
    fn a_key_and_the_list_are_one_path() {
        use crate::zoom::Command as Z;
        let _turn = TURN.lock();
        let keys = vec![
            Action::PlayStop,
            Action::GainStage { clear: true },
            Action::CompTake(3),
            Action::CursorBar(-1),
            Action::TrackStep {
                by: 1,
                extend: true,
            },
            Action::ToggleScene("overview-top"),
            Action::Visibility(Visibility::Toggle("DRUMS".into())),
            Action::Visibility(Visibility::HideAll),
            Action::View(Z::Step {
                vertical: true,
                inward: false,
            }),
            Action::View(Z::Project),
        ];
        for key in &keys {
            let (id, _) = id_and_args(key).expect("in the list");
            assert!(list().info(id).is_some(), "{id} registered");
        }
        let _ = take_window();
        assert_eq!(through_list(keys.clone()), keys);
        // A binding the window does not do goes straight through.
        let unbound = vec![Action::Unbound("40001".into())];
        assert_eq!(through_list(unbound.clone()), unbound);
    }

    /// A model runs a window action by id, with typed arguments; a scene
    /// the window does not have is refused naming the ones it does.
    #[test]
    fn a_model_runs_window_actions() {
        let _turn = TURN.lock();
        let _ = take_window();
        list()
            .run_json("SESSION_TRACK_STEP", br#"{"by":-2}"#)
            .expect("ran");
        assert_eq!(
            take_window(),
            vec![Action::TrackStep {
                by: -2,
                extend: false
            }]
        );
        let refused = list().run_json("SESSION_SCENE", br#"{"scene":"nope"}"#);
        assert!(refused.is_err_and(|e| e.to_string().contains("overview")));
    }
}

static SLIP: ActionMeta = meta(
    "DRUMS_SLIP",
    "drums_slip",
    "Slip-edit the drums",
    "Slip-edit the whole kit as one: every drum track cut `lead_ms` before each hit and the slices moved together, the seams crossfaded (`xfade_ms`), a gap filled from the slice's own source — no stretching. Runs on the drum-edit settings (DRUMS_EDIT_SETTINGS; tune them with DRUMS_EDIT_SET, watching DRUMS_DETECT), any of which a call may give for itself alone. The hits are the live triggers' (a flam across pieces one moment); each moves toward its target (Guided: the grid line its drum-MIDI note is written on, or the nearest when plainly near; else left and listed in `to_check`). Fills are left as played unless `fills` says otherwise — with `from`/`to` around one and `fills: Only`, a fill is edited with its own settings. Explicit `moves` (`[hit, to]`, seconds) skip detection. `dry_run` says what it would do and does nothing.",
    "Drums",
    "",
);
static FILLS: ActionMeta = meta(
    "DRUMS_FILLS",
    "drums_fills",
    "Find the fills",
    "Where the drummer leaves the groove: beats that, against the groove at that place in the bar in the eight bars before AND the eight after, add toms (`toms`), add three or more hits (`busier`), or play the hands another way where the groove is steady (`different`). Beats in a row are one fill. Fills are played against the grid on purpose, so DRUMS_SLIP leaves them alone unless asked. Uses the drum-edit settings' detector; `from`/`to` narrow it.",
    "Drums",
    "",
);
static EDIT_SETTINGS: ActionMeta = meta(
    "DRUMS_EDIT_SETTINGS",
    "drums_edit_settings",
    "Drum edit settings",
    "The drum-edit settings as they stand — sticky: what every detection, fill search and slip uses until changed, kept across restarts. detector (Level: within `under_db` of a trigger's loudest; Transient: Perfect Timing!'s — the attack outrunning the signal by `sensitivity_db` over `threshold_db`), `retrig_ms`, `offset_ms`, `grid` (a whole note's division), `grid_scan` (one hit per grid line, the loudest within `grid_tolerance_ms`), `target` (Guided, Reference, Grid), `window_ms`, `sure_ms`, `tolerance_ms`, `strength`, `lead_ms`, `xfade_ms`, `fills` (Skip, Include, Only).",
    "Drums",
    "",
);
static EDIT_SET: ActionMeta = meta(
    "DRUMS_EDIT_SET",
    "drums_edit_set",
    "Set drum edit settings",
    "Change any drum-edit settings (the ones given; `reset` first goes back to the defaults) — they stay changed — and get back at once what is detected with them (as DRUMS_DETECT, over `from`/`to`): tune, look, tune again. Giving `threshold_db` or `sensitivity_db` chooses the Transient detector.",
    "Drums",
    "",
);
static DETECT: ActionMeta = meta(
    "DRUMS_DETECT",
    "drums_detect",
    "Detect the drum hits",
    "What the drum-edit settings detect, from `from` to `to` (seconds): each piece's hit count, the kit's moments (a flam across pieces one; with `grid_scan`, one per grid line), the fills, and each moment — time, bar and beat, which pieces, how loud against its piece's loudest, its target and how far off, and what a slip would do with it (`move`, `stay`, `check`: no target near, `fill`: left in a fill). Paged by `offset`/`limit` (100, 300 at most). Settings given here are for this look alone. Fast after the first: the audio is read once and kept until its items change.",
    "Drums",
    "",
);
static PEAKS: ActionMeta = meta(
    "AUDIO_PEAKS",
    "audio_peaks",
    "Audio peaks",
    "A track's audio from `from` to `to` (seconds) as `points` peaks (each the loudest sample in its slice, linear 0..1, through the item volume; 400 unless asked, 4000 at most) — the waveform as numbers, to read a hit, a ring, a stray, a double. `db` gives them in dBFS.",
    "Data",
    "",
);
/// A slip edit.
#[derive(Facet)]
struct SlipArg {
    /// Settings for this slip alone.
    #[facet(flatten, default)]
    with: crate::drum_settings::Patch,
    /// Explicit moves, `[hit, to]` in seconds: no detection, no target.
    moves: Option<Vec<Vec<f64>>>,
    from: Option<f64>,
    to: Option<f64>,
    dry_run: Option<bool>,
}

/// One hit's move.
#[derive(Facet)]
struct SlipMove {
    time: f64,
    bar: i32,
    beat: f64,
    /// Where its target is, ms from it (late positive: the hit is late
    /// when this is negative).
    off_ms: f64,
    /// How far it is moved, ms.
    shift_ms: f64,
}

/// A fill, as a model reads it.
#[derive(Facet)]
struct FillOut {
    from: f64,
    to: f64,
    bar: i32,
    beat: f64,
    beats: u32,
    why: Vec<String>,
}

/// What a slip edit did, or would.
#[derive(Facet)]
struct SlipResult {
    hits: u32,
    moved: u32,
    /// Within tolerance already.
    in_place: u32,
    /// No target near: left as played.
    unmatched: u32,
    /// In a fill, and left as played.
    in_fills: u32,
    /// How far the moved hits were, ms: median and worst.
    median_off_ms: Option<f64>,
    worst_off_ms: Option<f64>,
    applied: bool,
    /// The biggest moves, first.
    biggest: Vec<SlipMove>,
    /// Left as played, with no target near enough to be sure of: to be
    /// listened to (the first 50).
    to_check: Vec<SlipMove>,
    /// The fills found in the span.
    fills: Vec<FillOut>,
}

/// The kit's hits: every live trigger's, merged — hits within this of
/// each other are one moment (a flam, a kick under a snare).
const KIT_MERGE: f64 = 0.015;

/// One live piece of the kit: its name, what it is, its hits.
pub(crate) type KitPiece = (
    String,
    crate::drum_edit::Piece,
    Vec<crate::drum_edit::KitHit>,
);

/// The kit's live pieces and their hits by the settings' detector, each
/// hit's loudness against its piece's loudest. The audio is read once
/// and kept, so this is quick the second time.
pub(crate) fn kit(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    settings: &crate::drum_settings::Settings,
) -> Vec<KitPiece> {
    // The hit markers, when the song has them: found once (HITS_DETECT),
    // corrected by hand, never detected again here
    // (`r[quantize.reads-markers]`).
    if let Some(kit) = crate::hits::kit_from_markers(daw, project) {
        return kit;
    }
    let rate = crate::fundamental::DETECT_RATE;
    let offset = settings.offset_ms / 1_000.0;
    crate::drum_reference::pieces(daw, project)
        .into_iter()
        .filter_map(|(name, trigger)| {
            let audio = crate::fundamental::detect_audio(daw, project, &trigger);
            let found: Vec<crate::drum_edit::Transient> = match settings.detector {
                crate::drum_settings::Detector::Level => crate::drum_edit::levels(
                    &audio,
                    rate,
                    settings.under_db,
                    settings.retrig_ms / 1_000.0,
                ),
                crate::drum_settings::Detector::Transient => {
                    let detect = settings.detect();
                    audio
                        .iter()
                        .flat_map(|(start, samples)| {
                            crate::drum_edit::transients(samples, rate, *start, &detect)
                        })
                        .collect()
                }
            };
            let loudest = found
                .iter()
                .map(|h| h.peak_db)
                .fold(f64::NEG_INFINITY, f64::max);
            // A trigger whose loudest is under the floor is dead — crosstalk
            // on a piezo nobody hit.
            (loudest >= DEAD_TRIGGER_DB).then(|| {
                let hits = found
                    .into_iter()
                    .map(|h| crate::drum_edit::KitHit {
                        time: h.time + offset,
                        relative_db: h.peak_db - loudest,
                    })
                    .collect();
                (name.clone(), crate::drum_edit::Piece::of(&name), hits)
            })
        })
        .collect()
}

/// Every beat from the bar holding `from` to the one holding `to`.
fn beats(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    (from, to): (f64, f64),
) -> Vec<crate::drum_edit::Beat> {
    let (mut bar, _, _) = daw.time_to_musical(project.clone(), from.max(0.0));
    let mut starts: Vec<(f64, u32)> = Vec::new();
    loop {
        let downbeat = daw.musical_to_time(project.clone(), bar, 1, 0.0);
        let (per_bar, _) = daw.get_time_signature_at(project.clone(), downbeat);
        for beat in 1..=per_bar.max(1) {
            let at = daw.musical_to_time(project.clone(), bar, beat, 0.0);
            starts.push((at, u32::try_from(beat.saturating_sub(1)).unwrap_or(0)));
        }
        bar = bar.saturating_add(1);
        if downbeat > to || starts.len() > 100_000 {
            break;
        }
    }
    starts
        .iter()
        .zip(starts.iter().skip(1))
        .map(|(&(start, in_bar), &(next, _))| crate::drum_edit::Beat {
            start,
            length: (next - start).max(1e-3),
            in_bar,
        })
        .collect()
}

/// The fills among the kit's hits, from `from` to `to`.
fn find_fills(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    kit: &[KitPiece],
    (from, to): (f64, f64),
) -> Vec<crate::drum_edit::Fill> {
    let hits: Vec<(crate::drum_edit::Piece, f64)> = kit
        .iter()
        .flat_map(|(_, piece, hits)| hits.iter().map(|h| (*piece, h.time)))
        .filter(|(_, t)| *t >= from && *t <= to)
        .collect();
    let Some(first) = hits.iter().map(|h| h.1).reduce(f64::min) else {
        return Vec::new();
    };
    let last = hits.iter().map(|h| h.1).fold(first, f64::max);
    crate::drum_edit::fills(&hits, &beats(daw, project, (first, last)))
}

/// A fill for a model: where, in bars and beats, and why.
fn fill_out(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    fill: &crate::drum_edit::Fill,
) -> FillOut {
    let (bar, beat, frac) = crate::song_bars::musical(daw, &project, fill.from + 1e-6);
    FillOut {
        from: fill.from,
        to: fill.to,
        bar,
        beat: f64::from(beat) + frac,
        beats: fill.beats,
        why: fill.why.iter().map(|w| (*w).to_owned()).collect(),
    }
}

/// One moment of the kit: when, which pieces, how loud the loudest of
/// them against its own.
#[derive(Clone, Debug)]
struct Moment {
    time: f64,
    pieces: Vec<String>,
    relative_db: f64,
}

/// The kit's moments: one hit per grid line (the loudest), or every hit
/// with flams merged to their first.
fn moments(kit: &[KitPiece], scan: Option<(&dyn Fn(f64) -> f64, f64)>) -> Vec<Moment> {
    let mut hits: Vec<(&str, crate::drum_edit::KitHit)> = kit
        .iter()
        .flat_map(|(name, _, h)| h.iter().map(move |h| (name.as_str(), *h)))
        .collect();
    hits.sort_by(|a, b| a.1.time.total_cmp(&b.1.time));
    let mut out: Vec<Moment> = Vec::new();
    for (name, hit) in &hits {
        match out.last_mut() {
            Some(m) if hit.time - m.time <= KIT_MERGE => {
                if !m.pieces.iter().any(|p| p == name) {
                    m.pieces.push((*name).to_owned());
                }
                m.relative_db = m.relative_db.max(hit.relative_db);
            }
            _ => out.push(Moment {
                time: hit.time,
                pieces: vec![(*name).to_owned()],
                relative_db: hit.relative_db,
            }),
        }
    }
    let Some((line_of, tolerance)) = scan else {
        return out;
    };
    let as_hits: Vec<crate::drum_edit::KitHit> = out
        .iter()
        .map(|m| crate::drum_edit::KitHit {
            time: m.time,
            relative_db: m.relative_db,
        })
        .collect();
    let kept = crate::drum_edit::grid_scan(&as_hits, line_of, tolerance);
    out.retain(|m| kept.iter().any(|k| (k.time - m.time).abs() < 1e-9));
    out
}

/// The nearest of `written` (sorted) within `window` of `h`.
fn nearest(written: &[f64], h: f64, window: f64) -> Option<f64> {
    let at = written.partition_point(|n| *n < h - window);
    written
        .iter()
        .skip(at)
        .take_while(|n| **n <= h + window)
        .min_by(|x, y| (*x - h).abs().total_cmp(&(*y - h).abs()))
        .copied()
}

/// The grid line nearest `h`, at `division` of a whole note.
fn grid_line(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    h: f64,
    division: u32,
) -> f64 {
    let step = 240.0 / daw.get_tempo_at(project.clone(), h).max(1.0) / f64::from(division.max(1));
    (h / step).round() * step
}

/// Each moment's target, `None` when none is near enough to be sure of.
fn targets(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    settings: &crate::drum_settings::Settings,
    kit: &[KitPiece],
    moments: &[f64],
) -> Result<Vec<(f64, Option<f64>)>, String> {
    use crate::drum_settings::Target;
    if settings.target == Target::Grid {
        return Ok(moments
            .iter()
            .map(|&h| (h, Some(grid_line(daw, project, h, settings.grid))))
            .collect());
    }
    // The drum MIDI, and which of its notes are which piece — learned
    // against these hits.
    let midi = crate::drum_reference::find_track(daw, project)
        .ok_or("no drum MIDI track to guide by — use `target: Grid`")?;
    let notes = crate::drum_reference::notes(daw, project, &midi);
    let named: Vec<(String, Vec<f64>)> = kit
        .iter()
        .map(|(n, _, h)| (n.clone(), h.iter().map(|h| h.time).collect()))
        .collect();
    let map = crate::drum_reference::learn_map(&notes, &named, NOTE_WINDOW);
    let mapped: Vec<u8> = map
        .iter()
        .filter(|m| m.piece.is_some())
        .map(|m| m.pitch)
        .collect();
    let mut written: Vec<f64> = notes
        .iter()
        .filter(|n| mapped.contains(&n.pitch))
        .map(|n| n.time)
        .collect();
    written.sort_by(f64::total_cmp);
    written.dedup_by(|x, y| (*x - *y).abs() < 0.002);
    let window = settings.window_ms / 1_000.0;
    let sure = settings.sure_ms / 1_000.0;
    Ok(moments
        .iter()
        .map(|&h| {
            let note = nearest(&written, h, window);
            let to = match settings.target {
                // The MIDI is quantized: its note IS the grid line it was
                // written on — straight or a triplet — which is all it is
                // asked here.
                Target::Guided => note.or_else(|| {
                    Some(grid_line(daw, project, h, settings.grid))
                        .filter(|g| (g - h).abs() <= sure)
                }),
                _ => note,
            };
            (h, to)
        })
        .collect())
}

/// Moments nearer than this are one gesture, moved as one, seconds:
/// under a 1/32 at 160 bpm (47 ms), so a double kick's 16ths and a
/// drummer's 32nds stay their own.
const GESTURE: f64 = 0.040;

/// Each gesture — moments nearer together than [`GESTURE`]: a kick into
/// a snare, a drag, a flam across pieces — moved by one shift, its
/// loudest targeted moment's, so the gesture keeps its own timing; none
/// when no moment of it has a target.
///
/// Moved apart, a slice moved later reaches back into its own source to
/// close the gap, and with the hit before it nearer than that reach, it
/// plays that hit's attack again: a kick 19 ms before a snare, the snare
/// sent 18.8 ms later on its own, was heard twice on Unbreakable.
fn hold_gestures(pairs: &[(f64, Option<f64>)], loudness: &[f64]) -> Vec<(f64, Option<f64>)> {
    let mut out: Vec<(f64, Option<f64>)> = Vec::with_capacity(pairs.len());
    let mut start = 0_usize;
    while start < pairs.len() {
        let mut end = start.saturating_add(1);
        while let (Some(prev), Some(next)) = (pairs.get(end.saturating_sub(1)), pairs.get(end)) {
            if next.0 - prev.0 >= GESTURE {
                break;
            }
            end = end.saturating_add(1);
        }
        let group = pairs.get(start..end).unwrap_or_default();
        let shift = group
            .iter()
            .enumerate()
            .filter_map(|(k, (h, t))| {
                let t = (*t)?;
                Some((
                    loudness
                        .get(start.saturating_add(k))
                        .copied()
                        .unwrap_or(0.0),
                    t - h,
                ))
            })
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, shift)| shift);
        out.extend(group.iter().map(|(h, _)| (*h, shift.map(|s| h + s))));
        start = end;
    }
    out
}

/// What the settings make of the kit: its pieces, its moments (with
/// their targets), its fills.
struct Detected {
    kit: Vec<KitPiece>,
    moments: Vec<Moment>,
    targets: Vec<(f64, Option<f64>)>,
    fills: Vec<crate::drum_edit::Fill>,
}

fn detect(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    settings: &crate::drum_settings::Settings,
    span: (f64, f64),
) -> Result<Detected, String> {
    let kit = kit(daw, project, settings);
    let line_of = |h: f64| grid_line(daw, project, h, settings.grid);
    let by_line: &dyn Fn(f64) -> f64 = &line_of;
    let tolerance = settings
        .grid_tolerance_ms
        .map_or(f64::INFINITY, |ms| ms / 1_000.0);
    let moments = moments(&kit, settings.grid_scan.then_some((by_line, tolerance)));
    let times: Vec<f64> = moments.iter().map(|m| m.time).collect();
    let loudness: Vec<f64> = moments.iter().map(|m| m.relative_db).collect();
    let targets = hold_gestures(&targets(daw, project, settings, &kit, &times)?, &loudness);
    let fills = find_fills(daw, project, &kit, span);
    Ok(Detected {
        kit,
        moments,
        targets,
        fills,
    })
}

/// Whether the hit at `h` is in one of `fills`.
fn in_a_fill(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    fills: &[crate::drum_edit::Fill],
    h: f64,
) -> bool {
    let beat = 60.0 / daw.get_tempo_at(project.clone(), h).max(1.0);
    fills.iter().any(|f| f.holds(h, beat))
}

static QUANTIZE: ActionMeta = meta(
    "QUANTIZE",
    "quantize",
    "Quantize the drums",
    "Quantize the kit from its hit markers (HITS_DETECT first): every drum track cut `lead_ms` before each hit and moved together so the HIT lands on its target — the item starts before the beat, the attack is on it — the seams crossfaded (`xfade_ms`). Over `sections` (by name, from the SECTIONS lane: `Verse 1`, `Chorus`) or `from`/`to` (seconds), each with these settings — so a verse and a chorus can each be quantized their own way. The edit zones (ZONES_LIST) are left as played; the drum-edit settings (grid, strength, target, tolerance, lead, crossfade…) can be given for this call alone. `dry_run` says what it would do. After an edit, a check from the markers: hits lost, hits doubled (a double attack), and how far the moved hits landed from where they were sent.",
    "Drums",
    "",
);
static ZONES_FIND: ActionMeta = meta(
    "ZONES_FIND",
    "zones_find",
    "Find the fills",
    "Find where the drummer leaves the groove, from the hit markers, and propose each fill as an `auto` zone on the ruler's Drums lane, in `from`/`to` (seconds; the whole song when left out): the auto zones there replaced, none laid over one already decided. Approve or reject them with ZONES_EDIT.",
    "Drums",
    "",
);

#[derive(Facet)]
struct QuantizeArg {
    /// Settings for this quantize alone.
    #[facet(flatten, default)]
    with: crate::drum_settings::Patch,
    #[facet(default)]
    sections: Vec<String>,
    from: Option<f64>,
    to: Option<f64>,
    dry_run: Option<bool>,
}

/// What a quantize did over one stretch.
#[derive(Facet)]
struct QuantizeSpan {
    /// The section's name, or `from`–`to`.
    name: String,
    from: f64,
    to: f64,
    result: SlipResult,
    /// How the edit left the hits (none for a dry run).
    check: Option<crate::hits::Check>,
}

/// The stretches a quantize covers: each named section (as the SECTIONS
/// lane has it), or the span given.
fn quantize_spans(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    a: &QuantizeArg,
) -> Result<Vec<(String, f64, f64)>, String> {
    if a.sections.is_empty() {
        let (from, to) = (a.from.unwrap_or(0.0), a.to.unwrap_or(f64::INFINITY));
        return Ok(vec![(format!("{from:.2}–{to:.2}"), from, to)]);
    }
    let sections: Vec<daw_proto::Region> = daw::service::Regions::all(daw, project.clone())
        .into_iter()
        .filter(|r| {
            r.lane
                .is_none_or(|l| l == session::ruler_lanes::CoreLane::Sections.lane_index())
        })
        .collect();
    a.sections
        .iter()
        .map(|want| {
            sections
                .iter()
                .find(|r| r.name.trim().eq_ignore_ascii_case(want.trim()))
                .map(|r| {
                    (
                        r.name.clone(),
                        r.time_range.start_seconds(),
                        r.time_range.end_seconds(),
                    )
                })
                .ok_or_else(|| {
                    format!(
                        "no section {want}: one of {:?}",
                        sections.iter().map(|r| &r.name).collect::<Vec<_>>()
                    )
                })
        })
        .collect()
}

/// Wait for a queued slip to reach the engine: until the kit's hits play
/// somewhere other than `before`, then a moment more for the rest of its
/// steps. At most a few seconds.
fn wait_for_slip(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    before: &[crate::hits::Played],
) {
    let print = |p: &[crate::hits::Played]| -> Vec<(String, i64)> {
        p.iter()
            .map(|h| {
                (
                    h.id.clone(),
                    num_traits::ToPrimitive::to_i64(&(h.time * 1e6).round()).unwrap_or(0),
                )
            })
            .collect()
    };
    let was = print(before);
    let started = std::time::Instant::now();
    let mut last = was.clone();
    let mut steady_since: Option<std::time::Instant> = None;
    while started.elapsed() < std::time::Duration::from_secs(8) {
        std::thread::sleep(std::time::Duration::from_millis(100));
        let now = print(&crate::hits::playback(daw, project));
        if now == was {
            continue;
        }
        if now == last {
            if steady_since.is_some_and(|t| t.elapsed() >= std::time::Duration::from_millis(300)) {
                return;
            }
            steady_since.get_or_insert_with(std::time::Instant::now);
        } else {
            steady_since = None;
            last = now;
        }
    }
}

#[cfg(test)]
mod gesture_tests {
    use super::hold_gestures;

    #[test]
    fn a_kick_into_a_snare_moves_with_it() {
        // Kick unmatched, snare 19 ms after it sent 18.8 ms later.
        let pairs = [(1.000, None), (1.019, Some(1.0378)), (1.500, Some(1.500))];
        let got = hold_gestures(&pairs, &[-6.0, 0.0, 0.0]);
        let shift = |k: usize| got[k].1.map(|t| t - got[k].0);
        assert!((shift(0).unwrap() - 0.0188).abs() < 1e-9, "{got:?}");
        assert!((shift(1).unwrap() - 0.0188).abs() < 1e-9);
        assert_eq!(got[2], (1.5, Some(1.5)), "a moment apart keeps its own");
    }

    #[test]
    fn a_gesture_with_no_target_stays() {
        let got = hold_gestures(&[(1.0, None), (1.02, None)], &[0.0, 0.0]);
        assert_eq!(got, vec![(1.0, None), (1.02, None)]);
    }

    #[test]
    fn the_louder_targeted_moment_decides() {
        let got = hold_gestures(&[(1.0, Some(1.01)), (1.03, Some(1.02))], &[-12.0, 0.0]);
        assert!((got[0].1.unwrap() - 0.99).abs() < 1e-9, "{got:?}");
        assert!((got[1].1.unwrap() - 1.02).abs() < 1e-9);
    }
}

fn register_quantize(list: &ActionList) {
    list.register(&QUANTIZE, Effect::Mutating, |a: QuantizeArg| {
        let (daw, project) = song()?;
        if crate::hits::kit_from_markers(daw, &project).is_none() {
            return Err("no hit markers: find the hits first (HITS_DETECT)".to_owned());
        }
        let settings = crate::drum_settings::current().with(&a.with);
        let dry = a.dry_run.unwrap_or(false);
        let zones = crate::hits::zones(daw, &project);
        let mut out: Vec<QuantizeSpan> = Vec::new();
        for (name, from, to) in quantize_spans(daw, &project, &a)? {
            let found = detect(daw, &project, &settings, (from, to))?;
            let inside = |h: f64| h >= from && h < to;
            let held = |h: f64| {
                if zones.is_empty() {
                    in_a_fill(daw, &project, &found.fills, h)
                } else {
                    crate::hits::zone_holds(
                        &zones,
                        h,
                        60.0 / daw.get_tempo_at(project.clone(), h).max(1.0),
                    )
                }
            };
            let pairs: Vec<(f64, Option<f64>)> = found
                .targets
                .iter()
                .filter(|(h, _)| inside(*h) && !held(*h))
                .copied()
                .collect();
            let left_out: Vec<f64> = found
                .targets
                .iter()
                .filter(|(h, _)| inside(*h) && held(*h))
                .map(|(h, _)| *h)
                .collect();
            let before = crate::hits::playback(daw, &project);
            let mut result = judge(daw, &project, &settings, (&pairs, &left_out), dry);
            result.in_fills = u32::try_from(left_out.len()).unwrap_or(u32::MAX);
            let check = result.applied.then(|| {
                wait_for_slip(daw, &project, &before);
                // Where each moved hit was sent: its target, or as far
                // toward it as the strength takes it.
                let tolerance = settings.tolerance_ms / 1_000.0;
                let sent: Vec<f64> = pairs
                    .iter()
                    .filter_map(|(h, t)| {
                        let t = (*t)?;
                        ((t - h).abs() > tolerance).then(|| (t - h).mul_add(settings.strength, *h))
                    })
                    .collect();
                crate::hits::check(daw, &project, &before, &sent, (from, to))
            });
            out.push(QuantizeSpan {
                name,
                from,
                to,
                result,
                check,
            });
        }
        Ok(out)
    });
    list.register(&ZONES_FIND, Effect::Mutating, |a: SpanArg| {
        let (daw, project) = song()?;
        let settings = crate::drum_settings::current().with(&a.with);
        let kit = kit(daw, &project, &settings);
        let span = a.span();
        let fills = find_fills(daw, &project, &kit, span);
        let proposed = crate::hits::find_zones(daw, &project, &fills, span)?;
        Ok(proposed)
    });
}

fn register_slip(list: &ActionList) {
    register_quantize(list);
    list.register(&SLIP, Effect::Mutating, |a: SlipArg| {
        let (daw, project) = song()?;
        let settings = crate::drum_settings::current().with(&a.with);
        let span = (
            a.from.unwrap_or(f64::NEG_INFINITY),
            a.to.unwrap_or(f64::INFINITY),
        );
        let (pairs, fills) = if let Some(moves) = &a.moves {
            let pairs = moves
                .iter()
                .filter_map(|m| Some((*m.first()?, Some(*m.get(1)?))))
                .collect();
            (pairs, Vec::new())
        } else {
            let found = detect(daw, &project, &settings, span)?;
            (found.targets, found.fills)
        };
        let mut in_fills = 0_u32;
        // The hits left out (a fill's, while fills are skipped): still cut
        // where they are, so the slice before them moving does not drag
        // them with it.
        let mut left_out: Vec<f64> = Vec::new();
        // The song's edit zones decide what a fill is, once it has any.
        let zones = crate::hits::zones(daw, &project);
        let pairs: Vec<(f64, Option<f64>)> = pairs
            .into_iter()
            .filter(|(h, _)| *h >= span.0 && *h <= span.1)
            .filter(|(h, _)| {
                let held = if zones.is_empty() {
                    in_a_fill(daw, &project, &fills, *h)
                } else {
                    crate::hits::zone_holds(
                        &zones,
                        *h,
                        60.0 / daw.get_tempo_at(project.clone(), *h).max(1.0),
                    )
                };
                let keep = match settings.fills {
                    crate::drum_settings::Fills::Skip => !held,
                    crate::drum_settings::Fills::Include => true,
                    crate::drum_settings::Fills::Only => held,
                };
                if held && !keep {
                    in_fills = in_fills.saturating_add(1);
                }
                if !keep {
                    left_out.push(*h);
                }
                keep
            })
            .collect();
        let anchors = if a.moves.is_some() {
            Vec::new()
        } else {
            left_out
        };
        let mut result = judge(
            daw,
            &project,
            &settings,
            (&pairs, &anchors),
            a.dry_run.unwrap_or(false),
        );
        result.in_fills = in_fills;
        result.fills = fills.iter().map(|f| fill_out(daw, &project, f)).collect();
        Ok(result)
    });
    list.register(&FILLS, Effect::ReadOnly, |a: SpanArg| {
        let (daw, project) = song()?;
        let settings = crate::drum_settings::current().with(&a.with);
        let kit = kit(daw, &project, &settings);
        Ok(find_fills(daw, &project, &kit, a.span())
            .iter()
            .map(|f| fill_out(daw, &project, f))
            .collect::<Vec<FillOut>>())
    });
    list.register(&EDIT_SETTINGS, Effect::ReadOnly, |(): ()| {
        Ok(crate::drum_settings::current())
    });
    list.register(&EDIT_SET, Effect::Mutating, |a: SetArg| {
        if a.reset.unwrap_or(false) {
            crate::drum_settings::reset();
        }
        let settings = crate::drum_settings::set(&a.with);
        crate::actions::list().changed("drums.settings", "the drum edit settings changed");
        let (daw, project) = song()?;
        let detected = detected_out(daw, &project, &settings, &a.look)?;
        Ok(SetResult { settings, detected })
    });
    list.register(&DETECT, Effect::ReadOnly, |a: DetectArg| {
        let (daw, project) = song()?;
        let settings = crate::drum_settings::current().with(&a.with);
        detected_out(daw, &project, &settings, &a.look)
    });
}

/// Where to look.
#[derive(Facet, Default)]
struct Look {
    from: Option<f64>,
    to: Option<f64>,
    /// The first moment shown, and how many (100).
    offset: Option<u32>,
    limit: Option<u32>,
}

/// Where, with settings for this look alone.
#[derive(Facet)]
struct SpanArg {
    #[facet(flatten, default)]
    with: crate::drum_settings::Patch,
    from: Option<f64>,
    to: Option<f64>,
}

impl SpanArg {
    fn span(&self) -> (f64, f64) {
        (
            self.from.unwrap_or(f64::NEG_INFINITY),
            self.to.unwrap_or(f64::INFINITY),
        )
    }
}

/// A look at the detection, with settings for it alone.
#[derive(Facet)]
struct DetectArg {
    #[facet(flatten, default)]
    with: crate::drum_settings::Patch,
    #[facet(flatten, default)]
    look: Look,
}

/// Settings to keep, and where to look with them.
#[derive(Facet)]
struct SetArg {
    #[facet(flatten, default)]
    with: crate::drum_settings::Patch,
    /// Back to the defaults first.
    reset: Option<bool>,
    #[facet(flatten, default)]
    look: Look,
}

/// The settings now, and what they detect.
#[derive(Facet)]
struct SetResult {
    settings: crate::drum_settings::Settings,
    detected: DetectedOut,
}

/// One piece's count.
#[derive(Facet)]
struct PieceCount {
    piece: String,
    hits: u32,
}

/// One moment, as a slip would see it.
#[derive(Facet)]
struct MomentOut {
    time: f64,
    bar: i32,
    beat: f64,
    pieces: Vec<String>,
    /// Against its piece's loudest, dB.
    level_db: f64,
    target: Option<f64>,
    /// Its target, ms from it.
    off_ms: Option<f64>,
    /// `move`, `stay`, `check` or `fill`.
    slip: String,
}

/// What the settings detect, in a span.
#[derive(Facet)]
struct DetectedOut {
    pieces: Vec<PieceCount>,
    moments: u32,
    would_move: u32,
    would_stay: u32,
    to_check: u32,
    in_fills: u32,
    fills: Vec<FillOut>,
    /// A page of them.
    shown: Vec<MomentOut>,
    /// How long it took, ms.
    took_ms: u32,
}

fn detected_out(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    settings: &crate::drum_settings::Settings,
    look: &Look,
) -> Result<DetectedOut, String> {
    let started = std::time::Instant::now();
    let span = (
        look.from.unwrap_or(f64::NEG_INFINITY),
        look.to.unwrap_or(f64::INFINITY),
    );
    // The whole song, so the arrangement can draw all of it; the answer
    // is the span's.
    let found = detect(daw, project, settings, (f64::NEG_INFINITY, f64::INFINITY))?;
    let inside = |t: f64| t >= span.0 && t <= span.1;
    let tolerance = settings.tolerance_ms / 1_000.0;
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    let mut all: Vec<MomentOut> = Vec::new();
    let mut drawn = crate::detect_overlay::Overlay {
        lines: Vec::with_capacity(found.moments.len()),
        fills: found.fills.iter().map(|f| (f.from, f.to)).collect(),
    };
    for (moment, (_, target)) in found.moments.iter().zip(&found.targets) {
        use crate::detect_overlay::Mark;
        let held = in_a_fill(daw, project, &found.fills, moment.time);
        let mark = match (target, settings.fills) {
            (_, crate::drum_settings::Fills::Skip) if held => Mark::Fill,
            (_, crate::drum_settings::Fills::Only) if !held => Mark::Fill,
            (None, _) => Mark::Check,
            (Some(t), _) if (t - moment.time).abs() <= tolerance => Mark::Stay,
            (Some(_), _) => Mark::Move,
        };
        drawn.lines.push(crate::detect_overlay::Line {
            time: moment.time,
            target: *target,
            mark,
        });
        if !inside(moment.time) {
            continue;
        }
        let slip = match mark {
            Mark::Fill => "fill",
            Mark::Check => "check",
            Mark::Stay => "stay",
            Mark::Move => "move",
        };
        let (bar, beat, frac) = crate::song_bars::musical(daw, &project, moment.time);
        all.push(MomentOut {
            time: moment.time,
            bar,
            beat: f64::from(beat) + frac,
            pieces: moment.pieces.clone(),
            level_db: (moment.relative_db * 10.0).round() / 10.0,
            target: *target,
            off_ms: target.map(|t| ((t - moment.time) * 10_000.0).round() / 10.0),
            slip: slip.to_owned(),
        });
    }
    crate::detect_overlay::publish(drawn);
    let tally = |what: &str| count(all.iter().filter(|m| m.slip == what).count());
    let offset = usize::try_from(look.offset.unwrap_or(0)).unwrap_or(0);
    // A page fits in one reply (the MCP's text cap): 300 at most.
    let limit = usize::try_from(look.limit.unwrap_or(100).min(300)).unwrap_or(100);
    Ok(DetectedOut {
        pieces: found
            .kit
            .iter()
            .map(|(name, _, hits)| PieceCount {
                piece: name.clone(),
                hits: count(hits.iter().filter(|h| inside(h.time)).count()),
            })
            .collect(),
        moments: count(all.len()),
        would_move: tally("move"),
        would_stay: tally("stay"),
        to_check: tally("check"),
        in_fills: tally("fill"),
        fills: found
            .fills
            .iter()
            .filter(|f| f.to >= span.0 && f.from <= span.1)
            .map(|f| fill_out(daw, project, f))
            .collect(),
        shown: all.into_iter().skip(offset).take(limit).collect(),
        took_ms: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    })
}

/// The moves `pairs` (hit, target) make, and — unless a dry run — the
/// slip queued for the window.
fn judge(
    daw: &daw::standalone::Standalone,
    project: &ProjectContext,
    settings: &crate::drum_settings::Settings,
    (pairs, left_out): (&[(f64, Option<f64>)], &[f64]),
    dry: bool,
) -> SlipResult {
    let tolerance = settings.tolerance_ms / 1_000.0;
    let strength = settings.strength;
    let at = |hit: f64, off: f64, shift: f64| {
        let (bar, beat, frac) = crate::song_bars::musical(daw, &project, hit);
        SlipMove {
            time: hit,
            bar,
            beat: f64::from(beat) + frac,
            off_ms: off * 1_000.0,
            shift_ms: shift * 1_000.0,
        }
    };
    let to_check: Vec<SlipMove> = pairs
        .iter()
        .filter(|(_, t)| t.is_none())
        .take(50)
        .map(|(h, _)| at(*h, 0.0, 0.0))
        .collect();
    let mut moves: Vec<(f64, f64, f64)> = Vec::new();
    let (mut in_place, mut unmatched) = (0_u32, 0_u32);
    for (hit, target) in pairs {
        match target {
            None => unmatched = unmatched.saturating_add(1),
            Some(t) if (t - hit).abs() <= tolerance => in_place = in_place.saturating_add(1),
            Some(t) => moves.push((*hit, t - hit, (t - hit) * strength)),
        }
    }
    let mut offs: Vec<f64> = moves.iter().map(|m| m.1 * 1_000.0).collect();
    offs.sort_by(|x, y| x.abs().total_cmp(&y.abs()));
    let mut biggest: Vec<&(f64, f64, f64)> = moves.iter().collect();
    biggest.sort_by(|x, y| y.1.abs().total_cmp(&x.1.abs()));
    if !dry && !moves.is_empty() {
        let us = |s: f64| num_traits::ToPrimitive::to_i64(&(s * 1_000_000.0).round()).unwrap_or(0);
        // Every hit not moved is cut where it is, unmoved: a hit left in a
        // slice rides with it, and the slice before it moving would take
        // it along — off its mark by that hit's shift.
        let mut cuts: Vec<(i64, i64)> = moves
            .iter()
            .map(|(h, _, shift)| (us(*h), us(*shift)))
            .chain(
                pairs
                    .iter()
                    .filter(|(h, t)| t.is_none_or(|t| (t - h).abs() <= tolerance))
                    .map(|(h, _)| (us(*h), 0))
                    .chain(left_out.iter().map(|h| (us(*h), 0))),
            )
            .collect();
        cuts.sort_unstable();
        queue(Action::SlipDrums {
            moves: cuts,
            pre_roll_us: us(settings.lead_ms / 1_000.0),
            crossfade_us: us(settings.xfade_ms / 1_000.0),
            stretch_over_us: settings.stretch_gaps_ms.map_or(0, |ms| us(ms / 1_000.0)),
        });
    }
    let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
    SlipResult {
        hits: count(pairs.len()),
        moved: count(moves.len()),
        in_place,
        unmatched,
        in_fills: 0,
        median_off_ms: offs.get(offs.len() / 2).copied(),
        worst_off_ms: offs.last().copied(),
        applied: !dry && !moves.is_empty(),
        biggest: biggest
            .into_iter()
            .take(30)
            .map(|(hit, off, shift)| at(*hit, *off, *shift))
            .collect(),
        to_check,
        fills: Vec::new(),
    }
}

/// Which audio.
#[derive(Facet)]
struct PeaksArg {
    track: String,
    from: f64,
    to: f64,
    points: Option<u32>,
    db: Option<bool>,
}

/// A track's audio as peaks.
#[derive(Facet)]
struct PeaksResult {
    from: f64,
    /// Seconds a point stands for.
    step: f64,
    peaks: Vec<f64>,
}

fn register_peaks(list: &ActionList) {
    list.register(&PEAKS, Effect::ReadOnly, |a: PeaksArg| {
        let (daw, project) = song()?;
        let guid = resolve(daw, &project, &a.track)?;
        if a.to <= a.from {
            return Err("`to` must be after `from`".to_owned());
        }
        let points = a.points.unwrap_or(400).clamp(1, 4_000);
        let n = usize::try_from(points).unwrap_or(400);
        let mut peaks = crate::fundamental::envelope(daw, &project, &guid, (a.from, a.to), n);
        if a.db.unwrap_or(false) {
            for p in &mut peaks {
                *p = (20.0 * p.max(1e-6).log10() * 10.0).round() / 10.0;
            }
        } else {
            for p in &mut peaks {
                *p = (*p * 10_000.0).round() / 10_000.0;
            }
        }
        Ok(PeaksResult {
            from: a.from,
            step: (a.to - a.from) / f64::from(points),
            peaks,
        })
    });
}
