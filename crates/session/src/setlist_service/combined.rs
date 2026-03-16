//! Combined setlist generation — merges all open song projects into a single
//! REAPER project with songs laid out sequentially on the timeline.

use crate::setlist_service::SetlistServiceImpl;
use crate::song_builder::SongBuilder;
use daw::Daw;
use session_proto::offset_map::SetlistOffsetMap;
use session_proto::SessionServiceError;
use std::path::PathBuf;
use tracing::{debug, info, warn};

/// ExtState section/key used to identify combined setlist projects.
const COMBINED_EXT_SECTION: &str = "FTS";
const COMBINED_EXT_KEY: &str = "is_combined_setlist";

impl SetlistServiceImpl {
    /// Generate a combined setlist project from open song projects.
    ///
    /// Pipeline:
    /// 1. Save all open projects to disk
    /// 2. Enumerate projects, skip combined setlists
    /// 3. Read each song project's RPP file from disk
    /// 4. Build offset map with gap between songs
    /// 5. Concatenate into combined RPP
    /// 6. Write combined RPP to disk
    /// 7. Open it as a new REAPER tab
    /// 8. Mark the new tab with ExtState so it's skipped next time
    ///
    /// Returns the GUID of the newly opened combined project.
    pub(crate) async fn generate_combined_setlist_impl(
        &self,
        gap_measures: u32,
    ) -> Result<String, SessionServiceError> {
        let daw = Daw::get();

        // ── 1. Save all open projects to ensure RPP files are current ─
        info!("Saving all open projects before generating combined setlist...");
        daw.save_all_projects()
            .await
            .map_err(|e| SessionServiceError::DawError(format!("Failed to save projects: {e}")))?;

        // Small delay to let REAPER finish writing files
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;

        // ── 2. Get all open projects, skip combined setlists ─────────
        let projects = daw
            .projects()
            .await
            .map_err(|e| SessionServiceError::DawError(format!("Failed to list projects: {e}")))?;

        let mut song_projects = Vec::new();
        for project in &projects {
            let is_combined = project
                .ext_state()
                .get(COMBINED_EXT_SECTION, COMBINED_EXT_KEY)
                .await
                .unwrap_or(None)
                .map(|v| v == "1")
                .unwrap_or(false);

            if is_combined {
                debug!("Skipping combined setlist project: {}", project.guid());
                continue;
            }

            let info = project
                .info()
                .await
                .map_err(|e| SessionServiceError::DawError(format!("{e}")))?;

            // Skip unsaved projects (no file path)
            if info.path.is_empty() {
                debug!("Skipping unsaved project: {}", info.name);
                continue;
            }

            song_projects.push((project.clone(), info));
        }

        if song_projects.is_empty() {
            return Err(SessionServiceError::Internal(
                "No saved song projects found".to_string(),
            ));
        }

        info!(
            "Generating combined setlist from {} projects",
            song_projects.len()
        );

        // ── 3. Build songs from each project (for metadata) ──────────
        let mut songs = Vec::new();
        let mut rpp_paths = Vec::new();

        for (project, info) in &song_projects {
            match SongBuilder::build(project).await {
                Ok(project_songs) => {
                    for song in project_songs {
                        debug!(
                            "  Song: {} ({:.1}s, {:.0} BPM) from {}",
                            song.name,
                            song.duration(),
                            song.tempo.unwrap_or(120.0),
                            info.path,
                        );
                        songs.push(song);
                    }
                    rpp_paths.push(PathBuf::from(&info.path));
                }
                Err(e) => {
                    warn!("Failed to extract songs from {}: {e}", info.name);
                }
            }
        }

        if songs.is_empty() {
            return Err(SessionServiceError::Internal(
                "No songs found in any project".to_string(),
            ));
        }

        // ── 4. Parse RPP files from disk ─────────────────────────────
        let mut parsed_projects = Vec::new();
        for path in &rpp_paths {
            match daw::file::read_project(path) {
                Ok(project) => parsed_projects.push(project),
                Err(e) => {
                    warn!("Failed to parse RPP file {}: {e}", path.display());
                    return Err(SessionServiceError::Internal(format!(
                        "Failed to parse {}: {e}",
                        path.display()
                    )));
                }
            }
        }

        // ── 5. Build offset map + song infos with gap ────────────────
        let default_tempo = songs.first().and_then(|s| s.tempo).unwrap_or(120.0);
        let gap_seconds = daw::file::setlist_rpp::measures_to_seconds(gap_measures, default_tempo, 4);

        let song_entries: Vec<(&str, f64)> = songs
            .iter()
            .map(|s| (s.name.as_str(), s.duration()))
            .collect();
        let song_infos = daw::file::setlist_rpp::build_song_infos(&song_entries, gap_seconds);

        // ── 6. Concatenate projects ──────────────────────────────────
        let combined = daw::file::setlist_rpp::concatenate_projects(&parsed_projects, &song_infos);

        // ── 7. Serialize and write combined RPP ──────────────────────
        let rpp_text = daw::file::setlist_rpp::project_to_rpp_text(&combined);

        // Write next to the first song's project file
        let output_dir = rpp_paths
            .first()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(std::env::temp_dir);
        let output_path = output_dir.join("Combined Setlist.RPP");

        std::fs::write(&output_path, &rpp_text).map_err(|e| {
            SessionServiceError::Internal(format!(
                "Failed to write combined RPP to {}: {e}",
                output_path.display()
            ))
        })?;

        info!(
            "Combined setlist RPP written: {} ({} tracks, {} markers/regions, {:.0} bytes)",
            output_path.display(),
            combined.tracks.len(),
            combined.markers_regions.all.len(),
            rpp_text.len(),
        );

        // ── 8. Open in REAPER as a new tab ───────────────────────────
        let new_project = daw
            .open_project(output_path.to_string_lossy().to_string())
            .await
            .map_err(|e| {
                SessionServiceError::DawError(format!("Failed to open combined setlist: {e}"))
            })?;

        // Mark as combined setlist so we skip it next time
        let _ = new_project
            .ext_state()
            .set(COMBINED_EXT_SECTION, COMBINED_EXT_KEY, "1", false)
            .await;

        let guid = new_project.guid().to_string();
        info!("Combined setlist opened as project {}", guid);

        Ok(guid)
    }
}
