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
//! use session::{SetlistServiceImpl, SongServiceImpl};
//! use session_proto::{SetlistServiceDispatcher, SongServiceDispatcher};
//!
//! // Create services
//! let setlist = SetlistServiceImpl::new();
//! let song = SongServiceImpl::new();
//!
//! // Create dispatchers for RPC
//! let setlist_dispatcher = SetlistServiceDispatcher::new(setlist);
//! let song_dispatcher = SongServiceDispatcher::new(song);
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

pub mod daw_module;

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
            description: "Insert an =START marker on the START/END ruler lane",
            category: Session,
            group: "Edit",
        }
        INSERT_END_MARKER = "insert_end_marker" {
            name: "Insert =END Marker",
            description: "Insert an =END marker on the START/END ruler lane",
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
