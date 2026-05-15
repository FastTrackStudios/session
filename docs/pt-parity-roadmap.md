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
| Key-signature ruler items | unknown | ❌ | ❌ | |
| Chord-symbol ruler items | unknown | ❌ | ❌ | |
| Loop / selection points | unknown | ❌ | ❌ | |
| Pre/post-roll | unknown | ❌ | ❌ | |
| Cycle/punch points | unknown | ❌ | ❌ | |

---

## 3. Audio files

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Filename list | `0x1004`/`0x103a` | ✅ | ✏️ | |
| Per-file sample length | `0x1001` | ✅ | ✏️ | |
| File ↔ region mapping | unknown | 🟡 | ✏️ | **Currently using region-name → file-stem heuristic.** Real `audio_file_index` field in the region payload reads garbage (we read u32 at "end of block" which lands in next block's magic). |
| Audio file path resolution | (filesystem) | ✅ | ✏️ | session_dir/`Audio Files/` |
| External file refs | unknown | ❌ | ❌ | Files outside session dir |

**Priority:** real region→file index — high. Without it, regions renamed away
from the auto-generated `<file>-NN.L` pattern lose their source link.

---

## 4. Audio regions

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Region name, start, length, offset | `0x100b`/`0x262a` | ✅ | ✏️ | |
| Region audio file index | inside region payload | ❌ | ❌ | bug (see §3) |
| Region gain | unknown | ❌ | ❌ | |
| Region pitch shift | unknown | ❌ | ❌ | |
| Region time-stretch / Elastic Audio | unknown | ❌ | ❌ | |
| Warp markers (Elastic) | unknown | ❌ | ❌ | |
| Region color | unknown | ❌ | ❌ | |
| Region clip group membership | unknown | ❌ | ❌ | |
| Compound regions | `0x2628`/`0x2629`/`0x262b`/`0x262c` | 🟡 | ❌ | parsed as containers only |

---

## 5. Tracks (definition)

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Track name | `0x1014` | ✅ | ✏️ | |
| Channel count (mono/stereo) | `0x1014` | ✅ | ✏️ | |
| Channel index map | `0x1014` | ✅ | ✏️ | |
| Track kind (audio/MIDI/aux/master) | `0x251a` `+2` byte | ✅ | ✏️ | 0x00 audio, 0x02 aux/MIDI, 0x05 master, 0x07 inst |
| Track UID | `0x251a` payload | 🟡 | ❌ | needed for round-trip |
| Master track | `0x251a` kind=0x05 | 🟡 | ❌ | currently filtered out at import |
| Aux/instrument tracks | `0x251a` kind=0x02/0x07 | 🟡 | ❌ | included in midi_tracks Vec |
| Track creation order | (block order) | ✅ | ✏️ | |

---

## 6. Track mix state (volume, pan, mute, solo)

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Volume (fader) | `0x1029` `+1..+5` i32 LE | ✅ | ✏️ | 0.1 dB units; verified -31 dB matches PT |
| Mute | `0x1029` `+5`?? | ❌ | ❌ | **byte +5 ≠ actual mute** — agent's interpretation wrong. No single byte in 0x1029 payload matches the user's known mute pattern across audio tracks. |
| Pan (left ch) | `0x1029` `+13..+17` i32 LE | 🟡 | ✏️ | `−100` for stereo = "natural state", we map to centered |
| Pan (right ch / multi-out) | `0x1029` `+17..+87` | ❌ | ❌ | |
| Solo | unknown | ❌ | ❌ | |
| Record-arm | unknown | ❌ | ❌ | |
| Input-monitor mode | unknown | ❌ | ❌ | |

**Priority: HIGH.** Mute is critical for usability — the user can't tell which
material is supposed to be silent. Search the whole 281-byte `0x1029` payload
for the real mute bit, OR look for a sibling block per track.

---

## 7. Track display & metadata

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Track color | unknown | ❌ | ❌ | Not in `0x1014` payload; possibly in `0x1029` extended area (+171..) |
| Track icon/image | unknown | ❌ | ❌ | |
| Track comment/notes | unknown | ❌ | ❌ | |
| Track height (mix/edit window) | unknown | ❌ | ❌ | |
| Track visibility | unknown | ❌ | ❌ | |
| Track delay (samples/ms) | unknown | ❌ | ❌ | |
| Phase invert | unknown | ❌ | ❌ | |
| Track timebase (samples vs ticks) | partly inferred from sub-entry `+16` byte | 🟡 | ❌ | |

---

## 8. Track routing

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Hardware I/O channels | `0x1021`/`0x1022` | ✅ | ✏️ | |
| I/O routing table | `0x2602`/`0x2603` | 🟡 | ❌ | parsed as containers; field semantics unknown |
| Track input (mic/line/bus) | unknown | ❌ | ❌ | |
| Track output (master/bus) | unknown | ❌ | ❌ | |
| Aux send count / levels / destinations | unknown | ❌ | ❌ | |
| Aux send pre/post-fader | unknown | ❌ | ❌ | |
| HW insert routing | unknown | ❌ | ❌ | |

**Priority: HIGH.** A printed mix uses bus routing extensively. Without
routing, the imported REAPER session has every track going straight to the
master, which is wrong for any serious session.

---

## 9. Track FX / plugins

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Session plugin registry | `0x1018`/`0x1017` | ✅ | ✏️ | list of plugin TYPES used |
| Per-track insert chain | unknown | ❌ | ❌ | |
| Insert plugin parameters | unknown | ❌ | ❌ | |
| Insert bypass state | unknown | ❌ | ❌ | |
| Insert wet/dry | unknown | ❌ | ❌ | |
| Insert ordering | unknown | ❌ | ❌ | |
| Side-chain routing | unknown | ❌ | ❌ | |

**Priority: HIGH.** No plugins means imported sessions sound like raw stems.

---

## 10. Region placements (track playlists)

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Active playlist regions | `0x1054`/`0x1052` | ✅ | ✏️ | |
| Region start position (samples) | sub-entry `+9..+12` | ✅ | ✏️ | |
| Region start position (ticks, for MIDI/inst) | sub-entry `+9` u40, when `+16==0x40` | ✅ | ✏️ | |
| Region clip-effect / mute | unknown | ❌ | ❌ | |
| Region clip gain | unknown | ❌ | ❌ | |
| Alternate playlists | `0x2428`/`0x2429`+`0x1054` | ✅ | ❌ | parsed but not emitted |

---

## 11. Fades & crossfades

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| Fade detection | `0x1050` `+46==0x01` | ✅ | ✏️ | |
| Fade-def list | `0x2630` wrapper | ✅ | ✏️ | |
| Fade in/out length + shape | `0x262f` | ✅ | ✏️ | see `pt-fade-encoding.md` |
| Custom curve shapes | `0x262f` trailing bytes | ❌ | ❌ | only linear/equal-power/equal-gain emitted |
| Fade preset file refs | `cFadePresetFile` | ❌ | ❌ | |

---

## 12. MIDI

| Feature | Block(s) | Read | Write | Notes |
|---|---|:-:|:-:|---|
| MIDI event chunks (note, vel, pos, dur) | `0x2000`/MdNLB | ✅ | ✏️ | 35-byte records; pos@+27, note@+9, vel@+10, dur@+11..+19 |
| MIDI regions | `0x2001`/`0x2634` | ✅ | ✏️ | length now via tick→sample |
| MIDI region→track assignment | `0x1058` | ✅ | ✏️ | with playlist-suffix-aware dedupe |
| MIDI CC events | unknown | ❌ | ❌ | only notes parsed |
| Pitch bend | unknown | ❌ | ❌ | |
| Aftertouch / program change | unknown | ❌ | ❌ | |
| Note metadata (channel) | unknown | ❌ | ❌ | channel always 0 |
| Tempo-mapped MIDI region timing | partial | 🟡 | ❌ | |
| Compound MIDI regions | `0x262b`/`0x262c` | 🟡 | ❌ | parsed as containers only |

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
