//! REAPER integration tests for transport subscription streaming.
//!
//! Verifies that subscribing to transport state changes delivers real-time
//! updates when playback starts, stops, or position changes.
//!
//! Run with:
//!   cargo xtask reaper-test -- subscription

use reaper_test::reaper_test;
use std::time::Duration;

/// Verify stop event is received via subscription.
#[reaper_test]
async fn subscription_a_stop_event(ctx: &reaper_test::ReaperTestContext) -> eyre::Result<()> {
    let transport = ctx.project.transport();
    transport.stop().await?;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let mut rx = transport.subscribe_state().await?;

    // Play then stop
    transport.play().await?;
    tokio::time::sleep(Duration::from_millis(300)).await;
    transport.stop().await?;

    // Collect updates for a bit
    let mut saw_stopped = false;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    loop {
        let timeout = deadline.saturating_duration_since(tokio::time::Instant::now());
        if timeout.is_zero() {
            break;
        }
        match tokio::time::timeout(timeout, rx.recv()).await {
            Ok(Ok(Some(state))) => {
                if state.is_stopped() {
                    saw_stopped = true;
                    break;
                }
            }
            _ => break,
        }
    }

    println!("Saw stopped state: {saw_stopped}");
    assert!(saw_stopped, "Should receive a stopped state after stop()");

    Ok(())
}

/// Subscribe to transport state and verify updates arrive during playback.
#[reaper_test]
async fn subscription_b_receives_updates(ctx: &reaper_test::ReaperTestContext) -> eyre::Result<()> {
    let transport = ctx.project.transport();
    transport.stop().await?;
    transport.set_position(0.0).await?;
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Subscribe to state changes
    let mut rx = transport.subscribe_state().await?;

    // Start playback
    transport.play().await?;

    // Collect updates for 1 second
    let mut updates = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1000);
    loop {
        let timeout = deadline.saturating_duration_since(tokio::time::Instant::now());
        if timeout.is_zero() {
            break;
        }
        match tokio::time::timeout(timeout, rx.recv()).await {
            Ok(Ok(Some(state))) => updates.push(state),
            _ => break,
        }
    }

    transport.stop().await?;

    println!("Received {} transport updates in 1s", updates.len());

    // Should receive updates during playback
    assert!(
        updates.len() >= 5,
        "Should receive at least 5 updates in 1s, got {}",
        updates.len()
    );

    // At least some updates should show playing state
    let playing_count = updates.iter().filter(|s| s.is_playing()).count();
    println!("  {playing_count} updates showed playing state");
    assert!(
        playing_count > 0,
        "At least some updates should show playing state"
    );

    Ok(())
}
