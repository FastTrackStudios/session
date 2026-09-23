# Handoff: the live Session — setlist, Organize mode, live keyflow

Written 2026-09-22 (evening), replacing the morning's version of this file.
Everything is **committed locally only** — nothing pushed, merged or
deployed, by the user's standing rule (commit locally; push once the WASM
demo is right). Six sibling checkouts under `/Volumes/build-disk/development/`
are in play, and the session app now builds **four of them from local
paths** (§5 — they must be pushed and pinned before session is pushed).

## 1. What we are building, in the user's words

Session is the live player and the DAW: a setlist of songs that plays
itself on a service, and the arrangement/mixer/chart to prepare them with.
Keyflow (`.kf`) is the song's chart and the source of its structure: tempo,
meter, sections, key, chords — and from them the click, count and spoken
cues. Prepare a song **once**, save it as a `.session`, open that from then on.

## 2. Run it, and read what it did

```bash
cd /Volumes/build-disk/development/session
just app "../sessions/Worship Set.setlist" "" organize   # the setlist, starting in Organize mode
just app "../sessions/imported/Praise/Praise.RPP"         # one song (its .session opens if there is one)
just prepare ../sessions/imported/*/*.RPP                  # prepare songs into .session ahead of time
```

- `just app` builds the **`release-fast`** profile (release optimisation,
  incremental — a one-line edit rebuilds in ~15 s instead of ~60 s; see the
  `build-performance` skill), wraps the binary in
  `target/release-fast/Session Dev.app` and starts it with `open`, so it
  comes to the front. (A bare binary started from a shell opens BEHIND other
  windows and cannot be raised.)
- **Logs:** everything it prints — tracing, panics with `RUST_BACKTRACE=1` —
  goes to `~/Library/Logs/Session Dev/session-dev.log`. When the user says
  "it crashed", read that first.
- **Seeing the window yourself:** computer-use cannot target the unbundled
  binary. Find its window id (Swift `CGWindowListCopyWindowInfo` by pid,
  window named "Session") and `screencapture -x -o -l <id> out.png`. You
  **cannot click or type into it** — interaction is tested by the user or
  by tests. Say which when reporting.
- `blitz_shot` renders a song headless, through the app's own panels:
  `FTS_BLITZ_SIZE=1800x900 ./target/release-fast/blitz_shot <song.RPP|.session> out.png`.

## 3. Where it stands

### Done this session

**Songs and the `.session` format**
- `daw_standalone::session_file` (daw repo): a live engine project saved as
  a `.session` and loaded back — the exact inverse of the `.rpp` loader,
  merged over the original file so what the engine does not model (FX
  state, unknown lines) survives. Round-trip tests on a fixture and on the
  real Always On Time.
- **Prepare once:** opening `Song.RPP` opens `Song.session` beside it if it
  exists; otherwise it prepares (organize → build from the chart → generate
  click/count/guide) and saves `Song.session`. `FTS_SESSION_REPREPARE=1`
  re-prepares. The `prepare` bin / `just prepare` does it ahead of time.
- All 7 imported songs + the curated `sessions/Always On Time` are prepared.
  Their charts are the real ones from battleship `~/Downloads/worship kf`,
  copied into each song folder as `<Song>.kf` (the importer's placeholder
  chart is kept as `<Song>.kf.imported`). Chart tempos were checked against
  the click stems — exact (the stems click at 2×); the importer's own tempos
  (129.2, 139.67, 143.55) were wrong and are replaced by the charts'.
- The importer's guessed `Cue N` regions now go on the SECTIONS lane, so a
  chart replaces them. The 7 imported RPPs were migrated (originals kept as
  `.RPP.bak`) and re-prepared — no `Cue N` regions remain.
- Media paths are anchored per song (`project_loader::anchor_media`) and a
  song opens from its absolute path — relative anchoring once broke every
  source (no peaks, no audio).

**The setlist** (`session-daw/src/setlist.rs`, `open.rs`, the desktop shell)
- One engine holds every song; `Setlist::open` opens each, the first current.
  A setlist is a folder of song folders or a `.setlist` file —
  `sessions/Worship Set.setlist` (the 7 songs, with the curated Always On Time).
- Safari-style tabs in the top bar: a colour dot (from the title, or set by
  hand from the dot — saved to the song's SONG region and its `.session`),
  and a progress line per tab (the playhead in the current song, where it
  was left in the others).
- Picking a tab remounts the views on that song and **moves the audio
  engine** to it (drop + re-attach: a short device gap). Live mode rolls
  into the next song at a song's end (`Move::PlayFrom`).

**Organize mode** (Mode menu → Organize, in the DAW view)
- Left: the song's `.kf` in `editor-view` (Blitz-native, from the editor
  repo), with keyflow colours and live diagnostics (`editor-keyflow-lang`).
- **Live:** each ~350 ms pause in typing → a worker thread →
  `prepare::apply_chart` (`rebuild_from_chart` + guide regeneration) → saves
  the `.kf` and the `.session` → the chart panel shows the new chart
  (`chart_panel::publish_live`) → the arrangement reads the song back
  (`studio::request_resync`). Text that does not parse changes nothing.
- Middle: the chart, one page fitted and following the song. A middle-drag,
  trackpad scroll or wheel takes it off the fit; a double-click puts it back.
- Right: the arrangement under the **Organize toolbar** — REAPER's
  "Organize 1" (`nix/reaper-config/reaper-menu.ini`): Count-In, =START,
  SONGSTART, the sections, SONGEND, =END, then the time signatures (Shift =
  one measure). A press hands the arrangement's edit cursor and time
  selection to the engine, runs the session's own action, regenerates the
  guide and resyncs.
- `rebuild_from_chart` (session crate) replaces exactly what a chart owns
  (SONG/SECTIONS regions; COUNT-IN, SONGSTART, SONGEND, =END; the tempo map;
  the KEY and CHORD items). Prepare uses it too.
- **Meter:** measures lay out at their own meter, meter changes are stamped
  as time-signature points, and chords are placed at their measure's real
  start. The keyflow parser was fixed so `!T2/4` alone on its line is a bar
  of 2/4. God, I'm Just Grateful's Breakdown is now that bar (chart edited,
  song re-prepared: 2/4 at 66.67 s, 4/4 again at 68.33 s).

**The arrangement and the app**
- Real waveforms from take peaks (the `.sessionpeaks` / `.reapeaks` cache),
  drawn normalized per take (display only; gain capped at about +27.6 dB).
- One arrangement painter: the widget. The vello window, `frame.rs`, the
  WRY window, the synthetic Blitz experiments and the unused
  `daw_ui::studio` parts are gone; `bench` and `blitz_shot` draw through the
  widget.
- A compact track panel in the docked (Overview) arrangement.
  `Viewport::panel_w` is the one number everything placed against time reads.
- Follow playhead (a toolbar toggle, on by default): the view pages, it
  does not slide.
- Space always plays/stops: the **window** reads the transport keys
  (`keys::use_window_transport_keys`), the widget leaves play/stop to it,
  and a rename or the chart editor holding the keyboard makes it stand aside.
- The window opens maximized. Crashes fixed: a `peek()` borrow held across a
  `set` in follow; Blitz panicking on a drag whose pressed node was removed.

### Not verified by an agent — the user should check

Anything needing a click or a key in the live window: typing in the chart
editor, the Organize toolbar with and without a time selection, switching
tabs with audio, Space after clicking the progress bar, the chart's
middle-drag and double-click. The engine-side behaviour under each has tests.

### Known issues and open threads

- **Asked for next:** DAW → `.kf` — regenerate the chart text from the
  session when the arrangement is edited (two-way sync; the kf → DAW half is
  live). Nothing exists for it yet. Until it does, Organize-toolbar inserts
  do not reach the `.kf` text either.
- **Left and right rail toolbars** in the arrangement — only the top one is
  back, in Organize. The old rails in `rails.rs` were the vello window's
  scene/phase/mode switchers, which is not what the user means; ask.
- Switching songs reopens the audio device (a gap). A seamless handover
  needs daw-standalone to swap the project under one stream.
- The browser demo (`session-daw-web`) opens one song: no setlist, and it
  does not fetch `Media/Peaks/*.sessionpeaks` (its items draw plain).
- `golden_scenes` fails: 8 mixer scenes drifted before this work, not ours.
  `just daw-scenes` refreshes them once the user has looked at them.
- Tab titles are clipped mid-glyph (Blitz has no `text-overflow: ellipsis`).
- `~/.config/fts/guide-samples/Counts/English Female - 8.wav` is missing;
  every prepare warns about it.
- `chart_to_layout` counts bpm in quarter notes (a bar of 6/8 is 3 quarters).
- Worktree `/Volumes/build-disk/development/session-onepath` (branch
  `one-arrangement-painter`) is fully cherry-picked into `macos-compat` and
  can be removed (`git worktree remove`); it shares `target/`.

## 4. What is next, in the order the user asked

1. Confirm the live window works (the "not verified" list): fix what the
   user reports, starting from `~/Library/Logs/Session Dev/session-dev.log`.
2. DAW → `.kf` generation and two-way sync. In the user's words: "the
   current DAW state should always be reflected as a .kf file text that we
   can live edit and vice versa" — toolbar inserts and arrangement edits
   must reach the text.
3. The left and right rail toolbars for the arrangement.
4. A section/measure progress bar under the main progress bar.
5. The web demo: setlist, peaks from the share link, then replace `/demo`
   in `apps/web` and deploy — only after the user says the demo is right.
6. The iPhone app.

## 5. Repos, branches, and what must be pushed first

| repo | branch | local-only work |
|---|---|---|
| `session` | `macos-compat` | ~60 commits past `origin/main` |
| `daw` | `tag-section` | `.session` bridge, sessionpeaks, anchor_media, the marker-GUID fix |
| `keyflow` | `tag-section` | Tag, `page_number_at_time`, editor-state without fences, **the `!T` parser fix** |
| `editor` | `main` (3 ahead of origin) | test CSS path, typst/mermaid as features, `EDITOR_CSS` |
| `blitz` | `session-stale-mousedown` (off `db4318a9`) | the drag panic fix |
| `task` | `session-share-cli` | share links (the morning's; untouched since) |

Session's `Cargo.toml` `[patch]` tables point at sibling paths for `daw` (as
before), `keyflow` (engraver-proto, editor-keyflow-lang, **keyflow,
keyflow-text**), `editor` (editor-view, -state, -vim, -syntax) and
**`blitz-*`**. Before session is pushed: push those branches or tags, move
the git pins (the blitz `rev`, the keyflow and editor tags) and drop the
path entries.

## 6. Traps learned the hard way

- **Never run cargo inside `nix develop`** — it recompiles the world.
- A `.peek()` in an `if let`/`match` scrutinee holds its borrow for the
  whole block; a `set` inside it panics "already borrowed".
- `open --stdout` onto the external `/Volumes/build-disk` fails
  (LaunchServices -10810), and so does launching a bundle whose binary was
  overwritten in place (rm, then cp).
- `git add justfile` matches nothing: the file is `Justfile` (the volume is
  case-insensitive, git is not).
- A relative folder handed to `anchor_media` anchored nothing (it is
  absolutized now).
- Guide generation writes ONE item per role over the whole song; do not
  "clear past the song" — that deletes the new item.
- keyflow: `!T2/4` on a section *header* line is not a meter change; it goes
  on its own line or before a chord.
- The in-app browser pane has no audio device; audio is checked by ear.
- This box's load is often 50–150 from other agents: compare CPU time, not
  wall time, when benchmarking.

## 7. Files worth reading first

| what | where |
|---|---|
| open, the engine, the audio slot, switching songs | `apps/session-daw/src/open.rs` |
| prepare, apply_chart, chart_beside | `apps/session-daw/src/prepare.rs` |
| StudioSession, the prepare-once policy, resync | `apps/session-daw/src/studio.rs` |
| the setlist, colours, reading a `.setlist` | `apps/session-daw/src/setlist.rs` |
| the tabs, the top bar | `apps/session-daw/src/shell.rs` |
| the desktop shell, the Organize layout, live advance | `apps/desktop/src/native/shell.rs`, `mod.rs` |
| the chart editor (live) | `apps/session-daw/src/chart_editor.rs` |
| the Organize toolbar | `apps/session-daw/src/organize.rs` |
| the chart panel (paged, live chart, pan/zoom) | `apps/session-daw/src/chart_panel.rs` |
| rebuild_from_chart | `crates/session/session/src/keyflow/from_chart.rs` |
| chart → timeline (meters) | `crates/session/session/src/setlist/chart_import.rs` |
| the window's transport keys | `apps/session-daw/src/keys.rs` |
| `.session` save and load | `daw/features/standalone/daw-standalone/src/session_file.rs` |
| the peaks cache | `daw/features/dawfile/dawfile-reaper/src/sessionpeaks.rs` |

## 8. The browser demo and Task (unchanged since the morning)

```bash
# Task's local servers (ACME on :18080) — plants the demo world first.
cd task && just demo serve           # background it; logs to the scratchpad

# The CLI, signed in as a demo owner (its own session dir keeps your real one clean)
export XDG_DATA_HOME=/tmp/task-cli TASK_PASSWORD=correct-horse-battery-staple
task/target/debug/task auth login --server ws://127.0.0.1:18080/vox \
    --org acme-audio --email alice@acme.test

# A session into Task, and a public link to it
task files root ensure session/<slug> --name "<Song>"
task files put <root-id> "<session>/<Song>.RPP"
task files put <root-id> "<session>/Media" --to Media
task files checkpoint <root-id> --message "…"
task share folder <root-id> --label demo --documents

# Proxies and the guide library (Ogg)
cd session
cargo run -p session-cli -- proxies "../sessions/imported/<Song>"
cargo run -p session-cli -- guide-library ~/.config/fts/guide-samples /tmp/guide-ogg

# Import multitracks (already done for the seven)
cargo run -p session-cli -- import ../sessions/multitracks --out ../sessions/imported

# The web bundle (release + wasm-opt, ~2 min), and a static server on :8765
just web-daw                      # optionally: SESSION RPP CHART to bake a session in
just web-daw-serve
```

The demo page takes its session from the query:

```
http://localhost:8765/?share=<link>&project=<Song>.RPP&chart=<Song>.kf&guide=<guide link>
```

Both links were minted against the local Task server during this work
(`acme-audio`, tokens `66db5c05…` for the session, `b2c6d8e8…` for the
guide library). Re-mint after a fresh `just demo fresh`.
