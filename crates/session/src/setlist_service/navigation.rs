//! Navigation methods: go_to_song, next/previous song/section, seeking

use super::SetlistServiceImpl;
use daw_control::Daw;
use session_proto::{QueuedTarget, SessionServiceError, Song};
use std::time::Duration;
use tracing::{debug, info, warn};

impl SetlistServiceImpl {
    /// Stop the previously active project if it's not playing.
    /// Called before switching to a new project so paused projects don't linger.
    pub(crate) async fn stop_previous_project_if_idle(&self, new_project_guid: &str) {
        let prev_song = self.get_cached_active_song().await;
        if let Some(prev) = prev_song {
            if prev.project_guid != new_project_guid {
                let daw = Daw::get();
                if let Ok(prev_project) = daw.project(&prev.project_guid).await {
                    let transport = prev_project.transport();
                    if let Ok(state) = transport.get_play_state().await {
                        if state != daw_proto::PlayState::Playing {
                            debug!(
                                "Stopping idle project {} (state={:?}) before switching",
                                prev.project_guid, state
                            );
                            let _ = transport.stop().await;
                        }
                    }
                }
            }
        }
    }

    /// Compute the seek position for a song: count-in position if available,
    /// otherwise song start (SONGSTART marker).
    ///
    /// Song.start_seconds already includes the count-in offset (it's the count-in
    /// start, not SONGSTART). So the SONGSTART position is:
    ///   start_seconds + count_in_seconds
    ///
    /// We want to seek to start_seconds (the count-in position), but if that's
    /// negative (count-in marker is before timeline origin), fall back to SONGSTART.
    pub(crate) fn song_seek_position(song: &Song) -> f64 {
        let count_in_pos = song.start_seconds();
        if count_in_pos >= 0.0 {
            count_in_pos
        } else {
            // Count-in is before timeline origin — fall back to SONGSTART
            let songstart = song.start_seconds() + song.count_in_seconds.unwrap_or(0.0);
            debug!(
                "song_seek_position: count-in pos {:.2} is negative, falling back to SONGSTART {:.2}",
                count_in_pos, songstart
            );
            songstart.max(0.0)
        }
    }

    /// Navigate to a song by index without requiring an RPC Context.
    /// Used by the transport loop for auto-advance.
    pub(crate) async fn seek_to_song_internal(&self, song_index: usize) {
        info!("seek_to_song_internal: song_index={}", song_index);

        let daw = Daw::get();

        let Some(skeleton) = self.get_song_internal(song_index).await else {
            warn!("seek_to_song_internal: song {} not found", song_index);
            return;
        };

        // Check if we're already on the correct project (skip tab switch for same-project songs)
        let current_project = daw.current_project().await.ok();
        let already_on_project = current_project
            .as_ref()
            .map(|p| p.guid().to_string() == skeleton.project_guid)
            .unwrap_or(false);

        if !already_on_project {
            self.stop_previous_project_if_idle(&skeleton.project_guid)
                .await;
        }
        self.set_active_song_id(skeleton.id.as_str()).await;

        let project = if already_on_project {
            current_project.unwrap()
        } else {
            match daw.select_project(&skeleton.project_guid).await {
                Ok(project) => project,
                Err(e) => {
                    warn!(
                        "seek_to_song_internal: failed to switch to project {}: {}",
                        skeleton.project_guid, e
                    );
                    return;
                }
            }
        };

        let song = match tokio::time::timeout(
            Duration::from_secs(5),
            self.ensure_song_hydrated(song_index),
        )
        .await
        {
            Ok(Some(song)) => song,
            _ => {
                warn!(
                    "seek_to_song_internal: hydration failed for song {} ({})",
                    song_index, skeleton.name
                );
                return;
            }
        };

        let transport = project.transport();
        let seek_pos = Self::song_seek_position(&song);
        if let Err(e) = transport.set_position(seek_pos).await {
            warn!("seek_to_song_internal: failed to seek: {}", e);
        }
        // Start playback for auto-advance
        if let Err(e) = transport.play().await {
            warn!("seek_to_song_internal: failed to start playback: {}", e);
        }
    }

    // =========================================================================
    // SetlistService trait: Navigation Commands
    // =========================================================================

    pub(crate) async fn go_to_song_impl(&self, index: usize) -> Result<(), SessionServiceError> {
        debug!("go_to_song: {}", index);

        let daw = Daw::get();

        // Get the skeleton song first (no RPC, just reads from the setlist).
        // This gives us the project GUID so we can switch tabs even if hydration fails.
        let Some(skeleton) = self.get_song_internal(index).await else {
            warn!("go_to_song: song {} not found in setlist", index);
            return Ok(());
        };

        // Stop previous project if it's not actively playing
        self.stop_previous_project_if_idle(&skeleton.project_guid)
            .await;

        // Update the cached active song ID for fast playback commands
        self.set_active_song_id(skeleton.id.as_str()).await;

        // Switch to the correct project tab first — this must always happen
        let project = match daw.select_project(&skeleton.project_guid).await {
            Ok(project) => project,
            Err(e) => {
                warn!(
                    "Failed to switch to project {} for song {}: {}",
                    skeleton.project_guid, index, e
                );
                return Ok(());
            }
        };

        // Now try to hydrate (best-effort, with timeout to prevent freezes)
        let song = match tokio::time::timeout(
            Duration::from_secs(5),
            self.ensure_song_hydrated(index),
        )
        .await
        {
            Ok(Some(song)) => song,
            Ok(None) => {
                warn!(
                    "go_to_song: hydration returned None for song {} ({}), staying on project tab",
                    index, skeleton.name
                );
                return Ok(());
            }
            Err(_) => {
                warn!(
                    "go_to_song: hydration timed out for song {} ({}), staying on project tab",
                    index, skeleton.name
                );
                return Ok(());
            }
        };

        let transport = project.transport();

        // Only seek if the project is NOT already playing
        // This preserves playback position when switching between songs
        let is_playing = transport.is_playing().await.unwrap_or(false);
        if !is_playing {
            let seek_pos = Self::song_seek_position(&song);
            if let Err(e) = transport.set_position(seek_pos).await {
                warn!("Failed to set position for song {}: {}", index, e);
            }
        }

        // Log the actual cursor position after navigation
        let actual_pos = transport.get_position().await.unwrap_or(f64::NAN);
        info!(
            "go_to_song: song {} ({}) — seek_target={:.2}s, actual_pos={:.2}s, is_playing={}",
            index,
            song.name,
            Self::song_seek_position(&song),
            actual_pos,
            is_playing
        );
        Ok(())
    }

    pub(crate) async fn next_song_impl(&self) -> Result<(), SessionServiceError> {
        let active = self.get_cached_indices().await;
        info!("next_song: cached song_index={:?}", active.song_index);
        if let Some(current_idx) = active.song_index {
            let next_idx = current_idx + 1;
            self.go_to_song_impl(next_idx).await?;
        } else {
            warn!("next_song: no active song index, cannot navigate");
        }
        Ok(())
    }

    pub(crate) async fn previous_song_impl(&self) -> Result<(), SessionServiceError> {
        let active = self.get_cached_indices().await;
        info!("previous_song: cached song_index={:?}", active.song_index);
        if let Some(current_idx) = active.song_index {
            if current_idx > 0 {
                let prev_idx = current_idx - 1;
                self.go_to_song_impl(prev_idx).await?;
            } else {
                info!("previous_song: already at first song (index 0)");
            }
        } else {
            warn!("previous_song: no active song index, cannot navigate");
        }
        Ok(())
    }

    pub(crate) async fn go_to_section_impl(
        &self,
        index: usize,
    ) -> Result<(), SessionServiceError> {
        debug!("go_to_section: {}", index);

        let daw = Daw::get();
        // Use cached indices for instant response (updated at 60Hz by polling loop)
        let active = self.get_cached_indices().await;

        if let Some(song_idx) = active.song_index {
            if let Some(song) = self.ensure_song_hydrated(song_idx).await {
                // Queue the target immediately for visual feedback
                self.queue_target(QueuedTarget::Section {
                    song_id: song.id.clone(),
                    song_index: song_idx,
                    section_index: index,
                })
                .await;

                if let Some(section) = song.sections.get(index) {
                    // First, switch to the correct project (in case we're on a different one)
                    match daw.select_project(&song.project_guid).await {
                        Ok(project) => {
                            // Then seek to the section's start position
                            if let Err(e) = project
                                .transport()
                                .set_position(section.start_seconds)
                                .await
                            {
                                warn!("Failed to navigate to section {}: {}", index, e);
                                // Clear queue on failure
                                self.clear_queued_target().await;
                            } else {
                                info!(
                                    "Navigated to section {} ({}) in song {} (project {})",
                                    index, section.name, song.name, song.project_guid
                                );
                            }
                        }
                        Err(e) => {
                            warn!(
                                "Failed to switch to project {} for section navigation: {}",
                                song.project_guid, e
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn next_section_impl(&self) -> Result<(), SessionServiceError> {
        debug!("next_section");

        // Use cached indices for instant response (updated at 60Hz by polling loop)
        let active = self.get_cached_indices().await;
        if let Some(section_idx) = active.section_index {
            let next_idx = section_idx + 1;
            self.go_to_section_impl(next_idx).await?;
        }
        Ok(())
    }

    pub(crate) async fn previous_section_impl(&self) -> Result<(), SessionServiceError> {
        debug!("previous_section");

        // Use cached indices for instant response (updated at 60Hz by polling loop)
        let active = self.get_cached_indices().await;
        if let Some(section_idx) = active.section_index {
            // Smart previous: if we're past the beginning of the section (>5% progress),
            // go to the start of the current section. Only go to previous section
            // if we're already at/near the beginning.
            let at_section_start = active
                .section_progress
                .map(|p| p < 0.05) // Within first 5% of section
                .unwrap_or(true);

            if at_section_start && section_idx > 0 {
                // Already at the start, go to previous section
                let prev_idx = section_idx - 1;
                self.go_to_section_impl(prev_idx).await?;
            } else {
                // Not at start, go to beginning of current section
                self.go_to_section_impl(section_idx).await?;
            }
        }
        Ok(())
    }

    pub(crate) async fn seek_to_impl(&self, seconds: f64) -> Result<(), SessionServiceError> {
        debug!("seek_to: {}", seconds);

        let daw = Daw::get();
        // Use cached indices for instant response (updated at 60Hz by polling loop)
        let active = self.get_cached_indices().await;

        if let Some(song_idx) = active.song_index {
            if let Some(song) = self.ensure_song_hydrated(song_idx).await {
                let absolute_pos = song.start_seconds() + seconds;
                match daw.project(&song.project_guid).await {
                    Ok(project) => {
                        if let Err(e) = project.transport().set_position(absolute_pos).await {
                            warn!("Failed to seek to {}: {}", seconds, e);
                        }
                    }
                    Err(e) => {
                        warn!("Failed to get project: {}", e);
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) async fn seek_to_time_impl(
        &self,
        song_index: usize,
        seconds: f64,
    ) -> Result<(), SessionServiceError> {
        info!(
            "seek_to_time: song_index={}, seconds={}",
            song_index, seconds
        );

        let daw = Daw::get();

        if let Some(song) = self.ensure_song_hydrated(song_index).await {
            // Queue the target immediately for visual feedback (comment marker)
            self.queue_target(QueuedTarget::Comment {
                song_id: song.id.clone(),
                song_index,
                position_seconds: seconds,
            })
            .await;
            // First, switch to the correct project
            match daw.select_project(&song.project_guid).await {
                Ok(project) => {
                    // Seek to the absolute time position
                    if let Err(e) = project.transport().set_position(seconds).await {
                        warn!(
                            "Failed to seek to {} seconds in song {}: {}",
                            seconds, song_index, e
                        );
                        // Clear queue on failure
                        self.clear_queued_target().await;
                    } else {
                        info!(
                            "Seeked to {} seconds in song {} ({})",
                            seconds, song_index, song.name
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to switch to project {} for song {}: {}",
                        song.project_guid, song_index, e
                    );
                }
            }
        } else {
            warn!("Song {} not found", song_index);
        }
        Ok(())
    }

    pub(crate) async fn seek_to_song_impl(
        &self,
        song_index: usize,
    ) -> Result<(), SessionServiceError> {
        info!("seek_to_song called: song_index={}", song_index);

        let daw = Daw::get();

        // Get the skeleton song first so we can switch tabs even if hydration fails.
        let Some(skeleton) = self.get_song_internal(song_index).await else {
            warn!("seek_to_song: song {} not found in setlist", song_index);
            return Ok(());
        };

        // Check if we're already on the correct project (skip tab switch for same-project songs)
        let current_project = daw.current_project().await.ok();
        let already_on_project = current_project
            .as_ref()
            .map(|p| p.guid().to_string() == skeleton.project_guid)
            .unwrap_or(false);

        // Only stop previous project if we're actually changing projects
        if !already_on_project {
            self.stop_previous_project_if_idle(&skeleton.project_guid)
                .await;
        }

        // Update the cached active song ID for fast playback commands
        self.set_active_song_id(skeleton.id.as_str()).await;

        // Switch to the correct project tab (or reuse if already there)
        let project = if already_on_project {
            current_project.unwrap()
        } else {
            match daw.select_project(&skeleton.project_guid).await {
                Ok(project) => project,
                Err(e) => {
                    warn!(
                        "Failed to switch to project {} for song {}: {}",
                        skeleton.project_guid, song_index, e
                    );
                    return Ok(());
                }
            }
        };

        // Now try to hydrate (best-effort, with timeout to prevent freezes)
        let song = match tokio::time::timeout(
            Duration::from_secs(5),
            self.ensure_song_hydrated(song_index),
        )
        .await
        {
            Ok(Some(song)) => song,
            Ok(None) => {
                warn!(
                    "seek_to_song: hydration returned None for song {} ({}), staying on project tab",
                    song_index, skeleton.name
                );
                return Ok(());
            }
            Err(_) => {
                warn!(
                    "seek_to_song: hydration timed out for song {} ({}), staying on project tab",
                    song_index, skeleton.name
                );
                return Ok(());
            }
        };

        let transport = project.transport();

        // Only seek if the project is NOT already playing
        let is_playing = transport.is_playing().await.unwrap_or(false);
        if !is_playing {
            let seek_pos = Self::song_seek_position(&song);
            if let Err(e) = transport.set_position(seek_pos).await {
                warn!("Failed to seek to song {}: {}", song_index, e);
            }
        }

        // Log the actual cursor position after navigation
        let actual_pos = transport.get_position().await.unwrap_or(f64::NAN);
        info!(
            "seek_to_song: song {} ({}) — seek_target={:.2}s, actual_pos={:.2}s, is_playing={}",
            song_index,
            song.name,
            Self::song_seek_position(&song),
            actual_pos,
            is_playing
        );
        Ok(())
    }

    pub(crate) async fn seek_to_section_impl(
        &self,
        song_index: usize,
        section_index: usize,
    ) -> Result<(), SessionServiceError> {
        debug!(
            "seek_to_section: song={}, section={}",
            song_index, section_index
        );

        let daw = Daw::get();

        if let Some(song) = self.ensure_song_hydrated(song_index).await {
            if let Some(section) = song.sections.get(section_index) {
                // First, switch to the correct project
                match daw.select_project(&song.project_guid).await {
                    Ok(project) => {
                        // Then seek to the section's start position
                        if let Err(e) = project
                            .transport()
                            .set_position(section.start_seconds)
                            .await
                        {
                            warn!(
                                "Failed to seek to section {} in song {}: {}",
                                section_index, song_index, e
                            );
                        } else {
                            info!(
                                "Seeked to section {} ({}) in song {} (project {})",
                                section_index, section.name, song.name, song.project_guid
                            );
                        }
                    }
                    Err(e) => {
                        warn!(
                            "Failed to switch to project {} for song {}: {}",
                            song.project_guid, song_index, e
                        );
                    }
                }
            } else {
                warn!(
                    "Section {} not found in song {}",
                    section_index, song_index
                );
            }
        } else {
            warn!("Song {} not found", song_index);
        }
        Ok(())
    }

    pub(crate) async fn seek_to_musical_position_impl(
        &self,
        song_index: usize,
        position: daw_proto::MusicalPosition,
    ) -> Result<(), SessionServiceError> {
        debug!(
            "seek_to_musical_position: song={}, position={}.{}.{}",
            song_index, position.measure, position.beat, position.subdivision
        );

        let daw = Daw::get();

        if let Some(song) = self.ensure_song_hydrated(song_index).await {
            // First, switch to the correct project
            match daw.select_project(&song.project_guid).await {
                Ok(project) => {
                    // The musical position is relative to the song start
                    // We need to convert it to absolute position using the tempo map
                    let fraction = position.subdivision as f64 / 1000.0;
                    let relative_seconds = project
                        .tempo_map()
                        .musical_to_time(position.measure, position.beat, fraction)
                        .await
                        .unwrap_or(0.0);

                    // Add the song start offset to get absolute position
                    let absolute_pos = song.start_seconds() + relative_seconds;

                    if let Err(e) = project.transport().set_position(absolute_pos).await {
                        warn!(
                            "Failed to seek to musical position in song {}: {}",
                            song_index, e
                        );
                    } else {
                        info!(
                            "Seeked to {}.{}.{} in song {} (project {})",
                            position.measure,
                            position.beat,
                            position.subdivision,
                            song.name,
                            song.project_guid
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to switch to project {} for song {}: {}",
                        song.project_guid, song_index, e
                    );
                }
            }
        } else {
            warn!("Song {} not found", song_index);
        }
        Ok(())
    }

    pub(crate) async fn goto_measure_impl(
        &self,
        song_index: usize,
        measure: i32,
    ) -> Result<(), SessionServiceError> {
        info!(
            "goto_measure: song_index={}, measure={}",
            song_index, measure
        );

        let daw = Daw::get();

        if let Some(song) = self.ensure_song_hydrated(song_index).await {
            // First, switch to the correct project
            match daw.select_project(&song.project_guid).await {
                Ok(project) => {
                    // Use the transport's goto_measure which handles tempo map conversion
                    if let Err(e) = project.transport().goto_measure(measure).await {
                        warn!(
                            "Failed to goto measure {} in song {}: {}",
                            measure, song_index, e
                        );
                    } else {
                        info!(
                            "Went to measure {} in song {} ({})",
                            measure, song.name, song.project_guid
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to switch to project {} for song {}: {}",
                        song.project_guid, song_index, e
                    );
                }
            }
        } else {
            warn!("Song {} not found", song_index);
        }
        Ok(())
    }
}
