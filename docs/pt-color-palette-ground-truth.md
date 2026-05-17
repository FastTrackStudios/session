# PT track color palette — ground truth from converter

Captured 2026-05-17 via Frida harness on `~/Downloads/Color Testing.ptx`
(23×3 palette grid fixture). The PT Reaper Converter emits one `PEAKCOL`
per track; we read `x22` at the emit instruction = the final REAPER
color value `(0x01 << 24) | (B << 16) | (G << 8) | R`.

## Structure

72 captures in track order:

- **0..2** `0x8c7850` — "default-style" color emitted for the 3 row-header tracks (`x1`, `x2`, `x3`)
- **3..25** — Row 1 (saturated / light): 23 distinct hues
- **26..48** — Row 2 (medium / dim): 23 distinct hues
- **49..71** — Identical to 26..48 (Row 3 cells in the fixture inherit Row 2 colors)

Either the fixture's Row 3 was left at Row 2 colors, or PT itself only has 2
brightness rows in its current palette. Either way, the unique color set
is **47** distinct RGBs (1 default + 23 saturated + 23 dim).

## Captured palette (PEAKCOL RGB values)

### Default / header (PT color byte = 0)

| RGB | R | G | B |
|---|---|---|---|
| `0x8c7850` | 140 | 120 | 80 |

### Row 1 — saturated hues (PT byte indices 1..23 presumed, hue order)

| Slot | RGB | R | G | B |
|---|---|---|---|---|
| 0 | `0xd86e41` | 216 | 110 | 65 | orange |
| 1 | `0xd84b41` | 216 | 75 | 65 | red-orange |
| 2 | `0xd84169` | 216 | 65 | 105 | red-pink |
| 3 | `0xd841a3` | 216 | 65 | 163 | pink-magenta |
| 4 | `0xd141d8` | 209 | 65 | 216 | magenta |
| 5 | `0x9941d8` | 153 | 65 | 216 | purple |
| 6 | `0x7841d8` | 120 | 65 | 216 | violet |
| 7 | `0x4d41d8` | 77 | 65 | 216 | blue-violet |
| 8 | `0x5041d8` | 80 | 65 | 216 | (near 0x4d41d8) |
| 9 | `0x4166d8` | 65 | 102 | 216 | blue |
| 10 | `0x4194d8` | 65 | 148 | 216 | light blue |
| 11 | `0x41d3d8` | 65 | 211 | 216 | cyan |
| 12 | `0x41d897` | 65 | 216 | 151 | mint-cyan |
| 13 | `0x41d85a` | 65 | 216 | 90 | green |
| 14 | `0x55d841` | 85 | 216 | 65 | yellow-green |
| 15 | `0x69d841` | 105 | 216 | 65 | lime |
| 16 | `0x87d841` | 135 | 216 | 65 | light green |
| 17 | `0xabd841` | 171 | 216 | 65 | yellow-green-2 |
| 18 | `0xd6d841` | 214 | 216 | 65 | yellow |
| 19 | `0xd8b541` | 216 | 181 | 65 | gold |
| 20 | `0xd89441` | 216 | 148 | 65 | orange-yellow |
| 21 | `0xd87841` | 216 | 120 | 65 | orange-2 |
| 22 | `0xd86e41` | 216 | 110 | 65 | (= slot 0, wrap) |

### Row 2 — dim hues (PT byte indices 24..46 presumed)

| Slot | RGB | R | G | B |
|---|---|---|---|---|
| 0 | `0x664538` | 102 | 69 | 56 |
| 1 | `0x663b38` | 102 | 59 | 56 |
| 2 | `0x663844` | 102 | 56 | 68 |
| 3 | `0x663855` | 102 | 56 | 85 |
| 4 | `0x633866` | 99 | 56 | 102 |
| 5 | `0x523866` | 82 | 56 | 102 |
| 6 | `0x483866` | 72 | 56 | 102 |
| 7 | `0x3b3866` | 59 | 56 | 102 |
| 8 | `0x3c3866` | 60 | 56 | 102 |
| 9 | `0x384366` | 56 | 67 | 102 |
| 10 | `0x385166` | 56 | 81 | 102 |
| 11 | `0x386466` | 56 | 100 | 102 |
| 12 | `0x386652` | 56 | 102 | 82 |
| 13 | `0x38663f` | 56 | 102 | 63 |
| 14 | `0x3e6638` | 62 | 102 | 56 |
| 15 | `0x446638` | 68 | 102 | 56 |
| 16 | `0x4d6638` | 77 | 102 | 56 |
| 17 | `0x586638` | 88 | 102 | 56 |
| 18 | `0x656638` | 101 | 102 | 56 |
| 19 | `0x665b38` | 102 | 91 | 56 |
| 20 | `0x665138` | 102 | 81 | 56 |
| 21 | `0x664838` | 102 | 72 | 56 |
| 22 | `0x664538` | 102 | 69 | 56 | (= slot 0, wrap) |

## Cross-check against LotF

Track "02 LORD OF THE FIGHT.01" has `color_byte = 0x06` in our parser
output. The converter's PEAKCOL for that track was `0xabd841`. In this
table, `0xabd841` is **Row 1, slot 17**.

That means PT's color-byte → palette-slot mapping is **NOT** a simple
linear index. The byte `0x06` doesn't translate to row 0 col 4 like we
assumed in `pt_color_to_rgb`. Need to map by reading the raw byte from
several known fixtures and building a lookup.

## Action items

1. Get our parser to correctly read the color_byte for Color Testing
   (currently returning 0 tracks — separate bug).
2. Run our parser on Color Testing to get every track's PT color byte.
3. Pair each PT byte with the captured RGB at the same index. Write the
   resulting 1:1 lookup table into `pt_color_to_rgb`.
4. Verify on LotF: parse → look up → compare against converter's
   PEAKCOL emits.

## Reproduce

On voyager:
```
frida -l /tmp/harness_emit.js -q -t 600 \
  -f "/Applications/PT Reaper Converter.app/Contents/MacOS/PT Reaper Converter" \
  > /tmp/harness.log 2>&1 &
# In the GUI: convert Color Testing.ptx
grep PEAKCOL /tmp/harness.log | python3 -c "
import sys, re
for line in sys.stdin:
    m = re.search(r'\"x22\":\"([^\"]+)\"', line)
    if m: print(f'0x{int(m.group(1),16) & 0xFFFFFF:06x}')"
```
