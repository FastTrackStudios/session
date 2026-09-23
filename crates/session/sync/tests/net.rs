//! A host and a joiner over a real vox connection (in-process link).

#![cfg(feature = "net")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::float_cmp,
    clippy::panic
)]

use std::time::Duration;

use session_sync::net::{CollabHost, CollabPeer, PresenceSink, session_id};
use session_sync::presence::{self, PeerState};
use session_sync::{ItemState, ORIGIN_LOCAL, SessionDoc, SessionModel, TrackState};

fn song() -> SessionModel {
    let mut m = SessionModel {
        tracks: vec![TrackState {
            guid: "bass".into(),
            name: "Bass".into(),
            volume: 1.0,
            ..TrackState::default()
        }],
        chart: "Song\n72bpm 4/4 #E\n".into(),
        ..SessionModel::default()
    };
    m.items.insert(
        "i1".into(),
        ItemState {
            track: "bass".into(),
            position: 1.0,
            length: 2.0,
            ..ItemState::default()
        },
    );
    m
}

async fn until(mut ok: impl FnMut() -> bool) {
    for _ in 0..200 {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_joiner_gets_the_session_and_edits_flow_both_ways() {
    let host_doc = SessionDoc::new();
    host_doc.write(&song(), ORIGIN_LOCAL).unwrap();
    let id = session_id("{PROJECT-GUID}");
    let host = CollabHost::new(id, &host_doc);

    let router = host.mount(architect::LayerRouter::new());
    let server = architect::LocalServer::serve(router, architect::Scope::new());
    let caller = server.caller().await.unwrap();

    let (peer_doc, peer_presence, mut synced, mut driver) = CollabPeer::new(id).into_parts();
    tokio::spawn(async move {
        let sync = crdt::sync::DocSyncClient::new(caller.clone());
        let presence = crdt::sync::DocPresenceClient::new(caller);
        let _ = tokio::join!(synced.run(&sync), driver.run(&presence));
    });

    // The joiner receives the host's session.
    until(|| peer_doc.read() == song()).await;

    // The joiner edits; the host sees it.
    let mut m = peer_doc.read();
    m.items.get_mut("i1").unwrap().position = 4.0;
    peer_doc.write(&m, ORIGIN_LOCAL).unwrap();
    until(|| host_doc.read().items["i1"].position == 4.0).await;

    // The host edits; the joiner sees it.
    let mut m = host_doc.read();
    m.tracks[0].name = "Bass DI".into();
    host_doc.write(&m, ORIGIN_LOCAL).unwrap();
    until(|| peer_doc.read().tracks[0].name == "Bass DI").await;

    // Presence: the joiner says who it is; the host hears.
    let me = PeerState {
        name: "Joiner".into(),
        view: "arrangement".into(),
        ..PeerState::default()
    };
    PresenceSink::set(
        &peer_presence,
        &presence::key("joiner", presence::STATE),
        me.encode(),
    );
    until(|| {
        host.states()
            .get(&presence::key("joiner", presence::STATE))
            .and_then(PeerState::decode)
            .is_some_and(|s| s.name == "Joiner")
    })
    .await;
    assert_eq!(host.peers(), 1);

    // The host is a participant too: its own presence reaches the joiner.
    let host_me = PeerState {
        name: "Host".into(),
        edit_cursor: Some(3.0),
        ..PeerState::default()
    };
    PresenceSink::set(
        &host,
        &presence::key("host", presence::STATE),
        host_me.encode(),
    );
    until(|| {
        peer_presence
            .states()
            .get(&presence::key("host", presence::STATE))
            .and_then(PeerState::decode)
            .is_some_and(|s| s.name == "Host" && s.edit_cursor == Some(3.0))
    })
    .await;
}
