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

// Re-export session-proto so app crates can use `session::` instead of
// `session_proto::` directly. `setlist` / `song` are NOT re-exported here:
// this crate has its own domain folders by those names, and each one
// re-exports the matching proto module itself so `session::setlist::…`
// still resolves to the proto types.
pub use session_proto::*;
pub use session_proto::{offset_map, ruler_lanes, services, track_structure};

// ── Domain folders ──────────────────────────────────────────────────────
//
// One per domain, each pairing the implementation with a re-export of its
// `session_proto` counterpart. The setlist stack drives the
// backend-agnostic `daw::get()` facade over tokio primitives that are all
// wasm-safe (the browser setlist engine builds/serves the same
// `SetlistServiceImpl` in-process). Only the REAPER coupling was the wasm
// blocker, now routed through `daw_proto::main_thread` (inline on
// non-REAPER backends). See setlist::service::live_daw_sync (native-only —
// the SynchronizationEngine is REAPER-linked).
pub mod setlist;
pub mod song;

// REAPER-hotkey action domains driving the `daw::reaper` backend directly.
// Not needed by the browser setlist engine — native-only. `track_manager`
// also drives `dynamic-template` (the native template engine).
#[cfg(not(target_arch = "wasm32"))]
pub mod keyflow;
#[cfg(not(target_arch = "wasm32"))]
pub mod modes;
pub mod playback;
// REAPER-side helper (routing-project mutation). Not needed by the browser
// build; kept native-only.
#[cfg(not(target_arch = "wasm32"))]
pub mod routing;
#[cfg(not(target_arch = "wasm32"))]
pub mod track_manager;

// ── Infrastructure ──────────────────────────────────────────────────────
//
// Cross-domain plumbing, deliberately flat.
//
// Pre-roll, record control, track grouping, take ranking and auto-colour
// moved to the `daw-actions` crate: they're plain DAW operations with
// nothing session-specific about them.
pub mod cache;
#[cfg(not(target_arch = "wasm32"))]
pub mod daw_module;
pub mod event_bus;
// Composes the mode actions behind a tokio-runtime pump.
#[cfg(not(target_arch = "wasm32"))]
pub mod rpc_services;

// Re-export service implementations for library use
pub use setlist::SetlistServiceImpl;
#[cfg(not(target_arch = "wasm32"))]
pub use song::SongServiceImpl;

// Re-export builders for advanced use cases
pub use setlist::SetlistBuilder;
pub use song::SongBuilder;

// Re-export demo setlist stamping (for extensions that have a local Daw instance)
pub use setlist::service::demo::{stamp_demo_into_project, stamp_demo_setlist};

/// Register every REAPER action this crate declares against `backend`.
///
/// One call instead of six. A module added here reaches every host that
/// already calls this, rather than silently not registering until each
/// call site remembers to add a line — which is exactly how
/// `FTS_SESSION_TOGGLE_PLAYBACK` sat dead in REAPER's action list.
///
/// Scope nesting is composed here, not declared by the leaf traits:
/// `track_manager` names only itself ("Track Manager") and gets wrapped
/// in a `SESSION`-scoped backend on the way in.
///
/// `daw` is the backend the handlers drive (`daw::reaper::Reaper` in
/// production, `daw_standalone::sync::Standalone` in tests).
#[cfg(not(target_arch = "wasm32"))]
pub fn register_all_actions<D, B>(backend: &B, daw: D)
where
    D: daw::service::Projects
        + daw::service::transport::service::Transport
        + daw::service::Markers
        + daw::service::Regions
        + daw::service::TempoMap
        + daw::service::Tracks
        + daw::service::Items
        + daw::service::UiDialogs
        + Clone
        + Send
        + Sync
        + 'static,
    B: architect::action::ActionBackend + Clone,
{
    keyflow::actions::register_actions(backend, daw.clone());
    keyflow::scaffold::register_actions(backend, daw.clone());
    modes::register_actions(backend);
    playback::register_actions(backend, daw.clone());
    setlist::actions::register_actions(backend, daw.clone());
    session_proto::track_manager::register_track_manager_actions(
        &architect::action::ScopedActionBackend::new(backend.clone(), "SESSION", "Session"),
        std::sync::Arc::new(track_manager::TrackManager::new(daw)),
    );
}

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
        setlist::actions::register(&setlist_impl);
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
        use crate::rpc_services::SessionModeServiceImpl;
        use daw::service::{
            record_control_service_service_descriptor, serve_record_control_service,
            serve_take_ranking_service, take_ranking_service_service_descriptor,
        };
        use daw_actions::record::RecordControlServiceImpl;
        use daw_actions::take_ranking::TakeRankingServiceImpl;
        use session_proto::services::{
            serve_session_mode_service, session_mode_service_service_descriptor,
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
        // TOGGLE_PLAYBACK / TOGGLE_SONG_LOOP moved to
        // `session_proto::playback::PlaybackActions`. They are NOT
        // redeclared here: the `#[architect::actions]` macro emits the
        // same `FTS_SESSION_*` command ids, so keeping both would
        // register each command twice.
        //
        // SMART_NEXT / SMART_PREVIOUS / NEXT_SONG / PREVIOUS_SONG /
        // NEXT_SECTION / PREVIOUS_SECTION: RPC-only, deliberately NOT
        // migrated to `#[architect::actions]` (see
        // `session_proto::playback`'s module doc for the full reasoning)
        // — they route through
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
        // The 19 INSERT_* region/marker commands and
        // CONVERT_MARKERS_TO_SESSION_FORMAT moved to
        // `session_proto::keyflow_actions::KeyflowActions`; not redeclared
        // here, or the same FTS_SESSION_* command ids would register twice.
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
        // BUILD_SETLIST / LOAD_DEMO_SETLIST / DUMP_RULER_STATE moved to
        // `session_proto::setlist_actions::SetlistActions`; not redeclared
        // here, or the same FTS_SESSION_* command ids would register twice.
        // ── Recording workflow ───────────────────────────────────────
        // ── Track-group manager ──────────────────────────────────────
        // ── Take ranking (Record mode workflow) ──────────────────────
    }
}

