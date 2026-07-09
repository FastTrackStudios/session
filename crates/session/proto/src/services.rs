//! Service trait definitions for session management
//!
//! These traits define the RPC interfaces for song and setlist operations.

use crate::setlist::{ActiveIndices, Setlist};
use crate::song::{Section, Song, SongChartHydration};
use crate::{SectionId, SongId};
use daw_proto::MusicalPosition;
use facet::Facet;
use serde::{Deserialize, Serialize};
use vox::Tx;

// ─── SessionServiceError ────────────────────────────────────────

/// Typed error for session service trait boundaries.
///
/// All methods return `Result<T, SessionServiceError>` using typed error variants
/// for structured diagnostics.
#[repr(C)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Facet, thiserror::Error)]
pub enum SessionServiceError {
    /// Entity not found by ID.
    #[error("{entity} not found: {id}")]
    NotFound { entity: String, id: String },

    /// A DAW operation failed.
    #[error("daw error: {0}")]
    DawError(String),

    /// Hydration (data enrichment) failed.
    #[error("hydration error: {0}")]
    HydrationError(String),

    /// Catch-all for unexpected failures.
    #[error("internal error: {0}")]
    Internal(String),
}

impl SessionServiceError {
    /// Convenience for creating a NotFound error.
    pub fn not_found(entity: impl Into<String>, id: impl ToString) -> Self {
        Self::NotFound {
            entity: entity.into(),
            id: id.to_string(),
        }
    }
}

impl From<String> for SessionServiceError {
    fn from(s: String) -> Self {
        Self::Internal(s)
    }
}

impl From<eyre::Report> for SessionServiceError {
    fn from(e: eyre::Report) -> Self {
        Self::Internal(format!("{e:#}"))
    }
}

/// Measure information for RPC
#[derive(Clone, Debug, PartialEq, Facet)]
pub struct MeasureInfo {
    /// Measure number
    pub measure: i32,
    /// Time in seconds
    pub time_seconds: f64,
    /// Time signature numerator
    pub time_sig_numerator: i32,
    /// Time signature denominator
    pub time_sig_denominator: i32,
}

/// Transport state for a specific song
#[derive(Clone, Debug, PartialEq, Facet)]
pub struct SongTransportState {
    /// Song index in the setlist
    pub song_index: usize,
    /// Current position with time, musical, and MIDI representations
    /// The musical position comes from REAPER's tempo map and properly
    /// accounts for tempo and time signature changes throughout the project
    pub position: daw_proto::Position,
    /// Position as progress within the song (0.0 - 1.0)
    pub progress: f64,
    /// Current section index (if in a section)
    pub section_index: Option<usize>,
    /// Progress within current section (0.0 - 1.0)
    pub section_progress: Option<f64>,
    /// Whether this song's project is currently playing
    pub is_playing: bool,
    /// Whether this song's project is looping
    pub is_looping: bool,
    /// Loop region start/end in seconds (relative to song start)
    /// Only present when looping is enabled and loop points are set
    pub loop_region: Option<daw_proto::LoopRegion>,
    /// Current tempo in BPM
    pub bpm: f64,
    /// Time signature numerator
    pub time_sig_num: u32,
    /// Time signature denominator
    pub time_sig_denom: u32,
}

impl Default for SongTransportState {
    fn default() -> Self {
        Self {
            song_index: 0,
            position: daw_proto::Position::default(),
            progress: 0.0,
            section_index: None,
            section_progress: None,
            is_playing: false,
            is_looping: false,
            loop_region: None,
            bpm: 120.0,
            time_sig_num: 4,
            time_sig_denom: 4,
        }
    }
}

/// Events emitted by the setlist service
#[repr(u8)]
#[derive(Clone, Debug, PartialEq, Facet)]
pub enum SetlistEvent {
    /// Setlist was changed/rebuilt
    SetlistChanged(Setlist),
    /// A single song entry was hydrated/updated in-place
    SongHydrated {
        song_id: SongId,
        index: usize,
        song: Song,
    },
    /// Chart/chord payload for a song was hydrated/updated.
    SongChartHydrated {
        song_id: SongId,
        index: usize,
        chart: SongChartHydration,
    },
    /// Active indices changed (which song/section is "current")
    ActiveIndicesChanged(ActiveIndices),
    /// Transport state updated for one or more songs
    /// This is sent at 60Hz and includes state for ALL songs that have active projects
    TransportUpdate(Vec<SongTransportState>),
    /// Entered a new song
    SongEntered {
        song_id: SongId,
        index: usize,
        song: Song,
    },
    /// Exited a song
    SongExited { song_id: SongId, index: usize },
    /// Entered a new section
    SectionEntered {
        song_id: SongId,
        song_index: usize,
        section_id: SectionId,
        section_index: usize,
        section: Section,
    },
    /// Exited a section
    SectionExited {
        song_id: SongId,
        song_index: usize,
        section_id: SectionId,
        section_index: usize,
    },
    /// Position changed (legacy - prefer TransportUpdate)
    PositionChanged {
        seconds: f64,
        indices: ActiveIndices,
    },
}

/// Service for building and retrieving song information
///
/// This service extracts song structure from DAW projects by analyzing
/// markers, regions, and tempo maps.
pub mod song_service {
    use super::{SessionServiceError, Song};

    #[architect::rpc]
    pub trait SongService {
        /// Build a song from the current active DAW project
        ///
        /// Analyzes the current project's markers (SONGSTART/SONGEND) and regions
        /// to extract song structure including sections, tempo, and time signature.
        ///
        /// Returns None if no valid song structure is found.
        async fn build_from_current_project(&self) -> Result<Song, SessionServiceError>;

        /// Get song information for a specific project by GUID
        ///
        /// Loads and analyzes the specified project to extract song information.
        async fn song(&self, project_guid: String) -> Result<Song, SessionServiceError>;
    }
}

pub use song_service::{
    Service as SongServiceLayer, SongService, SongServiceClient, SongServiceDispatcher,
    layer as song_service_layer, serve as serve_song_service, song_service_rpc_service_descriptor,
    song_service_service_descriptor,
};

/// Service for managing and controlling setlists
///
/// This service provides high-level control over setlist playback,
/// navigation, and state tracking.
pub mod setlist_service {
    use super::{
        ActiveIndices, AudioLatencyInfo, MeasureInfo, MusicalPosition, Section,
        SessionServiceError, Setlist, SetlistEvent, Song, Tx,
    };

    #[architect::rpc]
    pub trait SetlistService {
        // =========================================================================
        // Query Methods
        // =========================================================================

        /// Get the full setlist
        fn setlist(&self) -> Result<Setlist, SessionServiceError>;

        /// Get all songs in the setlist
        fn songs(&self) -> Result<Vec<Song>, SessionServiceError>;

        /// Get a specific song by index
        fn song(&self, index: usize) -> Result<Song, SessionServiceError>;

        /// Get all sections for a specific song
        fn sections(&self, song_index: usize) -> Result<Vec<Section>, SessionServiceError>;

        /// Get a specific section by song and section index
        fn section(
            &self,
            song_index: usize,
            section_index: usize,
        ) -> Result<Section, SessionServiceError>;

        /// Get measure information for a song
        fn measures(&self, song_index: usize) -> Result<Vec<MeasureInfo>, SessionServiceError>;

        // =========================================================================
        // Active State Queries
        // =========================================================================

        /// Get the currently active song
        fn active_song(&self) -> Result<Song, SessionServiceError>;

        /// Get the currently active section
        fn active_section(&self) -> Result<Section, SessionServiceError>;

        /// Get the song at a specific position in seconds
        fn song_at(&self, seconds: f64) -> Result<Song, SessionServiceError>;

        /// Get the section at a specific position in seconds
        fn section_at(&self, seconds: f64) -> Result<Section, SessionServiceError>;

        // =========================================================================
        // Navigation Commands
        // =========================================================================

        /// Navigate to a specific song by index
        async fn go_to_song(&self, index: usize) -> Result<(), SessionServiceError>;

        /// Navigate to the next song in the setlist
        async fn next_song(&self) -> Result<(), SessionServiceError>;

        /// Navigate to the previous song in the setlist
        async fn previous_song(&self) -> Result<(), SessionServiceError>;

        /// Navigate to a specific section within the current song
        async fn go_to_section(&self, index: usize) -> Result<(), SessionServiceError>;

        /// Navigate to the next section in the current song
        async fn next_section(&self) -> Result<(), SessionServiceError>;

        /// Navigate to the previous section in the current song
        async fn previous_section(&self) -> Result<(), SessionServiceError>;

        /// Seek to a specific time in the current song (song-relative seconds)
        async fn seek_to(&self, seconds: f64) -> Result<(), SessionServiceError>;

        /// Seek to a specific time within a song (absolute seconds from project start)
        ///
        /// This switches to the correct project and seeks to the specified time.
        async fn seek_to_time(
            &self,
            song_index: usize,
            seconds: f64,
        ) -> Result<(), SessionServiceError>;

        /// Seek to a specific song by index (goes to song start)
        async fn seek_to_song(&self, song_index: usize) -> Result<(), SessionServiceError>;

        /// Seek to a specific section within a song
        async fn seek_to_section(
            &self,
            song_index: usize,
            section_index: usize,
        ) -> Result<(), SessionServiceError>;

        /// Seek to a musical position within a song
        ///
        /// The position is relative to the song's start (measure 0 = first measure of the song).
        async fn seek_to_musical_position(
            &self,
            song_index: usize,
            position: MusicalPosition,
        ) -> Result<(), SessionServiceError>;

        /// Seek to a specific measure within a song (0-indexed from song start)
        ///
        /// This switches to the correct project and seeks to the start of the specified measure.
        async fn goto_measure(
            &self,
            song_index: usize,
            measure: i32,
        ) -> Result<(), SessionServiceError>;

        // =========================================================================
        // Playback Commands
        // =========================================================================

        /// Toggle playback (play/pause)
        async fn toggle_playback(&self) -> Result<(), SessionServiceError>;

        /// Start playback
        async fn play(&self) -> Result<(), SessionServiceError>;

        /// Pause playback
        async fn pause(&self) -> Result<(), SessionServiceError>;

        /// Stop playback
        async fn stop(&self) -> Result<(), SessionServiceError>;

        // =========================================================================
        // Loop Control
        // =========================================================================

        /// Toggle looping for the current song
        async fn toggle_song_loop(&self) -> Result<(), SessionServiceError>;

        /// Toggle looping for the current section
        async fn toggle_section_loop(&self) -> Result<(), SessionServiceError>;

        /// Set a custom loop region
        async fn set_loop_region(
            &self,
            start_seconds: f64,
            end_seconds: f64,
        ) -> Result<(), SessionServiceError>;

        /// Clear any active loop
        async fn clear_loop(&self) -> Result<(), SessionServiceError>;

        // =========================================================================
        // Recording — targets the active song's project (song-specific)
        // =========================================================================

        /// Start recording into the active song's project.
        async fn record(&self) -> Result<(), SessionServiceError>;

        /// Stop recording in the active song's project.
        async fn stop_recording(&self) -> Result<(), SessionServiceError>;

        /// Toggle recording in the active song's project.
        async fn toggle_recording(&self) -> Result<(), SessionServiceError>;

        /// Arm or disarm the selected tracks in the active song's project for
        /// recording. (Selected, not all — arming every track would capture
        /// silence onto click/guide/reference tracks.)
        async fn set_song_record_arm(&self, armed: bool) -> Result<(), SessionServiceError>;

        // =========================================================================
        // Build/Refresh
        // =========================================================================

        /// Build setlist from all currently open DAW projects
        ///
        /// Scans all open projects, extracts song structures from each,
        /// and assembles them into a complete setlist.
        async fn build_from_open_projects(&self) -> Result<(), SessionServiceError>;

        /// Refresh the setlist
        ///
        /// Rebuilds the setlist from open projects, updating any changes
        /// to song structures or project state.
        async fn refresh(&self) -> Result<(), SessionServiceError>;

        /// Stamp demo markers and regions into the current REAPER project
        ///
        /// Creates SONGSTART, SONGEND, =END, COUNT-IN markers and section regions
        /// (Intro, Verse, Chorus, Bridge, etc.) in the currently active project.
        /// After stamping, call `build_from_open_projects()` to build the setlist.
        async fn load_demo_setlist(&self) -> Result<(), SessionServiceError>;

        /// Generate a combined setlist project from all open song projects.
        ///
        /// Scans open projects, skips any that are already combined setlists
        /// (identified by ExtState `FTS/is_combined_setlist`), builds songs,
        /// generates a combined RPP with configurable gap between songs,
        /// and opens it as a new project tab.
        ///
        /// `gap_measures` — number of empty measures between songs (default: 2)
        async fn generate_combined_setlist(
            &self,
            gap_measures: u32,
        ) -> Result<String, SessionServiceError>;

        // =========================================================================
        // Subscriptions
        // =========================================================================

        /// Every setlist change, as it happens — remotes render from this
        /// stream instead of polling (architect `#[subscribe]`; works on
        /// wasm over WsLink, replacing the old reverse-dispatch
        /// WebClientService push).
        #[subscribe]
        fn events(&self) -> SetlistEvent;

        /// Active song/section cursor changes, as they happen.
        #[subscribe]
        fn active_indices(&self) -> ActiveIndices;

        // =========================================================================
        // Audio Engine (proxy)
        // =========================================================================

        /// Get audio output latency in seconds
        ///
        /// This proxies to the DAW's AudioEngineService. The latency represents
        /// the delay between when audio is rendered and when it reaches the speakers.
        /// Clients can use this to compensate visual display during playback.
        ///
        /// Returns 0.0 if the audio engine is not running or unavailable.
        async fn get_audio_latency(&self) -> Result<f64, SessionServiceError>;

        /// Get complete audio latency information
        ///
        /// Returns input/output latency, sample rate, and audio engine state.
        /// Useful for latency display badges.
        async fn get_audio_latency_info(&self) -> Result<AudioLatencyInfo, SessionServiceError>;
    }
}

pub use setlist_service::{
    Service as SetlistServiceLayer, SetlistService, SetlistServiceClient,
    SetlistServiceRpcDispatcher as SetlistServiceDispatcher, layer as setlist_service_layer,
    serve as serve_setlist_service, setlist_service_rpc_service_descriptor,
    setlist_service_rpc_service_descriptor as setlist_service_service_descriptor,
};

// =============================================================================
// Web Client Push Service (desktop → browser)
// =============================================================================


// ─── Mode control ──────────────────────────────────────────────────────
//
// FTS session modes (Organize / Write / Produce / Record / …). The data
// enum lives in the `session` crate (it carries layout / toolbar logic
// that doesn't belong in -proto). On the wire we use the stable
// lowercase slug — every implementation calls `Mode::from_slug` to
// reconstitute.

/// Service for inspecting + switching the active FTS session mode.
///
/// Mounted by `fts-extensions` and consumed by the CLI / desktop /
/// any other Vox peer (e.g. a mobile app). State is host-process-
/// global; per-project mode is a future extension.
pub mod session_mode_service {
    use super::{SessionServiceError, Tx};

    #[architect::rpc]
    pub trait SessionModeService {
        /// Lowercase slug of the currently active mode.
        /// E.g. `"organize"`, `"record"`.
        async fn current_mode(&self) -> Result<String, SessionServiceError>;

        /// Switch the active mode by slug. Idempotent if `slug`
        /// matches the current mode. Errors with `NotFound` if the
        /// slug doesn't map to a known mode.
        async fn set_mode(&self, slug: String) -> Result<(), SessionServiceError>;

        /// All known mode slugs in declaration order, for menus / CLI
        /// completion. Returned even when no mode change has happened
        /// yet (static set).
        async fn list_modes(&self) -> Result<Vec<String>, SessionServiceError>;

        /// Mode changes, as they happen: the new slug each time the
        /// active mode flips — `set_mode` over RPC, the in-REAPER hotkey
        /// action, or `restore_persisted_mode` at startup. Zero polling.
        #[subscribe]
        fn mode_changes(&self) -> String;
    }
}

pub use session_mode_service::{
    Service as SessionModeServiceLayer, SessionModeService, SessionModeServiceClient,
    SessionModeServiceDispatcher, layer as session_mode_service_layer,
    serve as serve_session_mode_service, session_mode_service_rpc_service_descriptor,
    session_mode_service_service_descriptor,
};

// ─── Take ranking ──────────────────────────────────────────────────────

/// Scope for [`TakeRankingService::apply_rank`]. Wire-side mirror of
/// the `Scope` enum in `session::take_ranking`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Facet)]
#[repr(u8)]
pub enum TakeRankScope {
    /// Active take of each selected item, marker at `(play_pos - 2s)`
    /// when playing or at edit cursor when stopped.
    PlayPosMinus2s,
    /// Active take of each selected item, marker at item start.
    ItemWide,
    /// Take under the mouse cursor, marker at mouse project-time.
    MouseCursor,
}

/// Rank level: 1..=3 stars (up-rank) or `Down`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Facet)]
#[repr(u8)]
pub enum TakeRankLevel {
    One,
    Two,
    Three,
    Down,
}

pub mod take_ranking_service {
    use super::{SessionServiceError, TakeRankLevel, TakeRankScope};

    #[architect::rpc]
    pub trait TakeRankingService {
        /// Apply `level` at `scope`. Behavior matches the session
        /// `take_ranking::apply` semantics: replace-if-near for
        /// position-based scopes, single marker per take for
        /// item-wide.
        async fn apply_rank(
            &self,
            scope: TakeRankScope,
            level: TakeRankLevel,
        ) -> Result<(), SessionServiceError>;
    }
}

pub use take_ranking_service::{
    Service as TakeRankingServiceLayer, TakeRankingService, TakeRankingServiceClient,
    TakeRankingServiceDispatcher, layer as take_ranking_service_layer,
    serve as serve_take_ranking_service, take_ranking_service_rpc_service_descriptor,
    take_ranking_service_service_descriptor,
};

// ─── Record control ────────────────────────────────────────────────────

pub mod record_control_service {
    use super::SessionServiceError;

    #[architect::rpc]
    pub trait RecordControlService {
        /// Stop the current recording (DELETE all recorded media this
        /// pass) and immediately start a fresh recording pass. One
        /// undo block.
        async fn restart_recording(&self) -> Result<(), SessionServiceError>;

        /// Toggle record monitor on selected tracks between **on (1)**
        /// and **off (0)** — skips auto/tape.
        async fn toggle_monitor_on_off(&self) -> Result<(), SessionServiceError>;

        /// Toggle record monitor on selected tracks between
        /// **auto/tape (2)** and **off (0)**.
        async fn toggle_monitor_tape_off(&self) -> Result<(), SessionServiceError>;
    }
}

pub use record_control_service::{
    RecordControlService, RecordControlServiceClient, RecordControlServiceDispatcher,
    Service as RecordControlServiceLayer, layer as record_control_service_layer,
    record_control_service_rpc_service_descriptor, record_control_service_service_descriptor,
    serve as serve_record_control_service,
};

/// Complete audio latency information for display
#[derive(Clone, Debug, Default, PartialEq, Facet)]
pub struct AudioLatencyInfo {
    /// Input latency in samples
    pub input_samples: u32,
    /// Output latency in samples
    pub output_samples: u32,
    /// Output latency in seconds
    pub output_seconds: f64,
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Whether audio engine is running
    pub is_running: bool,
}

// SelfRef compatibility: SetlistEvent has no lifetime parameters, so Ref<'a> = Self.
#[allow(unsafe_code)]
unsafe impl vox_types::Reborrow for SetlistEvent {
    type Ref<'a> = SetlistEvent;
}
