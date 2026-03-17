//! Session Protocol - Shared types and service definitions for Session cell
//!
//! This crate provides:
//! - Service trait definitions (`SetlistService`, `SongService`, `SessionService`)
//! - Domain types (`Setlist`, `Song`, `Section`, etc.)
//! - Generated service clients and dispatchers
//!
//! For UI components, see the `session-ui` crate.

use roam::service;

// ─── Typed IDs ─────────────────────────────────────────────────

utils::typed_uuid_id!(
    /// Unique identifier for a song in a session setlist.
    SongId
);

utils::typed_uuid_id!(
    /// Unique identifier for a section within a song.
    SectionId
);

// Re-export DefinesActions for convenience - session implements this
pub use actions_proto::{
    ActionCategory, ActionDefinition, ActionId, ActionResult, DefinesActions, DefinesActionsClient,
    DefinesActionsDispatcher,
};

// Domain modules
pub mod offset_map;
pub mod routing_project;
pub mod ruler_lanes;
pub mod services;
pub mod setlist;
pub mod song;
pub mod track_structure;

// Re-export common types
pub use services::{
    AudioLatencyInfo, MeasureInfo, SessionServiceError, SetlistEvent, SetlistService,
    SetlistServiceClient, SetlistServiceDispatcher, SongService, SongServiceClient,
    SongServiceDispatcher, SongTransportState, WebClientService, WebClientServiceClient,
    WebClientServiceDispatcher, setlist_service_service_descriptor,
    song_service_service_descriptor, web_client_service_service_descriptor,
};
pub use offset_map::{SetlistOffsetMap, SongOffset};
pub use setlist::{ActiveIndices, AdvanceMode, QueuedTarget, Setlist};
pub use track_structure::{
    GuideTrackRole, ReferenceStructure, SetlistTrackStructure, SongTrackMapping, TrackEntry,
    TrackIdentity,
};
pub use routing_project::{LoopbackConfig, RoutingChannel, RoutingGroup};
pub use song::{Comment, Section, SectionType, Song, SongChartHydration, SongDetectedChord};
// Typed IDs are defined at crate root and re-exported here for convenience
// (SongId and SectionId are already in scope from the definitions above)

// Re-export position types from daw-proto for convenience
pub use daw::service::MusicalPosition;

/// Session service - provides session-specific functionality
#[service]
pub trait SessionService {
    /// Get session cell status
    async fn get_status(&self) -> String;
}
