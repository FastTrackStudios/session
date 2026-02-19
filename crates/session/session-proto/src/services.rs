//! Service trait definitions for session management
//!
//! These traits define the RPC interfaces for song and setlist operations.

use crate::setlist::{ActiveIndices, Setlist};
use crate::song::{Section, Song, SongChartHydration};
use daw_proto::MusicalPosition;
use facet::Facet;
use roam::Tx;
use roam::service;

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
    SongHydrated { index: usize, song: Song },
    /// Chart/chord payload for a song was hydrated/updated.
    SongChartHydrated {
        index: usize,
        chart: SongChartHydration,
    },
    /// Active indices changed (which song/section is "current")
    ActiveIndicesChanged(ActiveIndices),
    /// Transport state updated for one or more songs
    /// This is sent at 60Hz and includes state for ALL songs that have active projects
    TransportUpdate(Vec<SongTransportState>),
    /// Entered a new song
    SongEntered { index: usize, song: Song },
    /// Exited a song
    SongExited { index: usize },
    /// Entered a new section
    SectionEntered {
        song_index: usize,
        section_index: usize,
        section: Section,
    },
    /// Exited a section
    SectionExited {
        song_index: usize,
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
#[service]
pub trait SongService {
    /// Build a song from the current active DAW project
    ///
    /// Analyzes the current project's markers (SONGSTART/SONGEND) and regions
    /// to extract song structure including sections, tempo, and time signature.
    ///
    /// Returns None if no valid song structure is found.
    async fn build_from_current_project(&self) -> Option<Song>;

    /// Get song information for a specific project by GUID
    ///
    /// Loads and analyzes the specified project to extract song information.
    /// Returns None if the project doesn't exist or has no valid song structure.
    async fn get_song(&self, project_guid: String) -> Option<Song>;
}

/// Service for managing and controlling setlists
///
/// This service provides high-level control over setlist playback,
/// navigation, and state tracking.
#[service]
pub trait SetlistService {
    // =========================================================================
    // Query Methods
    // =========================================================================

    /// Get the full setlist
    async fn get_setlist(&self) -> Option<Setlist>;

    /// Get all songs in the setlist
    async fn get_songs(&self) -> Vec<Song>;

    /// Get a specific song by index
    async fn get_song(&self, index: usize) -> Option<Song>;

    /// Get all sections for a specific song
    async fn get_sections(&self, song_index: usize) -> Vec<Section>;

    /// Get a specific section by song and section index
    async fn get_section(&self, song_index: usize, section_index: usize) -> Option<Section>;

    /// Get measure information for a song
    async fn get_measures(&self, song_index: usize) -> Vec<MeasureInfo>;

    // =========================================================================
    // Active State Queries
    // =========================================================================

    /// Get the currently active song
    async fn get_active_song(&self) -> Option<Song>;

    /// Get the currently active section
    async fn get_active_section(&self) -> Option<Section>;

    /// Get the song at a specific position in seconds
    async fn get_song_at(&self, seconds: f64) -> Option<Song>;

    /// Get the section at a specific position in seconds
    async fn get_section_at(&self, seconds: f64) -> Option<Section>;

    // =========================================================================
    // Navigation Commands
    // =========================================================================

    /// Navigate to a specific song by index
    async fn go_to_song(&self, index: usize);

    /// Navigate to the next song in the setlist
    async fn next_song(&self);

    /// Navigate to the previous song in the setlist
    async fn previous_song(&self);

    /// Navigate to a specific section within the current song
    async fn go_to_section(&self, index: usize);

    /// Navigate to the next section in the current song
    async fn next_section(&self);

    /// Navigate to the previous section in the current song
    async fn previous_section(&self);

    /// Seek to a specific time in the current song (song-relative seconds)
    async fn seek_to(&self, seconds: f64);

    /// Seek to a specific time within a song (absolute seconds from project start)
    ///
    /// This switches to the correct project and seeks to the specified time.
    async fn seek_to_time(&self, song_index: usize, seconds: f64);

    /// Seek to a specific song by index (goes to song start)
    async fn seek_to_song(&self, song_index: usize);

    /// Seek to a specific section within a song
    async fn seek_to_section(&self, song_index: usize, section_index: usize);

    /// Seek to a musical position within a song
    ///
    /// The position is relative to the song's start (measure 0 = first measure of the song).
    async fn seek_to_musical_position(&self, song_index: usize, position: MusicalPosition);

    /// Seek to a specific measure within a song (0-indexed from song start)
    ///
    /// This switches to the correct project and seeks to the start of the specified measure.
    async fn goto_measure(&self, song_index: usize, measure: i32);

    // =========================================================================
    // Playback Commands
    // =========================================================================

    /// Toggle playback (play/pause)
    async fn toggle_playback(&self);

    /// Start playback
    async fn play(&self);

    /// Pause playback
    async fn pause(&self);

    /// Stop playback
    async fn stop(&self);

    // =========================================================================
    // Loop Control
    // =========================================================================

    /// Toggle looping for the current song
    async fn toggle_song_loop(&self);

    /// Toggle looping for the current section
    async fn toggle_section_loop(&self);

    /// Set a custom loop region
    async fn set_loop_region(&self, start_seconds: f64, end_seconds: f64);

    /// Clear any active loop
    async fn clear_loop(&self);

    // =========================================================================
    // Build/Refresh
    // =========================================================================

    /// Build setlist from all currently open DAW projects
    ///
    /// Scans all open projects, extracts song structures from each,
    /// and assembles them into a complete setlist.
    async fn build_from_open_projects(&self);

    /// Refresh the setlist
    ///
    /// Rebuilds the setlist from open projects, updating any changes
    /// to song structures or project state.
    async fn refresh(&self);

    // =========================================================================
    // Subscriptions
    // =========================================================================

    /// Subscribe to setlist events
    async fn subscribe(&self, events: Tx<SetlistEvent>);

    /// Subscribe to active indices changes
    async fn subscribe_active(&self, indices: Tx<ActiveIndices>);

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
    async fn get_audio_latency(&self) -> f64;

    /// Get complete audio latency information
    ///
    /// Returns input/output latency, sample rate, and audio engine state.
    /// Useful for latency display badges.
    async fn get_audio_latency_info(&self) -> AudioLatencyInfo;
}

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
