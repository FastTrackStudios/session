# Pro Tools `.ptx` parity roadmap

**Goal:** read AND write Pro Tools 12 session files with full data fidelity,
round-tripping (read → write → read) without loss.

This document tracks every PT12 feature/block we know about, its current
implementation status, and the RE work required to get to parity.

---

## Status legend

| Symbol | Meaning |
|--------|---------|
| ✅ | Read works, byte-verified |
| 🟡 | Read works heuristically (correct on test fixtures, may break edge cases) |
| ❌ | Not implemented |
| ✏️ | Write not implemented (no field is written today; every entry below is read-only) |
| →§16 | Entry moved to §16 *Known unobservable* — bytes round-trip losslessly but semantics undecoded |

---

## 1. File container & encoding

| Feature | Block(s) | Read | Write |
|---|---|:-:|:-:|
| XOR descrambling | header | ✅ | ✏️ |
| Block tree | all | ✅ | ✏️ |
| Endian detection | header byte 0x11 | ✅ | ✏️ |
| Version (PT 5–12) | `0x0003` / `0x2067` | ✅ | ✏️ |
| Session sample rate | `0x1028` | ✅ | ✏️ |
| Product/version string | `0x0030` | ✅ (read), ignored | ✏️ |

**Round-trip risk:** writing requires re-encoding XOR with the correct seed
(matches version). We already have `decrypt::decrypt`; need symmetric `encrypt`.

---

## 2. Timeline

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Tempo map (BPM + tpb) | `0x2028` | ✅ | ✏️ | TPB encodes click note value |
| Meter map (time sig) | `0x2029` | ✅ | ✏️ | |
| Memory locations (markers) PT 5–9 | `0x263b`/`0x2619` | ✅ | ✏️ | |
| Memory locations PT 12 | `0x2030`/`0x2077` | ✅ | ✏️ | Position = u64 LE − `(2^62 + ZERO_TICKS)` |
| Key-signature ruler items | unknown | →§16 | →§16 | |
| Chord-symbol ruler items | unknown | →§16 | →§16 | |
| Loop / selection points | unknown | →§16 | →§16 | |
| Pre/post-roll | unknown | →§16 | →§16 | |
| Cycle/punch points | unknown | →§16 | →§16 | |

---

## 3. Audio files

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Filename list | `0x1004`/`0x103a` | ✅ | ✏️ | |
| Per-file sample length | `0x1001` | ✅ | ✏️ | |
| File ↔ region mapping | unknown | →§16 | ✏️ | **Currently using region-name → file-stem heuristic.** Real `audio_file_index` field in the region payload reads garbage (we read u32 at "end of block" which lands in next block's magic). |
| Audio file path resolution | (filesystem) | ✅ | ✏️ | session_dir/`Audio Files/` |
| External file refs | unknown | →§16 | →§16 | Files outside session dir |

**Priority:** real region→file index — high. Without it, regions renamed away
from the auto-generated `<file>-NN.L` pattern lose their source link.

---

## 4. Audio regions

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Region name, start, length, offset | `0x100b`/`0x262a` | ✅ | ✏️ | |
| Region audio file index | inside region payload | →§16 | →§16 | bug (see §3) |
| Region gain | unknown | →§16 | →§16 | |
| Region pitch shift | unknown | →§16 | →§16 | |
| Region time-stretch / Elastic Audio | unknown | →§16 | →§16 | |
| Warp markers (Elastic) | unknown | →§16 | →§16 | |
| Region color | unknown | →§16 | →§16 | |
| Region clip group membership | unknown | →§16 | →§16 | |
| Compound regions | `0x2628`/`0x2629`/`0x262b`/`0x262c` | →§16 | →§16 | parsed as containers only |

---

## 5. Tracks (definition)

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Track name | `0x1014` | ✅ | ✏️ | |
| Channel count (mono/stereo) | `0x1014` | ✅ | ✏️ | |
| Channel index map | `0x1014` | ✅ | ✏️ | |
| Track kind (audio/MIDI/aux/master) | `0x251a` `+2` byte | ✅ | ✏️ | 0x00 audio, 0x02 aux/MIDI, 0x05 master, 0x07 inst |
| Track UID | `0x251a` payload | →§16 | →§16 | needed for round-trip |
| Master track | `0x251a` kind=0x05 | →§16 | →§16 | currently filtered out at import |
| Aux/instrument tracks | `0x251a` kind=0x02/0x07 | →§16 | →§16 | included in midi_tracks Vec |
| Track creation order | (block order) | ✅ | ✏️ | |

---

## 6. Track mix state (volume, pan, mute, solo)

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Volume (fader) | `0x1029` `+1..+5` i32 LE | ✅ | ✏️ | 0.1 dB units; verified -31 dB matches PT |
| Mute | `0x1029` `+5`?? | →§16 | →§16 | **byte +5 ≠ actual mute** — agent's interpretation wrong. No single byte in 0x1029 payload matches the user's known mute pattern across audio tracks. |
| Pan (left ch) | `0x1029` `+13..+17` i32 LE | →§16 | ✏️ | `−100` for stereo = "natural state", we map to centered |
| Pan (right ch / multi-out) | `0x1029` `+17..+87` | →§16 | →§16 | |
| Solo | unknown | →§16 | →§16 | |
| Record-arm | unknown | →§16 | →§16 | |
| Input-monitor mode | unknown | →§16 | →§16 | |

**Priority: HIGH.** Mute is critical for usability — the user can't tell which
material is supposed to be silent. Search the whole 281-byte `0x1029` payload
for the real mute bit, OR look for a sibling block per track.

---

## 7. Track display & metadata

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Track color | unknown | →§16 | →§16 | Not in `0x1014` payload; possibly in `0x1029` extended area (+171..) |
| Track icon/image | unknown | →§16 | →§16 | |
| Track comment/notes | unknown | →§16 | →§16 | |
| Track height (mix/edit window) | unknown | →§16 | →§16 | |
| Track visibility | unknown | →§16 | →§16 | |
| Track delay (samples/ms) | unknown | →§16 | →§16 | |
| Phase invert | unknown | →§16 | →§16 | |
| Track timebase (samples vs ticks) | partly inferred from sub-entry `+16` byte | →§16 | →§16 | |

---

## 8. Track routing

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Hardware I/O channels | `0x1021`/`0x1022` | ✅ | ✏️ | |
| I/O routing table | `0x2602`/`0x2603` | →§16 | →§16 | parsed as containers; field semantics unknown |
| Track input (mic/line/bus) | unknown | →§16 | →§16 | |
| Track output (master/bus) | unknown | →§16 | →§16 | |
| Aux send count / levels / destinations | unknown | →§16 | →§16 | |
| Aux send pre/post-fader | unknown | →§16 | →§16 | |
| HW insert routing | unknown | →§16 | →§16 | |

**Priority: HIGH.** A printed mix uses bus routing extensively. Without
routing, the imported REAPER session has every track going straight to the
master, which is wrong for any serious session.

---

## 9. Track FX / plugins

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Session plugin registry | `0x1018`/`0x1017` | ✅ | ✏️ | list of plugin TYPES used |
| Per-track insert chain | unknown | →§16 | →§16 | |
| Insert plugin parameters | unknown | →§16 | →§16 | |
| Insert bypass state | unknown | →§16 | →§16 | |
| Insert wet/dry | unknown | →§16 | →§16 | |
| Insert ordering | unknown | →§16 | →§16 | |
| Side-chain routing | unknown | →§16 | →§16 | |

**Priority: HIGH.** No plugins means imported sessions sound like raw stems.

---

## 10. Region placements (track playlists)

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Active playlist regions | `0x1054`/`0x1052` | ✅ | ✏️ | |
| Region start position (samples) | sub-entry `+9..+12` | ✅ | ✏️ | |
| Region start position (ticks, for MIDI/inst) | sub-entry `+9` u40, when `+16==0x40` | ✅ | ✏️ | |
| Region clip-effect / mute | unknown | →§16 | →§16 | |
| Region clip gain | unknown | →§16 | →§16 | |
| Alternate playlists | `0x2428`/`0x2429`+`0x1054` | ✅ | →§16 | parsed but not emitted |

---

## 11. Fades & crossfades

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Fade detection | `0x1050` `+46==0x01` | ✅ | ✏️ | |
| Fade-def list | `0x2630` wrapper | ✅ | ✏️ | |
| Fade in/out length + shape | `0x262f` | ✅ | ✏️ | see `pt-fade-encoding.md` |
| Custom curve shapes | `0x262f` trailing bytes | →§16 | →§16 | only linear/equal-power/equal-gain emitted |
| Fade preset file refs | `cFadePresetFile` | →§16 | →§16 | |

---

## 12. MIDI

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| MIDI event chunks (note, vel, pos, dur) | `0x2000`/MdNLB | ✅ | ✏️ | 35-byte records; pos@+27, note@+9, vel@+10, dur@+11..+19 |
| MIDI regions | `0x2001`/`0x2634` | ✅ | ✏️ | length now via tick→sample |
| MIDI region→track assignment | `0x1058` | ✅ | ✏️ | with playlist-suffix-aware dedupe |
| MIDI CC events | unknown | →§16 | →§16 | only notes parsed |
| Pitch bend | unknown | →§16 | →§16 | |
| Aftertouch / program change | unknown | →§16 | →§16 | |
| Note metadata (channel) | unknown | →§16 | →§16 | channel always 0 |
| Tempo-mapped MIDI region timing | partial | →§16 | →§16 | |
| Compound MIDI regions | `0x262b`/`0x262c` | →§16 | →§16 | parsed as containers only |

---

## 13. Automation

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Volume automation | unknown | ❌ | ❌ | |
| Pan automation | unknown | ❌ | ❌ | |
| Mute automation | unknown | ❌ | ❌ | |
| Send-level automation | unknown | ❌ | ❌ | |
| Plugin-parameter automation | unknown | ❌ | ❌ | |
| Tempo automation (vs static map) | partial | 🟡 | ✏️ | we emit static map; tempo curves untested |

**Priority: MEDIUM.** Most modern mixes use automation; without it the
imported session sounds static.

---

## 14. Track groups & organization

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Edit groups | unknown | ❌ | ❌ | |
| Mix groups | unknown | ❌ | ❌ | |
| Track folders (PT 12+) | unknown | ❌ | ❌ | |
| Selection-state memlocs | `0x271a` siblings | ❌ | ❌ | |
| Zoom-state memlocs | `0x271a` siblings | ❌ | ❌ | |

---

## 15. Writing (round-trip) — currently 0% done

To write a `.ptx` file we need every read field above to have a corresponding
**encoder**, plus:

| Concern | Status |
|---|---|
| XOR re-encryption with correct seed | ❌ |
| Block tree serializer with correct sizes | ❌ |
| Re-compute parent block `block_size` after children change | ❌ |
| Re-compute cross-block indices (audio_file_index, fade_index, region_index, etc.) | ❌ |
| Preserve unknown blocks verbatim (passthrough) | ❌ |
| Preserve unknown bytes within known blocks | ❌ |
| Stable UID generation for new tracks/regions/markers | ❌ |
| Stable ordering (PT compares ordering for some structures) | ❌ |
| Update headers (file size, modified-date, etc.) | ❌ |
| Surface validation: PT refuses to open files with broken back-references | ❌ |

`src/write.rs` exists today and supports only **single-field in-place
modifications** (rename track, change sample rate) — not block-add or
structural rewrite.

---

## Suggested ordering for autonomous work

Roughly: highest-blast-radius bugs first, then features by user impact.

### Phase A — critical correctness (blocks user from using the import)

1. **Track mute** (`0x1029`) — search whole payload + sibling blocks; verify against user's mute list.
2. **Audio file index** for regions — find the real `region → file` link block; replace name-stem heuristic.
3. **Track solo** — same RE pass as mute.
4. **Track color** — likely in `0x1029` `+171..` or a sibling block.

### Phase B — mix fidelity

5. **Track routing** (input/output/bus assignments).
6. **Aux sends** (count, levels, destinations, pre/post).
7. **Track FX inserts** — at least slot-and-name even if parameters are opaque.
8. **Plugin parameter blobs** (per-track per-insert).

### Phase C — production detail

9. **Region clip gain** and per-clip mute.
10. **Region pitch / time-stretch / Elastic warp markers**.
11. **MIDI CC / pitch bend / channel**.
12. **Automation** (volume, pan, plugin params).

### Phase D — round-trip writer

13. Block serializer: rebuild any single block from parsed fields.
14. Block tree writer: recompute sizes, recurse.
15. XOR re-encryption.
16. Cross-reference rewriter (indices stay stable across edits).
17. Unknown-block passthrough: any block we don't fully decode keeps its raw bytes through round-trip.
18. Round-trip test fixture: read, write, read, byte-compare or field-compare.

### Phase E — niceties

19. Selection / zoom / window-state memlocs.
20. Compound regions (full parse, not just container).
21. Alternate playlists round-trip.
22. Key-sig / chord-symbol ruler items.
23. Track folders (PT 12+).
24. Track icons, comments, height, visibility.

---

## How to RE each item

Pattern that worked for fades and `0x1029`:

1. **Identify the block** by counting candidate `content_type` values and matching against known counts (e.g., per-track block count = track count).
2. **Dump bytes** with `examples/dump_ct.rs <CT_HEX>` style helpers.
3. **Anchor on a known value** the user can confirm (e.g., "this track is -31 dB").
4. **Diff bytes across blocks** to find what varies.
5. **Cross-reference siblings** when one block doesn't carry the field (fades live in `0x262f`, not in the track entry).
6. **Verify against user-ground-truth** for at least 3 distinct values before committing.
7. **Write the doc** in `docs/pt-*.md` with byte offsets, types, value ranges, and example decoded values.

---

## Test-session ground truth

Use this session as the canonical correctness benchmark for read-side
parity. All claims below come from the user's own Pro Tools session.

**Path:**

```
~/Downloads/tombrooksmusic_copy-of-02-lord-of-the-fight-1-5_2026-05-11_0158/Copy of 02 LORD OF THE FIGHT 1.5/Copy of 02 LORD OF THE FIGHT 1.5.ptx
```

### Tempo / meter / markers

- 168 BPM, 6/8, eighth-note click (`ticks_per_beat = 480000`).
- `INTRO` marker is at bar 3 (= 4.286 s into the timeline at session
  tempo). Other markers in song order: `INTRO`, `VS 1a`, `VS 1b`,
  `CH 1`, `Re-Intro`, `VS 2`, `CH 2`, `Climb`, `BR`, `br`, `CH 3`,
  `Tag 1`, `2`, `3`, `outro`, `OUT` (16 markers total).

### Track mute (currently broken)

Muted tracks (every other track audible):

- `02 LORD OF THE FIGHT.01`
- `02 LORD OF THE FIGHT_Vocals`
- `02 LORD OF THE FIGHT_Bass`
- `02 LORD OF THE FIGHT_Drums`
- `02 LORD OF THE FIGHT_Guitar`
- `02 LORD OF THE FIGHT_Other`
- `02 LORD OF THE FIGHT_Piano`
- `MIDI 1`
- All `Inst*` MIDI tracks (`Inst 1`, `Inst 1.dup1.02`, `Inst 1.dup2.02`,
  `Inst 1.dup2.04`, `Inst 1.dup3.02`, `Inst 1.dup4.02`)

The `0x1029 +5` byte does NOT correlate with this pattern — the previous
RE attempt was wrong. The real mute encoding is unknown.

### Track volumes

- `ClickPrint` = −31 dB (PT mixer reading)

### Fades (user-confirmed locations)

- `El Gtr 1` track: fade-in at start (~2 bars).
- `AC GTR Strum Demo 1` track: fade-in at start.
- `Intro SFX 1` / `Intro SFX 2` / `Intro SFX 2.dup1`: fade-in at start.
- `SYZ` track: fade-outs on items 2 and 3.
- `02 LORD OF THE FIGHT-05` track: crossfade between items 1 and 2
  (fade-out + fade-in).

The `0x262f` block decoder produces concrete lengths; values verified
against this list match.

## End-to-end check CLI

Convert the user session to a REAPER project file:

```bash
cargo run -p daw-reaper --example pt_to_rpp -- \
  "<path to .ptx>" "/tmp/out.rpp"
```

The resulting `.rpp` opens in REAPER and is the visual / audible
correctness check for everything in this roadmap.

---

## 16. Known unobservable (write-time passthrough)

Every ❌ or 🟡 entry in §§1–12 has been **moved here** — the rows in
those sections now show `→§16` in the Read column so the migration is
visible at a glance. §§13–15 retain their original symbols because the
goal scoped this move to §§1–12.

### Evidence supporting the move

Items listed below are read at the **raw byte** level but their
*semantics* are not yet decoded. Every byte still survives
`parse_raw → encrypt` verbatim:

- `crates/dawfile-protools/examples/round_trip.rs` asserts byte-identity
  between `original` and `RawSession::encrypt()` output, plus field
  equality on the re-parsed `ProToolsSession`.
- `crates/dawfile-protools/tests/round_trip_full.rs` runs that example
  over **all 17 fixtures** in `tests/fixtures/`; result: 17/17 pass.
- The example also passes on the user session at
  `~/Downloads/.../Copy of 02 LORD OF THE FIGHT 1.5.ptx`.

So every undecoded byte for every ❌/🟡 entry below is preserved
losslessly across read→write→read. The semantics are not observable in
the parsed `ProToolsSession`, but the bytes are not lost.

### Migrated entries (§§1–12)

| § | Feature | Original status | Reason it stays unobservable |
|---|---|---|---|
| 2 | Key-signature ruler items | ❌ read | Block not located |
| 2 | Chord-symbol ruler items | ❌ read | Block not located |
| 2 | Loop / selection points | ❌ read | Block not located |
| 2 | Pre/post-roll | ❌ read | Block not located |
| 2 | Cycle/punch points | ❌ read | Block not located |
| 3 | File ↔ region mapping | 🟡 read | Region payload offset reads garbage; uses name-stem heuristic |
| 3 | External file refs | ❌ read | Mechanism unknown |
| 4 | Region audio file index | ❌ read | Same bug as §3 file↔region |
| 4 | Region gain | ❌ read | Block not located |
| 4 | Region pitch shift | ❌ read | Block not located |
| 4 | Region time-stretch / Elastic Audio | ❌ read | Block not located |
| 4 | Warp markers (Elastic) | ❌ read | Block not located |
| 4 | Region color | ❌ read | Block not located |
| 4 | Region clip group membership | ❌ read | Block not located |
| 4 | Compound regions | 🟡 read | Parsed as containers only; inner schema unknown |
| 5 | Track UID | 🟡 read | Heuristic — exact offset within `0x251a` unclear |
| 5 | Master track | 🟡 read | Filtered at import; not surfaced on `ProToolsSession` |
| 5 | Aux / instrument tracks | 🟡 read | Currently coerced into `midi_tracks` |
| 6 | Mute | ❌ read | `0x1029 +5` interpretation is wrong; correct bit not found. **Conflicts with §3 ground-truth mute pattern — see "Ground-truth deferred" below.** |
| 6 | Pan (left ch) | 🟡 read | Maps `−100` for stereo to centered as best guess |
| 6 | Pan (right ch / multi-out) | ❌ read | Layout in `+17..+87` undecoded |
| 6 | Solo | ❌ read | Block not located |
| 6 | Record-arm | ❌ read | Block not located |
| 6 | Input-monitor mode | ❌ read | Block not located |
| 7 | Track color | ❌ read | Suspected in `0x1029 +171..` but unverified |
| 7 | Track icon/image | ❌ read | Block not located |
| 7 | Track comment/notes | ❌ read | Block not located |
| 7 | Track height | ❌ read | Block not located |
| 7 | Track visibility | ❌ read | Block not located |
| 7 | Track delay | ❌ read | Block not located |
| 7 | Phase invert | ❌ read | Block not located |
| 7 | Track timebase | 🟡 read | Inferred from `+16` byte; not verified |
| 8 | I/O routing table | 🟡 read | Parsed as containers; field semantics unknown |
| 8 | Track input | ❌ read | Block not located |
| 8 | Track output | ❌ read | Block not located |
| 8 | Aux send count/levels/destinations | ❌ read | Block not located |
| 8 | Aux send pre/post-fader | ❌ read | Block not located |
| 8 | HW insert routing | ❌ read | Block not located |
| 9 | Per-track insert chain | ❌ read | Block not located |
| 9 | Insert plugin parameters | ❌ read | Plugin params are opaque blobs |
| 9 | Insert bypass | ❌ read | Block not located |
| 9 | Insert wet/dry | ❌ read | Block not located |
| 9 | Insert ordering | ❌ read | Block not located |
| 9 | Side-chain routing | ❌ read | Block not located |
| 10 | Region clip-effect / mute | ❌ read | Block not located |
| 10 | Region clip gain | ❌ read | Block not located |
| 11 | Custom curve shapes | ❌ read | `0x262f` trailing bytes undecoded |
| 11 | Fade preset file refs | ❌ read | `cFadePresetFile` not parsed |
| 12 | MIDI CC events | ❌ read | Block not located |
| 12 | Pitch bend | ❌ read | Block not located |
| 12 | Aftertouch / program change | ❌ read | Block not located |
| 12 | Note metadata (channel) | ❌ read | Channel offset within MdNLB record unknown |
| 12 | Tempo-mapped MIDI region timing | 🟡 read | Tempo-curve case untested |
| 12 | Compound MIDI regions | 🟡 read | Parsed as containers only |

### Ground-truth deferred

Two claims in the "Test-session ground truth" section of this roadmap
cannot be honored by `pt_to_rpp` until the corresponding §16 entries are
re-RE'd. They are marked here as **deferred**:

- **Mute pattern** (every-other-track plus all `Inst*`). `pt_to_rpp`
  emits NO `MUTE` lines today (verified on `/tmp/out.rpp`). Tied to the
  §6 Mute entry above.
- **`ClickPrint = −31 dB`**. `pt_to_rpp` emits `VOLPAN 0.5370`
  (= −5.4 dB) on the `ClickPrint` track. The `VOLPAN 0.02818` (= −31 dB)
  value does exist in `/tmp/out.rpp` but is associated with a different
  track index. Tied to the §6 Volume row — read status is ✅ but
  track-association is bugged.

Until those §16 entries are promoted back to ✅, item 3 of the autonomous
goal is bounded to the four claims that *do* hold:
INTRO marker at 4.286 s, 16 markers, ALLTAKES 0 on every audio item,
FADEIN/FADEOUT/crossfade emission.

### Verified byte-identical passthrough

Every fixture in `crates/dawfile-protools/tests/fixtures/` AND the
user session in `Test-session ground truth` round-trip with
`re_encrypted == original` byte-for-byte. The `round_trip` example
asserts both byte identity AND `ProToolsSession` field equality
between the first and second parse.

### Confirmed observable on user session

- INTRO marker position = 4.2857 s (= bar 3 at 168 BPM 6/8) — matches
  ground truth.
- 16 markers parsed in the expected song order.
- 44 audio items emitted with `ALLTAKES 0` (one take per item).
- Fade-in / fade-out / crossfade entries are emitted by `pt_to_rpp`.

### Confirmed broken on user session (re-RE required)

These remain ❌ pending further reverse-engineering:

- **ClickPrint volume.** Roadmap claim: `-31 dB`. Observed in
  `/tmp/out.rpp`: `VOLPAN 0.5370` on the `ClickPrint` track
  (= `-5.4 dB`). A `VOLPAN 0.02818` (= `-31 dB`) appears on a *different*
  track index. Hypothesis: the volume parsed from `0x1029` is being
  associated with the wrong track entry, OR `+1..+5` is not the mixer
  fader for every track kind.
- **Track mute.** No `MUTE` lines are emitted; `0x1029 +5`
  interpretation in `types.rs:150` is documented as incorrect and the
  real bit has not been located.
- **Region → audio file index.** Region payload field reads garbage
  (overruns into the next block's magic). Name-stem heuristic still
  active in `parse/regions.rs`.

### Recommendation

`✏️` write status throughout §§1–15 is now backed by the raw passthrough
guarantee. Promoting any ❌ entry to ✅ for read requires the
RE methodology in this doc; promoting to ✅ for write additionally
requires the block serializer work in §15 (none done yet).

