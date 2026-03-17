//! Combined setlist generation — merges all open song projects into a single
//! REAPER project with songs laid out sequentially on the timeline.
//!
//! Uses the `combine_rpp_files` pipeline from the daw crate to handle:
//! - PREROLL/POSTROLL bounds resolution (trimming)
//! - Guide track merging (Click, Loop, Count, Guide → shared header)
//! - Per-song folder creation under TRACKS/
//! - Tempo envelope concatenation
//! - Marker/region offset + lane classification
//!
//! After the daw crate produces the combined RPP text, post-processing steps
//! run via the Daw facade on the opened project:
//! - Mark as combined setlist (ExtState)
//! - Wire routing receives (future: when routing folder is added at RPP level)

use crate::setlist_service::SetlistServiceImpl;
use daw::Daw;
use session_proto::SessionServiceError;
use std::path::PathBuf;
use tracing::{debug, info, warn};

/// ExtState section/key used to identify combined setlist projects.
const COMBINED_EXT_SECTION: &str = "FTS";
const COMBINED_EXT_KEY: &str = "is_combined_setlist";

/// ExtState keys for sync group identity.
/// Written to every project tab involved in a setlist so they can find each other.
const SYNC_SECTION: &str = "FTS_SYNC";
const SYNC_KEY_SETLIST_ID: &str = "setlist_id";
const SYNC_KEY_SONG_INDEX: &str = "song_index";
const SYNC_KEY_SETLIST_PATH: &str = "setlist_path";
const SYNC_KEY_SONG_COUNT: &str = "song_count";

impl SetlistServiceImpl {
    /// Generate a combined setlist project from open song projects.
    ///
    /// Pipeline:
    /// 1. Save all open projects to disk
    /// 2. Enumerate projects, skip combined setlists and routing projects
    /// 3. Collect RPP file paths
    /// 4. Combine via `combine_rpp_files` (handles bounds, guide merging, folders, tempo, markers)
    /// 5. Write combined RPP to disk
    /// 6. Open in REAPER as a new tab
    /// 7. Post-process: mark as combined, wire routing receives
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

        let mut rpp_paths = Vec::new();
        for project in &projects {
            // Skip combined setlist projects
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

            // Skip routing projects
            let is_routing = project
                .ext_state()
                .get(
                    session_proto::routing_project::EXT_STATE_SECTION,
                    session_proto::routing_project::EXT_STATE_KEY_IS_ROUTING,
                )
                .await
                .unwrap_or(None)
                .map(|v| v == "1")
                .unwrap_or(false);

            if is_routing {
                debug!("Skipping routing project: {}", project.guid());
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

            rpp_paths.push(PathBuf::from(&info.path));
        }

        if rpp_paths.is_empty() {
            return Err(SessionServiceError::Internal(
                "No saved song projects found".to_string(),
            ));
        }

        info!(
            "Generating combined setlist from {} projects",
            rpp_paths.len()
        );

        // ── 3. Combine RPP files using the daw crate pipeline ─────────
        //
        // This handles everything at the RPP level:
        // - Resolves song bounds (PREROLL → POSTROLL priority chain)
        // - Merges guide tracks (Click, Loop, Count, Guide) into shared header
        // - Creates per-song folders under TRACKS/
        // - Concatenates tempo envelopes with square-shape boundaries
        // - Offsets markers/regions with lane classification
        // - Resolves relative media paths to absolute
        let options = daw::file::setlist_rpp::CombineOptions {
            gap_measures,
            trim_to_bounds: true,
        };
        let (combined_rpp, song_infos) =
            daw::file::setlist_rpp::combine_rpp_files(&rpp_paths, &options).map_err(|e| {
                SessionServiceError::Internal(format!("Failed to combine RPP files: {e}"))
            })?;

        info!(
            "Combined {} songs: {}",
            song_infos.len(),
            song_infos
                .iter()
                .map(|s| format!("{} ({:.1}s)", s.name, s.duration_seconds))
                .collect::<Vec<_>>()
                .join(", ")
        );

        // ── 4. Write combined RPP to disk ─────────────────────────────
        let output_dir = rpp_paths
            .first()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(std::env::temp_dir);
        let output_path = output_dir.join("Combined Setlist.RPP");

        std::fs::write(&output_path, &combined_rpp).map_err(|e| {
            SessionServiceError::Internal(format!(
                "Failed to write combined RPP to {}: {e}",
                output_path.display()
            ))
        })?;

        info!(
            "Combined setlist RPP written: {} ({:.0} bytes)",
            output_path.display(),
            combined_rpp.len(),
        );

        // ── 5. Open in REAPER as a new tab ────────────────────────────
        let new_project = daw
            .open_project(output_path.to_string_lossy().to_string())
            .await
            .map_err(|e| {
                SessionServiceError::DawError(format!("Failed to open combined setlist: {e}"))
            })?;

        // ── 6. Post-process: mark all projects with sync identity ──────
        //
        // Every project in the setlist gets a shared setlist_id so they
        // can find each other for sync (within the same REAPER instance
        // or across the network via mDNS).
        let setlist_id = uuid::Uuid::new_v4().to_string();
        let setlist_path_str = output_path.to_string_lossy().to_string();
        let song_count = rpp_paths.len().to_string();

        // Mark the combined setlist project
        let ext = new_project.ext_state();
        let _ = ext.set(COMBINED_EXT_SECTION, COMBINED_EXT_KEY, "1", false).await;
        let _ = ext.set(SYNC_SECTION, SYNC_KEY_SETLIST_ID, &setlist_id, false).await;
        let _ = ext.set(SYNC_SECTION, SYNC_KEY_SONG_COUNT, &song_count, false).await;
        let _ = ext.set(SYNC_SECTION, SYNC_KEY_SETLIST_PATH, &setlist_path_str, false).await;

        // Mark each individual song project with the same setlist_id + its index
        let all_projects = daw.projects().await.unwrap_or_default();
        let mut song_idx = 0u32;
        for project in &all_projects {
            if project.guid() == new_project.guid() {
                continue;
            }

            let is_combined = project
                .ext_state()
                .get(COMBINED_EXT_SECTION, COMBINED_EXT_KEY)
                .await
                .unwrap_or(None)
                .map(|v| v == "1")
                .unwrap_or(false);
            if is_combined {
                continue;
            }
            let is_routing = project
                .ext_state()
                .get(
                    session_proto::routing_project::EXT_STATE_SECTION,
                    session_proto::routing_project::EXT_STATE_KEY_IS_ROUTING,
                )
                .await
                .unwrap_or(None)
                .map(|v| v == "1")
                .unwrap_or(false);
            if is_routing {
                continue;
            }

            let info = match project.info().await {
                Ok(i) if !i.path.is_empty() => i,
                _ => continue,
            };

            let ext = project.ext_state();
            let _ = ext.set(SYNC_SECTION, SYNC_KEY_SETLIST_ID, &setlist_id, false).await;
            let _ = ext.set(SYNC_SECTION, SYNC_KEY_SONG_INDEX, &song_idx.to_string(), false).await;
            let _ = ext.set(SYNC_SECTION, SYNC_KEY_SETLIST_PATH, &setlist_path_str, false).await;

            debug!("Song {} ({}) tagged with setlist_id={}", song_idx, info.name, setlist_id);
            song_idx += 1;
        }

        let guid = new_project.guid().to_string();
        info!(
            "Combined setlist opened: {} (setlist_id={}, {} songs tagged)",
            guid, setlist_id, song_idx
        );

        Ok(guid)
    }
}
