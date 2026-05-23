//! Session Services Library
//!
//! This crate provides session management services for FTS Control.
//! Services can be used either:
//! - As a cell (via `main.rs` and `run_cell!` macro)
//! - In-process (by importing and instantiating directly)
//!
//! # In-Process Usage
//!
//! ```rust,ignore
//! use session::SessionServices;
//! use daw::Services;
//!
//! // Create a router containing the canonical session service bundle.
//! let router = SessionServices.into_router();
//! ```

// Re-export session-proto so app crates can use `session::` instead of `session_proto::` directly.
pub use session_proto::*;
pub use session_proto::{offset_map, ruler_lanes, services, setlist, song, track_structure};

// Server-side modules — these use moire::sync, tokio, and Daw::get() which
// are not available on wasm32. The web app only needs session-proto types
// (re-exported above) and the action declarations (below).
#[cfg(not(target_arch = "wasm32"))]
pub mod cache;
#[cfg(not(target_arch = "wasm32"))]
pub mod event_bus;
#[cfg(not(target_arch = "wasm32"))]
mod keyflow_actions;
#[cfg(not(target_arch = "wasm32"))]
mod preroll_actions;
#[cfg(not(target_arch = "wasm32"))]
pub mod routing_project;
#[cfg(not(target_arch = "wasm32"))]
mod setlist_builder;
#[cfg(not(target_arch = "wasm32"))]
mod setlist_service;
#[cfg(not(target_arch = "wasm32"))]
mod song_builder;
#[cfg(not(target_arch = "wasm32"))]
mod song_service;

// Re-export service implementations for library use
#[cfg(not(target_arch = "wasm32"))]
pub use setlist_service::SetlistServiceImpl;
#[cfg(not(target_arch = "wasm32"))]
pub use song_service::SongServiceImpl;

// Re-export builders for advanced use cases
#[cfg(not(target_arch = "wasm32"))]
pub use setlist_builder::SetlistBuilder;
#[cfg(not(target_arch = "wasm32"))]
pub use song_builder::SongBuilder;

// Re-export demo setlist stamping (for extensions that have a local Daw instance)
#[cfg(not(target_arch = "wasm32"))]
pub use setlist_service::demo::{stamp_demo_into_project, stamp_demo_setlist};

pub mod auto_color_actions;
pub mod daw_module;
pub mod mode_actions;
pub mod track_manager_actions;

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct SessionServices;

#[cfg(not(target_arch = "wasm32"))]
impl SessionServices {
    pub fn mounted_services_with_daw<D>(daw: D) -> Vec<daw::Mounted>
    where
        D: Clone
            + daw::service::AudioEngine
            + daw::service::ExtState
            + daw::service::Projects
            + daw::service::TempoMap
            + daw::service::transport::service::Transport
            + Send
            + Sync
            + 'static,
    {
        use keyflow_daw_analysis::{
            KeyflowMidiAnalysis, MidiChartServiceDispatcher, midi_chart_service_service_descriptor,
        };

        vec![
            daw::Mounted::new(
                &setlist_service_service_descriptor(),
                serve_setlist_service(SetlistServiceImpl::with_daw(daw.clone())),
            ),
            daw::Mounted::new(
                &song_service_service_descriptor(),
                serve_song_service(SongServiceImpl::new()),
            ),
            daw::Mounted::new(
                midi_chart_service_service_descriptor(),
                MidiChartServiceDispatcher::new(KeyflowMidiAnalysis::from_global_daw()),
            ),
        ]
    }

    pub fn mounted_layers_with_daw<D>(daw: D) -> impl daw::Layer<Self>
    where
        D: Clone
            + daw::service::AudioEngine
            + daw::service::ExtState
            + daw::service::Projects
            + daw::service::TempoMap
            + daw::service::transport::service::Transport
            + Send
            + Sync
            + 'static,
    {
        let mut services = Self::mounted_services_with_daw(daw);
        let midi = services.pop().expect("session midi chart service");
        let song = services.pop().expect("session song service");
        let setlist = services.pop().expect("session setlist service");
        daw::layers![setlist, song, midi]
    }

    pub fn merge_into_with_daw<D>(mut handler: daw::LayerRouter, daw: D) -> daw::LayerRouter
    where
        D: Clone
            + daw::service::AudioEngine
            + daw::service::ExtState
            + daw::service::Projects
            + daw::service::TempoMap
            + daw::service::transport::service::Transport
            + Send
            + Sync
            + 'static,
    {
        for service in Self::mounted_services_with_daw(daw) {
            handler = handler.merge(service);
        }
        handler
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub mod daw_services {
    /// Add session-owned DAW-adjacent services to an in-process DAW router.
    ///
    /// The extension host owns the REAPER transport/channel. Session owns the
    /// chart-analysis service because setlist hydration is its consumer and the
    /// implementation depends on keyflow domain logic.
    pub fn layer_services_with_daw<D>(handler: daw::LayerRouter, daw: D) -> daw::LayerRouter
    where
        D: Clone
            + daw::service::AudioEngine
            + daw::service::ExtState
            + daw::service::Projects
            + daw::service::TempoMap
            + daw::service::transport::service::Transport
            + Send
            + Sync
            + 'static,
    {
        crate::SessionServices::merge_into_with_daw(handler, daw)
    }
}

// Session action definitions — single source of truth.
//
// The `define_actions!` macro generates:
// - `session_actions::TOGGLE_PLAYBACK` etc. (StaticActionId constants)
// - `session_actions::definitions()` (Vec<ActionDefinition>)
actions_proto::define_actions! {
    /// Session action ID constants and definitions.
    ///
    /// Use `session_actions::definitions()` to get all action definitions
    /// (for keybinding registration, command palette, etc.).
    pub session_actions {
        prefix: "fts.session",
        title: "Session",
        TOGGLE_PLAYBACK = "toggle_playback" {
            name: "Toggle Playback",
            description: "Toggle play/pause state",
            category: Transport,
            group: "Transport",
            shortcut: "Space",
            when: "tab:performance",
        }
        TOGGLE_SONG_LOOP = "toggle_song_loop" {
            name: "Toggle Song Loop",
            description: "Toggle looping for the current song",
            category: Transport,
            group: "Transport",
            shortcut: "L",
            when: "tab:performance",
        }
        SMART_NEXT = "smart_next" {
            name: "Smart Next",
            description: "Go to next section, or next song if at last section",
            category: Session,
            group: "Navigate",
            shortcut: "Right",
            when: "tab:performance",
        }
        SMART_PREVIOUS = "smart_previous" {
            name: "Smart Previous",
            description: "Go to previous section, or previous song if at first section",
            category: Session,
            group: "Navigate",
            shortcut: "Left",
            when: "tab:performance",
        }
        NEXT_SONG = "next_song" {
            name: "Next Song",
            description: "Go to the next song in the setlist",
            category: Session,
            group: "Navigate",
            shortcut: "Down",
            when: "tab:performance",
        }
        PREVIOUS_SONG = "previous_song" {
            name: "Previous Song",
            description: "Go to the previous song in the setlist",
            category: Session,
            group: "Navigate",
            shortcut: "Up",
            when: "tab:performance",
        }
        NEXT_SECTION = "next_section" {
            name: "Next Section",
            description: "Go to the next section in the current song",
            category: Session,
            group: "Navigate",
        }
        PREVIOUS_SECTION = "previous_section" {
            name: "Previous Section",
            description: "Go to the previous section in the current song",
            category: Session,
            group: "Navigate",
        }
        PRE_ROLL_DOUBLE_DURATION = "pre_roll_double_duration" {
            name: "Double Pre-Roll Duration",
            description: "Double the project pre-roll/count-in duration",
            category: Transport,
            group: "Pre-Roll",
        }
        PRE_ROLL_HALF_DURATION = "pre_roll_half_duration" {
            name: "Half Pre-Roll Duration",
            description: "Halve the project pre-roll/count-in duration",
            category: Transport,
            group: "Pre-Roll",
        }
        PRE_ROLL_SET_HALF_MEASURE = "pre_roll_set_half_measure" {
            name: "Set Pre-Roll to 1/2 Measure",
            description: "Set the project pre-roll/count-in duration to half a measure",
            category: Transport,
            group: "Pre-Roll",
        }
        PRE_ROLL_SET_1_MEASURE = "pre_roll_set_1_measure" {
            name: "Set Pre-Roll to 1 Measure",
            description: "Set the project pre-roll/count-in duration to one measure",
            category: Transport,
            group: "Pre-Roll",
        }
        PRE_ROLL_SET_2_MEASURES = "pre_roll_set_2_measures" {
            name: "Set Pre-Roll to 2 Measures",
            description: "Set the project pre-roll/count-in duration to two measures",
            category: Transport,
            group: "Pre-Roll",
        }
        INSERT_INTRO_REGION = "insert_intro_region" {
            name: "Insert Region - Intro (IN)",
            description: "Insert an Intro section region at the current edit cursor",
            category: Session,
            group: "Edit",
        }
        INSERT_VERSE_REGION = "insert_verse_region" {
            name: "Insert Region - Verse (VS)",
            description: "Insert a Verse section region at the current edit cursor",
            category: Session,
            group: "Edit",
        }
        INSERT_PRE_CHORUS_REGION = "insert_pre_chorus_region" {
            name: "Insert Region - Pre-Chorus (PRE-CH)",
            description: "Insert a Pre-Chorus section region at the current edit cursor",
            category: Session,
            group: "Edit",
        }
        INSERT_CHORUS_REGION = "insert_chorus_region" {
            name: "Insert Region - Chorus (CH)",
            description: "Insert a Chorus section region at the current edit cursor",
            category: Session,
            group: "Edit",
        }
        INSERT_BRIDGE_REGION = "insert_bridge_region" {
            name: "Insert Region - Bridge (BR)",
            description: "Insert a Bridge section region at the current edit cursor",
            category: Session,
            group: "Edit",
        }
        INSERT_OUTRO_REGION = "insert_outro_region" {
            name: "Insert Region - Outro (OUT)",
            description: "Insert an Outro section region at the current edit cursor",
            category: Session,
            group: "Edit",
        }
        INSERT_INSTRUMENTAL_REGION = "insert_instrumental_region" {
            name: "Insert Region - Instrumental (INST)",
            description: "Insert an Instrumental section region at the current edit cursor",
            category: Session,
            group: "Edit",
        }
        INSERT_SOLO_REGION = "insert_solo_region" {
            name: "Insert Region - Solo (SOLO)",
            description: "Insert a Solo section region at the current edit cursor",
            category: Session,
            group: "Edit",
        }
        INSERT_HITS_REGION = "insert_hits_region" {
            name: "Insert Region - Hits (HITS)",
            description: "Insert a Hits section region at the current edit cursor",
            category: Session,
            group: "Edit",
        }
        INSERT_INTERLUDE_REGION = "insert_interlude_region" {
            name: "Insert Region - Interlude (INT)",
            description: "Insert an Interlude section region at the current edit cursor",
            category: Session,
            group: "Edit",
        }
        INSERT_BREAKDOWN_REGION = "insert_breakdown_region" {
            name: "Insert Region - Breakdown (BD)",
            description: "Insert a Breakdown section region at the current edit cursor",
            category: Session,
            group: "Edit",
        }
        INSERT_VAMP_REGION = "insert_vamp_region" {
            name: "Insert Region - Vamp (VAMP)",
            description: "Insert a Vamp section region at the current edit cursor",
            category: Session,
            group: "Edit",
        }
        INSERT_COUNT_IN_REGION = "insert_count_in_region" {
            name: "Insert Region - Count-In (COUNT-IN)",
            description: "Insert a Count-In section region at the current edit cursor",
            category: Session,
            group: "Edit",
        }
        INSERT_END_REGION = "insert_end_region" {
            name: "Insert Region - End (END)",
            description: "Insert an End section region at the current edit cursor",
            category: Session,
            group: "Edit",
        }
        INSERT_COUNT_IN_MARKER = "insert_count_in_marker" {
            name: "Insert Count-In Marker",
            description: "Insert a Count-In marker on the MARKS ruler lane",
            category: Session,
            group: "Edit",
        }
        INSERT_START_MARKER = "insert_start_marker" {
            name: "Insert =START Marker",
            description: "Insert an =START marker on the MARKS ruler lane",
            category: Session,
            group: "Edit",
        }
        INSERT_END_MARKER = "insert_end_marker" {
            name: "Insert =END Marker",
            description: "Insert an =END marker on the MARKS ruler lane",
            category: Session,
            group: "Edit",
        }
        INSERT_SONGSTART_MARKER = "insert_songstart_marker" {
            name: "Insert SONGSTART Marker",
            description: "Insert a SONGSTART marker on the MARKS ruler lane",
            category: Session,
            group: "Edit",
        }
        INSERT_SONGEND_MARKER = "insert_songend_marker" {
            name: "Insert SONGEND Marker",
            description: "Insert a SONGEND marker on the MARKS ruler lane",
            category: Session,
            group: "Edit",
        }
        ORGANIZE_SESSION = "organize_session" {
            name: "Organize Session",
            description: "Organize the current session using the dynamic template track hierarchy",
            category: Session,
            group: "Tracks",
        }
        ORGANIZE_EVERYTHING = "organize_everything" {
            name: "Organize Everything",
            description: "Organize all project tracks using the dynamic template track hierarchy",
            category: Session,
            group: "Tracks",
        }
        ORGANIZE_SELECTED_TRACKS = "organize_selected_tracks" {
            name: "Organize Selected Tracks",
            description: "Organize selected tracks using the dynamic template track hierarchy",
            category: Session,
            group: "Tracks",
        }
        CREATE_NEW_DRUM_KIT = "create_new_drum_kit" {
            name: "Create New Drum Kit",
            description: "Create a new drum kit track group",
            category: Session,
            group: "Tracks",
        }
        CREATE_NEW_ELECTRONIC_DRUMS = "create_new_electronic_drums" {
            name: "Create New Electronic Drums",
            description: "Create a new electronic drums track group",
            category: Session,
            group: "Tracks",
        }
        CREATE_NEW_BASS_GUITAR = "create_new_bass_guitar" {
            name: "Create New Bass Guitar",
            description: "Create a new bass guitar track group",
            category: Session,
            group: "Tracks",
        }
        CREATE_NEW_SYNTH_BASS = "create_new_synth_bass" {
            name: "Create New Synth Bass",
            description: "Create a new synth bass track group",
            category: Session,
            group: "Tracks",
        }
        CREATE_NEW_ELECTRIC_GUITAR = "create_new_electric_guitar" {
            name: "Create New Electric Guitar",
            description: "Create a new electric guitar track group",
            category: Session,
            group: "Tracks",
        }
        CREATE_NEW_ACOUSTIC_GUITAR = "create_new_acoustic_guitar" {
            name: "Create New Acoustic Guitar",
            description: "Create a new acoustic guitar track group",
            category: Session,
            group: "Tracks",
        }
        CREATE_NEW_PIANO = "create_new_piano" {
            name: "Create New Piano",
            description: "Create a new piano track group",
            category: Session,
            group: "Tracks",
        }
        CREATE_NEW_ORGAN = "create_new_organ" {
            name: "Create New Organ",
            description: "Create a new organ track group",
            category: Session,
            group: "Tracks",
        }
        CREATE_NEW_ELECTRIC_KEYS = "create_new_electric_keys" {
            name: "Create New Electric Keys",
            description: "Create a new electric keys track group",
            category: Session,
            group: "Tracks",
        }
        CREATE_NEW_SYNTH_LEAD = "create_new_synth_lead" {
            name: "Create New Synth Lead",
            description: "Create a new synth lead track group",
            category: Session,
            group: "Tracks",
        }
        CREATE_NEW_SYNTH_PAD = "create_new_synth_pad" {
            name: "Create New Synth Pad",
            description: "Create a new synth pad track group",
            category: Session,
            group: "Tracks",
        }
        CREATE_NEW_SYNTH_ARP = "create_new_synth_arp" {
            name: "Create New Synth Arp",
            description: "Create a new synth arp track group",
            category: Session,
            group: "Tracks",
        }
        CREATE_NEW_LEAD_VOCALS = "create_new_lead_vocals" {
            name: "Create New Lead Vocals",
            description: "Create a new lead vocals track group",
            category: Session,
            group: "Tracks",
        }
        CREATE_NEW_BACKGROUND_VOCALS = "create_new_background_vocals" {
            name: "Create New Background Vocals",
            description: "Create a new background vocals track group",
            category: Session,
            group: "Tracks",
        }
        TOGGLE_DRUMS_VISIBILITY = "toggle_drums_visibility" {
            name: "Toggle Drums Visibility",
            description: "Toggle visibility for drum tracks",
            category: Session,
            group: "Visibility",
        }
        TOGGLE_PERCUSSION_VISIBILITY = "toggle_percussion_visibility" {
            name: "Toggle Percussion Visibility",
            description: "Toggle visibility for percussion tracks",
            category: Session,
            group: "Visibility",
        }
        TOGGLE_BASS_VISIBILITY = "toggle_bass_visibility" {
            name: "Toggle Bass Visibility",
            description: "Toggle visibility for bass tracks",
            category: Session,
            group: "Visibility",
        }
        TOGGLE_GUITARS_VISIBILITY = "toggle_guitars_visibility" {
            name: "Toggle Guitars Visibility",
            description: "Toggle visibility for guitar tracks",
            category: Session,
            group: "Visibility",
        }
        TOGGLE_KEYS_VISIBILITY = "toggle_keys_visibility" {
            name: "Toggle Keys Visibility",
            description: "Toggle visibility for keys tracks",
            category: Session,
            group: "Visibility",
        }
        TOGGLE_SYNTHS_VISIBILITY = "toggle_synths_visibility" {
            name: "Toggle Synths Visibility",
            description: "Toggle visibility for synth tracks",
            category: Session,
            group: "Visibility",
        }
        TOGGLE_HORNS_VISIBILITY = "toggle_horns_visibility" {
            name: "Toggle Horns Visibility",
            description: "Toggle visibility for horns tracks",
            category: Session,
            group: "Visibility",
        }
        TOGGLE_HARMONICA_VISIBILITY = "toggle_harmonica_visibility" {
            name: "Toggle Harmonica Visibility",
            description: "Toggle visibility for harmonica tracks",
            category: Session,
            group: "Visibility",
        }
        TOGGLE_STRINGS_VISIBILITY = "toggle_strings_visibility" {
            name: "Toggle Strings Visibility",
            description: "Toggle visibility for strings tracks",
            category: Session,
            group: "Visibility",
        }
        TOGGLE_VOCALS_VISIBILITY = "toggle_vocals_visibility" {
            name: "Toggle Vocals Visibility",
            description: "Toggle visibility for vocal tracks",
            category: Session,
            group: "Visibility",
        }
        TOGGLE_CHOIR_VISIBILITY = "toggle_choir_visibility" {
            name: "Toggle Choir Visibility",
            description: "Toggle visibility for choir tracks",
            category: Session,
            group: "Visibility",
        }
        TOGGLE_ORCHESTRA_VISIBILITY = "toggle_orchestra_visibility" {
            name: "Toggle Orchestra Visibility",
            description: "Toggle visibility for orchestra tracks",
            category: Session,
            group: "Visibility",
        }
        TOGGLE_SFX_VISIBILITY = "toggle_sfx_visibility" {
            name: "Toggle SFX Visibility",
            description: "Toggle visibility for SFX tracks",
            category: Session,
            group: "Visibility",
        }
        TOGGLE_GUIDE_VISIBILITY = "toggle_guide_visibility" {
            name: "Toggle Guide Visibility",
            description: "Toggle visibility for guide tracks",
            category: Session,
            group: "Visibility",
        }
        TOGGLE_REFERENCE_VISIBILITY = "toggle_reference_visibility" {
            name: "Toggle Reference Visibility",
            description: "Toggle visibility for reference tracks",
            category: Session,
            group: "Visibility",
        }
        TOGGLE_STEM_SPLIT_VISIBILITY = "toggle_stem_split_visibility" {
            name: "Toggle Stem Split Visibility",
            description: "Toggle visibility for stem split tracks",
            category: Session,
            group: "Visibility",
        }
        SHOW_ALL_TRACKS = "show_all_tracks" {
            name: "Show All Tracks",
            description: "Show all tracks in the session",
            category: Session,
            group: "Visibility",
        }
        HIDE_TEMPLATE_TRACKS = "hide_template_tracks" {
            name: "Hide Template Tracks",
            description: "Hide all dynamic template group tracks",
            category: Session,
            group: "Visibility",
        }
        VISIBILITY_PROFILE_DRUM_EDITING = "visibility_profile_drum_editing" {
            name: "Session Visibility - Drum Editing",
            description: "Show and size drum tracks for editing, hiding unrelated tracks",
            category: Session,
            group: "Visibility",
        }
        VISIBILITY_PROFILE_MIDI_EDITING = "visibility_profile_midi_editing" {
            name: "Session Visibility - MIDI Editing",
            description: "Show and size MIDI-oriented template groups for editing",
            category: Session,
            group: "Visibility",
        }
        REBUILD_VISIBILITY_CACHE = "rebuild_visibility_cache" {
            name: "Rebuild Visibility Cache",
            description: "Rebuild the dynamic template visibility cache",
            category: Session,
            group: "Visibility",
        }
        AUTO_COLOR_COLOR_ALL = "auto_color_color_all" {
            name: "Session Auto Color - Color All Tracks",
            description: "Apply session auto-color rules to all tracks and keep auto-color enabled",
            category: Session,
            group: "Auto Color",
        }
        AUTO_COLOR_COLOR_SELECTED = "auto_color_color_selected" {
            name: "Session Auto Color - Color Selected Tracks",
            description: "Apply session auto-color rules to selected tracks",
            category: Session,
            group: "Auto Color",
        }
        AUTO_COLOR_TOGGLE = "auto_color_toggle" {
            name: "Session Auto Color - Toggle",
            description: "Toggle session auto-color for all tracks",
            category: Session,
            group: "Auto Color",
        }
        AUTO_COLOR_CLEAR_ALL = "auto_color_clear_all" {
            name: "Session Auto Color - Clear All Tracks",
            description: "Clear colors from all tracks and disable session auto-color",
            category: Session,
            group: "Auto Color",
        }
        AUTO_COLOR_CLEAR_SELECTED = "auto_color_clear_selected" {
            name: "Session Auto Color - Clear Selected Tracks",
            description: "Clear colors from selected tracks",
            category: Session,
            group: "Auto Color",
        }
        TRACK_MANAGER_ADD_CHANNEL = "track_manager_add_channel" {
            name: "Add Channel",
            description: "Add the next dynamic-template channel to the selected track scope",
            category: Session,
            group: "Track Manager",
        }
        TRACK_MANAGER_ADD_LAYER = "track_manager_add_layer" {
            name: "Add Layer",
            description: "Add the next dynamic-template layer to the selected track scope",
            category: Session,
            group: "Track Manager",
        }
        TRACK_MANAGER_ADD_MULTI_MIC = "track_manager_add_multi_mic" {
            name: "Add Multi-Mic",
            description: "Add the next dynamic-template multi-mic track to the selected track scope",
            category: Session,
            group: "Track Manager",
        }
        TRACK_MANAGER_ADD_PERFORMER = "track_manager_add_performer" {
            name: "Add Performer",
            description: "Add a performer folder to the selected track scope",
            category: Session,
            group: "Track Manager",
        }
        TRACK_MANAGER_ADD_ARRANGEMENT = "track_manager_add_arrangement" {
            name: "Add Arrangement",
            description: "Add the next dynamic-template arrangement to the selected instrument scope",
            category: Session,
            group: "Track Manager",
        }
        TRACK_MANAGER_REORGANIZE_SELECTED_BY_PERFORMER = "track_manager_reorganize_selected_by_performer" {
            name: "Reorganize Selected by Performer",
            description: "Reorganize selected tracks with performer as the top metadata dimension",
            category: Session,
            group: "Track Manager",
        }
        TRACK_MANAGER_REORGANIZE_SELECTED_BY_ARRANGEMENT = "track_manager_reorganize_selected_by_arrangement" {
            name: "Reorganize Selected by Arrangement",
            description: "Reorganize selected tracks with arrangement as the top metadata dimension",
            category: Session,
            group: "Track Manager",
        }
        LOG_HELLO = "log_hello" {
            name: "Log Hello",
            description: "Logs 'Hello from session!'",
            category: Dev,
            group: "Dev",
        }
        LOG_STATUS = "log_status" {
            name: "Log Status",
            description: "Logs current session status",
            category: Dev,
            group: "Dev",
        }
        BUILD_SETLIST = "build_setlist" {
            name: "Build Setlist",
            description: "Build setlist from all open REAPER project tabs",
            category: Session,
        }
        LOAD_DEMO_SETLIST = "load_demo_setlist" {
            name: "Load Demo Setlist",
            description: "Load a demo setlist with mock song data (no DAW required)",
            category: Dev,
            group: "Dev",
        }
    }
}

// Mode actions live in their own block so they render as
// `FTS: Mode - <Name>` rather than `FTS: Session - Modes - Mode: <Name>`.
actions_proto::define_actions! {
    /// Session mode action IDs (Organize, Write, Produce, Record, Edit, Mix, Master, Live, Video) + Save variants.
    pub mode_defs {
        prefix: "fts.session.mode",
        title: "Mode",
        ORGANIZE = "organize" {
            name: "Organize",
            description: "Switch to Organize mode (planning, song structure, setlists)",
            category: Session,
        }
        WRITE = "write" {
            name: "Write",
            description: "Switch to Write mode (lyric/melody/idea capture)",
            category: Session,
        }
        PRODUCE = "produce" {
            name: "Produce",
            description: "Switch to Produce mode (arrangement, sound design, instrument selection)",
            category: Session,
        }
        RECORD = "record" {
            name: "Record",
            description: "Switch to Record mode (tracking, takes, monitoring)",
            category: Session,
        }
        EDIT = "edit" {
            name: "Edit",
            description: "Switch to Edit mode (comping, timing, cleanup)",
            category: Session,
        }
        MIX = "mix" {
            name: "Mix",
            description: "Switch to Mix mode (mixer focus, processing, automation)",
            category: Session,
        }
        MASTER = "master" {
            name: "Master",
            description: "Switch to Master mode (master bus processing, metering, export prep)",
            category: Session,
        }
        LIVE = "live" {
            name: "Live",
            description: "Switch to Live mode (performance/setlist playback view)",
            category: Session,
        }
        VIDEO = "video" {
            name: "Video",
            description: "Switch to Video mode (sync to picture / video editing layout)",
            category: Session,
        }
        MINIMAL = "minimal" {
            name: "Minimal",
            description: "Switch to Minimal mode (stripped layout, no mode toolbars)",
            category: Session,
        }
        SAVE_ORGANIZE = "save_organize" {
            name: "Save: Organize",
            description: "Capture current REAPER window state to Organize's native screenset slot",
            category: Session,
        }
        SAVE_WRITE = "save_write" {
            name: "Save: Write",
            description: "Capture current REAPER window state to Write's native screenset slot",
            category: Session,
        }
        SAVE_PRODUCE = "save_produce" {
            name: "Save: Produce",
            description: "Capture current REAPER window state to Produce's native screenset slot",
            category: Session,
        }
        SAVE_RECORD = "save_record" {
            name: "Save: Record",
            description: "Capture current REAPER window state to Record's native screenset slot",
            category: Session,
        }
        SAVE_EDIT = "save_edit" {
            name: "Save: Edit",
            description: "Capture current REAPER window state to Edit's native screenset slot",
            category: Session,
        }
        SAVE_MIX = "save_mix" {
            name: "Save: Mix",
            description: "Capture current REAPER window state to Mix's native screenset slot",
            category: Session,
        }
        SAVE_MASTER = "save_master" {
            name: "Save: Master",
            description: "Capture current REAPER window state to Master's native screenset slot",
            category: Session,
        }
        SAVE_LIVE = "save_live" {
            name: "Save: Live",
            description: "Capture current REAPER window state to Live's native screenset slot",
            category: Session,
        }
        SAVE_VIDEO = "save_video" {
            name: "Save: Video",
            description: "Capture current REAPER window state to Video's native screenset slot",
            category: Session,
        }
        SAVE_MINIMAL = "save_minimal" {
            name: "Save: Minimal",
            description: "Capture current REAPER window state to Minimal's native screenset slot",
            category: Session,
        }
        LOG_CURRENT = "log_current" {
            name: "Log Current",
            description: "Log the current session mode to the console (debug helper)",
            category: Session,
        }
    }
}
