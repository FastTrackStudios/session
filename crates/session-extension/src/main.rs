//! Session domain SHM guest process.
//!
//! Connects to REAPER via daw-bridge SHM and manages session state:
//! project tabs, arrangement, timeline, and transport coordination.
//!
//! Placed in `UserPlugins/fts-extensions/` and hot-reloaded by daw-bridge.

use daw_extension_runtime::GuestOptions;
use eyre::Result;
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

    // TODO: Register session-domain actions
    // TODO: Watch for project structure changes
    // TODO: Manage arrangement and timeline state

    // Keep the process alive — daw-bridge will kill us on reload
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}
