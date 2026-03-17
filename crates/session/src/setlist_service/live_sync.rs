//! Live setlist sync — watches individual song RPPs and applies changes
//! to the combined setlist in real time.
//!
//! ## Flow
//!
//! 1. [`SongFileWatcher`] detects a song RPP was saved to disk
//! 2. Parse the updated song RPP
//! 3. Diff against the song's section in the current setlist (with offset)
//! 4. Apply the diff to the setlist RPP in memory
//! 5. Write the updated setlist to disk
//! 6. Signal REAPER to reload the setlist project tab

use std::path::PathBuf;

use daw::file::diff::{self, ApplyOptions, DiffOptions};
use daw::file::io::read_project;
use daw::file::types::ReaperProject;
use session_proto::SetlistOffsetMap;
use tracing::{debug, info, warn};

use super::file_watcher::{SongFileChanged, SongFileWatcher};

/// State for live setlist sync.
pub struct LiveSyncState {
    /// Paths to individual song RPPs (indexed by song position in setlist)
    song_paths: Vec<PathBuf>,
    /// Path to the combined setlist RPP
    setlist_path: PathBuf,
    /// Offset map for position translation
    offset_map: SetlistOffsetMap,
    /// Last-known parsed version of each song (for diffing)
    song_snapshots: Vec<Option<ReaperProject>>,
    /// Current combined setlist in memory
    setlist_project: ReaperProject,
}

impl LiveSyncState {
    /// Create a new sync state from song paths and a generated setlist.
    pub fn new(
        song_paths: Vec<PathBuf>,
        setlist_path: PathBuf,
        offset_map: SetlistOffsetMap,
        setlist_project: ReaperProject,
    ) -> Self {
        // Take initial snapshots of each song
        let song_snapshots: Vec<Option<ReaperProject>> = song_paths
            .iter()
            .map(|p| read_project(p).ok())
            .collect();

        Self {
            song_paths,
            setlist_path,
            offset_map,
            song_snapshots,
            setlist_project,
        }
    }

    /// Handle a song file change: diff, apply, write.
    ///
    /// Returns `true` if the setlist was updated.
    pub fn handle_song_changed(&mut self, event: &SongFileChanged) -> bool {
        let idx = event.song_index;
        if idx >= self.song_paths.len() {
            warn!("Song index {} out of range (have {} songs)", idx, self.song_paths.len());
            return false;
        }

        // Parse the updated song
        let new_song = match read_project(&event.path) {
            Ok(project) => project,
            Err(e) => {
                warn!("Failed to parse updated song RPP {}: {}", event.path.display(), e);
                return false;
            }
        };

        // Get the old snapshot (or skip if we don't have one)
        let old_song = match &self.song_snapshots[idx] {
            Some(s) => s,
            None => {
                info!("No previous snapshot for song {}, taking initial snapshot", idx);
                self.song_snapshots[idx] = Some(new_song);
                return false;
            }
        };

        // Get offset for this song
        let song_offset = self.offset_map.songs.get(idx)
            .map(|s| s.global_start_seconds)
            .unwrap_or(0.0);
        let song_end = self.offset_map.songs.get(idx)
            .map(|s| s.global_start_seconds + s.duration_seconds)
            .unwrap_or(f64::MAX);

        // Diff the old song against the setlist section
        let diff_options = DiffOptions {
            position_offset: song_offset,
            time_window: Some((song_offset, song_end)),
            match_tracks_by_name: true,
            match_items_by_name: true,
        };

        let project_diff = diff::diff_projects_with_options(old_song, &self.setlist_project, &diff_options);

        if project_diff.is_empty() {
            debug!("No changes detected for song {} ({})", idx, event.song_name);
            self.song_snapshots[idx] = Some(new_song);
            return false;
        }

        info!(
            "Song {} ({}) changed: {}",
            idx, event.song_name, project_diff.summary()
        );

        // Now diff the NEW song against the setlist to find what needs updating
        let new_diff = diff::diff_projects_with_options(&new_song, &self.setlist_project, &diff_options);

        // Apply the diff to the setlist
        let apply_options = ApplyOptions {
            position_offset: song_offset,
            match_tracks_by_name: true,
        };

        diff::apply::apply_diff(&mut self.setlist_project, &new_diff, &apply_options);

        // Update the snapshot
        self.song_snapshots[idx] = Some(new_song);

        // Write the updated setlist to disk
        // TODO: use the RPP serializer once it's complete
        // For now, log that we would write
        info!(
            "Setlist updated, would write to {}",
            self.setlist_path.display()
        );

        true
    }
}

/// Start live sync for a setlist.
///
/// Creates a file watcher for all song RPPs and spawns a task that
/// processes change events.
pub fn start_live_sync(
    state: LiveSyncState,
) -> (moire::task::JoinHandle<()>, tokio::sync::mpsc::Receiver<SongFileChanged>) {
    // Create the file watcher
    let songs: Vec<_> = state.song_paths.iter().enumerate()
        .map(|(i, p)| (p.clone(), i, format!("Song {}", i + 1)))
        .collect();

    let (watcher, rx) = SongFileWatcher::new(songs);

    // Start the polling loop
    let _watcher_handle = watcher.start();

    // Create a channel for forwarding events (so caller can also react)
    let (fwd_tx, fwd_rx) = tokio::sync::mpsc::channel(64);

    // Spawn the sync processing task
    let handle = moire::task::spawn(async move {
        let mut state = state;
        let mut rx = rx;

        while let Some(event) = rx.recv().await {
            let updated = state.handle_song_changed(&event);
            if updated {
                // Forward the event so the caller knows a sync happened
                let _ = fwd_tx.send(event).await;
            }
        }

        debug!("Live sync task exiting (channel closed)");
    });

    (handle, fwd_rx)
}
