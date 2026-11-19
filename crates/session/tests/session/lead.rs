//! The session host's transport, published and followed from another
//! process's point of view (over its WebSocket).

use std::time::Duration;

use architect::connect::{Endpoint, pool};
use crdt::sync::{DocPresenceClient, PresencePeer};
use session::doc::transport_of;
use session::lead::Leader;
use session::sync::Tracker;
use uuid::Uuid;

/// This process's clock, microseconds: the one the presence clock's
/// offset is to (`crdt::presence::clock_ms`).
fn now_micros_f64() -> f64 {
    crdt::presence::clock_ms() * 1000.0
}

#[allow(clippy::arithmetic_side_effects)] // A deadline: an instant plus seconds.
async fn until(what: &str, mut pred: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !pred() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A TCP port nothing holds now, asked of the OS: a fixed one in the
/// ephemeral range is another program's outgoing connection on a busy
/// machine, and the host then fails to bind.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_follower_draws_where_the_leader_plays() {
    let doc = Uuid::new_v4();
    let port = free_port();
    let leader = Leader::serve(&format!("127.0.0.1:{port}"), doc)
        .await
        .unwrap();

    // A follower elsewhere: its presence, dialled (its driver pings the
    // host's clock into `PresencePeer::clock`).
    let url = leader.url().expect("it serves its own presence").to_owned();
    let presence: DocPresenceClient = pool().client(&Endpoint::url(&url)).await.unwrap();
    let (peer, mut driver) = PresencePeer::new(doc, 30_000);
    let clock = peer.clock().clone();
    tokio::spawn(async move {
        let _ = driver.run(&presence).await;
    });

    assert!(leader.publish(true, 48.0, 1.0));
    let mut heard = None;
    until("the follower hears the leader", || {
        heard = peer.states().get("lead/play").and_then(transport_of);
        heard.is_some() && clock.offset_ms().is_some()
    })
    .await;
    let mut tracker = Tracker::default();
    tracker.observe(heard.unwrap());
    tokio::time::sleep(Duration::from_millis(250)).await;
    let shown = tracker
        .position(now_micros_f64(), clock.offset_ms().unwrap() * 1000.0)
        .unwrap();
    assert!((48.2..48.6).contains(&shown), "shown {shown}");

    // A stop holds where it stopped.
    assert!(leader.publish(false, 60.0, 1.0));
    until("the follower hears the stop", || {
        peer.states()
            .get("lead/play")
            .and_then(transport_of)
            .is_some_and(|t| !t.is_playing)
    })
    .await;
}

/// A leader on a presence its node hosts and drives ([`Leader::on`],
/// session#192): it serves nothing of its own, and a follower of that
/// presence hears it, stamped in the host's clock.
#[tokio::test(flavor = "multi_thread")]
async fn a_leader_on_its_nodes_presence_is_followed() {
    use architect::LayerRouter;
    use architect::host::EngineHost;
    use crdt::sync::{DocPresenceDispatcher, PresenceHost, doc_presence_service_descriptor};

    let doc = Uuid::new_v4();
    let port = free_port();
    let router = LayerRouter::new().with(
        doc_presence_service_descriptor(),
        DocPresenceDispatcher::new(PresenceHost::new(doc, 30_000)),
    );
    let bind = format!("127.0.0.1:{port}");
    tokio::spawn(async move {
        let _ = EngineHost::ungated(router, bind).serve().await;
    });
    let url = format!("ws://127.0.0.1:{port}/vox");
    let join = |url: String| async move {
        let mut tries = 0_u32;
        let client: DocPresenceClient = loop {
            match pool().client(&Endpoint::url(&url)).await {
                Ok(client) => break client,
                Err(e) => {
                    assert!(tries < 200, "never served: {e}");
                    tries = tries.saturating_add(1);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        };
        let (peer, mut driver) = PresencePeer::new(doc, 30_000);
        tokio::spawn(async move {
            let _ = driver.run(&client).await;
        });
        peer
    };

    let leader = Leader::on(join(url.clone()).await);
    assert_eq!(leader.url(), None, "it serves nothing of its own");
    let follower = join(url).await;
    let clock = follower.clock().clone();
    until("the leader's clock is estimated", || {
        leader.publish(true, 12.0, 1.0)
    })
    .await;
    let mut heard = None;
    until("the follower hears the leader", || {
        heard = follower.states().get("lead/play").and_then(transport_of);
        heard.is_some() && clock.offset_ms().is_some()
    })
    .await;
    let mut tracker = Tracker::default();
    tracker.observe(heard.unwrap());
    let shown = tracker
        .position(now_micros_f64(), clock.offset_ms().unwrap() * 1000.0)
        .unwrap();
    assert!((12.0..12.5).contains(&shown), "shown {shown}");
}
