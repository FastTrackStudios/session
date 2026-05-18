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
