# How the DAW engine attaches to a session

Status: proposal, 2026-11-18, aligned with the arrangement and mixer plan
(FastTrackStudios/studio#519). For the thread building Studio's
arrangement view and mixer as runtime extensions, and for whoever makes the engine
(FastTrackStudios/daw, `daw:crates/daw/standalone`) play a session. Paths
are `repo:path`, symbols named, no line numbers. The session's own design
is `docs/design/session.md`; this note does not change it, it says how
`daw` meets it and what `session` must add.

## 1. What the engine is to a session

The engine is one more extension: id `daw`. It owns its lanes, items and
entries like any other (`docs/design/session.md` §4.1), and the session
knows nothing about audio. What makes it special is only that it makes
sound, so when it is present its transport is the one everything follows.

In a running Architect the engine is an extension of its own, `daw`
(studio#519): it holds the open session, plays it, draws nothing, and
provides two services: `fts.session` (this primitive's, unchanged) and
`fts.daw` (the DAW's own: tracks, items, peaks, transport, mixer edits).
The arrangement and mixer `[uses]` both, so with no engine the
arrangement still draws a session's lanes and items through
`fts.session` alone.

Two ways it sits beside the session, the same rules for both:

- **Linked, in the host's process** (Studio's window): it holds the
  `session::doc::SessionDoc` directly and edits it as `daw` (the
  `session::service::handle` rules, `crate::service`).
- **Its own process** (`studio engine`, a headless rig, a web worker): it
  asks `fts.session` over vox (`session::host`, the caller its principal)
  and follows changes through the stream in §5.2.

## 2. Who owns the transport

The transport is never in the document (§1.4 of the session design). It is
a `session::Transport` published as presence, stamped in the session
host's clock (Architect's presence clock, architect#845: `DocPresence::now`,
`PresencePeer::clock`, stamped with `Here::play_now`).

| present | leader | followers |
|---|---|---|
| an engine | the engine: its audio thread writes an `AudioSnapshot` per buffer to its `SnapshotCell` (`daw:crates/daw/standalone/src/transport_engine/engine.rs` `TransportShared` already owns one); a publisher thread passes each through `sync::PublishGate` and publishes it as a `Transport` in the shared clock (`Here::play_now`) | every view (chart, lights, arrangement playhead) follows with `sync::Tracker`; nothing in them steers the engine |
| several engines (two machines, both with audio) | the one whose transport was last pressed (the shared-transport rule: newest `(seq, by)` wins) | the others are `sync::TransportBackend`s driven by `sync::Follower`: rate nudged, locates landed on the sample (`TransportShared::begin_block` already lands a scheduled locate) |
| an engine, and views in the same app | the engine, as above; in the app it is also the playhead deck's active source (`architect.playhead`), so a view's playhead is a host animation on `Clock::Playhead` and a frame of playback costs the view nothing (studio#519, Architect's compositor) | views draw the deck's clock, not presence |
| no engine (a chart alone, lights alone, a browser) | a software clock: the peer that pressed play publishes a `Transport` from `sync::clock`, or Architect's playhead deck (`architect.playhead`) in one app | the same `Tracker` |

Rules:

- **Commands go to the leader, not into the document.** Play, stop, seek,
  rate and loop are calls on the engine's own service (a `daw` transport
  service), or on the software clock. A view's play button calls that,
  never `fts.session`.
- **The rate a follower sees is the transport's.** The tempo map is
  structure (document); the play rate, loop and metronome are the DAW's
  settings (`daw` registers) and its live state (the transport).
- **Leadership is presence, not document**: it changes every press and
  merges nothing.

## 3. Structure to the audio thread without locks or allocation

The audio thread never touches the document, a `Session`, Loro, Facet or
JSON. Between them is a **plan**: immutable, built off the audio thread,
swapped in whole.

1. **Watch** (control thread): the engine follows the document's changes
   (§5.2): the tempo map, its own lanes and items, its `daw` entries.
2. **Compile** (worker thread), into an `Arc<Plan>`:
   - the tempo map as precomputed segments (§5.1): seconds ↔ quarter notes
     ↔ samples, ramps included, so the engine plays the session's tempo
     and not a stepped copy;
   - per track, its items resolved to sample ranges under that tempo
     (`Item::seconds`), with their sources, fades and gains from the
     `daw.item` payloads, sorted for a binary search;
   - the graph: tracks, sends, FX (instances made off-thread from `fx:*`
     entries; plugin state loaded from blobs);
   - per automated parameter, its points in samples;
   - for other extensions that make sound or need sample accuracy
     (Signal's patch changes), a `sync::Schedule` each.
3. **Swap** at a buffer boundary: the engine's existing `arc-swap` (or an
   `rtrb` command), never a lock. The old plan goes back to a worker to
   be dropped, so no deallocation happens on the audio thread.
4. **Read** (audio thread), per buffer:
   - `TransportShared::begin_block` gives the block's timeline segments;
   - the plan answers what plays in them by binary search over slices
     (`Schedule::between` and its like);
   - `SnapshotCell::store` publishes the playhead.

   All of it is arithmetic over memory the plan already holds.

An edit while playing is a new plan at the next buffer. The playhead
stays continuous in seconds; a tempo edit moves musical items, not the
playhead.

## 4. The DAW's data as extension data

All of it is `daw`'s, in its namespace, through the same rules as any
extension. Large or binary data is a blob (Architect's blobs by hash); the
document holds the hash.

| what | stored as | merges |
|---|---|---|
| a track | lane `daw.track` (`parent` for folders, `order` for its place) | per field |
| its state (volume, pan, mute, solo, arm, monitor, input, channels, height) | map `track:<lane>` | per key |
| sends, hardware outputs | map `sends:<lane>` | per send |
| an FX | map `fx:<id>` (track, chain, index, plugin, enabled, params, preset); state a blob hash | per key |
| a media item | item `daw.item` at its time (seconds or quarter notes, its timebase); payload: source (blob hash or relative path), source offset, length in source, fades, gain, mute, lock, active take | per payload key |
| its takes | the payload's `takes` list (whole) | last writer |
| automation | register `env:<lane>/<param>`: points (time, value, shape), whole; per item envelopes in the payload | last writer |
| comping, fixed lanes | the track map and item payloads | per key |
| master, monitoring | maps `master`, `monitoring` | per key |
| project settings (sample rate, loop, metronome, record mode, play rate) | registers | last writer |
| import provenance (`.rpp` byte-exact export) | register `provenance` | last writer |

A view that is not the engine (the arrangement, the mixer) is a different
extension, so by the session's rules it sees `daw`'s items' kinds and
times but null payloads, and none of its entries. It gets the DAW's data
from the DAW: `fts.daw` (`tracks`, `items`, `peaks`, `state`), and edits
through it (`set`, `transport`), the engine writing the document as
`daw` (studio#519). Values that move every frame (meters, a dragged
fader) never cross as calls: they are slots of the engine's shared value
table, read by the host at compose.

## 5. What `session` must add

1. **A realtime tempo plan.** `TempoMap::plan() -> TempoPlan`: the map's
   segments precomputed (start seconds, start quarter notes, start bar,
   tempo at both ends, shape, meter), so seconds ↔ quarter notes ↔ bar
   and beat are a binary search and arithmetic with no allocation, ramps
   and bezier included. It must agree with `TempoMapEngine` (property
   test). The copied engine's bar and beat conversions allocate today
   (`docs/design/session.md` §3.2); this is that fix, and what lets the
   engine drop `DynamicTempoMap` (owner's decision 3).
2. **A change stream.** `SessionDoc::changes()` (a Loro subscription,
   filtered): what changed, by kind (tempo, markers, regions, a lane or
   item by owner, an extension's entry), so a plan is recompiled for what
   moved. Over the service, an `#[subscribe]` stream on `SessionService`.
   Polling `ReadTimeline` on a song of thousands of items is not a plan.
3. **Batches.** `Request::Batch(Vec<Request>)`: one transaction and one
   undo step for an import (an `.rpp` makes thousands of items) or a
   gesture that moves forty clips.
4. **Optional: reading another extension's data, by grant.**
   `Grants::reads: Vec<ExtensionId>`, for a third-party view that reads
   another extension's payloads directly. The DAW's own views do not need
   it: they read `fts.daw`.
5. **Blob references the session can find without knowing payloads.** A
   session kind's `DocKind::blobs` must list every blob a session holds
   (to fetch on open, to keep from collection), but it cannot read
   `daw`'s payloads. A `Value::Blob(String)` variant (a sha256) that any
   extension uses for a hash, collected by walking values.
6. **Transport leadership.** The shared-transport rule as a `session`
   type: `Lead { seq, by, transport }` in presence, newest wins. That is
   Studio's `SharedTransport` (`studio:features/session/src/sync/transport.rs`),
   so every follower and leader agrees on who leads.

Not in `session`: the plan, the graph, sample rates, plugin hosting, the
engine's transport service. Those are `daw`'s.

## 6. Order

1. `session`: the tempo plan (5.1) and the change stream (5.2). Engine
   and views need both before anything plays.
2. `daw`: an adapter crate (`daw-session`) that compiles a plan from a
   `SessionDoc` and publishes the transport (§2, §3), behind a feature
   of the engine; `DynamicTempoMap` replaced by the tempo plan.
3. `session`: blob values (5.5); read grants (5.4) only when a view
   outside the DAW needs another's data.
4. `session`: batches (5.3) and leadership (5.6), with the `.rpp` import
   path (`docs/design/session.md` §5.3 step 4).
