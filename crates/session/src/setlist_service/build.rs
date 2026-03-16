//! Setlist assembly from open DAW projects

use super::{SetlistServiceImpl, HYDRATION_CONCURRENCY};
use daw::Daw;
use rustc_hash::{FxHashMap, FxHashSet};
use session_proto::{AdvanceMode, SessionServiceError, Setlist, Song};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tracing::{debug, info, warn};

impl SetlistServiceImpl {
    pub(crate) async fn build_from_open_projects_impl(&self) -> Result<(), SessionServiceError> {
        debug!("Building setlist from open projects...");

        // Check if DAW is initialized (it may not be ready yet after cell startup)
        let Some(daw) = Daw::try_get() else {
            warn!("DAW not initialized yet, cannot build setlist");
            return Ok(());
        };

        let build_generation = self.build_generation.fetch_add(1, Ordering::SeqCst) + 1;

        let projects = match daw.projects().await {
            Ok(projects) => projects,
            Err(e) => {
                warn!("Failed to enumerate open projects: {}", e);
                return Ok(());
            }
        };

        let current_project_guid = daw
            .current_project()
            .await
            .ok()
            .map(|project| project.guid().to_string());

        let project_loads = Self::fetch_project_loads(projects).await;
        if project_loads.is_empty() {
            let empty = Setlist {
                id: None,
                name: "Empty Setlist".to_string(),
                advance_mode: AdvanceMode::default(),
                songs: Vec::new(),
            };
            *self.setlist.write().await = Some(empty.clone());
            self.notify_setlist_changed();
            return Ok(());
        }

        let open_guids: FxHashSet<String> =
            project_loads.iter().map(|load| load.guid.clone()).collect();
        self.song_cache
            .retain(|guid, _| open_guids.contains(guid))
            .await;
        self.chart_cache
            .retain(|guid, _| open_guids.contains(guid))
            .await;
        self.last_chart_refresh_attempt
            .retain(|guid, _| open_guids.contains(guid))
            .await;

        let existing_setlist = self.setlist.read().await.clone();
        let existing_songs_by_guid: FxHashMap<String, Song> = existing_setlist
            .as_ref()
            .map(|sl| {
                sl.songs
                    .iter()
                    .cloned()
                    .map(|song| (song.project_guid.clone(), song))
                    .collect()
            })
            .unwrap_or_default();
        let cached_songs: FxHashMap<String, Song> = self
            .song_cache
            .with_read(|map| {
                map.iter()
                    .map(|(guid, entry)| (guid.clone(), entry.song.clone()))
                    .collect()
            })
            .await;

        let focused_index = current_project_guid
            .as_ref()
            .and_then(|guid| project_loads.iter().position(|load| &load.guid == guid))
            .unwrap_or(0);

        // Phase 1: focused project full details, all others names only.
        // A single project may produce multiple songs (multi-song mode).
        let mut songs = Vec::with_capacity(project_loads.len());
        let mut focused_song_count = 0usize;
        for load in &project_loads {
            let existing_song = existing_songs_by_guid.get(&load.guid);
            let cached_song = cached_songs.get(&load.guid);

            if load.index == focused_index {
                let project_songs = self.build_songs_with_cache(load, existing_song).await;
                if !project_songs.is_empty() {
                    for song in &project_songs {
                        self.cache_chart_payload_for_song(song).await;
                    }
                    let light_songs: Vec<Song> = project_songs
                        .into_iter()
                        .map(|mut s| {
                            Self::strip_song_chart_payload(&mut s);
                            s
                        })
                        .collect();
                    focused_song_count = light_songs.len();
                    songs.extend(light_songs);
                    continue;
                }
            }

            songs.push(Self::make_name_only_song(
                &load.guid,
                &load.project_name,
                existing_song,
                cached_song,
            ));
        }

        // Compute the actual song index of the first focused song
        let focused_song_start = project_loads
            .iter()
            .take_while(|l| l.index != focused_index)
            .count(); // one placeholder per project before focused

        let setlist = Setlist {
            id: None,
            name: format!("Setlist - {}", chrono::Local::now().format("%Y-%m-%d")),
            advance_mode: AdvanceMode::default(),
            songs,
        };

        if let Some(active_song) = setlist.songs.get(focused_song_start) {
            *self.active_song_id.write().await = Some(active_song.id.to_string());
            debug!(
                "Set initial active song to: {} ({})",
                active_song.name, active_song.id
            );
        }

        debug!(
            "Phase 1 complete: focused project ready ({} songs), {} total songs loaded",
            focused_song_count,
            setlist.songs.len()
        );
        *self.setlist.write().await = Some(setlist);
        self.notify_setlist_changed();
        if let Some(load) = project_loads.get(focused_index) {
            self.emit_cached_chart_payload_for_song(focused_song_start, &load.guid)
                .await;
        }

        // Phase 2: hydrate remaining projects in background with bounded concurrency.
        // Each project may produce multiple songs, so we splice by project_guid.
        let this = self.clone();
        tokio::spawn(async move {
            let semaphore = Arc::new(Semaphore::new(HYDRATION_CONCURRENCY));
            let mut join_set = JoinSet::new();

            for load in project_loads {
                if load.index == focused_index {
                    continue;
                }

                let permit = semaphore.clone();
                let this_clone = this.clone();
                let existing_song = existing_songs_by_guid.get(&load.guid).cloned();
                join_set.spawn(async move {
                    let _permit = permit.acquire_owned().await.expect("semaphore closed");
                    let songs = this_clone
                        .build_songs_with_cache(&load, existing_song.as_ref())
                        .await;
                    (load.guid, songs)
                });
            }

            while let Some(result) = join_set.join_next().await {
                if this.build_generation.load(Ordering::SeqCst) != build_generation {
                    info!("Skipping stale hydration results from older build generation");
                    return;
                }

                match result {
                    Ok((project_guid, hydrated_songs)) if !hydrated_songs.is_empty() => {
                        for song in &hydrated_songs {
                            this.cache_chart_payload_for_song(song).await;
                        }
                        let light_songs: Vec<Song> = hydrated_songs
                            .into_iter()
                            .map(|mut s| {
                                Self::strip_song_chart_payload(&mut s);
                                s
                            })
                            .collect();

                        // Replace the placeholder(s) for this project with hydrated songs.
                        let emitted_songs = {
                            let mut guard = this.setlist.write().await;
                            let Some(ref mut current_setlist) = *guard else {
                                return;
                            };
                            // Find the range of songs belonging to this project
                            let first = current_setlist
                                .songs
                                .iter()
                                .position(|s| s.project_guid == project_guid);
                            let Some(first_idx) = first else { continue };
                            let count = current_setlist.songs[first_idx..]
                                .iter()
                                .take_while(|s| s.project_guid == project_guid)
                                .count();
                            // Splice: remove old range, insert new songs
                            let end_idx = first_idx + count;
                            let indexed_songs: Vec<(usize, Song)> = light_songs
                                .into_iter()
                                .enumerate()
                                .map(|(i, s)| (first_idx + i, s))
                                .collect();
                            current_setlist.songs.splice(
                                first_idx..end_idx,
                                indexed_songs.iter().map(|(_, s)| s.clone()),
                            );
                            indexed_songs
                        };

                        this.notify_setlist_changed();
                        for (song_index, song) in &emitted_songs {
                            this.hydration_bus.emit((*song_index, song.clone()));
                            this.emit_cached_chart_payload_for_song(*song_index, &project_guid)
                                .await;
                        }
                    }
                    Ok((_project_guid, _)) => {
                        // Keep placeholder if build returned empty.
                    }
                    Err(e) => {
                        warn!("Song hydration worker failed: {}", e);
                    }
                }
            }
        });
        Ok(())
    }

    pub(crate) async fn refresh_impl(&self) -> Result<(), SessionServiceError> {
        info!("Refreshing setlist...");
        self.build_from_open_projects_impl().await?;
        Ok(())
    }
}
