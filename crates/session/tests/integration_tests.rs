//! Integration tests for Session cell
//!
//! These tests demonstrate how the session cell uses daw-control

use daw_control::Daw;
use std::time::Duration;

/// Test that demonstrates the session using daw-control to call transport commands
///
/// This test shows the pattern:
/// 1. Initialize daw-control with a connection handle
/// 2. Get the current project
/// 3. Call transport commands (play, stop)
#[tokio::test]
async fn test_session_transport_controls() -> eyre::Result<()> {
    // In a real test, we would:
    // 1. Start the DAW standalone cell
    // 2. Get a ConnectionHandle to it via SHM
    // 3. Initialize daw-control
    // 4. Call transport commands

    // For now, this is a placeholder showing the intended API usage:
    //
    // let handle = /* get connection handle from SHM */;
    // Daw::init(handle)?;
    //
    // let project = Daw::current_project().await?;
    // project.transport().play().await?;
    // tokio::time::sleep(Duration::from_secs(1)).await;
    // project.transport().stop().await?;

    Ok(())
}

/// Test that demonstrates session getting project info
#[tokio::test]
async fn test_session_project_access() -> eyre::Result<()> {
    // Intended API:
    // let handle = /* get connection handle */;
    // Daw::init(handle)?;
    //
    // let project = Daw::current_project().await?;
    // println!("Project GUID: {}", project.guid());
    //
    // let transport = project.transport();
    // transport.play().await?;

    Ok(())
}
