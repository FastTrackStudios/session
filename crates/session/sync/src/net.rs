//! Sharing a session over vox: one peer hosts, the others join.
//!
//! Built on architect's `crdt::sync`: the host serves `DocSync` (the doc)
//! and `DocPresence` (who is doing what) for one session id; each joiner
//! holds both calls open for as long as it is connected. The transport is
//! whatever carries vox — iroh between native peers (`architect::iroh_link`),
//! a WebSocket from a browser, an in-process link in tests. It is hub and
//! spoke through the host, which is also a participant: its own engine
//! edits the very doc it serves.

use architect::LayerRouter;
use crdt::CrdtDoc;
use crdt::sync::{
    DocPresenceClient, DocPresenceDispatcher, DocSyncClient, DocSyncDispatcher, DocSyncHost,
    PresenceDriver, PresenceHost, PresencePeer, SyncedDoc, doc_presence_service_descriptor,
    doc_sync_service_descriptor,
};
use uuid::Uuid;

use crate::doc::SessionDoc;

/// How long a peer's presence outlives its last update.
pub const PRESENCE_TIMEOUT_MS: i64 = 30_000;

/// The id a song's session is shared under: the same on every machine
/// that has the song, so everyone who opens it meets in one doc. `song`
/// must be stable across machines and opens — a library song id, or the
/// song's name — never an engine project guid, which is minted per open.
#[must_use]
pub fn session_id(song: &str) -> Uuid {
    const NAMESPACE: Uuid = Uuid::from_u128(0x6a1f_0c3e_8d2b_4f5a_9e7c_1b3d_5f7a_9c2e);
    Uuid::new_v5(&NAMESPACE, song.as_bytes())
}

/// Something that can be asked to tell everyone about this peer.
pub trait PresenceSink: Send + Sync {
    fn set(&self, key: &str, value: loro::LoroValue);
    fn delete(&self, key: &str);
    /// Every peer's entries, this one's included.
    fn states(&self) -> std::collections::HashMap<String, loro::LoroValue>;
}

/// This peer, serving its session to others.
#[derive(Clone)]
pub struct CollabHost {
    id: Uuid,
    sync: DocSyncHost,
    presence: PresenceHost,
}

impl CollabHost {
    /// Serve `doc` — the same Loro doc this peer's own bridge edits.
    #[must_use]
    pub fn new(id: Uuid, doc: &SessionDoc) -> Self {
        let crdt = CrdtDoc::from_loro(doc.loro().clone());
        Self {
            id,
            sync: DocSyncHost::new(id, crdt),
            presence: PresenceHost::new(id, PRESENCE_TIMEOUT_MS),
        }
    }

    /// The session id joiners ask for.
    #[must_use]
    pub const fn id(&self) -> Uuid {
        self.id
    }

    /// Add the session's two services to `router`.
    #[must_use]
    pub fn mount(&self, router: LayerRouter) -> LayerRouter {
        router
            .with(
                doc_sync_service_descriptor(),
                DocSyncDispatcher::new(self.sync.clone()),
            )
            .with(
                doc_presence_service_descriptor(),
                DocPresenceDispatcher::new(self.presence.clone()),
            )
    }

    /// How many peers are connected.
    #[must_use]
    pub fn peers(&self) -> usize {
        self.sync.active_sessions()
    }
}

impl PresenceSink for CollabHost {
    fn set(&self, key: &str, value: loro::LoroValue) {
        self.presence.store().set(key, value);
    }
    fn delete(&self, key: &str) {
        self.presence.store().delete(key);
    }
    fn states(&self) -> std::collections::HashMap<String, loro::LoroValue> {
        self.presence.store().get_all_states().into_iter().collect()
    }
}

/// This peer, joined to someone else's session.
pub struct CollabPeer {
    id: Uuid,
    doc: SessionDoc,
    synced: SyncedDoc,
    presence: PresencePeer,
    driver: PresenceDriver,
}

impl CollabPeer {
    /// Prepare to join session `id`. The doc starts empty and fills from
    /// the host on [`Self::run`]; wire the bridge to [`Self::doc`] only
    /// after the first sync (see `Bridge::join`).
    #[must_use]
    pub fn new(id: Uuid) -> Self {
        let doc = SessionDoc::new();
        let synced = SyncedDoc::new(id, CrdtDoc::from_loro(doc.loro().clone()));
        let (presence, driver) = PresencePeer::new(id, PRESENCE_TIMEOUT_MS);
        Self {
            id,
            doc,
            synced,
            presence,
            driver,
        }
    }

    #[must_use]
    pub const fn id(&self) -> Uuid {
        self.id
    }

    /// The replica — the same Loro doc the sync driver fills.
    #[must_use]
    pub const fn doc(&self) -> &SessionDoc {
        &self.doc
    }

    /// This peer's presence handle (cheap to clone).
    #[must_use]
    pub const fn presence(&self) -> &PresencePeer {
        &self.presence
    }

    /// Split into what the UI keeps (doc, presence) and the two session
    /// drivers, each to be run on a connection until it drops.
    #[must_use]
    pub fn into_parts(self) -> (SessionDoc, PresencePeer, SyncedDoc, PresenceDriver) {
        (self.doc, self.presence, self.synced, self.driver)
    }

    /// Run both sessions until the connection drops. Two clients, each
    /// established on its own link: a vox caller is bound to one service
    /// once constructed.
    ///
    /// # Errors
    /// When either session fails to attach.
    pub async fn run(
        &mut self,
        sync: &DocSyncClient,
        presence: &DocPresenceClient,
    ) -> eyre::Result<()> {
        let (a, b) = tokio::join!(self.synced.run(sync), self.driver.run(presence));
        a.map_err(|e| eyre::eyre!("session sync: {e}"))?;
        b.map_err(|e| eyre::eyre!("session presence: {e}"))?;
        Ok(())
    }
}

impl PresenceSink for PresencePeer {
    fn set(&self, key: &str, value: loro::LoroValue) {
        Self::set(self, key, value);
    }
    fn delete(&self, key: &str) {
        Self::delete(self, key);
    }
    fn states(&self) -> std::collections::HashMap<String, loro::LoroValue> {
        Self::states(self)
    }
}
