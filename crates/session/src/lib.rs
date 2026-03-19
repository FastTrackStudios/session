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
    }
}
