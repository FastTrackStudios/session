//! The panel, attached to a REAPER it did not start.
//!
//! Everything else in this repo tests the window against a project it
//! owns, or against events handed to `apply_event` by hand. Both are
//! worth having and neither can answer the only question that matters
//! for a control surface: does an edit made HERE reach REAPER, and does
//! a change made THERE come back?
//!
//! So this attaches the way the window attaches — `attach_to_reaper`
//! discovering the host's socket, the global facade installed, the same
//! `Applier` and the same `Watch` — rather than reaching in through the
//! handle the harness already holds. A test that used the harness's
//! handle would prove the services work and say nothing about whether
//! the window can find a DAW it did not start.
//!
//! Run with: `just reaper-test` (see `session-reaper-xtask`).

#![cfg(feature = "reaper-tests")]

use daw::test::reaper_test;
use daw_proto::track::TrackEvent;
use session_daw::engine::{Applied, Applier, Edit, Watch, apply_event};

/// How long to give the host's 30 Hz poller.
///
/// Generous: what is being measured is whether an event is produced at
/// all, and a tight bound turns a busy machine into a failure that
/// reads like a missing feature.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(5);

/// The project the WINDOW is looking at.
///
/// Not `ctx.project()`, which is the tab the harness made for this
/// test. The window attaches and follows REAPER's CURRENT project —
/// that is what a control surface does, and its `Watch` filters the
/// stream to that project's guid — so a test driving some other tab
/// would make edits the window is right to ignore, and then fail for
/// the wrong reason.
async fn window_project() -> eyre::Result<daw::rpc::Project> {
    let daw = daw::rpc::Daw::try_get().ok_or_else(|| eyre::eyre!("the facade is not up"))?;
    Ok(daw.current_project().await?)
}

/// Attach the window's own facade to whatever REAPER is running.
///
/// Idempotent across tests in one binary: they share a process, so the
/// second call finds the facade already installed and that is fine —
/// it is the same REAPER.
/// Run on a thread of its own, because the window attaches with a
/// `block_on` and this test is already inside a runtime — nesting one
/// runtime in another panics. A window has no runtime around it when it
/// attaches, so this is the test accommodating the test harness rather
/// than the window doing anything unusual.
fn attach() -> eyre::Result<()> {
    if daw::rpc::Daw::try_get().is_some() {
        return Ok(());
    }
    std::thread::spawn(|| {
        session_daw::open::attach_to_reaper(None).map(|attached| {
            println!(
                "attached to {:?} ({} tracks)",
                attached.name, attached.track_count
            );
        })
    })
    .join()
    .map_err(|_| eyre::eyre!("the attach thread panicked"))?
    .map_err(|e| eyre::eyre!("the window could not attach: {e}"))
}

/// Drain the window's watch until it sees what it is waiting for.
///
/// Polled rather than awaited because `Watch` hands the window a
/// non-blocking drain, which is what a frame wants: the window asks
/// once per frame and applies whatever arrived.
fn watch_for(
    watch: &Watch,
    tracks: &mut Vec<daw_proto::Track>,
    mut wanted: impl FnMut(&TrackEvent) -> bool,
) -> eyre::Result<TrackEvent> {
    let deadline = std::time::Instant::now() + PATIENCE;
    loop {
        for event in watch.drain() {
            // Applied through the window's own applier, so what this
            // asserts is the list the window would DRAW, not the event
            // that happened to arrive.
            let _ = apply_event(tracks, &event);
            if wanted(&event) {
                return Ok(event);
            }
        }
        if std::time::Instant::now() > deadline {
            eyre::bail!("the window saw no matching event within {PATIENCE:?}");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Wait until the host's poller has seeded its cache for this project.
///
/// The cache is seeded the first tick that finds a subscriber, and
/// everything already true then arrives as `Added` rather than as a
/// field change. A window never notices, because it subscribes at
/// startup and the edits come later; a test makes its edit immediately,
/// so it has to wait for the same thing to be true or the edit is
/// folded into the snapshot that was meant to precede it.
fn settle(watch: &Watch, tracks: &mut Vec<daw_proto::Track>, guid: &str) -> eyre::Result<()> {
    watch_for(
        &watch,
        tracks,
        |event| matches!(event, TrackEvent::Added(track) if track.guid == guid),
    )?;
    Ok(())
}

/// The window, driving a REAPER it did not start.
///
/// **One test, not three**, and that is a finding rather than a style
/// choice: a second subscription to the same service on the same
/// connection is silently refused. The subscriber count on the host
/// stays at one however many `Watch`es are started, and every watch
/// after the first sits there receiving nothing. A window makes exactly
/// one and never notices; a test file with three tests makes three, and
/// two of them are testing a dead stream.
///
/// So this is one window, watching once, doing three things in the
/// order a session does them.
#[reaper_test(isolated)]
async fn the_window_drives_reaper_and_hears_it_back(
    _ctx: &daw::test::ReaperTestContext,
) -> eyre::Result<()> {
    attach()?;
    let project = window_project().await?;

    // Watching FIRST, which is the order a window has: it attaches, it
    // subscribes, and then things happen. Creating a track before
    // subscribing races the host's 40 ms tick — the `Added` can be
    // published before the subscription exists, and it is never
    // published again.
    let mut tracks: Vec<daw_proto::Track> = project.tracks().all().await?;
    let watch = Watch::start().ok_or_else(|| eyre::eyre!("the window could not watch"))?;

    // ── A track appearing in REAPER changes the window's LIST ────────
    //
    // The distinction is load-bearing: a field event repaints one row,
    // where the list changing invalidates every row map and the scene
    // resolved against it. A window told the wrong one draws the right
    // value in the wrong place.
    let before = tracks.len();
    let kick = project.tracks().add("Panel Kick", None).await?;
    let guid = kick.guid().to_owned();

    let deadline = std::time::Instant::now() + PATIENCE;
    let mut structural = false;
    while std::time::Instant::now() < deadline && !structural {
        for event in watch.drain() {
            if apply_event(&mut tracks, &event) == Applied::Structure
                && matches!(&event, TrackEvent::Added(track) if track.guid == guid)
            {
                structural = true;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(structural, "the window was never told the list changed");
    assert_eq!(tracks.len(), before + 1, "the window's list did not grow");

    // ── An edit made in the window reaches REAPER ────────────────────
    //
    // Through the window's own applier — the queue, the worker thread,
    // the coalescing — exactly as a click does.
    let applier = Applier::start().ok_or_else(|| eyre::eyre!("the window could not edit"))?;
    applier.send(Edit::ToggleMute(guid.clone()));
    let seen = watch_for(&watch, &mut tracks, |event| {
        matches!(event, TrackEvent::MuteChanged { muted: true, .. })
    })?;
    let TrackEvent::MuteChanged { guid: muted, .. } = &seen else {
        eyre::bail!("not a mute event");
    };
    assert_eq!(muted, &guid, "REAPER muted a different track");

    // The event is one claim and the DAW's own answer is another. A
    // window that believed only the echo of its own edit would look
    // perfect right up until something rejected one.
    assert!(
        kick.info().await?.muted,
        "the event said muted and the DAW says otherwise"
    );
    assert!(
        tracks
            .iter()
            .find(|t| t.guid == guid)
            .is_some_and(|t| t.muted),
        "the window applied the event to the wrong track, or not at all"
    );

    // ── A change made in REAPER, by nobody the window knows ──────────
    //
    // This is the half a window cannot fake. Its own edits it predicts;
    // this one it can only be told about, and being told is the whole
    // difference between a control surface and a remote control.
    kick.set_volume(0.25).await?;
    watch_for(
        &watch,
        &mut tracks,
        |event| matches!(event, TrackEvent::VolumeChanged { volume, .. } if (volume - 0.25).abs() < 1e-6),
    )?;
    let shown = tracks
        .iter()
        .find(|t| t.guid == guid)
        .map(|t| t.volume)
        .ok_or_else(|| eyre::eyre!("the window lost the track"))?;
    assert!(
        (shown - 0.25).abs() < 1e-6,
        "the window is drawing {shown}, REAPER is at 0.25"
    );

    Ok(())
}
