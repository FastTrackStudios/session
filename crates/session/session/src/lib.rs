// Lint debt: workspace flipped dead_code/unused to warn (task cleanup);
// this crate predates that — burn down separately.
#![allow(dead_code, unused)]

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

// The setlist-service stack + its support modules. These drive the
// backend-agnostic `daw::get()` facade over tokio primitives that are
// all wasm-safe (the browser setlist engine builds/serves the same
// `SetlistServiceImpl` in-process). Only the REAPER coupling was the wasm
// blocker, now routed through `daw_proto::main_thread` (inline on non-REAPER
// backends). See setlist_service::live_daw_sync (native-only — the
// SynchronizationEngine is REAPER-linked).
pub mod cache;

pub mod chart_import;
pub mod event_bus;
pub mod keyflow_actions;
pub mod keyflow_scaffold;
// REAPER-side helpers (preroll insertion, routing-project mutation). Not
// needed by the browser build; kept native-only.
#[cfg(not(target_arch = "wasm32"))]
pub mod preroll_actions;
#[cfg(not(target_arch = "wasm32"))]
pub mod routing_project;
pub mod setlist_builder;
pub mod setlist_service;
mod song_builder;
#[cfg(not(target_arch = "wasm32"))]
mod song_service;

// Re-export service implementations for library use
pub use setlist_service::SetlistServiceImpl;
#[cfg(not(target_arch = "wasm32"))]
pub use song_service::SongServiceImpl;

// Re-export builders for advanced use cases
pub use setlist_builder::SetlistBuilder;
pub use song_builder::SongBuilder;

// Re-export demo setlist stamping (for extensions that have a local Daw instance)
pub use setlist_service::demo::{stamp_demo_into_project, stamp_demo_setlist};

// Uses `daw::block_on` (native-only) and is only consumed by `daw_module`
// (also native-only) — the REAPER auto-coloring action.
#[cfg(not(target_arch = "wasm32"))]
pub mod auto_color_actions;
// `daw_module` + `track_manager_actions` drive `dynamic-template` (native
// template engine); `take_ranking` uses raw REAPER FFI. All native-only.
#[cfg(not(target_arch = "wasm32"))]
pub mod daw_module;
// REAPER-hotkey action modules: `group_manager`/`mode_actions`/`record_actions`
// drive the `daw::reaper` backend directly, `group_actions` wraps
// `group_manager`, and `rpc_services` composes them + `take_ranking` behind a
// tokio-runtime pump. None are needed by the browser setlist engine — native-only.
#[cfg(not(target_arch = "wasm32"))]
pub mod group_actions;
#[cfg(not(target_arch = "wasm32"))]
pub mod group_manager;
#[cfg(not(target_arch = "wasm32"))]
pub mod mode_actions;
pub mod playback_actions;
#[cfg(not(target_arch = "wasm32"))]
pub mod record_actions;
#[cfg(not(target_arch = "wasm32"))]
pub mod rpc_services;
// REAPER hotkey action registration (`build_setlist_sync` → `SongBuilder::
// build_native`) — native-only, consumed by `SessionServices`.
#[cfg(not(target_arch = "wasm32"))]
pub mod setlist_actions;
#[cfg(not(target_arch = "wasm32"))]
pub mod take_ranking;
#[cfg(not(target_arch = "wasm32"))]
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
            + daw::service::Tracks
            + Send
            + Sync
            + 'static,
    {
        use keyflow_daw_analysis::{
            KeyflowMidiAnalysis, MidiChartsDispatcher, midi_charts_service_descriptor,
        };

        // Share one SetlistServiceImpl between the RPC mount and the
        // action-handler chain (so the REAPER `build_setlist` hotkey
        // and `fts session setlist` see the same in-memory state).
        // SetlistServiceImpl is Clone over Arc'd fields, so cloning
        // gives a handle to the same setlist / song_cache / etc.
        let setlist_impl = SetlistServiceImpl::with_daw(daw.clone());
        setlist_actions::register(&setlist_impl);
        vec![
            daw::Mounted::new(
                setlist_service_service_descriptor(),
                serve_setlist_service(setlist_impl),
            ),
            daw::Mounted::new(
                song_service_service_descriptor(),
                serve_song_service(SongServiceImpl::new()),
            ),
            daw::Mounted::new(
                midi_charts_service_descriptor(),
                MidiChartsDispatcher::new(KeyflowMidiAnalysis::from_global_daw()),
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
            + daw::service::Tracks
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
            + daw::service::Tracks
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
            + daw::service::Tracks
            + Send
            + Sync
            + 'static,
    {
        crate::SessionServices::merge_into_with_daw(handler, daw)
    }

    /// Mount the session control surfaces (mode / take-ranking /
    /// record control) onto the in-process DAW router. These are
    /// independent of the chart / setlist services and don't need a
    /// `D` backend — each handler bounces to REAPER's main thread
    /// via `daw_reaper::main_thread`.
    pub fn layer_control_surfaces(mut handler: daw::LayerRouter) -> daw::LayerRouter {
        use crate::rpc_services::{
            RecordControlServiceImpl, SessionModeServiceImpl, TakeRankingServiceImpl,
        };
        use session_proto::services::{
            record_control_service_service_descriptor, serve_record_control_service,
            serve_session_mode_service, serve_take_ranking_service,
            session_mode_service_service_descriptor, take_ranking_service_service_descriptor,
        };
        handler = handler.merge(daw::Mounted::new(
            session_mode_service_service_descriptor(),
            serve_session_mode_service(SessionModeServiceImpl::default()),
        ));
        handler = handler.merge(daw::Mounted::new(
            take_ranking_service_service_descriptor(),
            serve_take_ranking_service(TakeRankingServiceImpl),
        ));
        handler = handler.merge(daw::Mounted::new(
            record_control_service_service_descriptor(),
            serve_record_control_service(RecordControlServiceImpl),
        ));
        handler
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
        // SMART_NEXT / SMART_PREVIOUS / NEXT_SONG / PREVIOUS_SONG /
        // NEXT_SECTION / PREVIOUS_SECTION: RPC-only, deliberately NOT
        // migrated to `#[architect::actions]` (see `playback_actions.rs`'s
        // module doc for the full reasoning) — they route through
        // `SetlistServiceImpl::go_to_song_impl`/`go_to_section_impl`,
        // which depend on `ensure_song_hydrated`'s async, timeout-bounded
        // rebuild path. There's no safe way to collapse that to a sync
        // REAPER action callback without either a main-thread deadlock
        // risk (blocking on the async work) or a silent behavior
        // regression (a sync fast-path that no-ops on a cache miss).
        // Reachable today only via `SetlistService`'s async RPC methods
        // (CLI/desktop/web clients) — not as REAPER named commands. This
        // predates this migration; `daw_module.rs`'s dispatch chain never
        // routed these to a sync handler either.
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
        CONVERT_MARKERS_TO_SESSION_FORMAT = "convert_markers_to_session_format" {
            name: "Convert Markers to Session Format",
            description: "Convert plain section-name markers into FTS section regions and add a SONG-lane region named after the project",
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
        CREATE_NEW_KEYS = "create_new_keys" {
            name: "Create New Keys",
            description: "Create a new keys track group (piano, organ, electric keys, etc.)",
            category: Session,
            group: "Tracks",
        }
        CREATE_NEW_SYNTH = "create_new_synth" {
            name: "Create New Synth",
            description: "Create a new synth track group (bass, lead, pad, arp, etc.)",
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
        CREATE_NEW_ORCHESTRAL_BRASS = "create_new_orchestral_brass" {
            name: "Create New Orchestral Brass",
            description: "Create a new orchestral brass track group",
            category: Session,
            group: "Tracks",
        }
        CREATE_NEW_ORCHESTRAL_WOODWINDS = "create_new_orchestral_woodwinds" {
            name: "Create New Orchestral Woodwinds",
            description: "Create a new orchestral woodwinds track group",
            category: Session,
            group: "Tracks",
        }
        CREATE_NEW_ORCHESTRAL_STRINGS = "create_new_orchestral_strings" {
            name: "Create New Orchestral Strings",
            description: "Create a new orchestral strings track group",
            category: Session,
            group: "Tracks",
        }
        CREATE_NEW_ORCHESTRAL_PERCUSSION = "create_new_orchestral_percussion" {
            name: "Create New Orchestral Percussion",
            description: "Create a new orchestral percussion track group",
            category: Session,
            group: "Tracks",
        }
        CREATE_NEW_SFX = "create_new_sfx" {
            name: "Create New SFX",
            description: "Create a new SFX track group",
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
        DUMP_RULER_STATE = "dump_ruler_state" {
            name: "Dump Ruler State",
            description: "Log every marker and region in the current project with its position, name, color and ruler lane index. Useful for debugging lane assignment after BUILD_SETLIST / LOAD_DEMO_SETLIST.",
            category: Dev,
            group: "Dev",
        }
        // ── Recording workflow ───────────────────────────────────────
        RECORD = "record" {
            name: "Record: Start recording (current song)",
            description: "Start a recording pass in the focused project — the current song's tab. Uses the existing arm / monitor / input settings.",
            category: Transport,
            group: "Recording",
        }
        RECORD_STOP = "record_stop" {
            name: "Record: Stop (keep media)",
            description: "Stop the transport in the focused project, keeping the media captured this pass.",
            category: Transport,
            group: "Recording",
        }
        RECORD_TOGGLE = "record_toggle" {
            name: "Record: Toggle recording (current song)",
            description: "Toggle recording in the focused project — the current song's tab.",
            category: Transport,
            group: "Recording",
        }
        ARM_SELECTED = "arm_selected" {
            name: "Track: Arm selected for recording",
            description: "Arm every selected track (I_RECARM = 1) in the focused project so it captures input on the next recording pass.",
            category: Tracks,
            group: "Recording",
        }
        DISARM_SELECTED = "disarm_selected" {
            name: "Track: Disarm selected",
            description: "Disarm every selected track (I_RECARM = 0) in the focused project.",
            category: Tracks,
            group: "Recording",
        }
        RECORD_RESTART = "record_restart" {
            name: "Record: Restart recording (delete + re-record)",
            description: "Stop the current recording (DELETE all recorded media this pass) and immediately start a fresh recording pass. For aborting a bad take without leaving stray media behind.",
            category: Transport,
            group: "Recording",
        }
        MONITOR_TOGGLE_ON_OFF = "monitor_toggle_on_off" {
            name: "Track: Toggle record monitor on/off (selected, skip auto/tape)",
            description: "Toggle the record-monitor state of every selected track between 'on' and 'off' only, skipping the auto/tape state that REAPER's native cycle action walks through. If any selected track is currently 'on', all go to off; otherwise all go to on.",
            category: Tracks,
            group: "Recording",
        }
        MONITOR_TOGGLE_TAPE_OFF = "monitor_toggle_tape_off" {
            name: "Track: Toggle record monitor auto-tape/off (selected)",
            description: "Toggle the record-monitor state of every selected track between 'auto/tape' (monitor input only while recording) and 'off'. If any selected track is currently 'auto/tape', all go to off; otherwise all go to auto/tape.",
            category: Tracks,
            group: "Recording",
        }
        // ── Track-group manager ──────────────────────────────────────
        GROUP_APPLY_NAMING = "group_apply_naming" {
            name: "Groups: Apply instrument-category naming scheme",
            description: "Name the project's 128 track groups by the FTS instrument partition (Drums 1-10, Bass 11-20, Electric Gtr 21-40, Acoustic Gtr 41-60, Keys 61-70, Synths 71-80, Lead Vocal 81-100, Background Vox 101-120, Spare 121-128).",
            category: Tracks,
            group: "Track Groups",
        }
        GROUP_ASSIGN_DRUMS = "group_assign_drums" {
            name: "Groups: Assign selected tracks to Drums",
            description: "Add the selected tracks to the next free Drums group slot as a mutual group (all flag families).",
            category: Tracks,
            group: "Track Groups",
        }
        GROUP_ASSIGN_BASS = "group_assign_bass" {
            name: "Groups: Assign selected tracks to Bass",
            description: "Add the selected tracks to the next free Bass group slot as a mutual group.",
            category: Tracks,
            group: "Track Groups",
        }
        GROUP_ASSIGN_ELECTRIC_GTR = "group_assign_electric_gtr" {
            name: "Groups: Assign selected tracks to Electric Gtr",
            description: "Add the selected tracks to the next free Electric Gtr group slot as a mutual group.",
            category: Tracks,
            group: "Track Groups",
        }
        GROUP_ASSIGN_ACOUSTIC_GTR = "group_assign_acoustic_gtr" {
            name: "Groups: Assign selected tracks to Acoustic Gtr",
            description: "Add the selected tracks to the next free Acoustic Gtr group slot as a mutual group.",
            category: Tracks,
            group: "Track Groups",
        }
        GROUP_ASSIGN_KEYS = "group_assign_keys" {
            name: "Groups: Assign selected tracks to Keys",
            description: "Add the selected tracks to the next free Keys group slot as a mutual group.",
            category: Tracks,
            group: "Track Groups",
        }
        GROUP_ASSIGN_SYNTHS = "group_assign_synths" {
            name: "Groups: Assign selected tracks to Synths",
            description: "Add the selected tracks to the next free Synths group slot as a mutual group.",
            category: Tracks,
            group: "Track Groups",
        }
        GROUP_ASSIGN_LEAD_VOCAL = "group_assign_lead_vocal" {
            name: "Groups: Assign selected tracks to Lead Vocal",
            description: "Add the selected tracks to the next free Lead Vocal group slot as a mutual group.",
            category: Tracks,
            group: "Track Groups",
        }
        GROUP_ASSIGN_BACKGROUND_VOX = "group_assign_background_vox" {
            name: "Groups: Assign selected tracks to Background Vox",
            description: "Add the selected tracks to the next free Background Vox group slot as a mutual group.",
            category: Tracks,
            group: "Track Groups",
        }
        // ── Take ranking (Record mode workflow) ──────────────────────
        TAKE_RANK_PLAYPOS_1 = "take_rank_playpos_1" {
            name: "Take: rank :) at play-pos -2s",
            description: "Set the active take's rank marker to :) at (play-pos - 2s) on every selected item, or at edit cursor if not playing.",
            category: Project,
            group: "Take Ranking",
        }
        TAKE_RANK_PLAYPOS_2 = "take_rank_playpos_2" {
            name: "Take: rank :)) at play-pos -2s",
            description: "Set the active take's rank marker to :)) at (play-pos - 2s) on every selected item, or at edit cursor if not playing.",
            category: Project,
            group: "Take Ranking",
        }
        TAKE_RANK_PLAYPOS_3 = "take_rank_playpos_3" {
            name: "Take: rank :))) at play-pos -2s",
            description: "Set the active take's rank marker to :))) at (play-pos - 2s) on every selected item, or at edit cursor if not playing.",
            category: Project,
            group: "Take Ranking",
        }
        TAKE_RANK_PLAYPOS_DOWN = "take_rank_playpos_down" {
            name: "Take: down-rank at play-pos -2s",
            description: "Set the active take's rank marker to :( at (play-pos - 2s) on every selected item, or at edit cursor if not playing.",
            category: Project,
            group: "Take Ranking",
        }
        TAKE_RANK_ITEM_1 = "take_rank_item_1" {
            name: "Take: rank :) (item-wide)",
            description: "Set the active take's rank marker to :) at item start for every selected item.",
            category: Project,
            group: "Take Ranking",
        }
        TAKE_RANK_ITEM_2 = "take_rank_item_2" {
            name: "Take: rank :)) (item-wide)",
            description: "Set the active take's rank marker to :)) at item start for every selected item.",
            category: Project,
            group: "Take Ranking",
        }
        TAKE_RANK_ITEM_3 = "take_rank_item_3" {
            name: "Take: rank :))) (item-wide)",
            description: "Set the active take's rank marker to :))) at item start for every selected item.",
            category: Project,
            group: "Take Ranking",
        }
        TAKE_RANK_ITEM_DOWN = "take_rank_item_down" {
            name: "Take: down-rank (item-wide)",
            description: "Set the active take's rank marker to :( at item start for every selected item.",
            category: Project,
            group: "Take Ranking",
        }
        TAKE_RANK_MOUSE_1 = "take_rank_mouse_1" {
            name: "Take: rank :) at mouse position",
            description: "Set the rank marker to :) on the take under the mouse at the mouse's project-time position.",
            category: Project,
            group: "Take Ranking",
        }
        TAKE_RANK_MOUSE_2 = "take_rank_mouse_2" {
            name: "Take: rank :)) at mouse position",
            description: "Set the rank marker to :)) on the take under the mouse at the mouse's project-time position.",
            category: Project,
            group: "Take Ranking",
        }
        TAKE_RANK_MOUSE_3 = "take_rank_mouse_3" {
            name: "Take: rank :))) at mouse position",
            description: "Set the rank marker to :))) on the take under the mouse at the mouse's project-time position.",
            category: Project,
            group: "Take Ranking",
        }
        TAKE_RANK_MOUSE_DOWN = "take_rank_mouse_down" {
            name: "Take: down-rank at mouse position",
            description: "Set the rank marker to :( on the take under the mouse at the mouse's project-time position.",
            category: Project,
            group: "Take Ranking",
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
        SCORING = "scoring" {
            name: "Scoring",
            description: "Switch to Scoring mode (multi-agent orchestration layout, no mode toolbars)",
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
        SAVE_SCORING = "save_scoring" {
            name: "Save: Scoring",
            description: "Capture current REAPER window state to Scoring's native screenset slot",
            category: Session,
        }
        LOG_CURRENT = "log_current" {
            name: "Log Current",
            description: "Log the current session mode to the console (debug helper)",
            category: Session,
        }
    }
}
