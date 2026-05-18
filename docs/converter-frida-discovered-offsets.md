# PTX field offsets discovered via Frida byte-read tracing

**Method**: hook `_$s10Foundation4DataV15_RepresentationOys5UInt8VSicig`
(Foundation `Data.subscript(_:Int) -> UInt8`) in the running converter
via Frida, run on probe fixtures, diff read offsets/values between
baseline and feature-isolated probes.

Each "read" line is `(file_offset, byte_value)`. By generating a probe
fixture with ONE field changed vs baseline and diffing the read
streams, the changed `(offset, value)` pair localizes the feature.

Hook script: `scripts/frida/trace_all_reads.js`.
Mapping helper: `crates/daw-reaper/examples/find_blocks_at.rs`.

## Already-known offsets (verified via Frida)

These match what `dawfile-protools` already has decoded:

| Feature | File offset | Block | Within-block | Roadmap row |
|---------|------------:|-------|-------------:|-------------|
| Track mute (stored bit) | 30546 | `0x1029` @ 0x7744 | +14 (payload +5) | §6 ✓ |
| Track solo | 30303 | `0x102d` @ 0x75b4 | +171 (payload +162) | §6 ✓ |
| Track solo defeat | 38206 | `0x200a` @ 0x9432 | +277 (payload +268) | §6 ✓ |
| Track color | 38044/45 | `0x2015` @ 0x943b | +97/+98 (payload +88) i16 LE | §7 ✓ |

(All previously decoded in the codebase. Confirms the Frida pipeline
is reading the right buffer.)

## NEW offsets — Phase A/B/C wins

### 1. Memory location color (markers) — `0x4826` block

**Was §16 "block not located".** PT12 stores markers as `0x4826`
entries inside the `0x2077` memory-location list (itself inside
`0x2030`). Each marker's color is split into 3 u16 fields:

| Within-block offset | Field | Value example |
|--------------------:|-------|---------------|
| +11 (payload +2) | R component (u16 LE) | 0x00D8 |
| +13 (payload +4) | G component (u16 LE) | 0x006E |
| +15 (payload +6) | B component (u16 LE) | 0x0041 |

Verified by probe `marker_colored` with REAPER color 0xD86E41 — bytes
at +11, +13, +15 read `D8, 6E, 41` (probe captured R + G; B inferred
since the marker_colored probe stopped at G in the trace, but the
block layout is consistent with R/G/B being u16 LE triplet).

### 2. I/O routing block — `0x2602` / `0x2603`

**Was §16 "field semantics unknown".** Reads observed at:

| File offset | Block | Within-block | Notes |
|------------:|-------|-------------:|-------|
| 21482 | `0x2603` @ 0x53d3 | +23 | I/O routing list entry 1 |
| 21500 | `0x2603` @ 0x53d3 | +41 | (18 bytes after previous = entry stride?) |
| 21691 | `0x2603` @ 0x53d3 | +232 | mid-block field |
| 21793 | `0x2603` @ 0x53d3 | +334 | |

Inside the 0x2603 wrapper, `0x2602` entries appear at +10, +28, +49.
The +18 spacing of the first two reads suggests an entry size of 18
bytes for routing entries.

**Action**: probe specific routing setups (`bus_explicit`,
`folder3`, `send` already done — see § Per-probe new reads) to
isolate which bytes encode source/destination/level.

### 3. Per-clip flag byte — `0x1052` payload +18

Reads at file offsets 29755, 29784 land in two `0x1052` entries at
within-block offset +27 (= payload +18, after the 4-byte length
prefix + 10-byte "ProbeTrack" name + 4 bytes).

**Hypothesis**: this is a per-clip / per-region flag byte
(roadmap §10 "Region clip-effect / mute"). The probe didn't toggle
it, but its position is now known.

### 4. Track kind / hierarchy — `0x251a` payload +25

Reads at 549, 642 inside two `0x251a` entries at +25 (= payload +16,
after `[u16][name_len_u32][name_bytes][separator]`). Value `1` in
baseline.

This matches the existing `Track.is_folder` decoder which reads
`0x251a +25`. ✓

## Per-probe new reads

### `marker` (one un-colored marker at t=1.0, name "M")

Compared to baseline, new reads at:

| File offset | Block | Within-block | Value | Likely meaning |
|------------:|-------|-------------:|------:|----------------|
| 45490 | `0x2077` @ 0xb19a | +24 | 0 | flag/counter |
| 45669 | inside `0x4826` @ 0xb263 | +6 | 38 | marker id? (u16 LE = `26 48`?) |
| 45670 | inside `0x4826` | +7 | 72 (=0x48) | marker id high byte? |
| 45673 | inside `0x4826` | +10 | 147 | timecode-related byte |
| 45675 | inside `0x4826` | +12 | 2 | ? |

### `marker_colored` (marker with color 0xD86E41)

Reads include the R/G bytes mapped above (see § 1). Other reads are
the same as plain marker probe but at slightly shifted file offsets
because the colored variant adds bytes.

### `folder` / `folder3` / `send`

These produce WHOLESALE shifts in file offsets because adding tracks
restructures the file layout. Individual feature bytes need a
different probe strategy (e.g. pre-allocate a fixed track count and
toggle features within).

### `mute_envelope` / `vol_envelope`

Same scan pattern as baseline — envelopes don't trigger extra
subscript reads. The converter probably reads breakpoints via direct
pointer arithmetic (not subscript), so they're invisible to this
hook. To see envelope reads we'd need to instrument the bulk-read
path (e.g. `memcpy` from `Data`).

### `inactive` / `fx_disabled` / `vol` / `pan`

Zero diff vs baseline — likely the probes don't actually mutate the
generated PTX bytes (REAPER builder issue or converter takes a
different code path for these fields).

## Trace on the LotF user session — new offsets

Running the same hook on
`Copy of 02 LORD OF THE FIGHT 1.5.ptx` (3.9 MB, 29 logical tracks)
produces 1,885 byte-reads. Aggregating by block CT and within-block
offset (see `crates/daw-reaper/examples/map_offsets.rs`):

### Newly-observed block CTs (not yet in our parser)

| CT | Read offsets | Count | Likely meaning |
|------|--------------|------:|----------------|
| `0x104f` | +9, +20, +24, +25, +26, +27 | 504 | Per-clip sub-block (lives INSIDE `0x1050` at its payload start). +25/+26 = `FE FF` = i16 LE `-2` (default-color marker), so 0x104f probably stores **clip color** / clip palette. |
| `0x2602` | +10, +31, +33, +35, +36, +47, +48, +50, +51, +52 | 536 | I/O routing entry. +10 is the "active" flag (toggles 0/1 across entries). |
| `0x2628` | +45..+48, +54..+59 | 312 | Audio region inner block. +54..+59 is a **6-byte UID identifying the source audio file** (regions referencing the same file share this UID). |
| `0x262f` | +17, +18, +21 | 112 | Fade definition. +17/+18 likely fade-length encoding bytes (per `pt-fade-encoding.md`). |
| `0x4826` | +7, +8, +11, +13, +15 | 80 | Marker color block — verified above. |
| `0x2077` | +24, ... | 16 | Marker entry list. |
| `0x204d` | +9 | 1 | Unknown — read once early in parse. |

### `0x2628 +54..+59` — **audio file UID** (region → file mapping!)

This is the long-standing §3 / §16 "Block not located: region audio
file index" win. The 6-byte UID identifies the source file. Multiple
regions pointing to the same WAV will share the UID. To build the
region→file mapping:

1. Walk each `0x2629` region; descend to its inner `0x2628`.
2. Read 6 bytes at payload `+54..+59` (block_start `+63..+68`).
3. Group regions by UID. Each unique UID = one source file.
4. Cross-reference UIDs to filenames via the file-list block (whose
   UID layout still needs probing).

This replaces the name-stem heuristic currently in
`parse/regions.rs`.

### `0x1050 +53` — per-clip flag (mute? gain?)

92 reads at +53 inside `0x1050` blocks. Values mostly 0 but TWO are
1 (offsets 390481 and 390537 in LotF). This is a per-clip boolean.

The existing parser knows `0x1050 +46 == 0x01` indicates a fade.
`+53` is 7 bytes later — a separate flag, likely **clip mute** or
**clip gain non-zero** marker (§10 "Region clip-effect / mute" or
"Region clip gain" in the roadmap).

### `0x2602` routing — additional fields

Beyond the +10 active flag:

- `+10`: enable flag (0 = inactive, 1 = active)
- `+33`: another byte flag (varies)
- `+36`: another flag — observed 1 on some entries
- `+47..+52`: cluster of bytes — likely destination ID encoding

### Confirmation of existing decoded offsets on LotF

- `0x1029 +14` mute: 20 reads (= 8 muted + checks)
- `0x102d +N` solo: 20 reads
- `0x200a +N` solo-defeat
- `0x2015 +97/+98` color (i16 LE)
- `0x4826 +11/+13/+15` marker color RGB (this session)

All match the current parser's offsets.

## Routing-examples fixture trace

Running the hook on `routing-examples.ptx` (24 tracks with many
routings) produces 724 reads. The routing block `0x2602` accounts
for 554 of them across 217 distinct entries. Hot offsets:

| Offset | Reads | Observed values | Likely meaning |
|--------|------:|-----------------|----------------|
| +10 | 217 | 0 / 1 | **active flag** (1 = routing entry is in use) |
| +33 | 9 | 0 | secondary flag |
| +35 | 28 | varies | data field |
| +36 | 7 | 0 / 1 | another active-like flag |
| +47..+52 | ~200 cluster | varies | 6-byte block — looks like a destination UID (same pattern as the region source-file UID) |
| +50 | 19 | varies | mid-UID byte |
| +54 | 7 | varies | trailing field |

The `+47..+52` cluster IS shaped like a UID (varying bytes per
entry, consecutive). Hypothesis: it's the destination UID that
points to a specific bus / output / send target. Resolving the UID
to a destination name needs probing of the bus/output list block.

Also confirmed on this trace:
- `0x260e +49 = 66` — a single read; likely a destination index
  for the track output assignment.
- `0x2015` has MULTIPLE i16-LE `FE FF` (default color) reads
  at `+51..+52`, `+54..+55`, `+97..+98` — three separate color
  positions within the same block (probably different states /
  default-vs-set colors).

## Multi-byte ranges captured via `Data.subscript(_:Range<Int>)`

Hooking the range-subscript variant (`_$s10Foundation4DataV15_RepresentationOyACSnySiGcig`)
at PLT stub `base + 0x2690c8` (script:
`scripts/frida/trace_range_reads.js`) reveals **all multi-byte
slice reads** that the single-byte hook misses.

Baseline trace produces 62 unique range reads. Mapping highlights:

| Range size | Within block | Likely meaning |
|------------|--------------|----------------|
| 4 bytes | `0x2637 +9..+12` | u32 — version/sample-rate |
| 8 bytes | `0x2602 +37..+44` | second UID inside routing entry |
| 8 bytes | `0x102d +43..+50` (within `0x2619`) | per-track UID? |
| 10 bytes | `0x2519 +79..+88` | first-track name slice ("ProbeTrack") |
| 10 bytes | `0x1052 +13..+23` | playlist-entry name |
| 18 bytes | `0x2602 +28..+45` (etc.) | routing entry UID + flags |
| 22 bytes | `0x4803 +13..+34` (under `0x204b`) | unknown block sub-field |
| 38 bytes | `0x0003 +9..+47` | session version/product chunk |
| 123 bytes | `0x0003 +9..+131` | larger session metadata |
| 256 bytes | `0x0003 +51..+306` | large session blob |

The range hook **doesn't capture byte values** (returns a Data
slice, not a copy). For value extraction, use the byte-subscript
hook OR read bytes from the buffer post-hoc with known offsets.

**Next-session wiring candidates** (offsets known, semantics need
ground-truth probes):
- `0x102d +41..+46` 6-byte UID (per-track identity?) — also at
  `+53..+58` (mirror).
- `0x2602 +15..+22` 8-byte field (additional routing UID).
- `0x260e +9..+10` u16 — track-output destination index
  (complement to the destination-string at `+45`/`+47`).
- `0x260e +20..+25` 6-byte UID for routing target.
- `0x2637 +9..+12` u32 — some session-level counter.

## Phase D — real-audio clip probes (2026-05-17)

WAV fixture: `/tmp/pt-re/input/clip_probe.wav` (96044 B silent mono 48k/16).
All probes prefix `clip_*` use that source.

### Verified clip offsets (wired in `TrackRegion`)

- `0x104f +9` u8 — **clip mute** (`clip_muted` probe sets to 1).
  Field: `TrackRegion.clip_muted`.
- `0x104f +25..+26` i16 LE — **clip color palette index**.
  `clip_with_wav` → `-2` (default); `clip_colored(0x6e41d8)` → `27`.
  Field: `TrackRegion.clip_color`.

### Pending — `clip_playrate_half` vs `clip_with_wav` diff

56 differing read lines. Concrete finds:

- New reads at file offsets `5953..=5955` (decoded vals `0, 119, 1`) resolve
  to `0x2628 +45..+47`. As 24-bit LE = `96000` = `48000 SR × (1 / 0.5)`,
  the converter's **stretched-length-in-samples** rederived from playrate +
  source length. Not the playrate primitive itself.
- New read inside `0x104f +20` (only present on playrate run) — strong
  candidate for the **per-clip playrate flag/byte**. Needs a name-length-
  equalized differential probe (`clip_playrate_quarter` vs `_half` vs
  `_double`) to isolate exact width + encoding.
- All `0x2603 +N` shifts (`+193 → +232 → +295`) are downstream of an
  inserted ~316-byte run earlier in the file — track elastic-time block
  candidate.

Probes that produced **zero** useful diff vs `clip_with_wav` (converter
discards or REAPER builder doesn't emit):

- `clip_selected`, `clip_at_offset`, `track_selected`, `track_locked`,
  `track_show_mixer`, `clip_named`, `clip_long_name` (clip name inherits
  from region `0x2629`, not stored per-clip).

### Fade probes — pending

`clip_fadein` / `clip_fadeout` produce complex shift patterns. Need
name-length-equalized + position-equalized control to localize fade
duration + curve bytes.

## Next probe ideas

To find more unobservable bytes:

1. **`item_basic` with a real WAV source** — would let us probe
   clip-mute (`0x1050 +53`), clip-gain (`0x104f +?`), clip-color.
2. **Routing-rich probes** (multi-track with explicit `send_to(bus)`)
   to enumerate `0x2602` fields.
3. **Pan probes that ACTUALLY mutate the file** — current `pan` probe
   produces zero diff; investigate whether REAPER builder is missing
   that field or converter ignores it.

## Methodology — extending this

To find more byte-level fields:

1. Build a probe fixture in `crates/daw-reaper/examples/rpp_to_ptx_probe.rs`
   that isolates ONE feature.
2. Convert via `scripts/pt-convert.sh --hook scripts/frida/trace_all_reads.js`.
3. Diff the read log against baseline.
4. Run differing offsets through `crates/daw-reaper/examples/find_blocks_at`
   to identify (CT, within-block offset).
5. Codify the offset in `crates/dawfile-protools/src/parse/` and a
   writer in `crates/dawfile-protools/src/write/`.

## Limitations

- **Only Foundation `Data` subscript reads visible.** Direct pointer
  derefs in scan loops (`*pcVar1 == 0x5A`) bypass this hook. Most
  4-byte numeric reads (vol, pan, time positions) likely fall here.
- **Probe shifts.** Multi-track / folder / send probes shift file
  offsets globally, making feature isolation harder.
- **Module ASLR.** Each run gets a different `module_base`. Always
  compute offsets relative to `base` for hook addresses.

## Files

- `scripts/frida/trace_all_reads.js` — the hook script
- `scripts/frida/trace_block_scan.js` — earlier hook on FUN_100175f6c
- `scripts/frida/trace_data_reads.js` — emit + read combo
- `/tmp/reads_<probe>.log` — captured logs per probe
- `crates/daw-reaper/examples/find_blocks_at.rs` — offset → block resolver
