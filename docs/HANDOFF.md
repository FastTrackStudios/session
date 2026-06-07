# FTS Extensions — Handoff (2026-06-06)

State of the world after the modes/phases/visibility push. Everything below
is **installed locally** (`just install-release` → `~/.fts-dev/UserPlugins/`)
and **pushed to Codeberg** on the branches listed at the bottom.

## What exists now

### Modes — every session mode has a visibility rule set

| mode | TCP | MCP |
|---|---|---|
| **Organize** | default-show: loose/unsorted tracks (the cargo) + all sources with parents, folders open | mirrors TCP |
| **Write** | collapsed playground: one row per group, `* MIDI` driver rows out in the open, scratch tracks visible | strips collapsed |
| **Produce** | Write's floor + **focus bubble**: the selected track's top-level group blooms fully open *including plumbing* (Parallel/FX/verb = sound-design surface) | focused group expanded, rest collapsed |
| **Record** | ALL sources + their parent folders; no SUM rows, no Sub/Fund/Verb/Trig/utility | same set, strips collapsed |
| **Edit** | one source per instrument: Kick In, Snare Top, T1–T4, Hi Hat, Ride, OH, Rooms Close | one collapsed strip per instrument |
| **Mix** | phase-driven (below) | phase-driven |
| **Master** | top-level group rows only, collapsed | group strips + **expanded MIX BUS chain** |
| **Live** | guide/click + playable `* MIDI` rows + prints, nothing else | same |
| **Video** | top-level reference rows + video/print/stem tracks | same |
| **Minimal** | bare top-level rows | same |

### Mix phases (Mode → Phase → Step; `docs/mixing-phases.md`)

`STAGING → RESCUE → BALANCE → TONE → POLISH → MIX → DEPTH → AUTOMATE → OVERVIEW`

- Actions `FTS_SESSION_MIX_PHASE_*` (toggleable — radio toolbar states),
  keys `p s/r/b/t/p/m/d/a/o` in the Mix-mode workflow overlay.
- Active phase per mode is **per-project** (`FTS_MODES/phase.<mode>` project
  ext state), restored at startup, on mode switch, and on project-tab switch
  (event-hub `ProjectEvent::CurrentChanged` subscription).
- AUTOMATE has no rules yet (needs envelope-lane control).

### Recording markers + create variants

- Create actions resolve a **variant**: plain = Primary, **Alt = Secondary**,
  **Ctrl = popup menu** (SWELL, same pattern as the mode selector).
  Configured: Drums (MIDI Kit primary / Recorded Kit / Stereo MIDI Kit),
  Bass (DI / MIDI Bass / DI+Amp), E-Guitar (DI / MIDI / Axe FX / Full Rig).
- The chosen variant writes `P_EXT:FTS_RECORDING` (`audio`/`midi`) +
  `P_EXT:FTS_VARIANT` on the group's version root. Lives in the .RPP.
- **MIDI-marked groups reduce to their `* MIDI` driver row in the TCP in
  every mode/phase**; MCP density varies per phase. At insert, MIDI variants
  skip TCP folding (a collapsed ancestor would swallow the row) and hide
  everything else; instrument folders fold in the MCP via BUSCOMP.

### Drum kit shape (create action `N d`)

MIDI[DRUM MIDI, DRUM MIDI SEND] · Kick[SUM[In,Out], Sub] ·
Snare[SUM[Top,Bottom], S Fund, Snare Verb[Short,Long]] ·
Toms[T1–T4, T1–T4 Trig, T Fund[T1–T4 Fund]] ·
Cymbals[Hi Hat, Ride, OH] · Rooms[Close, Far] · FX[Drum Verb] ·
Parallel[Clean,Tight,Punch,Smash,Crush]

- OH = single stereo track; Rooms = Close/Far pairs; **Kick Sub is utility**
  (fund-like), not a source.
- Parallel children auto-register as a **volume-balancer** link group.

### Visibility engine (`dynamic-template/src/visibility_rules.rs`)

Pure resolver: `(tracks, config, ModeVisibility) → Vec<TrackPlan>` with
per-surface show/hide + fold. Selector dimensions: band / instrument / role
(Bus/Leaf) / rank (TopmostPerInstrument) / `classified` / `name_contains` /
`name_is` (exact) / `name_ends_with` / `under` (ancestor contains) /
`recording` / `focused` (selection bubble) / `top_level`.

Engine smarts worth knowing:
- **Ancestor-qualified classification**: template leaves are named bare
  (`In`, `Top`, `Close`); when the bare name doesn't classify, the engine
  tries `"<ancestor> <name>"` up the chain (`SUM In` → `Kick In` ✓).
- **Mic-stem grouping**: TopmostPerInstrument keys on the classified name
  minus trailing mic tokens (`in/out/sub/top/bottom/l/r/close/far/…`) — so
  Kick In/Out/Sub collapse to one while T1–T4 stay distinct.
- **Container band adoption**: folders whose own name doesn't classify
  (`Guitars`, `Synths`) adopt the band of their classifying descendants.

### The strict test harness

`dynamic-template/tests/visibility_matrix.rs` — 19 tests. Builds a maximal
demo session as canonical `daw_proto::Track` state (drum kit, bass DI+Amp,
electric Full Rig, acoustic, two MIDI mockups, vocals/BGVs, bus skeleton,
guide/click/video) through the SAME `inputs_from_proto_tracks` conversion
production uses, and asserts the **exact visible set per surface per
mode/phase**. Change a rule, a table names the track that moved.
The workflow with Cody: he names the step + track that's wrong → flip the
expectation table → make the rules match.

`FTS: Demo: Insert Full Demo` (`FTS_DYNAMIC_TEMPLATE_INSERT_FULL_DEMO`)
builds this session in a live project through the real create machinery.

### Bus maintenance

Every create ensures its family's bus chain on demand (MIX BUS pinned to
the bottom of the track list, gray, collapsed; DRUM/BASS/EG/AG/LEAD BUS,
LEAD VOCAL BUS, BGVs BUS). `N m` backfills buses for existing groups.

### Volume balancer (`fts-extensions/src/volume_balancer.rs`)

Native port of the old ReaScript: constant-sum fader link groups in project
ext state (`FTS_VOL_BALANCER`: `groups` index + `group.<name>` GUID lists),
polled at 30Hz. Keeps the moved fader where the user put it. Actions:
`FTS_VOLBAL_TOGGLE` / `LINK_SELECTED` / `UNLINK_SELECTED`.

### daw API additions (pinned daw workspace, `daw_reaper::track`)

- `set_folder_compact_on_main_thread(guid, arrange, mixer)` — arrange via
  `I_FOLDERCOMPACT`, mixer via surgical `BUSCOMP` chunk-line edit.
- `set_track_info_value_on_main_thread(guid, key, value)` — raw attr escape
  hatch (`I_SPACER`, `I_CUSTOMCOLOR`, `B_SHOWINTCP`, …).
- `get/set_track_ext_state_on_main_thread(guid, key, value)` — `P_EXT:`.

## Build/pin topology (IMPORTANT)

`fts-extensions` builds git deps with TEMPORARY pinned clones under
`~/Development/FastTrackStudio/.pinned/`, wired via `[patch]` in the root
Cargo.toml: **dynamic-template**, **session**, **daw** (whole workspace —
17 crates patch together because daw-reaper's siblings are path deps; note
`xtask/Cargo.toml` also points at the pin or the lockfile collides).
Pins drop when the changes land on each repo's main. daw lineages have
diverged: codeberg daw HEAD already contains the folder-compact primitives
(features/ layout), starcommand main (crates/ layout) does not — reconcile
before dropping that pin.

Dev loop: `just install-release`, restart REAPER, check the
`=== FTS BUILD MARKER: <slug> ===` line in
`~/.local/state/fasttrackstudio/reaper-fts-extensions.log.<date>`.

## Audio stack (NixOS, `~/.flake` — canonical config repo)

- **Dante/Inferno gating**: the whole AoIP stack (statime PTP, inferno
  nodes, routing links, clock-ready) sits behind `dante.target`. CLI:
  `dante on|off|status`. Default OFF — a clockless Inferno was wedging
  PipeWire's loop and hanging REAPER's JACK calls on launch.
- **Inferno nodes** run as a standalone `pipewire -c` client service
  (`inferno-nodes.service`), not static conf — node creation is gated too.
- **Yamaha TF** pinned to `audio.channels = 34` (64-ch probe storms were
  stretching the device-churn window for a pipewire 1.6 lock race).
- **pipewire-watchdog.timer** (10s): probes the core, two misses → restarts
  the stack; also rescues clients stuck in jack `do_sync`.
- Gotcha: `~/Development/nix-fleet-pr-forgejo-lan-agents` is an OLD clone;
  edits there get reverted by the next `just switch` from `~/.flake`.

## Repos / branches (Codeberg, FastTrackStudios org)

| repo | branch | contents |
|---|---|---|
| dynamic-template | `modal-visibility` | visibility engine, modes/phases, variants, creates, matrix tests, demo action |
| session | `feat/mix-phases-toggle-actions` | phase actions (toggleable), percussion/mix-bus creates |
| daw | `feat/folder-compact-primitives` | BUSCOMP + P_EXT + raw-attr track primitives |
| fts-extensions | `feat/setlist-record-groups` | volume balancer, time sigs, daw pin wiring, docs |
| input | `feat/midi-editor-keybinds` | `p`-prefix phase keys, `N m`/`N c` menu entries |

`~/.flake` (Dante gating, TF pin, watchdog) pushes to
`git.starcommand.live/codywright/nix-fleet` — not Codeberg.

## Open threads

1. **AUTOMATE phase** — needs envelope-lane visibility primitives.
2. **Per-step track heights** (Edit giants vs context minis) — engine has no
   height dimension yet; `set_tcp_height_on_main_thread` exists.
3. **Styx config** for visibility rules + create variants (currently Rust
   consts; engine is config-shaped, the swap is a deserialize).
4. **MIDI variants for Keys/Synth** so the demo action midi-marks them like
   the matrix fixture does.
5. **Produce focus refresh** — bubble computed at apply time; Cody chose
   visibility-manager actions over auto-follow/refocus key.
6. **capn git hooks broken** in fts-extensions + input (missing binary at
   `~/Development/capn/target/debug/capn`) — commits need `--no-verify`.
7. **home-manager** fails activation over `~/.local/bin/claude` clobber
   (set `backupFileExtension` or force).
8. **Track routing** for created kits (sends to SUM/buses, record inputs) —
   golden template carries the data, nothing wires it yet.
9. **Upstreaming the pins** (see Build/pin topology).
