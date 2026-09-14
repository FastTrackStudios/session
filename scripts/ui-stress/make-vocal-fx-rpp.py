#!/usr/bin/env python3
"""A lead vocal and its returns, laid out the way a vocal template is.

    Vox Lead
      Chase Vox        Lead Vox, Vox Dbl
      Vox FX
        Delay          Slap, Short, Long, Throw
        Verb           Room, Short, Long, Moment, Throw
        Wide
        Pitch          Oct+, Oct-

One lead and a pyramid of depth behind it: slaps and short rooms tucked
in bright and narrow, long delays and halls falling away dark and wide,
a moment reverb and a throw for the scene changes. The returns are the
Depth stage's tracks, and each carries one instance — a delay with its
de-esser and EQ inside it, a reverb with its de-esser, its EQs and its
decay-rate EQ inside it.

    scripts/ui-stress/make-vocal-fx-rpp.py > /tmp/fts-vocal-fx.rpp
"""

import random
import sys

BPM = 92.0
SECS_PER_BAR = 4 * 60.0 / BPM
BARS = 48
random.seed(0xB0CA1)


def guid() -> str:
    hexes = "0123456789ABCDEF"
    part = lambda n: "".join(random.choice(hexes) for _ in range(n))
    return f"{{{part(8)}-{part(4)}-{part(4)}-{part(4)}-{part(12)}}}"


VOX = 0xB04A6A
DELAY = 0x8B5CF6
VERB = 0x6D6AD9
WIDE = 0x3B82F6
PITCH = 0xEC4899

TREE = [
    (
        "Vox Lead",
        VOX,
        [
            ("Chase Vox", VOX, [("Lead Vox", VOX, []), ("Vox Dbl", VOX, [])]),
            (
                "Vox FX",
                DELAY,
                [
                    ("Delay", DELAY, [(n, DELAY, []) for n in ("Slap", "Short", "Long", "Throw")]),
                    ("Verb", VERB, [(n, VERB, []) for n in ("Room", "Short", "Long", "Moment", "Throw")]),
                    ("Wide", WIDE, []),
                    ("Pitch", PITCH, [("Oct+", PITCH, []), ("Oct-", PITCH, [])]),
                ],
            ),
        ],
    ),
]


def flatten(nodes, depth=0, out=None):
    if out is None:
        out = []
    for name, colour, children in nodes:
        out.append((name, colour, depth, bool(children)))
        flatten(children, depth + 1, out)
    return out


MIN_WIDTH = 30
FOLDER_WIDTH = 56
TONE_WIDTH = 133


def main() -> None:
    tracks = flatten(TREE)
    out = sys.stdout.write
    guids = [guid() for _ in tracks]
    out("<REAPER_PROJECT 0.1 '7.0' 0\n")
    out(f"  TEMPO {BPM:g} 4 4\n")
    widths = []
    for g, (name, _, depth, is_folder) in zip(guids, tracks):
        if is_folder:
            w = FOLDER_WIDTH
        elif name == "Lead Vox":
            w = TONE_WIDTH
        else:
            w = MIN_WIDTH
        widths.append(f"{g}={w}")
    out("  <EXTSTATE\n    <FTSMCP\n")
    out(f"      WIDTHS {' '.join(widths)}\n")
    out("    >\n  >\n")
    for i, (name, colour, depth, is_folder) in enumerate(tracks):
        nxt = tracks[i + 1][2] if i + 1 < len(tracks) else 0
        delta = nxt - depth
        out(f"  <TRACK {guids[i]}\n")
        out(f'    NAME "{name}"\n')
        out(f"    TRACKID {guids[i]}\n")
        red, green, blue = (colour >> 16) & 0xFF, (colour >> 8) & 0xFF, colour & 0xFF
        out(f"    PEAKCOL {0x1000000 | (blue << 16) | (green << 8) | red}\n")
        out("    BEAT -1\n    AUTOMODE 0\n")
        out(f"    VOLPAN {0.55 + 0.45 * ((i % 7) / 6.0):.4f} 0.0 -1 -1 1\n")
        out("    MUTESOLO 0 0 0\n    IPHASE 0\n")
        out(f"    ISBUS {1 if is_folder else 0} {delta}\n")
        out("    SHOWINMIX 1 0.6667 0.5 1 0.5 0 0 0 0\n")
        out(f"    SEL {1 if name == 'Lead Vox' else 0}\n    REC {1 if name == 'Lead Vox' else 0} 0 1 0 0 0 0 0\n")
        out("    TRACKHEIGHT 0 0 0 0 0 0 0\n")
        if not is_folder:
            bar = random.randrange(0, 4)
            while bar < BARS:
                length = random.choice([4, 8, 8])
                out("    <ITEM\n")
                out(f"      POSITION {bar * SECS_PER_BAR:.6f}\n")
                out(f"      LENGTH {length * SECS_PER_BAR:.6f}\n")
                out(f'      NAME "{name} {bar // 4 + 1}"\n')
                out("      IGUID " + guid() + "\n      GUID " + guid() + "\n")
                out("    >\n")
                bar += length + random.choice([0, 4])
        out("  >\n")
    out(">\n")


if __name__ == "__main__":
    main()
