+++
title = "Real-time collaboration (Loro)"
description = "Offline-first collaborative state: Loro CRDT replicas synced over vox channels — local writes are instant, everything merges, reconnects are deltas."
weight = 57
+++

architect has two state models, and they answer different questions:

| | **Optimistic store** ([optimistic](@/architecture/optimistic.md)) | **CRDT replica** (this page) |
| --- | --- | --- |
| Source of truth | the server | the *document* (every replica holds it) |
| Local write | optimistic patch → reconcile or **rollback** | applied to the local doc, final — concurrent edits **merge** |
| Offline | reads stale cache, writes fail | fully functional; syncs when back |
| Conflicts | server wins | Loro merge semantics (CRDT) |
| Right for | server-owned rows, thin clients | collaborative, offline-first features (Task projects, DAW sessions) |

Both share the same wire (vox), the same fan-out (`PubSub`), the same
phase-match rendering. Pick per feature.

## The stack

`libs/crdt` is the local-first layer: one [`CrdtDoc`] per collaboration
boundary (a project, a workspace, a DAW session) wrapping a `LoroDoc` +
pluggable `Persistence` (SeaORM server-side via `crdt-seaorm`, in-memory
for tests), and typed `LoroRepo<E>` CRUD views per entity (the
`EntityCrdt` glue). `example-crdt` already mounts one as an
`ExampleRepo` backend.

What `crdt::sync` (the `vox` feature) adds is the **transport**:

```text
 client A                      server                       client B
 ┌────────────┐   sync(vv,up,down)  ┌──────────────┐  sync   ┌────────────┐
 │ CrdtDoc    │ ──────────────────▶ │ canonical    │ ◀────── │ CrdtDoc    │
 │ (replica)  │  ◀── backlog ────── │ CrdtDoc      │ ──────▶ │ (replica)  │
 │ local ops ─┼──── up channel ───▶ │ + Persistence│ ─down─▶ │            │
 │            │ ◀─── down channel ──│ + PubSub     │         │            │
 └────────────┘                     └──────────────┘         └────────────┘
```

- **`DocSync`** — one `#[vox::service]` method: the client sends its Loro
  **version vector** plus an up-channel (its future local updates) and a
  down-sink. The server answers with exactly the missing history
  (`ExportMode::Updates { from }` — an offline week is a delta, not a
  re-download), then streams every peer's updates. Uses `PubSub`'s
  buffered attach, so nothing is missed between catch-up and live
  traffic; overlap is harmless because Loro imports are idempotent.
- **`DocSyncHost`** — the server: canonical doc (persisting every update
  through its `Persistence`) + unbounded fan-out (update bytes are never
  dropped — unlike state-shaped entity events, a lost CRDT update is a
  lost edit until the next catch-up). Server-side writes to the same doc
  (other transports, background jobs) broadcast automatically via the
  doc's local-update subscription.
- **`SyncedDoc`** — the client driver: wires a local `CrdtDoc`'s
  local-update stream into an outbox (buffered while offline), runs one
  `sync` session at a time, merges everything that comes down. `run()`
  is a plain future — spawn it with tokio, `dioxus::spawn`, or
  wasm-bindgen; on disconnect, run it again and the version vector makes
  re-sync incremental.

Proven by `libs/crdt/tests/sync_convergence.rs`: two live replicas
converge bidirectionally through the in-process transport, and a late
joiner with an empty doc catches up the full history.

## Using it today (server + native client)

```rust
// server: canonical doc + host, mounted like any service
let doc = CrdtDoc::open(project_id, SeaOrmPersistence::new(db)).await?;
let host = DocSyncHost::new(project_id, doc.clone());
router = router.with(doc_sync_service_descriptor(), DocSyncDispatcher::new(host));
// the same doc also serves plain RPC reads: ExampleRepoLoro::new(&doc)

// client: a replica that survives offline
let mut synced = SyncedDoc::new(project_id, CrdtDoc::ephemeral());
let repo = synced.doc().repo::<TaskEntity>();   // typed CRUD, writes are local + instant
spawn(async move { loop { let _ = synced.run(&client).await; /* backoff */ } });
```

## The roadmap to full client integration

In dependency order; each step is independently shippable:

1. **Dioxus hooks** (`architect`, atom+vox): `use_synced_doc(doc_id)` —
   owns the `SyncedDoc`, spawns/respawns `run` against the shared
   `Connection<Caller>` (reconnect = delta catch-up), and bridges the
   doc's change subscription to a `Signal` revision so
   `use_crdt_list::<E>()` / `use_crdt_entry::<E>(id)` re-read the
   `LoroRepo` into the same `AtomResult` phases pages already match.
   Mutations write the local repo directly — no optimistic machinery,
   no rollback arm.
2. **Client persistence**: a `Persistence` impl for the browser
   (IndexedDB) and desktop (file), so the replica itself survives
   restarts offline. The trait is already object-safe and wasm-clean;
   this is one impl per platform.
3. **Derive integration**: `#[architect(crdt)]` emitting the
   `EntityCrdt` glue (field ↔ LoroMap mapping — `crdt-derive` already
   covers part of this), the hooks from step 1 bound to the entity, and
   the host wiring — making a collaborative feature one attribute, the
   same way `store`/`events` work for server-owned state.
4. **Presence/awareness**: cursors, selections, who's-online — Loro's
   `EphemeralStore` (already re-exported by `libs/crdt`) over the same
   channel pattern, fanned out by a second `PubSub` (sliding — presence
   is state-shaped and droppable).
5. **Compaction policy**: the host calling `CrdtDoc::compact` on
   quiesce/N-updates so server storage stays bounded; shallow snapshots
   (`ExportMode::ShallowSnapshot`) for fresh joiners of long-lived docs.

## What this means for Task and DAW

- **Task** (local-first, multi-device): one doc per project; tasks/
  cycles/milestones as `EntityCrdt` types in it. Devices work offline and
  merge on reconnect; Nextcloud/file backends slot in as `Persistence`.
- **DAW**: a session doc for collaborative arrangement state (markers,
  regions, routing), with the REAPER extension holding a replica
  in-process — host edits broadcast to web UIs and vice versa. Live
  *signal* data (peaks, playhead) stays on `PubSub` event streams; CRDT
  is for *document* state.
