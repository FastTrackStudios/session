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
pub use session_proto::{services, setlist, song};

pub mod cache;
pub mod event_bus;
mod setlist_builder;
mod setlist_service;
mod song_builder;
mod song_service;

// Re-export service implementations for library use
pub use setlist_service::SetlistServiceImpl;
pub use song_service::SongServiceImpl;

// Re-export builders for advanced use cases
pub use setlist_builder::SetlistBuilder;
pub use song_builder::SongBuilder;

// Session action definitions — single source of truth.
//
// The `declare_actions!` macro generates:
// - `session_actions::TOGGLE_PLAYBACK` etc. (StaticActionId constants)
// - `session_actions::definitions()` (Vec<ActionDefinition>)
//
// The cell binary (main.rs) uses `impl_action_dispatch!` to add handlers
// referencing these same constants.
actions_proto::declare_actions! {
    /// Session action ID constants and definitions.
    ///
    /// Use `session_actions::definitions()` to get all action definitions
    /// (for keybinding registration, command palette, etc.).
    pub session_actions {
        TOGGLE_PLAYBACK = "fts.session.toggle_playback" {
            name: "Toggle Playback",
            description: "Toggle play/pause state",
            category: Transport,
            menu_path: "FTS/Session/Transport",
            shortcut: "Space",
            when: "tab:performance",
        }
        TOGGLE_SONG_LOOP = "fts.session.toggle_song_loop" {
            name: "Toggle Song Loop",
            description: "Toggle looping for the current song",
            category: Transport,
            menu_path: "FTS/Session/Transport",
            shortcut: "L",
            when: "tab:performance",
        }
        SMART_NEXT = "fts.session.smart_next" {
            name: "Smart Next",
            description: "Go to next section, or next song if at last section",
            category: Session,
            menu_path: "FTS/Session/Navigate",
            shortcut: "Right",
            when: "tab:performance",
        }
        SMART_PREVIOUS = "fts.session.smart_previous" {
            name: "Smart Previous",
            description: "Go to previous section, or previous song if at first section",
            category: Session,
            menu_path: "FTS/Session/Navigate",
            shortcut: "Left",
            when: "tab:performance",
        }
        NEXT_SONG = "fts.session.next_song" {
            name: "Next Song",
            description: "Go to the next song in the setlist",
            category: Session,
            menu_path: "FTS/Session/Navigate",
            shortcut: "Down",
            when: "tab:performance",
        }
        PREVIOUS_SONG = "fts.session.previous_song" {
            name: "Previous Song",
            description: "Go to the previous song in the setlist",
            category: Session,
            menu_path: "FTS/Session/Navigate",
            shortcut: "Up",
            when: "tab:performance",
        }
        NEXT_SECTION = "fts.session.next_section" {
            name: "Next Section",
            description: "Go to the next section in the current song",
            category: Session,
            menu_path: "FTS/Session/Navigate",
        }
        PREVIOUS_SECTION = "fts.session.previous_section" {
            name: "Previous Section",
            description: "Go to the previous section in the current song",
            category: Session,
            menu_path: "FTS/Session/Navigate",
        }
        LOG_HELLO = "fts.session.log_hello" {
            name: "Log Hello",
            description: "Logs 'Hello from session!'",
            category: Dev,
            menu_path: "FTS/Session/Dev",
        }
        LOG_STATUS = "fts.session.log_status" {
            name: "Log Status",
            description: "Logs current session status",
            category: Dev,
            menu_path: "FTS/Session/Dev",
        }
    }
}
