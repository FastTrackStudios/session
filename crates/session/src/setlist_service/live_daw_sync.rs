//! Live DAW-level setlist sync — replicates actions from individual song tabs
//! to the combined setlist tab in real time via DAW service calls.
//!
//! Unlike file-based sync (which watches RPP files), this operates on live DAW
//! change streams: when a marker moves in Song B's tab, the corresponding
//! marker in the setlist tab is immediately updated with the offset applied.
//!
//! ## Item Resolution
//!
//! Items in the setlist have different GUIDs than their song-tab originals
//! (GUIDs are cleared during concatenation). To find the corresponding setlist
//! item, we maintain a [`SetlistItemIndex`] that maps `(take_name, track_name)`
//! → `Vec<(position, item_guid)>`. When a song item event arrives, we look up
//! by take name + offset-adjusted position to find the right setlist item.

use std::collections::HashMap;
use std::sync::Arc;

use daw::Project;
use daw::service::item::Item;
use daw::service::primitives::{Duration, PositionInSeconds};
use daw::service::{ItemEvent, MarkerEvent, RegionEvent};
use moire::sync::RwLock;
use session_proto::offset_map::SetlistOffsetMap;
use tracing::{debug, info, warn};

/// Position tolerance for matching items by position (0.01s = 10ms).
const POSITION_TOLERANCE: f64 = 0.01;

// ── Item Index ──────────────────────────────────────────────────────────────

/// Cached index of setlist items for fast lookup by take name + position.
///
/// Built by scanning all items in the setlist project and reading their
/// active take names. Rebuilt when the setlist structure changes.
pub struct SetlistItemIndex {
    /// Map from `(track_name, take_name)` → list of `(position_seconds, item_guid)`
    entries: HashMap<(String, String), Vec<(f64, String)>>,
}

impl SetlistItemIndex {
    /// Build the index by scanning all items in the setlist project.
    pub async fn build(setlist: &Project) -> Self {
        let mut entries: HashMap<(String, String), Vec<(f64, String)>> = HashMap::new();

        // Get all tracks
        let tracks = match setlist.tracks().all().await {
            Ok(t) => t,
            Err(e) => {
                warn!("Failed to get tracks for item index: {}", e);
                return Self { entries };
            }
        };

        for track_info in &tracks {
            let track = setlist.tracks().by_guid(&track_info.guid).await;
            let track_handle = match track {
                Ok(Some(t)) => t,
                _ => continue,
            };

            let items = match track_handle.items().all().await {
                Ok(items) => items,
                Err(_) => continue,
            };

            for item in &items {
                // Get the active take's name
                let take_name = Self::get_active_take_name(setlist, item).await;
                let pos = item.position.as_seconds();
                let key = (track_info.name.clone(), take_name);
                entries
                    .entry(key)
                    .or_default()
                    .push((pos, item.guid.clone()));
            }
        }

        let total: usize = entries.values().map(|v| v.len()).sum();
        debug!(
            "SetlistItemIndex built: {} entries across {} keys",
            total,
            entries.len()
        );

        Self { entries }
    }

    /// Find a setlist item by track name, take name, and expected position.
    ///
    /// Returns the item GUID if found within position tolerance.
    pub fn find(&self, track_name: &str, take_name: &str, expected_position: f64) -> Option<&str> {
        let key = (track_name.to_string(), take_name.to_string());
        let candidates = self.entries.get(&key)?;

        // Find the closest position match within tolerance
        candidates
            .iter()
            .filter(|(pos, _)| (pos - expected_position).abs() < POSITION_TOLERANCE)
            .min_by(|(a, _), (b, _)| {
                let da = (a - expected_position).abs();
                let db = (b - expected_position).abs();
                da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(_, guid)| guid.as_str())
    }

    /// Update position for an item after it moves.
    pub fn update_position(
        &mut self,
        track_name: &str,
        take_name: &str,
        item_guid: &str,
        new_position: f64,
    ) {
        let key = (track_name.to_string(), take_name.to_string());
        if let Some(entries) = self.entries.get_mut(&key) {
            if let Some(entry) = entries.iter_mut().find(|(_, g)| g == item_guid) {
                entry.0 = new_position;
            }
        }
    }

    async fn get_active_take_name(project: &Project, item: &Item) -> String {
        // Try to get the active take's name via the take API
        let item_handle = project.items().by_guid(&item.guid).await;
        if let Ok(Some(handle)) = item_handle {
            if let Ok(take) = handle.takes().active().await {
                if let Ok(take_info) = take.info().await {
                    return take_info.name;
                }
            }
        }
        // Fallback: use empty string
        String::new()
    }
}

// ── Sync Bridge ─────────────────────────────────────────────────────────────

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
    /// Shared item index for resolving song items → setlist items
    item_index: Arc<RwLock<SetlistItemIndex>>,
}

impl DawSyncBridge {
    /// Create a new sync bridge. Builds the item index asynchronously.
    pub async fn new(
        setlist_project: Project,
        song_projects: Vec<(usize, Project)>,
        offset_map: &SetlistOffsetMap,
    ) -> Self {
        let songs: Vec<SongTabBinding> = song_projects
            .into_iter()
            .map(|(idx, project)| {
                let offset = offset_map
                    .songs
                    .get(idx)
                    .map(|s| s.global_start_seconds)
                    .unwrap_or(0.0);
                SongTabBinding {
                    song_index: idx,
                    project,
                    offset_seconds: offset,
                }
            })
            .collect();

        info!("Building setlist item index...");
        let index = SetlistItemIndex::build(&setlist_project).await;
        let item_index = Arc::new(RwLock::new("session.daw_sync.item_index", index));

        info!("DawSyncBridge created with {} song bindings", songs.len());
        Self {
            setlist_project,
            songs,
            item_index,
        }
    }

    /// Rebuild the item index (call after structural setlist changes).
    pub async fn rebuild_index(&self) {
        let new_index = SetlistItemIndex::build(&self.setlist_project).await;
        *self.item_index.write().await = new_index;
        info!("Setlist item index rebuilt");
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
        let item_index = self.item_index.clone();

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
            });
        }

        // Item changes — uses the shared item index for resolution
        if let Ok(mut rx) = song.project.items().subscribe().await {
            let setlist = setlist.clone();
            let song_project = song.project.clone();
            let item_index = item_index.clone();
            moire::task::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(Some(event_ref)) => {
                            let event: ItemEvent = (*event_ref).clone();
                            handle_item_event(
                                &setlist,
                                &song_project,
                                &item_index,
                                &event,
                                offset,
                                song_idx,
                            )
                            .await;
                        }
                        Ok(None) => continue,
                        Err(_) => break,
                    }
                }
            });
        }

        info!(
            "Song {} subscriptions active (offset={:.1}s)",
            song_idx, offset
        );
    }
}

// ── Event Handlers ──────────────────────────────────────────────────────────

fn marker_seconds(marker: &daw::service::Marker) -> f64 {
    marker
        .position
        .time
        .as_ref()
        .map(|t| t.as_seconds())
        .unwrap_or(0.0)
}

async fn handle_marker_event(setlist: &Project, event: &MarkerEvent, offset: f64, song_idx: usize) {
    match event {
        MarkerEvent::Added(marker) => {
            let song_pos = marker_seconds(marker);
            let setlist_pos = song_pos + offset;
            debug!(
                "Song {} marker added '{}' {:.3}s → setlist {:.3}s",
                song_idx, marker.name, song_pos, setlist_pos
            );
            let _ = setlist.markers().add(setlist_pos, &marker.name).await;
        }
        MarkerEvent::Changed(marker) => {
            if let Some(id) = marker.id {
                let setlist_pos = marker_seconds(marker) + offset;
                debug!(
                    "Song {} marker changed '{}' → setlist {:.3}s",
                    song_idx, marker.name, setlist_pos
                );
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

async fn handle_region_event(setlist: &Project, event: &RegionEvent, offset: f64, song_idx: usize) {
    match event {
        RegionEvent::Added(region) => {
            let start = region.time_range.start_seconds() + offset;
            let end = region.time_range.end_seconds() + offset;
            debug!(
                "Song {} region added '{}' {:.1}→{:.1}s",
                song_idx, region.name, start, end
            );
            let _ = setlist.regions().add(start, end, &region.name).await;
        }
        RegionEvent::Changed(region) => {
            if let Some(id) = region.id {
                let start = region.time_range.start_seconds() + offset;
                let end = region.time_range.end_seconds() + offset;
                debug!(
                    "Song {} region changed '{}' → {:.1}→{:.1}s",
                    song_idx, region.name, start, end
                );
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

/// Resolve a song item's track and take name for index lookup.
async fn resolve_item_identity(
    song_project: &Project,
    item_guid: &str,
) -> Option<(String, String)> {
    // Get the item to find its track
    let item_handle = song_project.items().by_guid(item_guid).await.ok()??;
    let item_info = item_handle.info().await.ok()?;

    // Get track name
    let track = song_project
        .tracks()
        .by_guid(&item_info.track_guid)
        .await
        .ok()??;
    let track_info = track.info().await.ok()?;

    // Get active take name
    let take_name = if let Ok(take) = item_handle.takes().active().await {
        take.info().await.ok().map(|t| t.name).unwrap_or_default()
    } else {
        String::new()
    };

    Some((track_info.name, take_name))
}

async fn handle_item_event(
    setlist: &Project,
    song_project: &Project,
    item_index: &Arc<RwLock<SetlistItemIndex>>,
    event: &ItemEvent,
    offset: f64,
    song_idx: usize,
) {
    match event {
        ItemEvent::PositionChanged {
            item_guid,
            old_position,
            new_position,
            ..
        } => {
            let setlist_old_pos = old_position + offset;
            let setlist_new_pos = new_position + offset;

            // Resolve the item's identity in the song project
            let identity = resolve_item_identity(song_project, item_guid).await;
            let Some((track_name, take_name)) = identity else {
                debug!(
                    "Song {} item {} position changed but could not resolve identity",
                    song_idx, item_guid
                );
                return;
            };

            // Look up the corresponding setlist item
            let setlist_guid = {
                let index = item_index.read().await;
                index
                    .find(&track_name, &take_name, setlist_old_pos)
                    .map(String::from)
            };
            if let Some(setlist_guid) = setlist_guid {
                debug!(
                    "Song {} item '{}' on '{}' moved {:.3}→{:.3}s → setlist item {} → {:.3}s",
                    song_idx,
                    take_name,
                    track_name,
                    old_position,
                    new_position,
                    setlist_guid,
                    setlist_new_pos
                );
                if let Ok(Some(handle)) = setlist.items().by_guid(&setlist_guid).await {
                    let _ = handle
                        .set_position(PositionInSeconds::from_seconds(setlist_new_pos))
                        .await;
                }
                // Update the index with the new position
                item_index.write().await.update_position(
                    &track_name,
                    &take_name,
                    &setlist_guid,
                    setlist_new_pos,
                );
            } else {
                debug!(
                    "Song {} item '{}' on '{}' at {:.3}s not found in setlist index",
                    song_idx, take_name, track_name, setlist_old_pos
                );
            }
        }

        ItemEvent::LengthChanged {
            item_guid,
            old_length,
            new_length,
            ..
        } => {
            let identity = resolve_item_identity(song_project, item_guid).await;
            let Some((track_name, take_name)) = identity else {
                return;
            };

            // Get current position to find in index
            let pos = match song_project.items().by_guid(item_guid).await {
                Ok(Some(h)) => h
                    .info()
                    .await
                    .ok()
                    .map(|i| i.position.as_seconds() + offset),
                _ => None,
            };

            if let Some(pos) = pos {
                let index = item_index.read().await;
                if let Some(setlist_guid) = index.find(&track_name, &take_name, pos) {
                    debug!(
                        "Song {} item '{}' length {:.3}→{:.3}s → setlist item {}",
                        song_idx, take_name, old_length, new_length, setlist_guid
                    );
                    if let Ok(Some(handle)) = setlist.items().by_guid(setlist_guid).await {
                        let _ = handle.set_length(Duration::from_seconds(*new_length)).await;
                    }
                }
            }
        }

        ItemEvent::MuteChanged {
            item_guid, muted, ..
        } => {
            let identity = resolve_item_identity(song_project, item_guid).await;
            let Some((track_name, take_name)) = identity else {
                return;
            };

            let pos = match song_project.items().by_guid(item_guid).await {
                Ok(Some(h)) => h
                    .info()
                    .await
                    .ok()
                    .map(|i| i.position.as_seconds() + offset),
                _ => None,
            };

            if let Some(pos) = pos {
                let index = item_index.read().await;
                if let Some(setlist_guid) = index.find(&track_name, &take_name, pos) {
                    debug!(
                        "Song {} item '{}' mute={} → setlist item {}",
                        song_idx, take_name, muted, setlist_guid
                    );
                    if let Ok(Some(handle)) = setlist.items().by_guid(setlist_guid).await {
                        if *muted {
                            let _ = handle.mute().await;
                        } else {
                            let _ = handle.unmute().await;
                        }
                    }
                }
            }
        }

        _ => {}
    }
}
