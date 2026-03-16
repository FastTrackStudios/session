//! Playback commands: play, pause, stop, toggle, loop controls

use super::SetlistServiceImpl;
use daw::Daw;
use session_proto::SessionServiceError;
use tracing::{debug, warn};

impl SetlistServiceImpl {
    pub(crate) async fn toggle_playback_impl(&self) -> Result<(), SessionServiceError> {
        debug!("toggle_playback");

        // Use cached active song ID for instant lookup (no RPC calls)
        if let Some(song) = self.get_cached_active_song().await {
            let daw = Daw::get();
            match daw.project(&song.project_guid).await {
                Ok(project) => {
                    if let Err(e) = project.transport().play_pause().await {
                        warn!("Failed to toggle playback: {}", e);
                    }
                }
                Err(e) => {
                    warn!("Failed to get project {}: {}", song.project_guid, e);
                }
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
            let daw = Daw::get();
            match daw.project(&song.project_guid).await {
                Ok(project) => {
                    if let Err(e) = project.transport().play().await {
                        warn!("Failed to play: {}", e);
                    }
                }
                Err(e) => {
                    warn!("Failed to get project {}: {}", song.project_guid, e);
                }
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
            let daw = Daw::get();
            match daw.project(&song.project_guid).await {
                Ok(project) => {
                    if let Err(e) = project.transport().pause().await {
                        warn!("Failed to pause: {}", e);
                    }
                }
                Err(e) => {
                    warn!("Failed to get project {}: {}", song.project_guid, e);
                }
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
            let daw = Daw::get();
            match daw.project(&song.project_guid).await {
                Ok(project) => {
                    if let Err(e) = project.transport().stop().await {
                        warn!("Failed to stop: {}", e);
                    }
                }
                Err(e) => {
                    warn!("Failed to get project {}: {}", song.project_guid, e);
                }
            }
        } else {
            warn!("No active song to stop (navigate to a song first)");
        }
        Ok(())
    }

    pub(crate) async fn toggle_song_loop_impl(&self) -> Result<(), SessionServiceError> {
        debug!("toggle_song_loop");

        let daw = Daw::get();
        match daw.current_project().await {
            Ok(project) => {
                if let Err(e) = project.transport().toggle_loop().await {
                    warn!("Failed to toggle song loop: {}", e);
                }
            }
            Err(e) => {
                warn!("Failed to get current project: {}", e);
            }
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

        let daw = Daw::get();
        match daw.current_project().await {
            Ok(project) => {
                if let Err(e) = project.transport().set_loop(false).await {
                    warn!("Failed to clear loop: {}", e);
                }
            }
            Err(e) => {
                warn!("Failed to get current project: {}", e);
            }
        }
        Ok(())
    }
}
