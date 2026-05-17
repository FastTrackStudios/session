# PT block field map — discovered via RPP→PTX probe-and-diff

Method: generate a minimal RPP with ONE feature set (e.g. mute=true on a
single track), convert RPP→PTX via the converter's `--convert` CLI,
plaintext-diff against a `baseline` PTX with the same tracks but no
feature set. Differing bytes = the encoding location.

Probes live in `crates/daw-reaper/examples/rpp_to_ptx_probe.rs`; the
diff tool in `crates/daw-reaper/examples/ptx_plaintext_diff.rs`.

## Verified field locations (2026-05-17)

### Track mute (bool)

Stored redundantly across many per-track wrappers — the converter writes
the same bit to all of these:

| Block | Offset | Encoding |
|---|---|---|
| `0x1029` | `+5`   | u8 (0/1) ← what our parser reads |
| `0x260a[1]` | `+26` | u8 (the master-send mute) |
| `0x260d` | `+14`, `+447` | u8 × 2 |
| `0x261b` | `+414`, `+847` | u8 × 2 |
| `0x261c` | `+423`, `+856` | u8 × 2 |
| `0x2624` | `+436`, `+869` | u8 × 2 |

The `+5` byte we previously decoded WAS the mute storage location for
output. The over-mute on LotF is a parsing-direction issue (in real PT
sessions, the +5 bit may also reflect a separate "inactive" /
"bouncedSource" state — the converter cross-references it with mute
automation envelopes when reading).

### Track color (i16 LE, signed)

| Block | Offset | Encoding |
|---|---|---|
| `0x200a` | `+97..+98` | i16 LE |
| `0x200b` | `+106..+107` | i16 LE ← **our parser was reading wrong offset (+163)** |
| `0x2015` | `+88..+89` | i16 LE |

Default value (no color) = `0xfffe` = -2 as i16. Explicit colors are
small positive integers (palette indices). The full byte→RGB lookup
table is in `pt-color-palette-ground-truth.md`.

### Track solo (bool)

| Block | Offset | Encoding |
|---|---|---|
| `0x102d` | `+162` | u8 (0/1) |
| `0x261b` | `+171` | u8 |
| `0x261c` | `+180` | u8 |
| `0x2624` | `+193` | u8 |

### Track volume (i16 LE, centibel)

| Block | Offset | Encoding |
|---|---|---|
| `0x260a[0]` | `+26..+27` | i16 LE (master-send level) |
| `0x260d` | `+407..+408` | i16 LE mirror |
| `0x261b` | `+807..+808` | i16 LE mirror |
| `0x261c` | `+816..+817` | i16 LE mirror |
| `0x2624` | `+829..+830` | i16 LE mirror |

Default = 0 (= 0 dB). Reaper pan 0.5 → centibel -60 → ~-6 dB.

### Track pan (i16 LE)

| Block | Offset | Encoding |
|---|---|---|
| `0x260a[2]` | `+26..+27` | i16 LE (left-channel pan) |
| `0x260a[16]` | `+26` | u8 (right-channel pan? only 1 byte differs) |
| `0x260c[0]` | `+36..+37` | i16 LE |
| `0x260c[1]` | `+36` | u8 |
| `0x260d` | `+497..+498`, `+1066` | mirror |
| `0x261b` | `+897..+898`, `+1466` | mirror |

Default = `0xff9c` = -100 (= full left, the PT convention even for
stereo tracks where REAPER would call this "balance"). Explicit pan
0.5 → +50.

### Track folder grouping

New block type **`0x200d`** appears ONLY when a track has child tracks
(folder structure). Encoding TBD — needs a "folder with 3 children"
probe to map the children-list format.

## Methodology — extending to next features

For each new feature probe:

1. Add a case to `rpp_to_ptx_probe::build_rpp()` that exercises just
   that one feature on top of `baseline`.
2. Run:
   ```
   cargo run -p daw-reaper --example rpp_to_ptx_probe -- <probe>
   cargo run -p daw-reaper --example ptx_plaintext_diff -- \
     /tmp/probe_baseline.ptx /tmp/probe_<feat>.ptx
   ```
3. Filter to short-run diffs (the long-run diffs are
   UID/timestamp/keystream noise that varies per file generation).
4. The remaining 1-2-byte diffs are the encoding location(s).
5. Add the field to this document.

## Additional probe findings (2026-05-17)

### `marker` (vs baseline)

Marker storage is **entirely inside the `0x0002[0]` container** (6550 bytes,
136 bytes differ between probes). Pattern: pairs of adjacent bytes
change at sparse offsets (+28..+29, +454..+455, +2554..+2555, ...).
This indicates a multi-entry list with i32/i64 position fields where
adjacent bytes represent the high-byte difference between baseline
positions and explicit marker positions.

Marker storage TBD — needs differential probes:
- `marker_two` — 2 markers (to see the per-marker delta)
- `marker_named_long` — marker with a long name (find name field)
- `marker_colored` — explicit color (find color field within marker entry)

### `send` (`Source` + `Dest` with `Dest.receive(0)`)

Send creates a SUBSTANTIAL structural change:
- New `0x200d × 1` (the same folder/child-relationship block)
- `0x1014` count 2→1 (Dest hidden from audio track list — became a bus)
- `0x102d[1]` +13..+16 = name change "Beta"→"Dest", and +28..+35 = 8-byte
  UID/route identifier change
- Many block-size shifts (0x1015, 0x1054, 0x2107, etc.)

This shows the converter MODELS REAPER receives as PT buses. Routing
the second track as a destination promotes it to a bus, removing it
from the audio-track list and creating a `0x200d` linkage.

### `folder3` (Parent + 3 children, vs `folder` Parent + 1 child)

Most counts triple (3 vs 1 children):
- `0x251a` 4→8 (= 2× n_tracks)
- `0x1029`, `0x102d`, `0x200a`, `0x200b`, `0x2015` all scale per-track
- `0x201f[0]` +76..+79 = `05 00 00 00` → `ff ff ff ff` (= 5 → -1)

The `0x201f` u32 field at +76 looks like a **count/sentinel**: when the
folder has 1 child, value is 5; when 3 children, value is -1 (which
typically means "all"). May be a "folder collapse depth" or similar.

### `item_basic`

A track with one `item(0.0, 1.0, |i| i.name("Clip"))` produces an
**IDENTICAL PTX** to baseline (zero diffs). The converter requires the
item to have a real audio source (`source_wave` pointing to an
existing file) to materialize it as a PT clip. Item probes need an
actual file fixture, not just metadata.

## Probes still needed (next pass)

- `marker_two`, `marker_colored` — to isolate name vs color encoding
- `clip_simple_with_source` — one item with real WAV source
- `clip_muted`, `clip_gain`, `clip_color`
- `vol_env`, `mute_env` — track automation envelopes
- `fade_in`, `fade_out`, `crossfade`
- `bus_explicit` — explicit bus track with multiple senders

## Process for next features

1. Add probe case to `rpp_to_ptx_probe::build_rpp()`
2. Run probe + diff against appropriate baseline
3. Filter to short-run diffs (the long-run ones are file-UID noise)
4. Document field locations in this file
5. Update parser to read those bytes
