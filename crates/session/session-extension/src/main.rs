//! Session domain SHM guest process.
//!
//! Connects to REAPER via daw-bridge SHM and manages session state:
//! project tabs, arrangement, timeline, and transport coordination.
//!
//! Registers session-domain actions (including transport and marker/region
//! actions) with REAPER and handles their execution locally when triggered.
//! Transport actions dispatch through daw-bridge's TransportService and
//! ActionRegistryService — no direct REAPER API access needed.
//!
//! Placed in `UserPlugins/fts-extensions/` and hot-reloaded by daw-bridge.

use daw::service::markers_regions::fts_markers_regions_actions;
use daw::service::transport::fts_transport_actions;
use daw_extension_runtime::GuestOptions;
use eyre::Result;
use session::session_actions;
use tracing::info;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(run())
}

async fn run() -> Result<()> {
    let pid = std::process::id();
    info!("[session:{pid}] Session extension starting");

    let daw = daw_extension_runtime::connect(GuestOptions {
        role: "session",
        ..Default::default()
    })
    .await?;

    info!("[session:{pid}] Connected to REAPER via SHM");

    // Signal that we're alive — tests read this to verify the extension connected
    daw.ext_state()
        .set("FTS_SESSION_EXT", "status", "ready", false)
        .await?;
    daw.ext_state()
        .set("FTS_SESSION_EXT", "pid", &pid.to_string(), false)
        .await?;
    info!("[session:{pid}] Health beacon written");

    // Register session-domain actions with REAPER.
    let registry = daw.action_registry();
    for def in session_actions::definitions() {
        let cmd_name = def.id.to_command_id();
        let cmd_id = registry.register(&cmd_name, &def.description).await?;
        if cmd_id == 0 {
            tracing::warn!("[session:{pid}] Failed to register action: {cmd_name}");
        } else {
            info!("[session:{pid}] Registered {cmd_name} (cmd_id={cmd_id})");
        }
    }

    // Register transport actions (play, stop, record, etc.)
    for def in fts_transport_actions::definitions() {
        let cmd_name = def.id.to_command_id();
        let cmd_id = registry.register(&cmd_name, &def.description).await?;
        if cmd_id == 0 {
            tracing::warn!("[session:{pid}] Failed to register transport action: {cmd_name}");
        } else {
            info!("[session:{pid}] Registered transport {cmd_name} (cmd_id={cmd_id})");
        }
    }

    // Register marker/region actions
    for def in fts_markers_regions_actions::definitions() {
        let cmd_name = def.id.to_command_id();
        let cmd_id = registry.register(&cmd_name, &def.description).await?;
        if cmd_id == 0 {
            tracing::warn!("[session:{pid}] Failed to register marker action: {cmd_name}");
        } else {
            info!("[session:{pid}] Registered marker {cmd_name} (cmd_id={cmd_id})");
        }
    }

    info!("[session:{pid}] All session/transport/marker actions registered");

    // Subscribe to action trigger events and handle them locally.
    let mut rx = registry.subscribe_actions().await?;
    info!("[session:{pid}] Subscribed to action events");

    // TODO: Move setlist_nav.rs from reaper-extension into this process
    // TODO: Move ruler_manager.rs from reaper-extension into this process
    // TODO: Initialize SetlistServiceImpl + SongServiceImpl

    // Event loop — handle action triggers from REAPER
    while let Ok(Some(event)) = rx.recv().await {
        match &*event {
            daw::service::ActionEvent::Triggered { command_name } => {
                handle_action(&daw, command_name).await;
            }
        }
    }

    info!("[session:{pid}] Action event stream ended");
    Ok(())
}

/// Dispatch action triggers to the appropriate handler.
///
/// Transport and marker actions are dispatched through daw-bridge services.
/// Session-specific actions (setlist nav, ruler) will be handled locally.
async fn handle_action(daw: &daw::Daw, command_name: &str) {
    let registry = daw.action_registry();

    // Transport actions — use native REAPER command IDs via execute_command
    match command_name {
        n if n == fts_transport_actions::PLAY.to_command_id() => {
            // REAPER: Transport: Play (1007)
            let _ = registry.execute_command(1007).await;
        }
        n if n == fts_transport_actions::PLAY_STOP.to_command_id() => {
            // REAPER: Transport: Play/stop (40044)
            let _ = registry.execute_command(40044).await;
        }
        n if n == fts_transport_actions::PLAY_PAUSE.to_command_id() => {
            // REAPER: Transport: Play/pause (40073)
            let _ = registry.execute_command(40073).await;
        }
        n if n == fts_transport_actions::PLAY_SKIP_TIME_SELECTION.to_command_id() => {
            // REAPER: Transport: Play (skip time selection) (40317)
            let _ = registry.execute_command(40317).await;
        }
        n if n == fts_transport_actions::PLAY_FROM_LAST_START_POSITION.to_command_id() => {
            // TODO: Track last play start position and restore before play.
            // For now, just start playback (1007).
            let _ = registry.execute_command(1007).await;
        }
        n if n == fts_transport_actions::TOGGLE_RECORDING.to_command_id() => {
            // REAPER: Transport: Toggle record (1013)
            let _ = registry.execute_command(1013).await;
        }

        // Marker/region actions
        n if n == fts_markers_regions_actions::INSERT_REGION_AND_EDIT.to_command_id() => {
            // REAPER: Insert region from time selection and edit (40306)
            let _ = registry.execute_command(40306).await;
        }
        n if n == fts_markers_regions_actions::INSERT_MARKER_AND_EDIT.to_command_id() => {
            // REAPER: Insert marker (40157) then Edit marker near cursor (40171)
            let _ = registry.execute_command(40157).await;
            let _ = registry.execute_command(40171).await;
        }

        // Session-specific actions
        _ => {
            // TODO: Dispatch to setlist navigation, ruler manager, etc.
            info!("[session] Action triggered: {command_name}");
        }
    }
}
