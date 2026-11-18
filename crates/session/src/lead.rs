//! The session host's transport, published: what a DAW engine (or any
//! host) calls so every view on every machine follows it.
//!
//! [`Leader::serve`] serves the session's presence over vox on a WebSocket
//! (Architect's `PresenceHost`, which also serves the host's clock:
//! `DocPresence::now`), and joins that presence as peer `lead`.
//! [`Leader::publish`] then puts the transport there as `lead/play`,
//! stamped in the host's clock (`Here::play_now`): Keyflow's chart,
//! Ignition's room and cue list, and any `sync::Tracker` follow it
//! (`crate::doc::transport_of`). The host's own audio clock stays the
//! truth: publish on every move (play, stop, seek, rate) and now and then
//! while playing, as a playhead deck is re-anchored.

use std::time::Duration;

use architect::LayerRouter;
use architect::connect::{Endpoint, pool};
use architect::host::EngineHost;
use crdt::presence::{Here, PeerState};
use crdt::sync::{
    DocPresenceClient, DocPresenceDispatcher, PresenceHost, PresencePeer,
    doc_presence_service_descriptor,
};
use uuid::Uuid;

/// The peer name the transport is published under (`lead/play`).
pub const LEAD: &str = "lead";

/// A session's transport, led from here.
pub struct Leader {
    url: String,
    here: Here<PresencePeer>,
}

impl Leader {
    /// Serves session `doc`'s presence and this host's clock on `bind`
    /// (`0.0.0.0:4142`), and joins it as the leader.
    ///
    /// # Errors
    ///
    /// The endpoint could not be dialled once served.
    pub async fn serve(bind: &str, doc: Uuid) -> Result<Self, String> {
        let router = LayerRouter::new().with(
            doc_presence_service_descriptor(),
            DocPresenceDispatcher::new(PresenceHost::new(doc, 30_000)),
        );
        let port = bind.rsplit(':').next().unwrap_or("4142").to_owned();
        let bind = bind.to_owned();
        tokio::spawn(async move {
            let _ = EngineHost::ungated(router, bind).serve().await;
        });
        let local = format!("ws://127.0.0.1:{port}/vox");
        let mut tries = 0_u32;
        let client: DocPresenceClient = loop {
            match pool().client(&Endpoint::url(&local)).await {
                Ok(client) => break client,
                Err(e) if tries >= 50 => return Err(e.to_string()),
                Err(_) => {
                    tries = tries.saturating_add(1);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        };
        let (peer, mut driver) = PresencePeer::new(doc, 30_000);
        let clock = peer.clock().clone();
        tokio::spawn(async move {
            let _ = driver.run(&client).await;
        });
        // The driver's first ping gives the clock estimate `publish`
        // stamps with: wait for it, so the first publish is not dropped.
        let mut waited = 0_u32;
        while clock.offset_ms().is_none() {
            if waited >= 250 {
                return Err("the host's clock gave no estimate in 5 s".to_owned());
            }
            waited = waited.saturating_add(1);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let here = Here::new(peer, LEAD, PeerState::new(LEAD, "Transport"));
        Ok(Self { url: local, here })
    }

    /// Where it is served on this machine (`ws://127.0.0.1:<port>/vox`):
    /// another machine dials this host's address on the same port.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The transport now: playing or not, at `position_seconds` of the
    /// session, at `rate`. Stamped in this host's clock, which is the
    /// shared clock. `false`, and nothing sent, while there is no clock
    /// estimate (never once [`Self::serve`] has returned).
    pub fn publish(&self, playing: bool, position_seconds: f64, rate: f64) -> bool {
        self.here.play_now(playing, position_seconds, rate)
    }
}
