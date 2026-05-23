//! Navigation methods: go_to_song, next/previous song/section, seeking

use super::SetlistServiceImpl;
use daw::service::transport::service::Transport;
use daw::service::{ProjectContext, Projects, TempoMap};
use session_proto::{QueuedTarget, SessionServiceError, Song};
use std::time::Duration;
use tracing::{debug, info, warn};

fn project_ctx(project_guid: &str) -> ProjectContext {
    ProjectContext::Project(project_guid.to_string())
}

impl<D> SetlistServiceImpl<D>
where
    D: Projects + TempoMap + Transport,
{
    fn current_project_guid(&self) -> Option<String> {
        self.daw.current().map(|p| p.guid)
    }

    fn select_project(&self, project_guid: &str) -> bool {
        self.daw.select(project_guid)
    }

    /// Stop the previously active project if it's not playing.
    /// Called before switching to a new project so paused projects don't linger.
    pub(crate) async fn stop_previous_project_if_idle(&self, new_project_guid: &str) {
        let prev_song = self.get_cached_active_song().await;
        if let Some(prev) = prev_song
            && prev.project_guid != new_project_guid
        {
            let ctx = project_ctx(&prev.project_guid);
            let state = self.daw.get_play_state(ctx.clone());
            if state != daw::service::PlayState::Playing {
                debug!(
                    "Stopping idle project {} (state={:?}) before switching",
                    prev.project_guid, state
                );
                let _ = self.daw.stop(ctx);
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
        let Some(skeleton) = self.get_song_internal(song_index).await else {
            warn!("seek_to_song_internal: song {} not found", song_index);
            return;
        };

        // Check if we're already on the correct project (skip tab switch for same-project songs)
        let already_on_project = self
            .current_project_guid()
            .map(|guid| guid == skeleton.project_guid)
            .unwrap_or(false);

        if !already_on_project {
            self.stop_previous_project_if_idle(&skeleton.project_guid)
                .await;
        }
        self.set_active_song_id(skeleton.id.as_str()).await;

        if !already_on_project && !self.select_project(&skeleton.project_guid) {
            warn!(
                "seek_to_song_internal: failed to switch to project {}",
                skeleton.project_guid
            );
            return;
        }

        let song = match moire::time::timeout(
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

        let ctx = project_ctx(&skeleton.project_guid);
        let seek_pos = Self::song_seek_position(&song);
        if let Err(e) = self.daw.set_position(ctx.clone(), seek_pos) {
            warn!("seek_to_song_internal: failed to seek: {}", e);
        }
        // Start playback for auto-advance
        if let Err(e) = self.daw.play(ctx) {
            warn!("seek_to_song_internal: failed to start playback: {}", e);
        }
    }

    // =========================================================================
    // SetlistService trait: Navigation Commands
    // =========================================================================

    pub(crate) async fn go_to_song_impl(&self, index: usize) -> Result<(), SessionServiceError> {
        debug!("go_to_song: {}", index);
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
        if !self.select_project(&skeleton.project_guid) {
            warn!(
                "Failed to switch to project {} for song {}",
                skeleton.project_guid, index
            );
            return Ok(());
        }

        // Now try to hydrate (best-effort, with timeout to prevent freezes)
        let song = match moire::time::timeout(
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

        // Only seek if the project is NOT already playing
        // This preserves playback position when switching between songs
        let ctx = project_ctx(&skeleton.project_guid);
        let is_playing = self.daw.is_playing(ctx.clone());
        if !is_playing {
            let seek_pos = Self::song_seek_position(&song);
            if let Err(e) = self.daw.set_position(ctx.clone(), seek_pos) {
                warn!("Failed to set position for song {}: {}", index, e);
            }
        }

        // Log the actual cursor position after navigation
        let actual_pos = self.daw.get_position(ctx);
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

    pub(crate) async fn go_to_section_impl(&self, index: usize) -> Result<(), SessionServiceError> {
        debug!("go_to_section: {}", index);
        // Use cached indices for instant response (updated at 60Hz by polling loop)
        let active = self.get_cached_indices().await;

        if let Some(song_idx) = active.song_index
            && let Some(song) = self.ensure_song_hydrated(song_idx).await
        {
            // Queue the target immediately for visual feedback
            self.queue_target(QueuedTarget::Section {
                song_id: song.id.clone(),
                song_index: song_idx,
                section_index: index,
            })
            .await;

            if let Some(section) = song.sections.get(index) {
                if !self.select_project(&song.project_guid) {
                    warn!(
                        "Failed to switch to project {} for section navigation",
                        song.project_guid
                    );
                } else if let Err(e) = self
                    .daw
                    .set_position(project_ctx(&song.project_guid), section.start_seconds)
                {
                    warn!("Failed to navigate to section {}: {}", index, e);
                    self.clear_queued_target().await;
                } else {
                    info!(
                        "Navigated to section {} ({}) in song {} (project {})",
                        index, section.name, song.name, song.project_guid
                    );
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
        // Use cached indices for instant response (updated at 60Hz by polling loop)
        let active = self.get_cached_indices().await;

        if let Some(song_idx) = active.song_index
            && let Some(song) = self.ensure_song_hydrated(song_idx).await
        {
            let absolute_pos = song.start_seconds() + seconds;
            if let Err(e) = self
                .daw
                .set_position(project_ctx(&song.project_guid), absolute_pos)
            {
                warn!("Failed to seek to {}: {}", seconds, e);
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
        if let Some(song) = self.ensure_song_hydrated(song_index).await {
            // Queue the target immediately for visual feedback (comment marker)
            self.queue_target(QueuedTarget::Comment {
                song_id: song.id.clone(),
                song_index,
                position_seconds: seconds,
            })
            .await;
            if !self.select_project(&song.project_guid) {
                warn!(
                    "Failed to switch to project {} for song {}",
                    song.project_guid, song_index
                );
            } else if let Err(e) = self
                .daw
                .set_position(project_ctx(&song.project_guid), seconds)
            {
                warn!(
                    "Failed to seek to {} seconds in song {}: {}",
                    seconds, song_index, e
                );
                self.clear_queued_target().await;
            } else {
                info!(
                    "Seeked to {} seconds in song {} ({})",
                    seconds, song_index, song.name
                );
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
        // Get the skeleton song first so we can switch tabs even if hydration fails.
        let Some(skeleton) = self.get_song_internal(song_index).await else {
            warn!("seek_to_song: song {} not found in setlist", song_index);
            return Ok(());
        };

        // Check if we're already on the correct project (skip tab switch for same-project songs)
        let already_on_project = self
            .current_project_guid()
            .map(|guid| guid == skeleton.project_guid)
            .unwrap_or(false);

        // Only stop previous project if we're actually changing projects
        if !already_on_project {
            self.stop_previous_project_if_idle(&skeleton.project_guid)
                .await;
        }

        // Update the cached active song ID for fast playback commands
        self.set_active_song_id(skeleton.id.as_str()).await;

        // Switch to the correct project tab (or reuse if already there)
        if !already_on_project && !self.select_project(&skeleton.project_guid) {
            warn!(
                "Failed to switch to project {} for song {}",
                skeleton.project_guid, song_index
            );
            return Ok(());
        }

        // Now try to hydrate (best-effort, with timeout to prevent freezes)
        let song = match moire::time::timeout(
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

        // Only seek if the project is NOT already playing
        let ctx = project_ctx(&skeleton.project_guid);
        let is_playing = self.daw.is_playing(ctx.clone());
        if !is_playing {
            let seek_pos = Self::song_seek_position(&song);
            if let Err(e) = self.daw.set_position(ctx.clone(), seek_pos) {
                warn!("Failed to seek to song {}: {}", song_index, e);
            }
        }

        // Log the actual cursor position after navigation
        let actual_pos = self.daw.get_position(ctx);
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
        if let Some(song) = self.ensure_song_hydrated(song_index).await {
            if let Some(section) = song.sections.get(section_index) {
                if !self.select_project(&song.project_guid) {
                    warn!(
                        "Failed to switch to project {} for song {}",
                        song.project_guid, song_index
                    );
                } else if let Err(e) = self
                    .daw
                    .set_position(project_ctx(&song.project_guid), section.start_seconds)
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
            } else {
                warn!("Section {} not found in song {}", section_index, song_index);
            }
        } else {
            warn!("Song {} not found", song_index);
        }
        Ok(())
    }

    pub(crate) async fn seek_to_musical_position_impl(
        &self,
        song_index: usize,
        position: daw::service::MusicalPosition,
    ) -> Result<(), SessionServiceError> {
        debug!(
            "seek_to_musical_position: song={}, position={}.{}.{}",
            song_index, position.measure, position.beat, position.subdivision
        );
        if let Some(song) = self.ensure_song_hydrated(song_index).await {
            if !self.select_project(&song.project_guid) {
                warn!(
                    "Failed to switch to project {} for song {}",
                    song.project_guid, song_index
                );
            } else {
                let fraction = position.subdivision as f64 / 1000.0;
                let relative_seconds = self.daw.musical_to_time(
                    project_ctx(&song.project_guid),
                    position.measure,
                    position.beat,
                    fraction,
                );
                let absolute_pos = song.start_seconds() + relative_seconds;

                if let Err(e) = self
                    .daw
                    .set_position(project_ctx(&song.project_guid), absolute_pos)
                {
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
        if let Some(song) = self.ensure_song_hydrated(song_index).await {
            if !self.select_project(&song.project_guid) {
                warn!(
                    "Failed to switch to project {} for song {}",
                    song.project_guid, song_index
                );
            } else {
                let ctx = project_ctx(&song.project_guid);
                let seconds = self.daw.musical_to_time(ctx.clone(), measure, 1, 0.0);
                if let Err(e) = self.daw.set_position(ctx, seconds) {
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
        } else {
            warn!("Song {} not found", song_index);
        }
        Ok(())
    }
}
