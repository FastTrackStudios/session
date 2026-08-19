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
pub mod section_kinds;
pub mod setlist;
pub mod song;

// REAPER-hotkey action domains driving the `daw::reaper` backend directly.
// Not needed by the browser setlist engine — native-only. `track_manager`
// also drives `dynamic-template` (the native template engine).
#[cfg(not(target_arch = "wasm32"))]
pub mod color;
#[cfg(not(target_arch = "wasm32"))]
pub mod guide;
#[cfg(not(target_arch = "wasm32"))]
pub mod key;
#[cfg(not(target_arch = "wasm32"))]
pub mod key_actions;
// NOT gated, deliberately: `chart_import`, `setlist::service::demo` and
// `task-player-ui` (the browser setlist engine) all reach into
// `keyflow::actions`. Adding gated modules directly above this line once
// left its `#[cfg]` attached to `keyflow` instead, which silently
// wasm-gated it and broke the task-web image build — so keep a
// non-attribute line between this and any gated module added above.
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
        + daw::service::Midi
        + daw::service::PositionConversion
        + daw::service::UiDialogs
        + Clone
        + Send
        + Sync
        + 'static,
    B: architect::action::ActionBackend + Clone,
{
    color::register_actions(backend);
    guide::register_actions(backend, daw.clone());
    key_actions::register_actions(backend, daw.clone());
    keyflow::actions::register_actions(backend, daw.clone());
    keyflow::scaffold::register_actions(backend, daw.clone());
    modes::register_actions(backend);
    playback::register_actions(backend, daw.clone());
    setlist::actions::register_actions(backend, daw.clone());
    session_proto::track_manager::register_track_manager_actions(
        // "FTS_SESSION", not "SESSION": every other command in the tree
        // carries the FTS_ prefix, and reaper-input's tracks.styx binds
        // `_FTS_SESSION_TRACK_MANAGER_ADD_*`. Registering as plain
        // `SESSION_*` left all five of those keybindings resolving to
        // nothing.
        &architect::action::ScopedActionBackend::new(backend.clone(), "FTS_SESSION", "Session"),
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
