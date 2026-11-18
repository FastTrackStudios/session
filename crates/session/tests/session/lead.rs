//! The session host's transport, published and followed from another
//! process's point of view (over its WebSocket).

use std::time::Duration;

use architect::connect::{Endpoint, pool};
use crdt::sync::{DocPresenceClient, PresencePeer};
use session::clock::{SessionClockClient, SharedClock};
use session::doc::transport_of;
use session::lead::Leader;
use session::sync::Tracker;
use session::sync::clock::now_micros_f64;
use uuid::Uuid;

#[allow(clippy::arithmetic_side_effects)] // A deadline: an instant plus seconds.
async fn until(what: &str, mut pred: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !pred() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_follower_draws_where_the_leader_plays() {
    let doc = Uuid::new_v4();
    let port = 47_000 + (std::process::id() % 1000);
    let leader = Leader::serve(&format!("127.0.0.1:{port}"), doc)
        .await
        .unwrap();

    // A follower elsewhere: the host's clock and its presence, dialled.
    let url = leader.url().to_owned();
    let host: SessionClockClient = pool().client(&Endpoint::url(&url)).await.unwrap();
    let clock = SharedClock::default();
    let pinger = clock.clone();
    tokio::spawn(async move {
        let _ = pinger.run(&host).await;
    });
    let presence: DocPresenceClient = pool().client(&Endpoint::url(&url)).await.unwrap();
    let (peer, mut driver) = PresencePeer::new(doc, 30_000);
    tokio::spawn(async move {
        let _ = driver.run(&presence).await;
    });

    leader.publish(true, 48.0, 1.0);
    let mut heard = None;
    until("the follower hears the leader", || {
        heard = peer.states().get("lead/play").and_then(transport_of);
        heard.is_some() && clock.offset_micros().is_some()
    })
    .await;
    let mut tracker = Tracker::default();
    tracker.observe(heard.unwrap());
    tokio::time::sleep(Duration::from_millis(250)).await;
    let shown = tracker
        .position(now_micros_f64(), clock.offset_micros().unwrap())
        .unwrap();
    assert!((48.2..48.6).contains(&shown), "shown {shown}");

    // A stop holds where it stopped.
    leader.publish(false, 60.0, 1.0);
    until("the follower hears the stop", || {
        peer.states()
            .get("lead/play")
            .and_then(transport_of)
            .is_some_and(|t| !t.is_playing)
    })
    .await;
}
