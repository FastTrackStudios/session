//! Session Protocol - Shared types and service definitions for Session cell
//!
//! This crate provides:
//! - Service trait definitions (`SetlistService`, `SongService`, `SessionService`)
//! - Domain types (`Setlist`, `Song`, `Section`, etc.)
//! - Generated service clients and dispatchers
//!
//! For UI components, see the `session-ui` crate.

use roam::service;

// Re-export DefinesActions for convenience - session implements this
pub use actions_proto::{
    ActionCategory, ActionDefinition, ActionId, ActionResult, DefinesActions, DefinesActionsClient,
    DefinesActionsDispatcher,
};

// Domain modules
pub mod services;
pub mod setlist;
pub mod song;

// Re-export common types
pub use services::{
    AudioLatencyInfo, MeasureInfo, SetlistEvent, SetlistService, SetlistServiceClient,
    SetlistServiceDispatcher, SongService, SongServiceClient, SongServiceDispatcher,
    SongTransportState,
};
pub use setlist::{ActiveIndices, AdvanceMode, QueuedTarget, Setlist};
pub use song::{Comment, Section, SectionType, Song, SongChartHydration, SongDetectedChord};

// Re-export position types from daw-proto for convenience
pub use daw_proto::MusicalPosition;

/// Session service - provides session-specific functionality
#[service]
pub trait SessionService {
    /// Get session cell status
    async fn get_status(&self) -> String;
}
