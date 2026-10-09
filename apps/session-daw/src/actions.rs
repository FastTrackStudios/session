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
    WINDOW
        .lock()
        .map(|mut q| q.drain(..).collect())
        .unwrap_or_default()
}

/// Whether any are waiting.
pub fn window_pending() -> bool {
    WINDOW.lock().is_ok_and(|q| !q.is_empty())
}

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
        Action::Unbound(_) => return None,
    })
}

/// An action's description, as the list and its menus show it.
const fn meta(
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

fn register_data(list: &ActionList) {
    register_tracks(list);
    register_hits(list);
    register_fundamental(list);
    register_track_set(list);
    register_reference(list);
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
    register_reference_map(list);
    register_reference_notes(list);
    register_compare(list);
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
                let (bar, beat, frac) = daw.time_to_musical(project.clone(), n.time);
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
            let (bar, beat, frac) = daw.time_to_musical(project.clone(), t);
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
            .map(|(t, path)| TrackRow {
                guid: t.guid.clone(),
                name: t.name.clone(),
                path,
                folder: t.is_folder,
                muted: t.muted,
                soloed: t.soloed,
                volume_db: 20.0 * t.volume.max(1e-9).log10(),
                pan: t.pan,
                items: u32::try_from(
                    daw::service::Items::get_items(
                        daw,
                        project.clone(),
                        TrackRef::Guid(t.guid.clone()),
                    )
                    .len(),
                )
                .unwrap_or(u32::MAX),
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
                let (bar, beat, frac) = daw.time_to_musical(project.clone(), h.time);
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
fn song() -> Result<(&'static daw::standalone::Standalone, ProjectContext), String> {
    let daw = crate::open::local_engine().ok_or("no song open in this window's engine")?;
    let guid = crate::open::current_song().ok_or("no song open")?;
    Ok((daw, ProjectContext::Project(guid)))
}

/// A track by its GUID, its path (`Kick/Sum/Trig`, any tail of it) or its
/// name — refused, naming the candidates, when more than one fits.
fn resolve(
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
