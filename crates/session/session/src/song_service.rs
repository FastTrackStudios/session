//! SongService implementation — builds songs from the current DAW project.

use crate::song_builder::SongBuilder;
use daw_control::Daw;
use session_proto::{SessionServiceError, Song, SongService};
use tracing::{debug, info, warn};

/// Implementation of SongService
#[derive(Clone)]
pub struct SongServiceImpl;

impl SongServiceImpl {
    pub fn new() -> Self {
        Self
    }
}

impl SongService for SongServiceImpl {
    async fn build_from_current_project(&self) -> Result<Song, SessionServiceError> {
        debug!("build_from_current_project called");

        let daw = Daw::get();

        // Get current project
        let project = match daw.current_project().await {
            Ok(p) => p,
            Err(e) => {
                warn!("Failed to get current project: {}", e);
                return Err(SessionServiceError::DawError(format!(
                    "Failed to get current project: {e}"
                )));
            }
        };

        info!("Got current project: {}", project.guid());

        // Build song(s) from project — return the first song
        match SongBuilder::build(&project).await {
            Ok(songs) => {
                let song = songs
                    .into_iter()
                    .next()
                    .ok_or_else(|| SessionServiceError::not_found("Song", "current project"))?;
                info!(
                    "SONG BUILT: {} ({} sections)",
                    song.name,
                    song.sections.len()
                );
                Ok(song)
            }
            Err(e) => {
                warn!("Failed to build song from current project: {}", e);
                Err(SessionServiceError::HydrationError(format!(
                    "Failed to build song from current project: {e}"
                )))
            }
        }
    }

    async fn get_song(&self, project_guid: String) -> Result<Song, SessionServiceError> {
        info!(
            "SONG SERVICE: get_song called for project: {}",
            project_guid
        );

        let daw = Daw::get();

        // Get specific project
        let project = match daw.project(project_guid.clone()).await {
            Ok(p) => p,
            Err(e) => {
                warn!("Failed to get project {}: {}", project_guid, e);
                return Err(SessionServiceError::DawError(format!(
                    "Failed to get project {project_guid}: {e}"
                )));
            }
        };

        // Build song(s) from project — return the first song
        match SongBuilder::build(&project).await {
            Ok(songs) => {
                let song = songs
                    .into_iter()
                    .next()
                    .ok_or_else(|| SessionServiceError::not_found("Song", &project_guid))?;
                info!(
                    "SONG BUILT: {} ({} sections)",
                    song.name,
                    song.sections.len()
                );
                Ok(song)
            }
            Err(e) => {
                warn!("Failed to build song from project {}: {}", project_guid, e);
                Err(SessionServiceError::HydrationError(format!(
                    "Failed to build song from project {project_guid}: {e}"
                )))
            }
        }
    }
}
