# session: the primitive everything agrees on

Status: first version, 2026-11-17. Decided by the owner on 2026-11-17
(FastTrackStudios/studio#510, #509; FastTrackStudios/architect#840);
this document designs it against the code as it is. Paths are
`repo:path`; `studio:` is FastTrackStudios/studio at `6019839d0`,
`architect:` is FastTrackStudios/architect at v0.12.6 (the tag Studio
pins) unless marked `main`.

## 0. In one page

- A **session** is song structure plus a timeline that knows nothing about
  what is on it: the tempo map, meters, bars and beats, markers, regions;
  **lanes** and **timed items** with a kind and a payload it does not read;
  and **per-extension data**, each extension's own namespace, merged the
  way the extension chose. It assumes no audio tracks.
- **Transport is data**: a playhead, its rate, whether it plays, and the
  instant that was true. Anything follows a playhead from that and the
  tempo map, with no engine.
- **Two kinds of sync, kept apart.** Project state is Architect's: a
  session is a Loro document of kind `fts.session`, synced, shared and
  undone by Architect. Playback is this crate's `sync`: a lock-free
  snapshot, clock-offset estimation, drift correction, a follower, a
  realtime schedule. The playhead never goes in the document.
- `daw`, Keyflow, Ignition, Signal and lyrics each store what they need as
  lanes, items and entries of their own, and own their logic.

The crate (`session:crates/session`) is that, small on purpose. What it
leaves out is listed in §1.6.

## 1. The model

### 1.1 Time

`session::time` is `daw-proto`'s time model, copied unchanged with its
tests (`studio:crates/daw/proto/src/primitives`, minus the DAW-only
`enums`; `studio:crates/daw/proto/src/tempo_map/{engine,tempo_point}.rs`):
`PositionInSeconds`, `PositionInQuarterNotes`, `PositionInBeats`,
`MusicalPosition`, `PositionInPpq`, the durations, `Tempo`,
`TimeSignature`, `TempoPoint`, and `TempoMapEngine`. That engine is the
REAPER-faithful one: tempo is integrated in quarter notes (REAPER BPM is
quarter notes a minute), hold, linear and bezier segments, and bar and
beat counting across meter changes in the meter's own beat unit.

On top of it the session adds (`session:crates/session/src/timeline.rs`):

- **`TempoMap`**: a default tempo and meter and the change points (in
  seconds, as REAPER and both of Studio's models keep them), with the
  engine built once per edit. It is **one value**: two people editing the
  tempo map at once is last-writer-wins for the whole of it. Studio's
  `SessionDoc` already decided that (`studio:features/session/src/sync/doc.rs`,
  module docs: "a tempo map is only meaningful whole"), and it holds.
- **`Time`**: a point placed in **seconds** or in **quarter notes**. A
  chord on bar 5 is musical and moves with the tempo; a lighting cue on a
  hit point, or a recorded take, is absolute and stays. A `Time` keeps its
  base; the tempo map resolves either into the other (`Time::seconds`,
  `Time::quarter_notes`, `Time::bar_beat`).
- **`BarBeat`**: bar and beat, 1-based, with the fraction of the beat.

**Markers and regions** are the session's own: `Marker { at, name,
color, ruler }` and `Region { start, end, name, color, ruler }`, by
`MarkId`, with `Session::rulers` naming the rulers they sit on ("Song",
"Sections"). They are structure everyone reads (a chart's sections, a
song's bounds in a set, a lighting designer's scenes), so they are core
data, not an extension's.

### 1.2 Lanes and timed items

`Lane { owner, kind, name, order, parent }` is a row something is drawn
and played on; `Item { owner, lane, kind, at, end, payload }` is
something at a time on one. `owner` is the extension that made it;
`kind` is namespaced by it (`daw.track`, `ignition.cue`); `payload` is a
`Value` (Loro's value model: null, bool, i64, f64, string, bytes, list,
map) that the session carries and never reads. `order` is a fractional
sort key, so a concurrent insert and move both land; `parent` nests lanes
(a folder track, a group of cue lists). `end: None` is an instant (a cue,
a patch change).

Why generic lanes rather than "tracks": a view that follows the timeline
(Studio's arrangement, a minimap, a setlist overview) can show *that*
something is there and *when* without knowing what. That is what lets the
arrangement view be a replaceable extension (architect#840).

### 1.3 Per-extension data

`ExtensionData { version, entries }` is one extension's namespace, by its
`ExtensionId` (its Architect manifest id). Each entry's kind is its merge
rule, chosen by the extension when it first writes it:

| entry      | two people changing it at once                         | for                                |
|------------|--------------------------------------------------------|------------------------------------|
| `Register` | last writer wins, for the whole value                  | a setting, a small record          |
| `Map`      | per key: different keys both land; one key, last wins  | a mixer, a rig, a set of presets   |
| `Text`     | per character                                          | Keyflow source, a lyric sheet      |

`version` is the extension's own schema for its data: it migrates its
entries, the session never does. The API is by id (`Session::ext`,
`Session::ext_mut`); nothing lists another extension's entries, and
`Session::remove_extension` removes an extension's data, lanes and every
item it owns or that sits on its lanes, and nothing else. Whether one
extension may *ask* for another's is a permission, not a data-model
question (§2.4).

Large or binary data is never a value: an FX chain's state, a MIDI take
or audio is a blob, and the value holds its sha256 (Architect's blobs by
hash, `architect:docs/content/guide/documents.md` §6).

### 1.4 Transport as data

`Transport { host_micros, playhead_seconds, playrate, is_playing }`
(`session:crates/session/src/transport.rs`): where a playhead was, how
fast it moves, and the instant that was true, in a named clock.
`Transport::at(now)` extrapolates; `shifted(offset)` restamps it in
another clock; `bar_beat_at(now, tempo)` is the place a chart draws. It is
published when it changes (and a keepalive between), never streamed.

It is **the** transport type of the stack. `daw-transport-sync`'s
`Position` was the same four fields with the same `at` and `shifted`; in
the copy it is now this type (`session::sync::Position` re-exports
`Transport`), so the leader's playhead and transport-as-data cannot drift
apart. Architect's presence `PlayState`
(`architect:features/crdt/crdt/src/presence.rs`) is the same four numbers
in milliseconds; `session::doc::play_state` and `transport_of` convert.

### 1.5 The session

`Session { name, tempo, rulers, markers, regions, lanes, items,
extensions }` (`session:crates/session/src/session.rs`). Plain data: what
the document holds, what a file round-trips through, what a test builds.
`Session::timed(kind)` resolves a kind's items to seconds under the tempo
map, which is what a player builds its schedule from (§3).

### 1.6 Left out, and why

- **Tracks, clips, takes, FX, sends, envelopes, comping, the master bus,
  monitoring FX, the media bay**: the DAW's (§4.1), by decision.
- **Loop range, metronome, record mode, the play-rate setting**: a
  player's settings, so the DAW's extension data (`SessionModel::project`
  holds them today). A follower needs none of them: a loop reaches it as
  a locate in the transport.
- **Edit cursor, selections, time selection, focused track or item**:
  one person's, so presence. `SessionModel` already says so;
  `DawDocument::editor` moves there too.
- **Shared-transport mode** (Studio's `SharedTransport`: independent or
  shared, `seq`, `by`, which song): app behaviour over presence, built on
  `Transport`. It stays in the app until Architect has a shared clock
  (§2.5).
- **A setlist**: a list of sessions, not part of one (Studio's
  `features/setlists`).
- **PPQ, MIDI, stretch markers, source offsets**: the DAW's payloads.
- **Minting ids**: the caller mints them, so the core has no randomness
  and a re-import can derive ids (`session-format`'s `EntityId::derived`).
- **`TempoPoint::selected`** is not stored (UI state). `TempoPoint` keeps
  the field only because it is a verbatim copy.
- **The session's own id**: the Architect document's id is it.

## 2. A session is an Architect document

### 2.1 What an Architect document is

From `architect:docs/content/guide/documents.md` and
`architect:features/crdt/crdt`: a document is a Loro CRDT. `DocKind`
gives it a kind id, a file extension, a schema stamp with migrations and
the blobs it points at, stamped into its `meta` map (`kind`, `schema`).
`History` makes every change a named step undone as one, and only this
peer's (`History::transact`, `begin`/`commit`). `DocFile` keeps it in one
file, append-only, with branches, checkpoints and crash recovery.
`OrgSyncs` and `OrgDocs` sync it to the org's server; `DocLink` and
`SyncedDoc` share it peer to peer. `Here` and `Roster` carry presence
beside it (never in it). `TextBinding` binds a `LoroText` to an editor
with remote carets, and blobs travel by hash through
`architect::blob::Fetcher`. `DocRegistry` asks `read` or `write` on
`org/<org>/doc/<id>` for every call.

### 2.2 The session's shape in the document

`session::doc::SessionDoc` (feature `architect`) writes and reads a
`Session` in a `LoroDoc`:

```text
meta       : Map  { kind: "fts.session", schema: 1, name }
tempo      : Map  { map: <the whole tempo map, one value> }
rulers     : Map  "<index>" → name
markers    : Map  id → Map { at, name, color, ruler }
regions    : Map  id → Map { start, end, name, color, ruler }
lanes      : Map  id → Map { owner, kind, name, order, parent }
items      : Map  id → Map { owner, lane, kind, at, end, payload }
                    payload: a Map container when it is a map (keys
                    merge one by one), else a value
extensions : Map  extension id → its data version
ext:<id>   : Map  "r:<name>" → a register's value, "m:<name>" / "t:<name>" → true
ext:<id>:m:<name> : Map   a map entry
ext:<id>:t:<name> : Text  a text entry
```

- A `Time` is one value, `["s", 4.0]` or `["qn", 16.0]`, so a move that
  changes base cannot half-merge with another.
- **Map and text entries are root containers**, named rather than
  created. Two peers that create the same child container concurrently
  each make one, and one wins whole, losing the other's keys. Two peers'
  `ext:ignition:m:presets`, by contrast, are one map
  (`a_map_entry_created_on_two_machines_at_once_keeps_both` proves it).
  Loro 1.16 has `LoroMap::ensure_mergeable_map` for this; root containers
  work on every Loro that Architect accepts (`^1.12`). Ids and names are
  percent-escaped: a root container's name may not hold `/` (Loro
  panics), and `:` separates ours.
- Records (markers, lanes, items) are child maps by id. Their creator
  mints the ids, so no two peers create the same one (but see §10).
- `SessionDoc::write` reconciles: it writes only what differs (Loro skips
  a write of the value already there), so writing the same session twice
  records nothing. `ExtWriter` is the in-place path an extension uses per
  edit (one key, a few characters), which is what makes concurrent edits
  merge finely.

### 2.3 How two people converge, and what undo covers

Everything above is one Loro document, so convergence is Loro's: two
replicas that have seen the same operations hold the same session.
Granularity is per field of a marker, lane or item; per key of a map
payload or map entry; per character of a text entry. The tempo map, a
register, a list payload and a non-map payload are last-writer-wins
whole. The tests in `session:crates/session/tests/session/doc.rs` run two
peers: Keyflow's chart typed at both ends at once, a key added to
Ignition's rig, both payload keys of one cue changed. All converge with
both edits kept.

**Undo** is the document's. `History` over the same `LoroDoc` covers the
core, the timeline and every extension's entries, and undoes only this
peer's steps (`undo_takes_back_my_extension_edit_and_not_yours`).
Presence and the transport are never in it.

### 2.4 Where the binding stops, and why

- **`crdt`, with Architect's workspace-hack patched out.** The binding
  is `crdt` itself: `session::doc::kind()` is a `crdt::DocKind`
  (`fts.session`, extension `session`, schema 1), stamped by `DocKind`.
  The app registers it in its `DocKinds`, wraps `SessionDoc::doc()` in
  `History`, `DocFile` and the sync drivers, and the test syncs it through
  `SyncedDoc` and presence. Why `crdt` once looked unusable here: every
  Architect member depends on `architect-workspace-hack` (the fleet's
  feature union: Dioxus, Blitz, winit) behind
  `cfg(not(any(target_family = "wasm", no_workspace_hack)))`. A `cfg`
  decides what is built, but Cargo resolves every target table whatever
  the cfg, so a crate outside the union resolved the whole UI stack and
  failed to resolve without Architect's and Studio's patch tables.
  `--cfg no_workspace_hack` does not change resolution. With the hack
  off, `crdt` is about 216 crates and `architect` 77, with no UI, and
  Architect's `boundaries.toml` L0 rows already hold that. So nothing
  needed splitting. The session's root `Cargo.toml` patches the hack
  with an empty crate (`vendor/no-workspace-hack`), takes Studio's pins
  for the vox wire forks, and changes no rustflags. A fleet repo that
  depends on `session` (Studio) keeps its own union. Architect documents
  the pattern ("Outside the union" in its
  `skills/architect/references/xtask-tooling.md`, the PR beside this one).
  The same holds for the RPC crate, `architect` (its `vox` feature is an
  L0 row): nothing blocks the `fts.session` service wrapper moving into
  `session`.
- **No host service yet.** A linked (compiled-in) extension holds the
  `SessionDoc`. A **runtime** extension cannot. Architect's documents
  service is vault notes as text (`read_document`, `edit_document`), and
  an extension's `data`/`set_data` is a device-local file or
  `localStorage`, not synced and not in any document
  (`architect:skills/architect-extensions/references/host-api.md`,
  "Documents" and "Files, settings and data"). **Architect gap 1:** a way
  for an extension to read and write *its namespace* of a structured
  document. The shape that fits Architect today is a service. The host
  provides `fts.session` (`[provides]`) with methods `ext_get`, `ext_set`,
  `map_set`, `text_edit`, `items_put` and `items_remove`, each scoped to
  the calling extension's id as the host names it. The precedent is
  `architect.playhead`'s own-source rule
  (`architect:features/playhead/src/deck.rs` `Caller`). The rules are
  built (`session::service`, §7), and the `#[architect::rpc]` wrapper
  can now live in `session` too (the first item of §9).
- **No per-namespace permission.** `DocRegistry` checks read or write on
  the whole document. A linked extension that can write the session can
  write any namespace at the CRDT level; only the service above can scope
  a runtime one (`session::service` does). **Architect gap 2.**
- **No per-extension undo.** `History` is one stack per document and
  peer, so "undo the last lighting change" is not possible. Loro's
  `UndoManager` can exclude origins; `History` does not expose that. Undo
  also does not survive reopening (the guide says so).

### 2.5 Presence and the clock

A person's playhead rides presence (`<peer>/play`, `PlayState`), and
`play_state(&Transport)` writes exactly that encoding. **Architect gap
3:** `PlayState::at_ms` is documented as being in "the clock the two
sides share", but `crdt::presence::clock_ms` is each process's own
monotonic clock. Architect has no shared clock. Studio built one
(`studio:features/session/src/sync/clock.rs` `SessionClock` over
`daw_transport_sync::ClockEstimator`). Until Architect serves one, a
follower estimates its offset to the leader with
`session::sync::ClockEstimator` over whatever link it has.

`architect.playhead` (`architect:features/playhead`) is a different
thing: one deck per app, with its own small time model
(`Tempo { bpm, at, meter }`, `Section`, `Mark`), and not synced ("a
position is one device's"). That is consistent with this design (the
playhead is not document state). A session-backed source for it would
publish the session's sections and marks and drive its `Clock` from a
`Transport`; not done here.

## 3. Playback sync with realtime guarantees

### 3.1 What `sync` holds

`session::sync` (feature `sync`) is `daw-transport-sync`, copied
(`studio:crates/daw/transport-sync`):

- `AudioSnapshot` and `SnapshotCell`: the audio thread's per-buffer
  playhead and the instant it was true, published with a seqlock.
- `clock`: the process's monotonic microsecond clock (`performance.now()`
  in a browser).
- `BufferClock`: a delay-locked loop over callback times.
- `ClockEstimator`: NTP's four stamps, with the fastest round trips
  trusted.
- `DriftController` and `Follower`: rate nudge, locate, stop, for an
  engine that can be steered (`TransportBackend`).
- `PublishGate`: what a carrier sends.

Its simulation test (`session:crates/session/tests/sync_sim.rs`) holds a
follower within a few samples on a LAN and under a millisecond across the
internet.

New here: **`Tracker`**, a follower with no engine, and **`Schedule`**,
timed events an audio thread reads.

### 3.2 What an audio thread may call

| safe on the audio thread (no lock, no allocation)                      | not                                                                 |
|------------------------------------------------------------------------|---------------------------------------------------------------------|
| `SnapshotCell::store` (wait-free, one writer), `load` (bounded retries) | anything on `Session`, `SessionDoc`, `ExtensionData` (maps, strings) |
| `Transport::at`, `shifted`                                             | `TempoMap::new`, `TempoMapEngine::new` (sort, allocate)             |
| `Schedule::between`, `last_at_or_before` (binary search on a slice)    | `bar_beat`, `at_bar`, `quarter_notes_to_musical`, `musical_to_quarter_notes` (allocate the meter changes) |
| `TempoMapEngine::seconds_to_quarter_notes`, `quarter_notes_to_seconds`, `tempo_at`, `time_signature_at` (no allocation; linear in points, at most 64 bisection steps in a ramp) | `ClockEstimator::record` (sorts a copy) |
| `DriftController::step`                                                | `Schedule::new` (sorts)                                             |

The bar/beat conversions allocate because the copied engine collects the
meter changes on each call. Precomputing them at construction would make
them audio-thread safe. That is a change to the copy, deferred until the
two copies are one (§6.3).

### 3.3 How a follower tracks a playhead

A chart, a lyric display or a lighting rig on another machine:

1. The leader (the DAW's engine) writes an `AudioSnapshot` per buffer.
   Its carrier offers each one to a `PublishGate` and sends the ones that
   pass as `Transport`s (`AudioSnapshot::position`), stamped in the
   shared clock.
2. The follower pings the leader (or the session host) ten times a
   second and feeds a `ClockEstimator`: `offset = shared − local`.
3. Each message goes to `Tracker::observe`; an older stamp than the one
   held is ignored, since messages arrive out of order. Each frame,
   `Tracker::position(local_now, offset)` says where to draw.

Because every message says when it was true, network delay does not move
the answer; only the clock estimate's error does. A move of the leader's
own (play, stop, a seek, a rate change) shows at once. A correction (the
offset estimate settling) is slewed over 250 ms, so a playhead drawn at
240 fps never twitches.

The test `a_follower_on_another_clock_tracks_the_playhead_within_a_millisecond`
runs a follower 3.7 s of clock away, with 5 to 80 ms of jitter and
asymmetric pings, through play, a seek and a stop, and holds it within
1 ms. `Transport::bar_beat_at` then gives the bar through a tempo change.

A follower that *plays audio* (a second DAW, a click) is a
`TransportBackend` driven by `Follower` instead: the same messages, the
rate nudged, locates landed on the sample.

### 3.4 The engine and the document

The DAW engine (`studio:crates/daw/standalone/src/transport_engine/engine.rs`
`TransportShared`) already owns a `SnapshotCell`. What it plays from the
session it reads off the audio thread. On a document change it rebuilds
what it needs (its tempo map, a `Schedule` per kind it plays) and hands
it to the audio thread whole, through its own command queue, dropping the
old one off the audio thread. Architect is async; none of it runs on the
audio thread.

**A contradiction to resolve:** the engine's realtime tempo map
(`studio:crates/daw/standalone/src/transport_engine/tempo_map.rs`
`DynamicTempoMap`) is **stepped**, while `TempoMapEngine` integrates
linear and bezier ramps. A ramp drawn in the session plays as steps
(§7.3).

## 4. The consumers, concretely

### 4.1 `daw`: audio tracks, takes, FX chains

| what                 | stored as                                                                                                                                                                                            |
|----------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| a track              | `Lane { owner: "daw", kind: "daw.track", name, order, parent }` (folders by `parent`)                                                                                                                 |
| its state            | map entry `track:<lane id>`: volume, pan, mute, solo, phase, parent send, visibility, armed, monitor, record input, channels, automation mode, height, lane display, groups (one key each, so per-field merge, as `SessionDoc` schema 4 does) |
| sends, hw outputs    | map entry `sends:<lane id>`, a key per send                                                                                                                                                         |
| FX                   | map entry `fx:<fx id>`: track, chain (`""`/`input`/`monitoring`), index, plugin, name, enabled, offline, params, preset; plugin **state is a blob** by hash                                           |
| envelopes            | register `env:<lane id>/<what>`: the points, whole (as today)                                                                                                                                        |
| a media item         | `Item { owner: "daw", lane, kind: "daw.item", at, end, payload: Map { … } }`: fades, gain, mute, lock, loop, snap offset, colour, label, timing, active take; `at` in seconds or quarter notes by its timebase |
| takes                | the payload's `takes` (a list, whole, as today), each with a source: a path relative to the project, or a blob hash for MIDI and opaque sources (`SourceRef::Object` today)                          |
| comping, fixed lanes | the track's map entry and the item payload                                                                                                                                                           |
| master, monitoring   | map entries `master` and `monitoring`                                                                                                                                                                |
| project settings     | registers `sample_rate`, `loop`, `metronome`, `record_mode`, `playrate`, `automation_override`, `bay` (a blob)                                                                                        |
| import provenance    | register `provenance` { format, source blob, original name, edited }: what keeps `.rpp` export byte-exact                                                                                            |
| take markers         | map entry `take_markers`, keyed `<source>#<id>` (they belong to recordings, not to the timeline)                                                                                                     |

The DAW plays a session: it builds its graph from its own lanes and data,
reads the core's tempo map, and ignores lanes it does not own.

### 4.2 Keyflow: the chart

Text entry `chart`: the Keyflow source, bound to the editor with
Architect's `TextBinding` (carets included). Its parsed chords and
sections are derived, not stored. Sections it scaffolds are written to
the core's **regions** on a "Sections" ruler when the person asks (as
`studio:features/session/src/keyflow/scaffold.rs` does today), since
markers and regions are structure everyone reads. A **chart alone** is a
session with a tempo map, maybe regions, and `keyflow`'s entries: no
engine, no lanes. When a DAW opens the same session, both follow one
transport.

### 4.3 Ignition: cue lists, timed or stepped

A cue list is a lane `{ owner: "ignition", kind: "ignition.cues" }`. A
cue is an item `{ kind: "ignition.cue", at, end: None, payload: Map {
number, label, look, fade_in, fade_out, follow } }`, at a musical time
(on the downbeat of the chorus) or in seconds (on a hit). The list's mode
is a map entry `list:<lane id>` { mode: `timed` | `manual` | `both`,
armed }.

- **Stepped manually**, a list is the same items in cue-number order. GO
  is the operator's action, and *which cue is live* is playback state,
  not document: Ignition publishes it like a transport (a cue pointer and
  when it went), so a second console follows it.
- **Timed**, the renderer follows the transport with a `Tracker` and
  fires `Schedule::between` each frame. If it outputs DMX from an engine
  clock, it is a schedule on that clock.

The rig, fixtures and looks are Ignition's own data (map entry `rig`), or
its own documents if they are shared across songs.

### 4.4 Signal: timed patch changes that reach the audio thread

A lane per rig `{ owner: "signal", kind: "signal.patches" }`; items
`{ kind: "signal.patch", at, payload: Map { rig, preset, scene } }`. On a
document change the engine builds a `Schedule<PatchChange>` from
`Session::timed("signal.patch")` (resolved to seconds under the tempo
map) and hands it to the audio thread. Each buffer,
`between(start, end)` says which changes fall inside it, and so at which
sample; on a locate, `last_at_or_before(playhead)` says which patch to
chase. No lock and no allocation
(`a_schedule_answers_a_buffer_and_a_locate`). The preset library is
Signal's own documents, not the session.

### 4.5 Lyrics

Text entry `source` in `lyrics` (the sheet), a lane `lyrics.lines`, and
an item per line `{ kind: "lyrics.line", at (musical), end, payload: Map
{ text } }`. Today the lines are stamped as items on a DAW "Lyrics"
track (`studio:features/session/src/lyrics.rs` `stamp_lines`); here they
need no DAW. A lyric display follows with a `Tracker`.

## 5. The migration map

### 5.1 `DawDocument` (`studio:crates/daw/session-format/src/document.rs`)

| field                                   | goes to                                                                       |
|-----------------------------------------|-------------------------------------------------------------------------------|
| `format_version`                        | the file's own version (stays in `session-format`)                            |
| `id`                                    | the Architect document id                                                     |
| `name`                                  | core `Session::name`                                                          |
| `sample_rate`                           | `daw` register `sample_rate`                                                  |
| `saves` (`SaveEntry`)                   | Architect's `DocFile` checkpoints; until then, `session-format`'s             |
| `tempo_map: Vec<TempoPoint>`            | core `TempoMap` (the same `TempoPoint`)                                       |
| `markers: Vec<MarkerNode>`              | core `markers` (no `region_end_seconds`) and `regions` (with it); `MarkerNode::id` → `MarkId`; `Marker::lane` → `ruler`; REAPER's numeric `Marker::id` → `daw` map `marker_numbers` |
| `tracks: Vec<TrackNode>`                | `daw`: a lane per track (`parent` → `Lane::parent`), map `track:<id>` from `Track` |
| `TrackNode::envelopes`                  | `daw` registers `env:…`                                                       |
| `TrackNode::items` (`ItemNode`)         | `daw.item` items; `ItemNode::envelopes` in the payload                        |
| `TakeNode` (`take`, `source`, `stretch_markers`, `envelopes`) | the item payload's `takes`; `SourceRef::Object` → a blob hash |
| `fx_chain`, `input_fx_chain` (`ObjectId`) | `daw` `fx:*` entries, state as blobs                                        |
| `comping: LaneComping`                  | `daw` track map                                                               |
| `receives: Vec<ReceiveNode>`            | `daw` `sends:<lane>` (source by id, as today)                                 |
| `provenance: Provenance`                | `daw` register `provenance`                                                   |
| `editor: EditorState`                   | presence (one person's)                                                       |
| `oplog: OplogRef`                       | gone: the document is the history                                             |

### 5.2 `SessionModel` (`studio:features/session/src/sync/model.rs`)

| field                                   | goes to                                                       |
|-----------------------------------------|---------------------------------------------------------------|
| `tempo: Vec<TempoPoint>` (at, bpm, beats_per_bar, beat_unit) | core `TempoMap` (a `TempoPoint` with a meter)  |
| `markers`, `regions` (`MarkerState`, `RegionState`) | core markers and regions (`lane` → `ruler`)        |
| `project.ruler_lanes` (`RulerLaneState`) | names → core `rulers`; REAPER `flags` → `daw` map `ruler_flags` |
| `chart: String`                         | `keyflow` text `chart`                                        |
| `tracks: Vec<TrackState>`               | `daw` lanes + `track:<id>` maps; `ext`, `ext_sections` (REAPER ext-state) → `daw` map `track_ext:<id>` |
| `items: BTreeMap<String, ItemState>` (with `TakeState`, `MidiEvents`, …) | `daw.item` items; MIDI events as blobs or whole payload values |
| `take_markers` (`TakeMarkerState`)      | `daw` map `take_markers`; `HitState` (drum hits) → the editing feature's own namespace (`editing`, map `hits`, the same keys) |
| `project.master`, `monitoring_fx`, `bay`, `automation_override` | `daw` entries                  |
| `project.ext` (REAPER `ProjExtState`)   | `daw` map `project_ext` (REAPER's, last-writer-wins per section, as today) |
| `project.looping`, `metronome`, `playrate`, `record_mode` | `daw` registers                    |
| presence `PeerState`, `SharedTransport` | stay presence; `SharedTransport`'s position, `at` and playing → `Transport` |

Setlists and lyrics are not in `SessionModel`; they live in Studio's
`session` crate (`setlist`, `lyrics`) and move with their features.

### 5.3 The order, without breaking a saved session

Every step keeps the `.session` and `.rpp` round trips green
(`studio:crates/daw/session-format/tests/{rpp_roundtrip,session_roundtrip}.rs`,
and the `studio:features/session` sync tests).

1. **This crate exists; nothing uses it.** (This PR.)
2. **One copy of the time model and sync** (§6.3). The `daw` repo's
   `daw-proto` re-exports `session::time`, and `daw-transport-sync`
   re-exports `session::sync`. The types are the same Rust with the same
   Facet shapes, so neither the wire nor the files change.
3. **The name** (§6.1): Studio's `features/session` crate becomes
   `session-app`. No behaviour change.
4. **The DAW's data as extension data, behind adapters.** In the `daw`
   repo: a `DawDocument ⇄ (Session, daw data)` conversion with a
   round-trip test over every fixture. `session-format` keeps reading
   `FORMAT_VERSION = 1` manifests and writes them unchanged until step 6.
   RPP import and export keep going through `DawDocument`, so byte-exact
   export (provenance plus `edited`) is untouched.
5. **The live document.** Studio's `SessionDoc` gets a `DocKind`
   migration, schema 4 → 5, to the shape in §2.2. Two offline devices may
   both run it and both runs merge, so it must be idempotent: it **never
   copies text or lists** (two copies would merge as both) but adopts
   them in place. The chart is the hard case: `keyflow`'s `chart` reads
   the legacy root `chart` text while it exists (an alias table in the
   binding, not built here). Records written by id (markers, items) move
   under the same ids, so concurrent runs write identical values. Readers
   of schema 4 keep working until every device has moved.
6. **The file.** `.session` becomes the session document in a `DocFile`
   (Architect's single file), and the directory format (a Styx manifest
   plus `objects/`) is read as an import. `FORMAT_VERSION` 2 marks the
   new layout; a build that cannot read it refuses it, as today.
7. **Out of Studio's session crate**: Keyflow's chart, lyrics and drum
   hits onto their extensions' namespaces; setlists to
   `features/setlists`.
8. **Sync.** Studio's relay
   (`studio:features/session/src/sync/{relay,net,direct}.rs`) is replaced
   by `OrgDocs` and `SyncedDoc`. `SessionClock` stays until Architect has
   a shared clock.

## 6. Names, the old repo, and the two copies

### 6.1 The name collision

Studio's `features/session` is a crate named `session` ("Session:
setlists, songs, the live session's sync, the song library and the
guide"): the Session *app's* feature crate. **Recommendation: rename it
`session-app`** (lib `session_app`). That is what it is; the
`apps/session/*` crates (`session-desktop`, `session-engine`,
`session-ui`, `session-web`) already follow that naming; and the
primitive keeps the plain name that every other repo will write in its
`Cargo.toml`.

The file extension `.session` is a second collision. Architect's guide
uses `DocKind::new("studio.song").extension("session")` as its example,
and `session-format` uses `.session` for its directory. Step 6 of §5.3
makes them one thing.

### 6.2 The old `session` repo

FastTrackStudios/session was the Session app until the consolidation.
Studio's `imported.txt` records `session . frame-mixer
f03928a8be851edbbc1b6755660ee011407a2f34`; `main` here was `51c7f47b`
(2026-09-30), an ancestor of `frame-mixer`.

The import was a **snapshot, not a merge**: Studio's history starts at
`f3930adb` (2026-10-12), and `f03928a8` is not in it. Every file of that
commit landed in Studio's first tree except 14 that the import edited
(the Justfile, `.gitignore`, `libs/vendor/anyrender_vello`, five
`apps/session-daw` files, two UI files). So the **content** is contained
in Studio; the **history** exists only here.

Sixteen branches carry commits that are not in `frame-mixer`, all older
than the import:

- `release/v0.0.2.6`: dependency bumps on the 0.0.2 line.
- `watch-guide`: the watchOS app. Studio has
  `apps/session/desktop/watchos`, so the work appears to have been
  carried over; not compared file by file.
- Fourteen research, compat and wip branches.

None is newer than Studio's copy. This branch's first commit removes the
old tree, keeping LICENSE; the history stays in this repository.

### 6.3 Two copies, and how they become one

`session::time` and `session::sync` are copies of code that is moving to
the `daw` repo in parallel. The layering decides the direction: `daw`
sits on `session`, so **`session` is the source**, and the `daw` repo's
crates become re-exports: `pub use session::time::*` in `daw-proto`'s
`primitives` and `tempo_map`, and `daw-transport-sync` either a
`pub use session::sync::*` facade or gone.

Until then, fixes land in `session` first. The copies are kept verbatim
except for module paths and `Position` = `Transport`, so a `diff`
against the originals shows exactly that. They are marked as lint debt
per module, because Studio's workspace lints are not clippy-clean on
them. The one deliberate divergence is `sync::Position`, which is now
`Transport` with the same fields and methods.

## 7. The owner's decisions

Decided on 2026-11-17:

1. **Who serves an extension its namespace:** the session host, the same
   everywhere. The rules are `session::service` (feature `service`): one
   `handle(doc, caller, grants, request)` every host answers with. The
   host names the caller and passes its grants. The extension reads and
   writes its own namespace only, and edits only the lanes and items it
   owns (it may put its items on another's lane). Everyone reads every
   lane and item's kind and time, while another extension's payload reads
   as null. The song's structure (tempo, markers, regions) is written only
   with `Grants::structure`. The host serves it as `fts.session` over
   `#[architect::rpc]`, inside `crdt::History::transact` for a named undo
   step. That wrapper can now live in `session` too (§2.4); it is not
   written yet (§9).
2. **The `crdt` split: yes**, with the light core the goal. Looking first
   found the core already light: the UI stack came only from the
   workspace-hack's feature union, which Cargo resolves for any consumer.
   The smaller correct change is how an outside consumer depends on it (a
   `[patch]` with an empty hack crate), not a new crate. Done that way;
   §2.4 says why.
3. **Ramps in the engine:** the engine evaluates the session's tempo model
   (`TempoMapEngine`, precomputed for the audio thread) rather than its
   stepped `DynamicTempoMap`. A change in the `daw` repo, later.
4. **Markers and regions** stay in the core, as built. Name, time,
   colour and ruler are core; REAPER's marker numbers and ruler flags are
   the DAW's.
5. **A shared clock:** Architect serves the session host's clock beside
   presence. Not built yet: the end-to-end proof runs both peers in one
   process, so it needed none. The smallest version:
   - **On the host:** one more method beside presence on the session's
     document lane, `now() -> f64`, the host's `session::sync::clock`
     in microseconds. That is Studio's `SessionClock`
     (`studio:features/session/src/sync/clock.rs`), moved where every
     host has it.
   - **On each peer:** `PresenceDriver` pings it ten times a second and
     feeds a `ClockEstimator`, and exposes `shared_now_ms()` and the
     offset.
   - **In `PlayState`:** `Here::play` stamps `at_ms` in the shared clock;
     `PlayState::at_ms` then means what its doc says. A follower passes
     the estimator's offset to `Tracker::position`.
   - **Size:** a fifth field in no wire type, one RPC, one estimator per
     peer. The host's own clock is offset zero.

Recommendation, not yet acted on: Studio's crate named `session` becomes
`session-app` (§6.1), when Studio adopts the primitive.

## 8. Two peers, end to end

`session:crates/session/tests/sync_e2e.rs`: two peers, each holding the
session as an Architect document, kept in step through Architect's sync.

- On A, Ignition (through `session::service`) sets a rig key, makes a cue
  lane and puts a cue at 4 s, as one `History` step. On B at the same
  time, Keyflow types its chart. Each sees the other's, B's own
  extension's data is untouched, and both replicas read the same session.
- A adds a second cue as another step while B types a chorus. Undo on A
  takes back the second cue only: B sees it go, B's chorus and A's first
  step stand.
- A publishes play, a seek to 42 s and a stop with Architect's presence
  (`Here::play`, a `PlayState`). B reads it from its own presence,
  `session::doc::transport_of` turns it into a `Transport`, and `Tracker`
  draws B's playhead within 1 ms of A's for twenty frames after each.

**Real:**
- Architect's document sync (`SyncedDoc` against a `DocSyncHost` in a
  `DocRegistry`), its presence (`PresencePeer`, `PresenceHost`), its
  undo (`History`) and its kind (`DocKind`).
- The vox services and codecs between them, over Architect's in-process
  transport (`architect::LocalServer`).
- The session's own document shape, service rules, tracker and transport
  encoding.

**Simulated:**
- The network: both peers are in one process.
- The clock: one process has one clock, so the offset is zero. Following
  across two clocks over a jittery network is
  `session:crates/session/tests/session/follow.rs`, by simulation, until
  Architect serves the shared clock (§7.5).

## 9. Before Studio, Keyflow or Ignition can share a session

In order:

1. **The `fts.session` service wrapper**, built: `session::host` (feature
   `vox`), `SessionService::ask(doc, request)` over `service::handle`.
   - The caller is the call's principal. An extension's call
     (`Principal::Extension`) names its namespace; any other principal is
     refused.
   - The action is the permission engine's answer on `doc/<id>`: `read`,
     `write`, or `structure` for tempo, markers and regions.
   - The gate checks `read` before decoding (`doc/**`); the handler
     checks the exact document.
   - The host and its gate ask the same engine, the org's one, which
     answers a share link's guest by what the link grants. A read-only
     link reads and every edit is refused, with no guest code path.
2. **A host that holds the document**, for the first consumer, which
   should be Keyflow, the smallest: a chart with a tempo map and no
   engine. Its app opens a `.session` as a `DocFile` of `kind()`, syncs it
   with `OrgSyncs` (or a share link), and serves `fts.session` to its
   chart view. Keyflow's chart moves into `keyflow`'s `chart` text,
   bound with `TextBinding`.
3. **The shared clock** (§7.5), so a follower on another machine follows
   for real. Until then playheads across machines are as good as the
   process clocks' agreement (unrelated).
4. **Ignition** on the same session: its cue lists as lanes and items,
   following the transport with `Tracker`, through the service.
5. **Studio**: rename its `session` crate to `session-app`, then the
   migration of §5.3, which is the long pole (the DAW's data as extension
   data, the live document's 4 → 5 migration, the file).

## 10. Not sure of

- Payload containers rest on ids being unique per creator. An importer
  that derives ids on two machines at once (the same `.rpp` imported on
  both) would create the same item twice concurrently, and one payload's
  keys would be lost. Derived-id imports should run on one device, or use
  mergeable containers once Architect requires Loro ≥ 1.16.
- The tracker's thresholds (50 ms jump, 250 ms slew) were chosen for
  displays, not measured at 240 fps.
- The per-field tables in §4.1 and §5 are a target. The `daw` schema is
  the `daw` repo's to design and may well group differently.
