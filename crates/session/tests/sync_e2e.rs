//! Two peers sharing one session through Architect's real sync path.
//!
//! What is real: each peer holds the session as an Architect document (a
//! `crdt::CrdtDoc` of kind `fts.session`) kept in step by `crdt::SyncedDoc`
//! against a `DocSyncHost` (`crdt::DocRegistry`), and publishes its
//! playhead with Architect's presence (`Here::play` → `PresenceHost` →
//! the other's `PresencePeer`), over Architect's in-process vox transport
//! (`architect::LocalServer`): the same services, codecs and drivers a
//! networked peer runs, without the network. Extensions write through
//! `session::service::handle`, as a host would answer them; undo is
//! `crdt::History`.
//!
//! What is simulated: the network (in-process), and the clock. Both peers
//! are in one process, so they share `session::sync::clock` and the
//! offset is zero. Following across two clocks over a jittery network is
//! `tests/session/follow.rs`, by simulation, because Architect serves no
//! shared clock yet (`docs/design/session.md`, §2.5).

#![cfg(not(target_arch = "wasm32"))]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::float_cmp,
    clippy::missing_panics_doc,
    // One story: data, then undo, then the transport.
    clippy::too_many_lines,
    clippy::arithmetic_side_effects
)]

use std::sync::Arc;
use std::time::Duration;

use architect::{LayerRouter, LocalServer, Scope};
use crdt::presence::{Here, PeerState, PlayState};
use crdt::sync::{
    DocPresenceClient, DocPresenceDispatcher, DocSyncClient, DocSyncDispatcher, PresencePeer,
    SyncedDoc, doc_presence_service_descriptor, doc_sync_service_descriptor,
};
use crdt::{CrdtDoc, DocRegistry, History};
use session::doc::{SessionDoc, transport_of};
use session::service::{Grants, Request, handle};
use session::sync::Tracker;
use session::sync::clock::now_micros_f64;
use session::{ExtensionId, Item, ItemId, Lane, LaneId, Time, Transport, Value};
use uuid::Uuid;

/// One machine: its replica of the session, and its presence.
struct Peer {
    session: SessionDoc,
    presence: PresencePeer,
    here: Here<PresencePeer>,
}

impl Peer {
    async fn join(local: &LocalServer, doc_id: Uuid, me: &str) -> Self {
        let client: DocSyncClient = local.establish().await.unwrap();
        let mut synced = SyncedDoc::new(doc_id, CrdtDoc::ephemeral());
        let session = SessionDoc::from_doc(synced.doc().loro().clone());
        tokio::spawn(async move {
            let _ = synced.run(&client).await;
        });
        let client: DocPresenceClient = local.establish().await.unwrap();
        let (presence, mut driver) = PresencePeer::new(doc_id, 30_000);
        tokio::spawn(async move {
            let _ = driver.run(&client).await;
        });
        let here = Here::new(presence.clone(), me, PeerState::new(me, me));
        Self {
            session,
            presence,
            here,
        }
    }

    /// Asks the session as extension `ext` would, through its host.
    fn ask(&self, ext: &str, request: Request) {
        handle(
            &self.session,
            &ExtensionId::from(ext),
            Grants::default(),
            request,
        )
        .unwrap();
    }

    /// The transport `who` publishes, as this peer's presence holds it.
    fn transport_of(&self, who: &str) -> Option<Transport> {
        self.presence
            .states()
            .get(&format!("{who}/play"))
            .and_then(transport_of)
    }
}

/// The host: one sync and one presence service for every document.
fn serve(scope: &Arc<Scope>) -> LocalServer {
    let registry = DocRegistry::new(|_doc_id| Box::pin(async { Ok(CrdtDoc::ephemeral()) }));
    let router = LayerRouter::new()
        .with(
            doc_sync_service_descriptor(),
            DocSyncDispatcher::new(registry.clone()),
        )
        .with(
            doc_presence_service_descriptor(),
            DocPresenceDispatcher::new(registry),
        );
    LocalServer::serve(router, scope.clone())
}

async fn eventually(what: &str, mut pred: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while !pred() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for: {what}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn cue(at: f64, look: &str) -> Item {
    Item {
        owner: ExtensionId::from("ignition"),
        lane: LaneId::from("cues"),
        kind: "ignition.cue".into(),
        at: Time::Seconds(at),
        end: None,
        payload: Value::map([("look", Value::from(look))]),
    }
}

fn chart(peer: &Peer) -> Option<String> {
    peer.session
        .read_ext(&ExtensionId::from("keyflow"))
        .text("chart")
        .map(str::to_owned)
}

fn cue_ids(peer: &Peer) -> Vec<String> {
    peer.session
        .items()
        .keys()
        .map(|id| id.as_str().to_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn two_peers_share_a_session_undo_and_a_transport() {
    let scope = Scope::new();
    let local = serve(&scope);
    let doc_id = Uuid::new_v4();
    let a = Peer::join(&local, doc_id, "ann").await;
    let b = Peer::join(&local, doc_id, "bob").await;
    let mut history = History::new(a.session.doc(), 100);

    // ── data, both ways at once ───────────────────────────────────────
    // On A, Ignition sets up its rig, a cue list and a cue at 4 s: one
    // step. On B, at the same time, Keyflow types its chart.
    history.transact(a.session.doc(), "Ignition: cue list", |_| {
        a.ask(
            "ignition",
            Request::MapSet {
                name: "rig".into(),
                key: "universe".into(),
                value: Value::Int(1),
            },
        );
        a.ask(
            "ignition",
            Request::PutLane {
                id: "cues".into(),
                lane: Lane {
                    owner: ExtensionId::from("ignition"),
                    kind: "ignition.cues".into(),
                    name: "Main".into(),
                    order: 0.0,
                    parent: None,
                },
            },
        );
        a.ask(
            "ignition",
            Request::PutItem {
                id: "cue-1".into(),
                item: cue(4.0, "Warm wash"),
            },
        );
    });
    b.ask(
        "keyflow",
        Request::TextSplice {
            name: "chart".into(),
            at: 0,
            delete: 0,
            insert: "Verse: C G Am F\n".into(),
        },
    );

    eventually("B sees Ignition's rig, lane and cue", || {
        b.session.item(&ItemId::from("cue-1")).is_some()
            && b.session.lane(&LaneId::from("cues")).is_some()
            && b.session
                .read_ext(&ExtensionId::from("ignition"))
                .map("rig")
                .is_some_and(|rig| rig.get("universe") == Some(&Value::Int(1)))
    })
    .await;
    eventually("A sees Keyflow's chart", || {
        chart(&a).as_deref() == Some("Verse: C G Am F\n")
    })
    .await;
    // B's own extension is untouched by A's writes, and both converge.
    assert_eq!(chart(&b).as_deref(), Some("Verse: C G Am F\n"));
    let cue1 = b.session.item(&ItemId::from("cue-1")).unwrap();
    assert_eq!(
        (cue1.at, cue1.payload.get("look")),
        (Time::Seconds(4.0), Some(&Value::from("Warm wash")))
    );
    eventually("both replicas hold one session", || {
        a.session.read() == b.session.read()
    })
    .await;

    // ── undo takes back A's step only ────────────────────────────────
    history.transact(a.session.doc(), "Ignition: add cue 2", |_| {
        a.ask(
            "ignition",
            Request::PutItem {
                id: "cue-2".into(),
                item: cue(8.0, "Blackout"),
            },
        );
    });
    b.ask(
        "keyflow",
        Request::TextSplice {
            name: "chart".into(),
            at: 16,
            delete: 0,
            insert: "Chorus: F G C\n".into(),
        },
    );
    eventually("A has B's chorus and its own cue 2", || {
        chart(&a).as_deref() == Some("Verse: C G Am F\nChorus: F G C\n")
            && cue_ids(&a) == ["cue-1", "cue-2"]
    })
    .await;
    assert_eq!(history.undo().as_deref(), Some("Ignition: add cue 2"));
    eventually("B sees cue 2 undone", || cue_ids(&b) == ["cue-1"]).await;
    // B's edit, made meanwhile, and A's earlier step both stand.
    assert_eq!(
        chart(&a).as_deref(),
        Some("Verse: C G Am F\nChorus: F G C\n")
    );
    assert_eq!(cue_ids(&a), ["cue-1"]);

    // ── B follows A's transport ───────────────────────────────────────
    let publish = |playing: bool, position: f64| -> Transport {
        let now = now_micros_f64();
        a.here.play(&PlayState {
            playing,
            position,
            at_ms: now / 1000.0,
            rate: 1.0,
        });
        Transport {
            host_micros: now,
            playhead_seconds: position,
            playrate: 1.0,
            is_playing: playing,
        }
    };
    let mut tracker = Tracker::default();
    // Both peers are in one process: one clock, offset zero.
    let offset = 0.0;

    for (what, playing, position) in [
        ("play", true, 0.0),
        ("seek", true, 42.0),
        ("stop", false, 43.5),
    ] {
        let truth = publish(playing, position);
        eventually(&format!("B hears {what}"), || {
            b.transport_of("ann").is_some_and(|t| {
                t.is_playing == playing && (t.playhead_seconds - position).abs() < 1e-9
            })
        })
        .await;
        // Twenty frames, 5 ms apart: B draws where A really is.
        for _ in 0..20 {
            tracker.observe(b.transport_of("ann").unwrap());
            let now = now_micros_f64();
            let shown = tracker.position(now, offset).unwrap();
            let error = (shown - truth.at(now)).abs();
            assert!(
                error < 0.001,
                "{what}: B shows {shown}, A is at {}, off by {error} s",
                truth.at(now)
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}
