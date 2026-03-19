//! Session domain SHM guest process.
//!
//! Connects to REAPER via daw-bridge SHM and manages session state:
//! project tabs, arrangement, timeline, and transport coordination.
//!
//! Registers session-domain actions with REAPER and handles their execution
//! locally when triggered. The host (daw-bridge) is domain-agnostic.
//!
//! Placed in `UserPlugins/fts-extensions/` and hot-reloaded by daw-bridge.

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
    // Action definitions live in session — single source of truth.
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
    info!("[session:{pid}] All session actions registered");

    // Subscribe to action trigger events and handle them locally.
    let mut rx = registry.subscribe_actions().await?;
    info!("[session:{pid}] Subscribed to action events");

    // TODO: Move setlist_nav.rs from reaper-extension into this process:
    //   - init_setlist(): scan [S] song folders, [L] variation tracks
    //   - next/prev song/section/variation navigation
    //   - Mute/unmute track groups for navigation
    // TODO: Move ruler_manager.rs from reaper-extension into this process:
    //   - scaffold_core_lanes(): create 8 core ruler lanes
    //   - ensure_instrument_lane(): create instrument note lanes
    //   - Marker/region insertion for each lane type
    // TODO: Initialize SetlistServiceImpl + SongServiceImpl (from session.rs)

    // Event loop — handle action triggers from REAPER
    while let Ok(Some(event)) = rx.recv().await {
        match &*event {
            daw::service::ActionEvent::Triggered { command_name } => {
                handle_action(command_name);
            }
        }
    }

    info!("[session:{pid}] Action event stream ended");
    Ok(())
}

fn handle_action(command_name: &str) {
    // TODO: Dispatch to setlist navigation (next_song, prev_song, etc.)
    // TODO: Dispatch to ruler manager (scaffold lanes, insert markers)
    // TODO: Dispatch to SetlistService methods
    info!("[session] Action triggered: {command_name}");
}
