//! Live DAW-level setlist sync — replicates actions from individual song tabs
//! to the combined setlist tab in real time via DAW service calls.
//!
//! Unlike file-based sync (which watches RPP files), this operates on live DAW
//! change streams: when a marker moves in Song B's tab, the corresponding
//! marker in the setlist tab is immediately updated with the offset applied.

use daw::Project;
use daw::service::{ItemEvent, MarkerEvent, RegionEvent};
use session_proto::offset_map::SetlistOffsetMap;
use tracing::{debug, info, warn};

/// Tracks the mapping between a song project tab and its section in the setlist.
struct SongTabBinding {
    song_index: usize,
    project: Project,
    offset_seconds: f64,
}

/// Bridge that replicates live DAW changes from song tabs to the setlist tab.
pub struct DawSyncBridge {
    setlist_project: Project,
    songs: Vec<SongTabBinding>,
}

impl DawSyncBridge {
    /// Create a new sync bridge.
    ///
    /// `song_projects` is `(song_index, project_handle)` for each song tab.
    pub fn new(
        setlist_project: Project,
        song_projects: Vec<(usize, Project)>,
        offset_map: &SetlistOffsetMap,
    ) -> Self {
        let songs: Vec<SongTabBinding> = song_projects
            .into_iter()
            .map(|(idx, project)| {
                let offset = offset_map.songs.get(idx)
                    .map(|s| s.global_start_seconds)
                    .unwrap_or(0.0);
                SongTabBinding { song_index: idx, project, offset_seconds: offset }
            })
            .collect();

        info!("DawSyncBridge created with {} song bindings", songs.len());
        Self { setlist_project, songs }
    }

    /// Start subscribing to all song tab change streams.
    pub async fn start(&self) {
        for song in &self.songs {
            self.subscribe_song(song).await;
        }
    }

    async fn subscribe_song(&self, song: &SongTabBinding) {
        let setlist = self.setlist_project.clone();
        let offset = song.offset_seconds;
        let song_idx = song.song_index;

        // Marker changes
        if let Ok(mut rx) = song.project.markers().subscribe().await {
            let setlist = setlist.clone();
            moire::task::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(Some(event_ref)) => {
                            let event: MarkerEvent = (*event_ref).clone();
                            handle_marker_event(&setlist, &event, offset, song_idx).await;
                        }
                        Ok(None) => continue,
                        Err(_) => break,
                    }
                }
                debug!("Marker subscription ended for song {}", song_idx);
            });
        }

        // Region changes
        if let Ok(mut rx) = song.project.regions().subscribe().await {
            let setlist = setlist.clone();
            moire::task::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(Some(event_ref)) => {
                            let event: RegionEvent = (*event_ref).clone();
                            handle_region_event(&setlist, &event, offset, song_idx).await;
                        }
                        Ok(None) => continue,
                        Err(_) => break,
                    }
                }
                debug!("Region subscription ended for song {}", song_idx);
            });
        }

        // Item changes
        if let Ok(mut rx) = song.project.items().subscribe().await {
            let setlist = setlist.clone();
            moire::task::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(Some(event_ref)) => {
                            let event: ItemEvent = (*event_ref).clone();
                            handle_item_event(&setlist, &event, offset, song_idx).await;
                        }
                        Ok(None) => continue,
                        Err(_) => break,
                    }
                }
                debug!("Item subscription ended for song {}", song_idx);
            });
        }

        info!("Song {} subscriptions active (offset={:.1}s)", song_idx, offset);
    }
}

// ── Event Handlers ──────────────────────────────────────────────────────────

/// Get seconds from a marker's position.
fn marker_seconds(marker: &daw::service::Marker) -> f64 {
    marker.position.time.as_ref().map(|t| t.as_seconds()).unwrap_or(0.0)
}

async fn handle_marker_event(
    setlist: &Project,
    event: &MarkerEvent,
    offset: f64,
    song_idx: usize,
) {
    match event {
        MarkerEvent::Added(marker) => {
            let song_pos = marker_seconds(marker);
            let setlist_pos = song_pos + offset;
            debug!("Song {} marker added '{}' {:.3}s → setlist {:.3}s", song_idx, marker.name, song_pos, setlist_pos);
            if let Err(e) = setlist.markers().add(setlist_pos, &marker.name).await {
                warn!("Failed to add marker to setlist: {}", e);
            }
        }
        MarkerEvent::Changed(marker) => {
            if let Some(id) = marker.id {
                let setlist_pos = marker_seconds(marker) + offset;
                debug!("Song {} marker changed '{}' → setlist {:.3}s", song_idx, marker.name, setlist_pos);
                // TODO: find setlist marker by name within song's time window
                let _ = setlist.markers().move_to(id, setlist_pos).await;
            }
        }
        MarkerEvent::Removed(id) => {
            debug!("Song {} marker removed id={}", song_idx, id);
            let _ = setlist.markers().remove(*id).await;
        }
        _ => {}
    }
}

async fn handle_region_event(
    setlist: &Project,
    event: &RegionEvent,
    offset: f64,
    song_idx: usize,
) {
    match event {
        RegionEvent::Added(region) => {
            let start = region.time_range.start_seconds() + offset;
            let end = region.time_range.end_seconds() + offset;
            debug!("Song {} region added '{}' {:.1}→{:.1}s in setlist", song_idx, region.name, start, end);
            if let Err(e) = setlist.regions().add(start, end, &region.name).await {
                warn!("Failed to add region to setlist: {}", e);
            }
        }
        RegionEvent::Changed(region) => {
            if let Some(id) = region.id {
                let start = region.time_range.start_seconds() + offset;
                let end = region.time_range.end_seconds() + offset;
                debug!("Song {} region changed '{}' → setlist {:.1}→{:.1}s", song_idx, region.name, start, end);
                let _ = setlist.regions().set_bounds(id, start, end).await;
            }
        }
        RegionEvent::Removed(id) => {
            debug!("Song {} region removed id={}", song_idx, id);
            let _ = setlist.regions().remove(*id).await;
        }
        _ => {}
    }
}

async fn handle_item_event(
    _setlist: &Project,
    event: &ItemEvent,
    offset: f64,
    song_idx: usize,
) {
    // Item mutations need a way to look up the setlist's copy of the item
    // (by name+position since GUIDs differ). This requires an item search API
    // that doesn't exist yet. For now, log the intent.
    match event {
        ItemEvent::PositionChanged { item_guid, old_position, new_position, .. } => {
            let setlist_new_pos = new_position + offset;
            debug!(
                "Song {} item {} moved {:.3}→{:.3}s → setlist {:.3}s (TODO: apply)",
                song_idx, item_guid, old_position, new_position, setlist_new_pos
            );
        }
        ItemEvent::LengthChanged { item_guid, old_length, new_length, .. } => {
            debug!(
                "Song {} item {} length {:.3}→{:.3}s (TODO: apply)",
                song_idx, item_guid, old_length, new_length
            );
        }
        ItemEvent::MuteChanged { item_guid, muted, .. } => {
            debug!("Song {} item {} mute={} (TODO: apply)", song_idx, item_guid, muted);
        }
        _ => {}
    }
}
