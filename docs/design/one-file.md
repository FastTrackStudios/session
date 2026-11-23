# A song is one file

A song is one file, `<Title>.session`. It replaces the pair that a song
folder held before:

| before | what it was |
|---|---|
| `<Title>.session` (+ `-wal`, `-tshm`) | the DAW engine's project: a Turso (SQLite) store, the `daw` repo's `session-format` |
| `<Title>.fts-session` | this crate's session: an Architect document file (`ARCHDOC`), kind `fts.session` |

The session is now a **part** of the engine's store. The store keeps its
format and adds a few tables (the `daw` repo, `session-format`,
`store::parts`). Nothing about the session's document changes: it is the
same Loro document, with the same id and the same shape (`session.md`), and
it is kept somewhere else.

## Inside the file

```text
meta            schema = 1
                id = <the song's UUID>          what setlists name the song by
                part:<part>:<branch> = <UUID>   a part's doc id (not session@main)
blobs           hash → kind, bytes (zstd)       plugin states, MIDI, imported text
branches        name → parent, base             main, and its arrangements
updates         seq → branch, label, at, update the engine's project, per branch
snapshots       branch → upto, snapshot
history         seq → branch, kind, label, …    undo steps and marks
part_updates    seq → part, branch, at, update  each commit of a part (zstd Loro update)
part_snapshots  (part, branch) → upto, snapshot a part folded (zstd Loro snapshot)
```

- **The song's id** is `meta.id`. For a converted song it is the UUID that
  the old `.fts-session` had in its header, so a setlist that names the
  song by id keeps working. The session part on `main` syncs under the same
  id (`Store::part_id("session", "main")` returns `meta.id`), so a peer that
  synced the old document finds the same document, with the same history.
- **The session is the part `session`.** It holds the timing (the tempo map
  and its meters), the markers, the sections as regions, `bar_one`, the
  generic timeline (lanes and items), and every extension's namespace:
  - Keyflow's chart, as typed data: lanes `keyflow.bars`, `keyflow.chords`,
    `keyflow.rhythm`, `keyflow.cues` and `keyflow.dynamics`, plus the text
    entry `keyflow/chart` for whatever is not typed yet (keyflow
    `features/session/src/typed.rs`). The chart lives only in the file.
    There is no `.kf` export.
  - Ignition's cue lists, in the `ignition` namespace and on `ignition.*`
    lanes. They are generic: the venue's overrides live beside the setlist,
    not in the song.
  - Signal's patch changes, as items on a `signal` lane, at a time. They
    are generic: a set's own guitar version lives beside the setlist.

  The session does not interpret any extension's data. It is all one Loro
  document, so it all merges, undoes and syncs in the same way.
- **The engine's project is unchanged.** Its tables, its branches and its
  history keep their meaning. A build from before parts reads the file and
  ignores the part tables. The tables are only added, so the schema stays
  `1`.

## Arrangements are branches

An arrangement or variation is a branch of the store (`branches` already
records `parent` and `base`). A branch has its own engine project and its
own session part. `Store::fork` copies the parent's parts, history
included, which is cheap because a session is kilobytes. The two copies
stay in one lineage, so they can still be merged. Each branch's session
syncs on its own doc id (`part:session:<branch>`).

A setlist will name an arrangement as `<song id>@<branch>`. Without
`@`, the setlist means `main`. Nothing parses that yet.

## Whole, and movable

The store writes ahead into `-wal` while it is open. Closing it
(`Store::close`, `song_file::SongFile::close`) checkpoints with
`TRUNCATE`, so after that the `.session` alone is the song. It can be
copied or carried on an external drive, and a reader that has only its
bytes can open it (`session_format::read::ProjectFile`, which is pure
Rust and needs no Turso, so it also runs in a browser). The engine already
checkpoints after each save.

## Which file is which: by header

Until a folder is converted, readers must handle both layouts. They decide
by the file's first bytes, never by its extension
(`session_format::sniff`, `session::file::sniff`):

| first bytes | it is |
|---|---|
| `SQLite format 3\0` | a store: the song's one file (or, beside a `.fts-session`, the old pair's engine half) |
| `ARCHDOC\0` | an Architect document: the old pair's session |

A folder holds an old pair when it has an `ARCHDOC` file. In that case the
session is read from that file. Otherwise the folder holds one file, and
the session is read from that file's `session` part (`song_file::Layout`).

## Opening the session in its file

`session::file::SessionFile::over_persistence(id, persistence)` opens the
session over any `crdt::Persistence`. `session_format::store::PartReplica`
is that persistence for a song's parts: each commit becomes a
`part_updates` row, and a save folds them into `part_snapshots`. Whatever
holds both crates (Studio's `song_file::SongFile`) connects the two. This
crate does not depend on the DAW's format.

## Drive

Drive syncs two things today (`drive` repo):

- **Live Loro documents**, by `DocId` over `drive-node`'s `/sync`
  WebSocket. A peer sends `Join { doc, version_vector }`, the node answers
  `Synced { export(updates since it) }`, and after that each edit travels
  as an `Update`.
- **Plain files**, as whole BLAKE3 blobs.

The one file fits both:

1. **The live parts are addressable documents.** Each part of each branch
   has a doc id (`Store::part_id`). Drive names it
   `<org>/studio:song:<song id>` for `session@main`, and
   `…:<song id>@<branch>` for an arrangement's. `Store::part_updates_since(part,
   branch, version_vector)` answers a `Join` from the file, and
   `PartReplica::append_update` stores an incoming `Update` as a row. The
   timing, the chart and the cues therefore sync live and peer to peer,
   without sending the file.
2. **The whole file syncs as a file**, for a first copy, a backup, or the
   engine's project, which does not sync live. Only a closed, checkpointed
   file is complete. Drive must not read a `.session` that has a non-empty
   `-wal` beside it, and must never sync `-wal`, `-shm` or `-tshm`.
3. **A file that arrives** replaces the local copy only if the local copy
   holds nothing it lacks. Otherwise the two are merged part by part. Loro
   merges the parts. For the engine's project, one side's branch is kept
   and the other side's is kept as a branch beside it, never overwritten.

### What Drive still needs (not built)

- A kind for `.session` (`drive-spine` `kinds::register`, app `studio`,
  `doc_prefix: "studio:song:"`), so a song file and its live document are
  shown as one thing.
- Ignore rules for `*.session-wal`, `*.session-shm` and `*.session-tshm`.
  Defer hashing and serving a `.session` while its `-wal` is non-empty, or
  serve a checkpointed copy through the SQLite backup API.
- A bridge in the app (`drive-client`) between a `Drive` document and the
  song file's `PartReplica`. Join with the part's version vector, import
  what arrives, append it to the file, and send local commits. This is the
  `crdt::Persistence` that `drive-client` already uses for its offline
  copy, with the song file in place of a folder.
- Merging a file that arrives whole (point 3 above).

## Converting a folder

`song-file convert <folder>` (Studio, `features/song-file`) folds an old
pair into one file:

1. It writes a **new** file, `<Title>.session.new`: a copy of the engine's
   store with its log, plus the session's branches as parts and the
   document's id as `meta.id`. The old files are only read.
2. It **verifies** the new file. From its bytes alone it checks the id and,
   for every branch of the session, the deep value and the version vector,
   so history is preserved. It checks the engine's project (each branch's
   document, the blobs, the history) against a scratch copy of the old
   store. If anything differs, it stops and leaves the old pair untouched.
3. It moves the old pair (`.fts-session`, `.session`, `-wal`, `-tshm`) to
   the desktop's **Trash**, then renames the new file to `<Title>.session`.
   It never deletes and never writes over a file.

Running it again on a converted folder does nothing. A folder where a run
stopped between steps 2 and 3 is verified again, and the run continues.
`--dry-run` builds and verifies in a temporary folder and changes nothing.
