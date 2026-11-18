//! A playhead read across processes in the presence host's clock:
//! Architect's (`DocPresence::now`, each peer's `PresencePeer::clock`,
//! `Here::play_now`), decoded by the session (`doc::transport_of`) and
//! drawn by `sync::Tracker`, though the host's clock is seconds from both
//! peers'.

use std::sync::Arc;
use std::time::Duration;

use architect::{LayerRouter, LocalServer, Scope};
use crdt::presence::{Here, PeerState, clock_ms};
use crdt::sync::{
    DocPresenceClient, DocPresenceDispatcher, PresenceHost, PresencePeer,
    doc_presence_service_descriptor,
};
use session::doc::transport_of;
use session::sync::Tracker;
use uuid::Uuid;

/// The host's clock runs this far ahead of both peers'.
const AHEAD_MS: f64 = 7_000.0;

async fn join(local: &LocalServer, doc: Uuid) -> PresencePeer {
    let client: DocPresenceClient = local.establish().await.unwrap();
    let (peer, mut driver) = PresencePeer::new(doc, 30_000);
    tokio::spawn(async move {
        let _ = driver.run(&client).await;
    });
    peer
}

#[allow(clippy::arithmetic_side_effects)] // A deadline: an instant plus seconds.
async fn until(what: &str, mut pred: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !pred() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_follower_draws_the_leaders_playhead_in_the_hosts_clock() {
    let doc = Uuid::new_v4();
    let host = PresenceHost::new(doc, 30_000).with_clock(Arc::new(|| clock_ms() + AHEAD_MS));
    let router = LayerRouter::new().with(
        doc_presence_service_descriptor(),
        DocPresenceDispatcher::new(host),
    );
    let local = LocalServer::serve(router, Scope::new());
    let lead = join(&local, doc).await;
    let follow = join(&local, doc).await;
    for peer in [&lead, &follow] {
        let clock = peer.clock().clone();
        until("the host's clock is estimated", || {
            clock.offset_ms().is_some()
        })
        .await;
    }

    let here = Here::new(lead.clone(), "lead", PeerState::new("lead", "Lead"));
    assert!(here.play_now(true, 20.0, 1.0));
    let started = clock_ms();
    let mut leader = None;
    until("the follower hears lead/play", || {
        leader = follow.states().get("lead/play").and_then(transport_of);
        leader.is_some()
    })
    .await;

    let mut tracker = Tracker::default();
    tracker.observe(leader.unwrap());
    tokio::time::sleep(Duration::from_millis(300)).await;
    // This process's clock and its offset to the host's, both Architect's.
    let local_micros = clock_ms() * 1000.0;
    let offset_micros = follow.clock().offset_ms().unwrap() * 1000.0;
    let shown = tracker.position(local_micros, offset_micros).unwrap();
    let truth = 20.0 + (clock_ms() - started) / 1000.0;
    assert!((shown - truth).abs() < 0.01, "shown {shown}, truth {truth}");
}
