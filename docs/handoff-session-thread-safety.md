# Handoff — session crate thread-safety / convention audit

**Status:** unfixed; deferred from the BUILD_SETLIST / LOAD_DEMO_SETLIST
refactor pass (commits `7ecd334` in `session`, `087c9a2` in `daw`).

**Context.** Two anti-patterns surfaced repeatedly while wiring the
setlist action chain:

1. **Async RPC handler touches REAPER main-thread-only FFI without
   bouncing via `daw_reaper::main_thread::query`.** Looks like
   `async fn` calling `self.daw.list()` / `current()` / `set_position()`
   directly. From a tokio worker, REAPER's `EnumProjects` / similar
   APIs hang on REAPER's internal lock. We saw this kill
   `SetlistService::build_from_open_projects`.

2. **Fire-and-forget `moire::task::spawn` / `tokio::spawn` from a
   REAPER timer or action callback.** Timer callbacks fire on the
   main thread with no Tokio runtime context — the spawn panics with
   `there is no reactor running, must be called from the context of a
   Tokio 1.x runtime`. We saw this kill the first version of
   `setlist_actions::dispatch(LoadDemo)`.

Both were fixed in the setlist / demo paths by rewriting them to be
synchronous and run inline on the REAPER main thread (the same
pattern `keyflow_actions::dispatch` already uses). The audit below
flags every other call site in the session crate that still has one
of these smells. Listed in priority order — top items will panic /
hang the first time they're exercised; bottom items are technical
debt.

---

## 1. `auto_color_actions.rs:150` — spawn from timer callback

```rust
fn ensure_reactive_updates() -> eyre::Result<()> {
    …
    let mut rx = daw::reaper::event_hub().subscribe_tracks();

    moire::task::spawn(async move {                       // ← panics
        loop { match rx.recv().await { … } }
    });
    …
}
```

`ensure_reactive_updates` is reached from `auto_color_timer`
(`register_timer` → `daw::register_timer(auto_color_timer)` at line
207). Same panic mode that bit `setlist_actions::dispatch(LoadDemo)`
in commit `e6883c6`.

**Fix.** The spawn is one-shot (it's gated by the `subscribed`
`AtomicBool::swap`), so the work doesn't need to live in the timer.
Move it to a point that already runs in Tokio context — either
hoist the subscription into module init, or do the
`daw::block_on(async move { tokio::spawn(…); })` pattern we use in
`setlist_actions::dispatch(LoadDemo)`'s `daw::block_on` shim
(see commit `0d28e37c` for the shape — that one ended up sync but the
block_on-then-spawn shim is the right escape hatch for cases that
genuinely need a tokio task).

---

## 2. `setlist_service/{polling, navigation, combined}.rs` — async paths call sync REAPER FFI

These are reachable through the live `SetlistService` RPC surface
that the CLI / TUI already exposes. The build path was the loudest
symptom but the same root cause is live across navigation, polling,
and combined-setlist generation.

| File | Lines | Surface |
|---|---|---|
| `setlist_service/polling.rs` | 273, 521, 937 | `calculate_active_indices`, polling loop, `subscribe_active` |
| `setlist_service/navigation.rs` | 19, 116, 184, 433, 558 | `current`, `set_position` from `next_song`/`previous_song`/`seek_to` |
| `setlist_service/combined.rs` | 63, 184, 210, 246, 296 | `daw.list()` + `daw.set_project()` from `generate_combined_setlist` |

Each direct `self.daw.list()` / `daw.current()` / `daw.set_position()`
in an `async fn` needs to be wrapped in
`daw_reaper::main_thread::query(move || { … })` (await the future,
treat `None` as "main thread not initialised"). For sequences of
calls that all need to run together, batch them inside one query so
the whole chunk pays one main-thread bounce instead of N.

**Sketch:**
```rust
// before
let projects = self.daw.list();
let current = self.daw.current().map(|p| p.guid);
let next = compute_next(&projects, current);
self.daw.set_project(next).ok();

// after
let daw = self.daw.clone();
let next = daw_reaper::main_thread::query(move || {
    let projects = daw.list();
    let current = daw.current().map(|p| p.guid);
    compute_next(&projects, current)
})
.await
.ok_or_else(|| SessionServiceError::Internal("main thread unavailable".into()))?;

let daw = self.daw.clone();
daw_reaper::main_thread::query(move || daw.set_project(next))
    .await
    .ok_or_else(|| SessionServiceError::Internal("main thread unavailable".into()))?;
```

We did exactly this in `setlist_service/build.rs` (commit `e627fb8`)
for the build path; mirror that shape in each of the three files.

**Test coverage.** The current `fts -i` TUI doesn't yet exercise these
navigation RPCs, so the hang isn't visible day-to-day. Once the song-
nav keybindings (`[` / `]` for prev/next song) land, every press
will hit this code path and stall the TUI until the RPC times out.
Fix before wiring those keys, or they'll feel broken.

---

## 3. `preroll_actions.rs:95, 135` — direct project-config key access

Bare access to `PROJECT_PRE_ROLL_MEASURES_KEY` via
`set_project_info` / `get_project_info_string` instead of a typed
helper. Not broken — REAPER accepts the key. Just fragile if anyone
renames the key without grepping for the literal string.

**Fix.** Wrap in a small `preroll::measures()` /
`preroll::set_measures(n)` getter/setter that's the one place the
literal lives. Nice-to-have; not on a critical path.

---

## Reference commits

- `e627fb8` (session) — `build_from_open_projects_impl` main-thread fix
- `0d28e37c` / `e6883c6` (FastTrackStudio / session) — `LoadDemo` sync rewrite
- `7ecd334` (session) — lane index 0-based + SONG/SECTIONS/MARKS layout
- `087c9a2` (daw) — `RULER_LANE_NAME` auto-creates lane, no
  `RULER_LANE_ORDER` pre-extend
- `f6869e4` (session) — demo chains existing keyflow actions (the
  "use the existing primitive" lesson the rewrites below should
  follow)

## CLI primitives you'll want while debugging

```bash
# Trigger any registered REAPER named command
fts session action FTS_SESSION_LOAD_DEMO_SETLIST

# Inspect what landed where (markers, regions, lane names, flags, hidden)
fts session action FTS_SESSION_DUMP_RULER_STATE
tail ~/.local/state/fasttrackstudio/reaper-fts-extensions.log.*

# Save current REAPER project state so the .RPP can be grep'd directly
fts session action 40026   # REAPER's File: Save Project

# Direct project-info pokes (used while diagnosing lane numbering)
fts session rename-lane <index> <name>
fts session set-project-info RULER_LANE_FLAGS:1 8
```

The "open REAPER on a known .rpp path, then `fts session action 40026`
to save it back" loop is what finally pinned down the
`I_LANENUMBER == name_key_index` (both 0-based) reality after several
wrong guesses.
