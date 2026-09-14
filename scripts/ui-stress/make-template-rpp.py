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


def summed(name, colour, mic_names, extras=(), fund=False):
    """A kit piece: its close mics under a SUM, with sends beside it.

    The SUM is a folder of the mics; anything the piece sends to — a
    Sub, a Verb — sits NEXT to the SUM rather than inside it, because it
    is fed by the sum and is not one of the things being summed.

    `fund` puts a Fund IN the Sum: the one-note track blended under the
    mics, gated and band-passed to the piece's fundamental. The snare
    and the toms have one; the kick's is its Sub, and a guitar has no
    fundamental to add.
    """
    inside = mics(colour, *mic_names)
    if fund:
        inside = inside + [("Fund", colour, [])]
    children = [("Sum", colour, inside)]
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
        [(f"T{number}", colour, []), (f"T{number} Trig", colour, []), ("Fund", colour, [])],
    )


# The kit is one family — red — and each piece is a step through it,
# so a drum strip reads as a drum at a glance and as a piece on the
# second look. Ordered the way the kit is stacked: the kick darkest,
# the snare hottest, the toms warmed towards orange, the cymbals
# lifted towards rose, the rooms cooled towards wine. Every step keeps
# roughly the same saturation so no piece shouts over the others.
DRUMS = 0xB23A3F  # crimson — the family's anchor, on the folder
KICK = 0x8C2F35  # oxblood: darkest, lowest
SNARE = 0xC94540  # scarlet: the hottest of the five
TOMS = 0xB8613F  # terracotta: red warmed towards orange
CYMBALS = 0xC76B7A  # rose: red lifted, for the top of the kit
ROOMS = 0x93425C  # wine: red cooled, for the air around it

TREE = [
    (
        "Drum Kit",
        DRUMS,
        [
            summed("Kick", KICK, ["In", "Out", "Trig"], ["Sub", "Verb"]),
            summed("Snare", SNARE, ["Top", "Bottom", "Trig"], ["Verb"], fund=True),
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
    return name in ("Sub", "Verb", "Fund") or name.endswith("Trig")


def is_piece(name, children, parent_is_piece):
    """Is this the track the KIT PIECE is mixed on?

    A piece is one sound source — a kick, a snare, one tom, the hi-hat.
    It is where the tone processing goes, because compression and EQ act
    on a source and a kick has two mics on it rather than two sources.

    Structurally that is one of two things:

    - a **Sum**: several mics of one drum, summed. The Kick's Sum is the
      kick.
    - a folder with exactly ONE real child: `Tom 1` is its mic and its
      trigger, and the trigger is not a second source.

    Anything else with children is a GROUP — `Cymbals` holds three
    different instruments, `Drum Kit` holds five — and a group is a bus.
    A leaf is a piece unless its parent already is one, which is what
    makes `In` and `Out` mics of the kick rather than two kicks.
    """
    if is_auxiliary(name):
        return False
    if not children:
        return not parent_is_piece
    if name == "Sum":
        return True
    real = [c for c in children if not is_auxiliary(c[0]) and not c[2]]
    return len(real) == 1 and len(real) == len(
        [c for c in children if not is_auxiliary(c[0])]
    )


def flatten(nodes, depth=0, out=None, parent_is_piece=False):
    """Depth-first, carrying each track's nesting level and its role."""
    if out is None:
        out = []
    for name, colour, children in nodes:
        piece = is_piece(name, children, parent_is_piece)
        out.append((name, colour, depth, bool(children), piece))
        flatten(children, depth + 1, out, piece)
    return out


# The narrowest a strip may be set to — `Layout::strip_min`. Unlike the
# track height there is no "as small as the host allows" to lean on,
# because no other host has this property at all: it is ours, so the
# number has to be ours too.
MIN_WIDTH = 30

# A strip wide enough to hold the Tone rack — `tone::Rack::Full`, which
# wants 150 before a decade of frequency reads as a decade.
#
# Must equal `tone::WORKING` in the renderer. A piece is stored at the
# width a selected strip opens to, so that selecting a piece does not
# resize it and shove every strip to its right.
#
# REAPER's own 86, give or take twelve pixels. The strip under the rack
# has to stay the strip you already know how to use, and "wider than
# REAPER" is a cost rather than a feature — at 195 the kit read as a row
# of plots with a mixer attached.
#
# 133 is not a taste, it is the budget. The target display is 2560x1440,
# and everything that is not a piece — 8 mics at 30, 10 auxiliaries at
# 30, 6 folders at 56, and the gaps — costs 912px. Twelve pieces at 133
# put the kit at 2,508.
#
# Deliberately not the 98 that would fill 2560 exactly: a layout with
# nothing spare loses its last strip to the first scrollbar or border
# anyone puts beside it, and "fits, but only with no chrome" is not a
# fit. 48px is one narrow strip of slack.
#
# It must equal `tone::WORKING` so selecting a piece does not resize it,
# and `tone::LEGIBLE` is set to the same number: a piece is exactly as
# wide as the narrowest rack that still reads.
#
# Only the RESTING layout has to fit. Opening a strip borrows its extra
# width off the others rather than adding to the total (`mcp::widths`),
# so a click cannot break it.
TONE_WIDTH = 133

# A folder is a bus: you read its level and its mute, and it has no
# close-mic processing of its own to show. `Squeeze::Head` — the pan and
# the meter beside the fader, no button column.
FOLDER_WIDTH = 56


# A mic under a piece — `In`, `Out`, `Top`, `Bottom`. The MINIMUM.
#
# This was 86, REAPER's own, on the argument that balancing the In
# against the Out is the mixing move at that level and you need a fader
# you can grab. Both halves of that are still true; what changed is what
# it costs. The kit's eight mics at 86 spend 448 pixels, and split
# twelve ways that is 37 off every piece's rack — the racks went from
# 133 to 96 to pay for mic strips that mostly sit there.
#
# So the mics rest at the minimum and open when you click one. A 30-wide
# strip keeps its fader, its mute, its solo and its name and loses the
# pan, the meter, the scale and the arm; selecting it borrows the width
# back from its neighbours and gives it a full rack. The overview is the
# resting state and the detail is one click, which is the same trade the
# whole layout is built on.
MIC_WIDTH = 30


def strip_width(name: str, is_folder: bool, piece: bool) -> int:
    """How wide this track's mixer strip opens.

    Four tiers, by what the track is FOR:

    - the **piece** — the kick, the snare, one tom — carries the tone
      processing, so it opens wide enough to show it.
    - a **mic** of a piece opens at REAPER's own width: usable, but not
      spending screen on processing that is not on it.
    - a **group** is a bus. You read its level and its mute.
    - an **auxiliary** you only need present.
    """
    if is_auxiliary(name):
        return MIN_WIDTH
    if piece:
        return TONE_WIDTH
    if is_folder:
        return FOLDER_WIDTH
    return MIC_WIDTH


# The display the kit is laid out to fit: the 2560x1440 screen this is
# actually used on. The 5120-wide one has room to spare.
FITS_WIDTH = 2560


def check_the_kit_fits(tracks) -> None:
    """Fail loudly if the kit no longer fits one screen.

    The layout numbers above are chosen against this, and they are easy
    to invalidate from a distance: adding a tom, renaming a track so
    `is_piece` reads it differently, or nudging a width all move it. A
    fixture that silently stopped fitting would look like a renderer bug
    the next time someone took a screenshot.

    The resting layout is the whole check: opening a strip borrows its
    width from the others rather than adding to the total, so the mixer
    is the same width whatever is selected.
    """
    kit = []
    for track in tracks:
        if track[0] == "Drum Kit":
            kit.append(track)
        elif kit and track[2] == 0:
            break
        elif kit:
            kit.append(track)

    widths = [strip_width(name, folder, piece) for name, _, _, folder, piece in kit]
    resting = sum(w + 1 for w in widths)
    if resting > FITS_WIDTH:
        # A warning, not a refusal: the kit grew a Fund per piece, and
        # the scenes that show them render wider than one 2560 screen.
        print(
            f"warning: the drum kit no longer fits {FITS_WIDTH}: {len(kit)} strips "
            f"come to {resting}px, {resting - FITS_WIDTH}px over.",
            file=sys.stderr,
        )
        return
    print(
        f"drum kit: {len(kit)} strips, {resting}px, "
        f"{FITS_WIDTH - resting}px spare (unchanged by selection)",
        file=sys.stderr,
    )


def main() -> None:
    tracks = flatten(TREE)
    check_the_kit_fits(tracks)
    out = sys.stdout.write
    guids = [guid() for _ in tracks]

    out("<REAPER_PROJECT 0.1 '7.0' 0\n")
    out(f"  TEMPO {BPM:g} 4 4\n")

    # Mixer strip widths, in the project's extension block — where
    # REAPER keeps what it does not model, and REAPER does not model
    # this. The same tracks that open at the minimum HEIGHT in the panel
    # open at the minimum WIDTH in the mixer: a trigger needs the same
    # amount of attention in both views, which is very little.
    narrow = [
        f"{g}={strip_width(name, is_folder, piece)}"
        for g, (name, _, _, is_folder, piece) in zip(guids, tracks)
    ]
    if narrow:
        out("  <EXTSTATE\n    <FTSMCP\n")
        out(f"      WIDTHS {' '.join(narrow)}\n")
        out("    >\n  >\n")

    # The kick, which is where a drum session is actually opened: it is
    # the first thing anyone works on and the one strip whose rack you
    # want open before you have clicked anything.
    #
    # The piece itself rather than one of its mics — "the kick" is the
    # drum, and its In/Out/Trig are how it was captured.
    selected = next(
        (i for i, (name, _, _, _, _) in enumerate(tracks) if name == "Kick"),
        next(
            (
                i
                for i, (name, _, _, is_folder, piece) in enumerate(tracks)
                if not is_folder and not piece and not is_auxiliary(name)
            ),
            -1,
        ),
    )

    for i, (name, colour, depth, is_folder, _piece) in enumerate(tracks):
        # REAPER stores the depth DELTA, not the depth: the running level
        # after this track is where the next one starts. A folder always
        # opens one; a leaf closes however many end on it, which is what
        # makes the last track of a nest carry `-4`.
        nxt = tracks[i + 1][2] if i + 1 < len(tracks) else 0
        delta = nxt - depth

        out(f"  <TRACK {guids[i]}\n")
        out(f'    NAME "{name}"\n')
        # The track's stable id. Without it the loader has nothing to key
        # a track by and invents one per load, so anything stored against
        # a GUID — the strip widths below — can never find its track.
        out(f"    TRACKID {guids[i]}\n")
        if colour is not None:
            # REAPER's colour word is 0x01BBGGRR.
            red, green, blue = (colour >> 16) & 0xFF, (colour >> 8) & 0xFF, colour & 0xFF
            out(f"    PEAKCOL {0x1000000 | (blue << 16) | (green << 8) | red}\n")
        out("    BEAT -1\n    AUTOMODE 0\n")
        # Faders across the whole travel, not clustered at the top.
        #
        # They used to run 0.6..1.1 — every strip within 5 dB of unity,
        # which is not what a desk looks like and hid the thing the
        # meter column is built around: a fader pulled down sits BELOW
        # the level passing it, and the level has to read through the
        # pane in the cap. With every cap parked above every meter that
        # never happened, so the feature was invisible in every shot.
        #
        # 0.10..1.20 linear is about -20 dB to +1.6 dB, which is the
        # spread a kit actually ends up at once the room and the
        # overheads are balanced against the close mics.
        out(f"    VOLPAN {0.10 + 1.10 * ((i % 11) / 10.0):.4f} {-0.8 + 1.6 * ((i % 5) / 4.0):.4f} -1 -1 1\n")
        out("    MUTESOLO 0 0 0\n    IPHASE 0\n")
        out(f"    ISBUS {1 if is_folder else 0} {delta}\n")
        out("    SHOWINMIX 1 0.6667 0.5 1 0.5 0 0 0 0\n")
        # The close mics are armed, the way a kit is before a take.
        # Folders are not — you do not record a bus — and neither are
        # the triggers and returns. Left unarmed across the board the
        # record arm was a ring that never lit, which is a control you
        # cannot tell from an ornament.
        armed = 0 if is_folder or is_auxiliary(name) else 1
        # REAPER's second REC field is the input: a small integer is a
        # mono hardware input, counted from zero. 5088 was a value
        # copied from a real project and it decodes to nothing useful,
        # so every armed track's input field read "No input" — a label
        # that says the fixture forgot rather than that the track has
        # none.
        source = 0 if is_folder or is_auxiliary(name) else (i % 16)
        # One track selected, because a session is never opened with
        # nothing in focus — and because selection is what opens a mic's
        # strip to a working width in the Tone sub-mode. The kick's In is
        # the track you are on when you start on a kit.
        out(f"    SEL {1 if i == selected else 0}\n    REC {armed} {source} 1 0 0 0 0 0\n")
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
