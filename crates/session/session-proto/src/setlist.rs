//! Setlist domain types
//!
//! Represents a setlist as a collection of songs with playback state tracking.

use crate::song::Song;
use daw_proto::TimeRange;
use facet::Facet;

/// A queued navigation target
///
/// Represents a pending seek/navigation that hasn't been confirmed yet.
/// Only one target can be queued at a time. The queue clears when the
/// transport position reaches the target.
#[repr(u8)]
#[derive(Clone, Debug, PartialEq, Facet)]
pub enum QueuedTarget {
    /// Queued navigation to a section
    Section {
        song_index: usize,
        section_index: usize,
    },
    /// Queued navigation to a specific time position
    Time {
        song_index: usize,
        position_seconds: f64,
    },
    /// Queued navigation to a measure
    Measure { song_index: usize, measure: i32 },
    /// Queued navigation to a comment marker
    Comment {
        song_index: usize,
        position_seconds: f64,
    },
}

/// A complete setlist
///
/// A setlist is an ordered collection of songs, typically built from
/// multiple DAW projects or from sections within a single project.
#[derive(Clone, Debug, PartialEq, Facet)]
pub struct Setlist {
    /// Unique identifier
    pub id: Option<String>,
    /// Setlist name
    pub name: String,
    /// Songs in the setlist (in order)
    pub songs: Vec<Song>,
}

impl Default for Setlist {
    fn default() -> Self {
        Self {
            id: None,
            name: String::new(),
            songs: Vec::new(),
        }
    }
}

impl Setlist {
    /// Get total duration of all songs in the setlist
    pub fn total_duration(&self) -> f64 {
        self.songs.iter().map(|s| s.duration()).sum()
    }

    /// Get total duration including count-ins
    pub fn total_duration_with_count_in(&self) -> f64 {
        self.songs.iter().map(|s| s.duration_with_count_in()).sum()
    }

    /// Get a song by index
    pub fn get_song(&self, index: usize) -> Option<&Song> {
        self.songs.get(index)
    }

    /// Find the song containing a given absolute position
    pub fn song_at(&self, seconds: f64) -> Option<(usize, &Song)> {
        self.songs
            .iter()
            .enumerate()
            .find(|(_, song)| seconds >= song.start_seconds && seconds < song.end_seconds)
    }
}

/// Active playback state for the setlist
///
/// Tracks which song and section is currently active, along with
/// playback position and state.
#[derive(Clone, Debug, Default, PartialEq, Facet)]
pub struct ActiveIndices {
    /// Index of the currently active song (None if no song is active)
    pub song_index: Option<usize>,
    /// Index of the currently active section within the song
    pub section_index: Option<usize>,
    /// Index of the currently active slide (for lyrics/presentation)
    pub slide_index: Option<usize>,
    /// Progress through the current song (0.0 to 1.0)
    pub song_progress: Option<f64>,
    /// Progress through the current section (0.0 to 1.0)
    pub section_progress: Option<f64>,
    /// Whether playback is currently active
    pub is_playing: bool,
    /// Whether looping is enabled
    pub looping: bool,
    /// Current loop selection (if any)
    pub loop_selection: Option<TimeRange>,
    /// Queued navigation target (flashes until confirmed)
    pub queued_target: Option<QueuedTarget>,
}
