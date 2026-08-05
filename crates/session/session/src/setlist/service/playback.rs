//! Playback commands: play, pause, stop, toggle, loop controls

use super::SetlistServiceImpl;
use daw::service::ProjectContext;
use daw::service::transport::service::Transport;
use session_proto::SessionServiceError;
use tracing::{debug, warn};

impl<D> SetlistServiceImpl<D>
where
    D: Transport,
{
    pub(crate) async fn toggle_playback_impl(&self) -> Result<(), SessionServiceError> {
        debug!("toggle_playback");

        // Use cached active song ID for instant lookup (no RPC calls)
        if let Some(song) = self.get_cached_active_song().await {
            if let Err(e) = self
                .daw
                .play_pause(ProjectContext::Project(song.project_guid.clone()))
            {
                warn!("Failed to toggle playback: {}", e);
            }
        } else {
            warn!("No active song to toggle playback (navigate to a song first)");
        }
        Ok(())
    }

    pub(crate) async fn play_impl(&self) -> Result<(), SessionServiceError> {
        debug!("play");

        // Use cached active song ID for instant lookup (no RPC calls)
        if let Some(song) = self.get_cached_active_song().await {
            if let Err(e) = self
                .daw
                .play(ProjectContext::Project(song.project_guid.clone()))
            {
                warn!("Failed to play: {}", e);
            }
        } else {
            warn!("No active song to play (navigate to a song first)");
        }
        Ok(())
    }

    pub(crate) async fn pause_impl(&self) -> Result<(), SessionServiceError> {
        debug!("pause");

        // Use cached active song ID for instant lookup (no RPC calls)
        if let Some(song) = self.get_cached_active_song().await {
            if let Err(e) = self
                .daw
                .pause(ProjectContext::Project(song.project_guid.clone()))
            {
                warn!("Failed to pause: {}", e);
            }
        } else {
            warn!("No active song to pause (navigate to a song first)");
        }
        Ok(())
    }

    pub(crate) async fn stop_impl(&self) -> Result<(), SessionServiceError> {
        debug!("stop");

        // Use cached active song ID for instant lookup (no RPC calls)
        if let Some(song) = self.get_cached_active_song().await {
            if let Err(e) = self
                .daw
                .stop(ProjectContext::Project(song.project_guid.clone()))
            {
                warn!("Failed to stop: {}", e);
            }
        } else {
            warn!("No active song to stop (navigate to a song first)");
        }
        Ok(())
    }

    pub(crate) async fn toggle_song_loop_impl(&self) -> Result<(), SessionServiceError> {
        debug!("toggle_song_loop");

        if let Err(e) = self.daw.toggle_loop(ProjectContext::Current) {
            warn!("Failed to toggle song loop: {}", e);
        }
        Ok(())
    }

    pub(crate) async fn toggle_section_loop_impl(&self) -> Result<(), SessionServiceError> {
        debug!("toggle_section_loop");
        // TODO: Implement section-specific loop using loop points
        warn!("toggle_section_loop not yet implemented");
        Ok(())
    }

    pub(crate) async fn set_loop_region_impl(
        &self,
        _start_seconds: f64,
        _end_seconds: f64,
    ) -> Result<(), SessionServiceError> {
        debug!("set_loop_region: {} - {}", _start_seconds, _end_seconds);
        // TODO: Implement setting loop region
        warn!("set_loop_region not yet implemented");
        Ok(())
    }

    pub(crate) async fn clear_loop_impl(&self) -> Result<(), SessionServiceError> {
        debug!("clear_loop");

        if let Err(e) = self.daw.set_loop(ProjectContext::Current, false) {
            warn!("Failed to clear loop: {}", e);
        }
        Ok(())
    }
}
