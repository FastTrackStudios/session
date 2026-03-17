//! Standalone FTS Routing Project — receives stems from song projects via loopback.
//!
//! The routing project is a singleton REAPER project with a track for each
//! `RoutingChannel`. Each track is configured with:
//! - Stereo input from the corresponding hardware loopback pair
//! - Record-armed (for monitoring)
//! - Parent send disabled (each stem routes independently)
//!
//! The project can be opened/created via the `ensure_routing_project` function,
//! which checks for an existing routing project before creating one.

use daw::Daw;
use daw::service::RecordInput;
use session_proto::SessionServiceError;
use session_proto::routing_project::*;
use tracing::info;

// r[impl routing.project.ensure]
/// Ensure the FTS routing project exists and is open. Returns the project GUID.
///
/// Resolution order:
/// 1. Scan open projects for ExtState `FTS/is_routing_project == "1"`
/// 2. Check disk at `<fts_home>/Reaper/FTS-Routing.RPP` and open if found
/// 3. Create from scratch with the canonical track hierarchy
pub async fn ensure_routing_project(
    config: &LoopbackConfig,
) -> Result<String, SessionServiceError> {
    let daw = Daw::get();

    // Step 1: Check open projects for existing routing project
    if let Some(guid) = find_open_routing_project(daw).await? {
        info!("Found open routing project: {guid}");
        return Ok(guid);
    }

    // Step 2: Check disk
    let routing_path = utils::paths::reaper_dir().join(ROUTING_PROJECT_FILENAME);
    if routing_path.exists() {
        info!(
            "Opening routing project from disk: {}",
            routing_path.display()
        );
        let project = daw
            .open_project(routing_path.to_string_lossy().as_ref())
            .await
            .map_err(|e| {
                SessionServiceError::DawError(format!("Failed to open routing project: {e}"))
            })?;
        return Ok(project.guid().to_string());
    }

    // Step 3: Create from scratch
    info!("Creating new routing project");
    create_routing_project(daw, config).await
}

/// Scan open projects for one with the routing project ExtState marker.
async fn find_open_routing_project(daw: &Daw) -> Result<Option<String>, SessionServiceError> {
    let projects = daw
        .projects()
        .await
        .map_err(|e| SessionServiceError::DawError(format!("Failed to list projects: {e}")))?;

    for project in projects {
        let value = project
            .ext_state()
            .get(EXT_STATE_SECTION, EXT_STATE_KEY_IS_ROUTING)
            .await
            .unwrap_or(None);
        if value.as_deref() == Some(EXT_STATE_VALUE_TRUE) {
            return Ok(Some(project.guid().to_string()));
        }
    }

    Ok(None)
}

// r[impl routing.project.structure]
/// Create the routing project from scratch with the canonical FTS track hierarchy.
async fn create_routing_project(
    daw: &Daw,
    config: &LoopbackConfig,
) -> Result<String, SessionServiceError> {
    let map_err = |e: daw::Error| SessionServiceError::DawError(e.to_string());

    // Create a new empty project tab
    let project = daw.create_project().await.map_err(map_err)?;

    // Mark as routing project via ExtState
    let _ = project
        .ext_state()
        .set(
            EXT_STATE_SECTION,
            EXT_STATE_KEY_IS_ROUTING,
            EXT_STATE_VALUE_TRUE,
            true, // persist
        )
        .await;

    // Create "Click + Guide" folder with children
    create_folder_with_channels(
        &project,
        RoutingGroup::ClickGuide.display_name(),
        RoutingChannel::click_guide_channels(),
        config,
    )
    .await?;

    // Create "TRACKS" folder with children
    create_folder_with_channels(
        &project,
        RoutingGroup::Tracks.display_name(),
        RoutingChannel::track_channels(),
        config,
    )
    .await?;

    // Save the project (will prompt Save As for first save)
    project.save().await.map_err(map_err)?;

    let guid = project.guid().to_string();
    info!("Routing project created: {guid}");
    Ok(guid)
}

/// Create a folder track with child routing channel tracks.
async fn create_folder_with_channels(
    project: &daw::Project,
    folder_name: &str,
    channels: &[RoutingChannel],
    config: &LoopbackConfig,
) -> Result<(), SessionServiceError> {
    let map_err = |e: daw::Error| SessionServiceError::DawError(e.to_string());

    // Create folder track
    let folder = project
        .tracks()
        .add(folder_name, None)
        .await
        .map_err(map_err)?;
    folder.set_folder_depth(1).await.map_err(map_err)?;

    // Create child tracks
    let last_idx = channels.len() - 1;
    for (i, channel) in channels.iter().enumerate() {
        let track = project
            .tracks()
            .add(channel.display_name(), None)
            .await
            .map_err(map_err)?;

        // Configure for stereo loopback input
        track.set_num_channels(2).await.map_err(map_err)?;
        track
            .set_record_input(RecordInput::Raw(config.recinput_value(*channel)))
            .await
            .map_err(map_err)?;
        track.arm().await.map_err(map_err)?;
        track.set_parent_send(false).await.map_err(map_err)?;

        // Close the folder on the last child
        if i == last_idx {
            track.set_folder_depth(-1).await.map_err(map_err)?;
        }
    }

    Ok(())
}
