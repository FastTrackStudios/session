#!/usr/bin/env python3
"""A session laid out the way the dynamic template lays one out.

The synthetic fixture (`make-synthetic-rpp.py`) is a stress test: two
thousand flat tracks with a bus every eighth, which says nothing about
how a REAL template nests. This one is the other case — small, and deep.

The hierarchy is the one the templater's insert-group actions build:

    Drum Kit
      Kick          Sum { In, Out, Trig }, Sub, Verb
      Snare         Sum { Top, Bottom, Trig }, Verb
      Toms          Tom 1 { T1, T1 Trig } ... Tom 4, Verb
      Cymbals       OH, Hi-Hat, Ride
      Rooms         Mono, Stereo L, Stereo R

Triggers, subs and reverb returns open at the minimum height: they are
tracks you want present and almost never want to read.

Four levels before an audio track, and the sends sit BESIDE the Sum
rather than inside it — a Verb is fed by the sum, it is not one of the
things being summed. The toms have no Sum of their own: a folder holding
one mic and one trigger would be a level that says nothing.

The group and mic-position names come from `groups/drums/drum_kit/*.rs`
— `Kick`'s collection is literally `patterns(["In", "Out", "Trig"])`
there.

Why it matters for the panel: every indent level shifts a row's rail,
and nothing in the flat fixture goes past one. A template like this is
where "show me the folder structure" is a question with an answer.

    scripts/ui-stress/make-template-rpp.py > /tmp/fts-template.rpp
"""

import random
import sys

BPM = 120.0
BEATS_PER_BAR = 4
SECS_PER_BAR = BEATS_PER_BAR * 60.0 / BPM
BARS = 64

random.seed(0xD3110)


def guid() -> str:
    hexes = "0123456789ABCDEF"
    part = lambda n: "".join(random.choice(hexes) for _ in range(n))
    return f"{{{part(8)}-{part(4)}-{part(4)}-{part(4)}-{part(12)}}}"


# (name, colour, children). A track with children is a folder; one
# without is where the audio actually lives.
def mics(colour, *names):
    """Close mics, in their piece's colour.

    Inherited rather than left unset: a template is read by colour as
    much as by indent, and a kit piece whose mics were grey looked like
    three unrelated tracks that happened to sit under it.
    """
    return [(n, colour, []) for n in names]


def summed(name, colour, mic_names, extras=()):
    """A kit piece: its close mics under a SUM, with sends beside it.

    The SUM is a folder of the mics; anything the piece sends to — a
    Sub, a Verb — sits NEXT to the SUM rather than inside it, because it
    is fed by the sum and is not one of the things being summed.
    """
    children = [("Sum", colour, mics(colour, *mic_names))]
    children += [(extra, colour, []) for extra in extras]
    return (name, colour, children)


def tom(number, colour):
    """A tom: its mic and its trigger, and nothing else.

    No Sum and no Verb of its own. Two tracks do not need a sum to sit
    under — the piece IS the pair — and the toms share one reverb, which
    lives beside them at the Toms level.
    """
    return (
        f"Tom {number}",
        colour,
        [(f"T{number}", colour, []), (f"T{number} Trig", colour, [])],
    )


KICK = 0x8E5A3B
SNARE = 0xA8873E
TOMS = 0x7B5EA7
CYMBALS = 0x3F8E7D
ROOMS = 0x9A5A6B

TREE = [
    (
        "Drum Kit",
        0x4A6FA5,
        [
            summed("Kick", KICK, ["In", "Out", "Trig"], ["Sub", "Verb"]),
            summed("Snare", SNARE, ["Top", "Bottom", "Trig"], ["Verb"]),
            (
                "Toms",
                TOMS,
                # One Verb for the kit's toms, not one per tom: they are
                # sent to it together, so it belongs at the Toms level
                # beside them rather than repeated inside each.
                [tom(n, TOMS) for n in (1, 2, 3, 4)] + [("Verb", TOMS, [])],
            ),
            (
                "Cymbals",
                CYMBALS,
                [
                    ("OH", CYMBALS, []),
                    ("Hi-Hat", CYMBALS, []),
                    ("Ride", CYMBALS, []),
                ],
            ),
            (
                "Rooms",
                ROOMS,
                [
                    ("Mono", ROOMS, []),
                    ("Stereo L", ROOMS, []),
                    ("Stereo R", ROOMS, []),
                ],
            ),
        ],
    ),
    (
        "Bass",
        0x6B8E3F,
        [
            summed("Electric", 0x6B8E3F, ["DI", "Amp"], ["Sub"]),
            ("Synth", 0x6B8E3F, []),
        ],
    ),
    (
        "Guitars",
        0xA85A3B,
        [
            summed("Rhythm L", 0xA85A3B, ["Amp", "DI"]),
            summed("Rhythm R", 0xA85A3B, ["Amp", "DI"]),
            ("Lead", 0xA85A3B, []),
        ],
    ),
    (
        "Vocals",
        0xB04A6A,
        [
            summed("Lead", 0xB04A6A, ["Close", "Room"], ["Verb"]),
            ("Doubles", 0xB04A6A, []),
            ("Harmonies", 0xB04A6A, []),
        ],
    ),
]


# "As small as this host allows", not a number.
#
# Every DAW has its own floor and this fixture is opened by more than
# one; writing the renderer's own minimum here would duplicate a
# constant that is a user SETTING on the other side, and go stale the
# first time anyone changed it. One pixel is below every floor there is,
# so each host clamps it to its own.
MIN_HEIGHT = 1


def is_auxiliary(name: str) -> bool:
    """Is this a track you look at, or one you only need present?

    A trigger, a sub and a reverb return are all things you want IN the
    session and almost never want to READ: the trigger is a spike track
    for a sampler, the sub and the verb are sends whose level you set
    once. Collapsed to the minimum they stay reachable and stop spending
    the vertical space that the mics and the sums actually need.
    """
    return name in ("Sub", "Verb") or name.endswith("Trig")


def flatten(nodes, depth=0, out=None):
    """Depth-first, carrying each track's nesting level."""
    if out is None:
        out = []
    for name, colour, children in nodes:
        out.append((name, colour, depth, bool(children)))
        flatten(children, depth + 1, out)
    return out


def main() -> None:
    tracks = flatten(TREE)
    out = sys.stdout.write

    out("<REAPER_PROJECT 0.1 '7.0' 0\n")
    out(f"  TEMPO {BPM:g} 4 4\n")

    for i, (name, colour, depth, is_folder) in enumerate(tracks):
        # REAPER stores the depth DELTA, not the depth: the running level
        # after this track is where the next one starts. A folder always
        # opens one; a leaf closes however many end on it, which is what
        # makes the last track of a nest carry `-4`.
        nxt = tracks[i + 1][2] if i + 1 < len(tracks) else 0
        delta = nxt - depth

        out(f"  <TRACK {guid()}\n")
        out(f'    NAME "{name}"\n')
        if colour is not None:
            # REAPER's colour word is 0x01BBGGRR.
            red, green, blue = (colour >> 16) & 0xFF, (colour >> 8) & 0xFF, colour & 0xFF
            out(f"    PEAKCOL {0x1000000 | (blue << 16) | (green << 8) | red}\n")
        out("    BEAT -1\n    AUTOMODE 0\n")
        out(f"    VOLPAN {0.6 + 0.5 * ((i % 7) / 6.0):.4f} {-0.8 + 1.6 * ((i % 5) / 4.0):.4f} -1 -1 1\n")
        out("    MUTESOLO 0 0 0\n    IPHASE 0\n")
        out(f"    ISBUS {1 if is_folder else 0} {delta}\n")
        out("    SHOWINMIX 1 0.6667 0.5 1 0.5 0 0 0 0\n")
        out("    SEL 0\n    REC 0 5088 1 0 0 0 0 0\n")
        height = MIN_HEIGHT if is_auxiliary(name) else 0
        out(f"    TRACKHEIGHT {height} 0 0 0 0 0 0\n")

        # Audio only on the leaves. A folder's items are its children's.
        if not is_folder:
            bar = random.randrange(0, 8)
            while bar < BARS:
                length = random.choice([4, 8, 8, 16])
                out("    <ITEM\n")
                out(f"      POSITION {bar * SECS_PER_BAR:.6f}\n")
                out(f"      LENGTH {length * SECS_PER_BAR:.6f}\n")
                out(f"      NAME \"{name} {bar // 4 + 1}\"\n")
                out("      IGUID " + guid() + "\n      GUID " + guid() + "\n")
                out("    >\n")
                bar += length + random.choice([0, 0, 4])
        out("  >\n")

    out(">\n")


if __name__ == "__main__":
    main()
