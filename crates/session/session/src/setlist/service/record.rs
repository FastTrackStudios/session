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
    D: Transport + Tracks + architect::MaybeSendSync,
{
    pub(crate) async fn record_impl(&self) -> Result<(), SessionServiceError>
    where
        D: Clone + 'static,
    {
        debug!("record");
        if let Some(song) = self.get_cached_active_song().await {
            let daw = self.daw.clone();
            let result = daw_proto::main_thread::query(move || {
                daw.record(ProjectContext::Project(song.project_guid))
            })
            .await;
            if let Some(Err(e)) = result {
                warn!("Failed to record: {}", e);
            }
        } else {
            warn!("No active song to record into (navigate to a song first)");
        }
        Ok(())
    }

    pub(crate) async fn stop_recording_impl(&self) -> Result<(), SessionServiceError>
    where
        D: Clone + 'static,
    {
        debug!("stop_recording");
        if let Some(song) = self.get_cached_active_song().await {
            let daw = self.daw.clone();
            let result = daw_proto::main_thread::query(move || {
                daw.stop_recording(ProjectContext::Project(song.project_guid))
            })
            .await;
            if let Some(Err(e)) = result {
                warn!("Failed to stop recording: {}", e);
            }
        } else {
            warn!("No active song to stop recording (navigate to a song first)");
        }
        Ok(())
    }

    pub(crate) async fn toggle_recording_impl(&self) -> Result<(), SessionServiceError>
    where
        D: Clone + 'static,
    {
        debug!("toggle_recording");
        if let Some(song) = self.get_cached_active_song().await {
            let daw = self.daw.clone();
            let result = daw_proto::main_thread::query(move || {
                daw.toggle_recording(ProjectContext::Project(song.project_guid))
            })
            .await;
            if let Some(Err(e)) = result {
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
    ) -> Result<(), SessionServiceError>
    where
        D: Clone + 'static,
    {
        debug!("set_song_record_arm: {armed}");
        let Some(song) = self.get_cached_active_song().await else {
            warn!("No active song to arm (navigate to a song first)");
            return Ok(());
        };
        let daw = self.daw.clone();
        let project_guid = song.project_guid.clone();
        let song_name = song.name.clone();
        daw_proto::main_thread::query(move || {
            let ctx = ProjectContext::Project(project_guid);
            let selected = daw.selected(ctx.clone());
            if selected.is_empty() {
                warn!(
                    "No tracks selected in '{}' to {}",
                    song_name,
                    if armed { "arm" } else { "disarm" }
                );
                return;
            }
            for track in selected {
                if let Err(e) = daw.set_armed(ctx.clone(), track.as_ref(), armed) {
                    warn!("Failed to set arm on track '{}': {}", track.name, e);
                }
            }
        })
        .await;
        Ok(())
    }
}
