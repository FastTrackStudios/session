//! Recording commands: record / stop / toggle plus track-arm, all targeting
//! the **active song's** project (song-specific), mirroring the playback
//! commands in `playback.rs`.

use super::SetlistServiceImpl;
use daw::service::ProjectContext;
use daw::service::Tracks;
use daw::service::transport::service::Transport;
use session_proto::SessionServiceError;
use tracing::{debug, warn};

impl<D> SetlistServiceImpl<D>
where
    D: Transport + Tracks,
{
    pub(crate) async fn record_impl(&self) -> Result<(), SessionServiceError> {
        debug!("record");
        if let Some(song) = self.get_cached_active_song().await {
            if let Err(e) = self
                .daw
                .record(ProjectContext::Project(song.project_guid.clone()))
            {
                warn!("Failed to record: {}", e);
            }
        } else {
            warn!("No active song to record into (navigate to a song first)");
        }
        Ok(())
    }

    pub(crate) async fn stop_recording_impl(&self) -> Result<(), SessionServiceError> {
        debug!("stop_recording");
        if let Some(song) = self.get_cached_active_song().await {
            if let Err(e) = self
                .daw
                .stop_recording(ProjectContext::Project(song.project_guid.clone()))
            {
                warn!("Failed to stop recording: {}", e);
            }
        } else {
            warn!("No active song to stop recording (navigate to a song first)");
        }
        Ok(())
    }

    pub(crate) async fn toggle_recording_impl(&self) -> Result<(), SessionServiceError> {
        debug!("toggle_recording");
        if let Some(song) = self.get_cached_active_song().await {
            if let Err(e) = self
                .daw
                .toggle_recording(ProjectContext::Project(song.project_guid.clone()))
            {
                warn!("Failed to toggle recording: {}", e);
            }
        } else {
            warn!("No active song to toggle recording (navigate to a song first)");
        }
        Ok(())
    }

    /// Arm/disarm the selected tracks in the active song's project. Selected
    /// (not all) so click/guide/reference tracks aren't armed by accident —
    /// the performer selects what they're tracking, then arms.
    pub(crate) async fn set_song_record_arm_impl(
        &self,
        armed: bool,
    ) -> Result<(), SessionServiceError> {
        debug!("set_song_record_arm: {armed}");
        let Some(song) = self.get_cached_active_song().await else {
            warn!("No active song to arm (navigate to a song first)");
            return Ok(());
        };
        let ctx = ProjectContext::Project(song.project_guid.clone());
        let selected = self.daw.selected(ctx.clone());
        if selected.is_empty() {
            warn!(
                "No tracks selected in '{}' to {}",
                song.name,
                if armed { "arm" } else { "disarm" }
            );
            return Ok(());
        }
        for track in selected {
            if let Err(e) = self.daw.set_armed(ctx.clone(), track.as_ref(), armed) {
                warn!("Failed to set arm on track '{}': {}", track.name, e);
            }
        }
        Ok(())
    }
}
