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
def node(name, colour, children=(), **opts):
    """A track with routing options — see `flatten` for what they mean."""
    return (name, colour, list(children), opts)


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
PROCESS = 0x7A2E3A  # garnet: the kit's family, darker — what is fed off it

# Bus colours: a cool slate for the bus tree, so it reads as
# plumbing rather than as another instrument.
BUS = 0x3E4C5E
BASS = 0x6B8E3F
# The instruments that are not drums, a voice or a bass, in the order
# they sit in the session and on the hue wheel: electrics blue,
# acoustics the seafoam between, keys green, synths past them.
ELECTRIC = 0x3A6FB5
ACOUSTIC = 0x3FA9A0
KEYS = 0x4E9A55
SYNTHS = 0x8CAA3A
VOX = 0xB04A6A

# An instrument folder is the VCA lead of its bus, with mute and solo
# along for the ride: the folder's fader is the instrument's fader,
# at the bus. Electrics are group 1, acoustics group 2.
def vca_lead(group):
    return {"vca_lead": group, "mute_lead": group, "solo_lead": group}


def vca_follow(group):
    return {"vca_follow": group, "mute_follow": group, "solo_follow": group}


def pair(name, colour, **opts):
    """A stereo pair: ONE stereo track, not a folder over an L and an R.

    Two channels on one track, recording from a stereo input, with the
    processing on it — a pair is one instrument, and its halves are
    almost never processed apart."""
    return node(name, colour, [], stereo=True, **opts)


def to_bus(name, colour, children, bus):
    """An instrument folder that reaches the mix by a send to its bus."""
    return node(name, colour, children, send=bus)


TREE = [
    node(
        "Drum Kit",
        DRUMS,
        [
            summed("Kick", KICK, ["In", "Out", "Trig"], ["Sub", "Verb"]),
            # The snare's verb is a folder of three: a short one, a long
            # one and a nonlin — the three rooms a snare is put in, so
            # the one the song wants is a mute away rather than a
            # patch change.
            (
                "Snare",
                SNARE,
                [
                    ("Sum", SNARE, mics(SNARE, "Top", "Bottom", "Trig") + [("Fund", SNARE, [])]),
                    ("Verb", SNARE, [("Short", SNARE, []), ("Long", SNARE, []), ("Nonlin", SNARE, [])]),
                ],
            ),
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
                # Two stereo pairs: the room, and the far room.
                [
                    pair("Rooms", ROOMS),
                    pair("Rooms Far", ROOMS),
                ],
            ),
        ],
        send="DRUM BUS",
    ),
    # What the kit is sent to: a Compress folder of parallel
    # compressors from dry to crushed, and an FX folder with a fake
    # room for when there are no room mics or bad ones and a bank of
    # reverbs to pick the room the band is in — short and bright down
    # to long and dark, plus the odd ones.
    node(
        "Process",
        PROCESS,
        [
            (
                "Compress",
                PROCESS,
                # A balance group (see session-daw's `balance`): the
                # Dry is the kit uncompressed, and the four are its
                # colours; bring one up and the others come down.
                [("Dry", PROCESS, []), ("Tight", PROCESS, []), ("Punch", PROCESS, []), ("Smash", PROCESS, []), ("Crunch", PROCESS, [])],
            ),
            (
                "FX",
                PROCESS,
                [
                    ("Room Sim", PROCESS, []),
                    (
                        "Verb",
                        PROCESS,
                        [
                            ("Wood Room", PROCESS, []),
                            ("Music Club", PROCESS, []),
                            ("Stadium", PROCESS, []),
                            ("RMX 16", PROCESS, []),
                            ("Nonlin", PROCESS, []),
                            ("Brick Wall", PROCESS, []),
                        ],
                    ),
                ],
            ),
        ],
        send="DRUM BUS",
    ),
    # Bass, maximally: a bass guitar (DI and amp, summed) and a synth
    # bass (its sub and the synth), to the bass bus.
    to_bus(
        "Bass",
        BASS,
        [
            summed("Guitar", BASS, ["DI", "Amp"]),
            ("Synth", BASS, [("Sub", BASS, []), ("Synth", BASS, [])]),
        ],
        "BASS BUS",
    ),
    # Guitars take a different shape every song, so there are no Sum
    # folders: each part is a track, and each part goes to one of the
    # three electric buses — RHYTHM, LEAD, SOLO — by a send, which is
    # what makes stems of the three fall out and lets a part move
    # between them mid-song by automating the sends.
    #
    # Electrics and acoustics are their own instruments, with their
    # own buses. Each folder's fader still does something: its bus
    # sends back into the folder — so the folder meters the instrument
    # after its bus — and the folder is the VCA lead of that bus, so
    # turning the folder down turns the instrument down at the bus.
    # The folders are dead ends. A stereo pair — Rhythm — is a folder
    # over L and R with the processing on the folder.
    node(
        "Electric",
        ELECTRIC,
        [
            pair("Rhythm", ELECTRIC, send="GTR RHYTHM"),
            node("Lead", ELECTRIC, send="GTR LEAD"),
            node("Solo", ELECTRIC, send="GTR SOLO"),
        ],
        no_parent=True,
        group=vca_lead(1),
    ),
    node(
        "Acoustic",
        ACOUSTIC,
        [
            node("Steel", ACOUSTIC, send="ACOUSTIC BUS"),
            node("Nylon", ACOUSTIC, send="ACOUSTIC BUS"),
            # High-strung, doubling the steel an octave up: a layer,
            # not a second guitar, so it sits with the steel it doubles.
            node("Nashville", ACOUSTIC, send="ACOUSTIC BUS"),
        ],
        no_parent=True,
        group=vca_lead(2),
    ),
    to_bus(
        "Keys",
        KEYS,
        [
            pair("Piano", KEYS),
            ("Rhodes", KEYS, []),
            ("Organ", KEYS, []),
        ],
        "KEYS BUS",
    ),
    to_bus(
        "Synths",
        SYNTHS,
        [
            ("Pad", SYNTHS, []),
            ("Lead Synth", SYNTHS, []),
            ("Arp", SYNTHS, []),
        ],
        "KEYS BUS",
    ),
    # One set of effects for everything that is not drums or a voice —
    # electric and acoustic guitars, piano, keys, synths, strings if
    # there are any — the way a mixer's template keeps them: rooms to
    # sit an instrument in without changing it, plates bright to dark,
    # halls short to endless, springs for guitars, delays slap to
    # long, and movement. Every return is a slot: its presets are
    # takes on the one job.
    to_bus(
        "Inst FX",
        0x5C6B7A,
        [
            ("Ambience", 0x5C6B7A, [("Short Room", 0x5C6B7A, []), ("Slap Room", 0x5C6B7A, []), ("Early", 0x5C6B7A, [])]),
            ("Plate", 0x5C6B7A, [("Fat Plate", 0x5C6B7A, []), ("Dark Plate", 0x5C6B7A, []), ("Gold Plate", 0x5C6B7A, [])]),
            ("Hall", 0x5C6B7A, [("Large Hall", 0x5C6B7A, []), ("Vienna", 0x5C6B7A, []), ("Atmosphere", 0x5C6B7A, [])]),
            ("Spring", 0x5C6B7A, [("Big Sky", 0x5C6B7A, []), ("XL35", 0x5C6B7A, [])]),
            ("Delay", 0x5C6B7A, [("Slap", 0x5C6B7A, []), ("Tape", 0x5C6B7A, []), ("Echo Boy", 0x5C6B7A, []), ("Space Echo", 0x5C6B7A, [])]),
            ("Mod", 0x5C6B7A, [("Chorus", 0x5C6B7A, []), ("Flanger", 0x5C6B7A, [])]),
        ],
        "INST BUS",
    ),
    to_bus(
        "Vocals",
        VOX,
        [
            summed("Lead", VOX, ["Close", "Room"], ["Verb"]),
            ("Doubles", VOX, []),
            ("Harmonies", VOX, []),
        ],
        "LEAD VOX BUS",
    ),
    # The bus tree — the dynamic template's canonical one (see
    # `dynamic_template::buses`): group buses into a stem bus, stem
    # buses into the mix. The three electric-guitar buses sit under
    # ELECTRIC BUS, in the bus list rather than in the Guitars folder.
    node(
        "MIX BUS",
        BUS,
        [
            node(
                "INST BUS",
                BUS,
                [
                    node("DRUM BUS", DRUMS, bus=True),
                    node("BASS BUS", BASS, bus=True),
                    # Each guitar bus goes on to INST BUS and ALSO back
                    # to its folder, which meters it and leads it.
                    node(
                        "ELECTRIC BUS",
                        ELECTRIC,
                        [
                            node("GTR RHYTHM", ELECTRIC, bus=True),
                            node("GTR LEAD", ELECTRIC, bus=True),
                            node("GTR SOLO", ELECTRIC, bus=True),
                        ],
                        bus=True,
                        send="Electric",
                        keep_parent=True,
                        group=vca_follow(1),
                    ),
                    node(
                        "ACOUSTIC BUS",
                        ACOUSTIC,
                        bus=True,
                        send="Acoustic",
                        keep_parent=True,
                        group=vca_follow(2),
                    ),
                    node("KEYS BUS", KEYS, bus=True),
                ],
                bus=True,
            ),
            node(
                "VOX BUS",
                BUS,
                [node("LEAD VOX BUS", VOX, bus=True), node("BGV BUS", VOX, bus=True)],
                bus=True,
            ),
        ],
        bus=True,
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
    """Depth-first, carrying each track's nesting level, its role and
    its routing.

    A node is `(name, colour, children)` or, from `node()`, the same
    with an options dict:

    - `send="X BUS"`: an explicit send to that bus, and the parent send
      OFF — the track reaches the mix through the bus, not the folder.
    - `keep_parent=True` with `send`: the parent send stays ON as well,
      so the folder above still sums (and meters) what it holds while
      the audio that reaches the mix goes by the send. The folder is
      then made a dead end (`no_parent`) so nothing is heard twice.
    - `no_parent=True`: the parent send OFF and no send — a dead end
      whose fader and meter are still real, for a folder that is a
      group lead rather than a sum point.
    - `group={...}`: REAPER track-grouping flags, by field name.
    - `bus=True`: a bus — no items, not armed.
    """
    if out is None:
        out = []
    for entry in nodes:
        name, colour, children = entry[0], entry[1], entry[2]
        opts = entry[3] if len(entry) > 3 else {}
        piece = is_piece(name, children, parent_is_piece)
        out.append((name, colour, depth, bool(children), piece, opts))
        flatten(children, depth + 1, out, piece)
    return out


# REAPER's GROUP_FLAGS fields, in the order REAPER writes them. Each is
# a bitmask of groups: group 1 is bit 0.
#
# Seven lead/follow pairs come FIRST — volume, pan, mute, solo, rec-arm,
# polarity, automation mode — then the reverse and no-lead flags, and
# width only at 19/20. That is not the order the grouping dialog lists
# them in, so it was checked against a project REAPER itself saved
# (helgobox/resources/test-projects/issue-45-grouping-vca-test.RPP: a
# lead of everything writes `1 0 1 0 1 0 1 0 1 0 1 0 1 0 0 0 0 0 1`),
# and it is the order daw-standalone's `decode_grouping` reads. Width
# at 5/6 shifted mute, solo, rec-arm, polarity and automode by two, so
# a folder meant to be mute/solo lead of its bus came out solo lead and
# rec-arm lead instead. `test_template_rpp.py` pins it.
GROUP_FIELDS = [
    "volume_lead", "volume_follow", "pan_lead", "pan_follow", "mute_lead", "mute_follow",
    "solo_lead", "solo_follow", "recarm_lead", "recarm_follow", "polarity_lead", "polarity_follow",
    "automode_lead", "automode_follow",
    "volume_reverse", "pan_reverse", "no_lead_when_following", "width_reverse",
    "width_lead", "width_follow",
    "vca_lead", "vca_follow", "vca_prefx_follow", "media_edit_lead", "media_edit_follow",
]


def group_flags(group):
    """The GROUP_FLAGS line for a `group=` dict of field -> group number."""
    fields = [0] * len(GROUP_FIELDS)
    for field, number in group.items():
        fields[GROUP_FIELDS.index(field)] |= 1 << (number - 1)
    return " ".join(str(f) for f in fields)


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

    widths = [strip_width(name, folder, piece) for name, _, _, folder, piece, _ in kit]
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

    # The ruler's lanes — REAPER 7.62's — the way an FTS session keeps
    # them: lane 1 is the SONG, one region over the whole song; lane 2
    # the SECTIONS; lane 3 the MARKS. A region is a MARKER line with
    # flag 1 and a closing line at its end; the lane is the last field.
    out('  RULERLANE 1 8 "SONG" 0 -1\n  RULERLANE 2 8 "SECTIONS" 0 -1\n  RULERLANE 3 8 "MARKS" 0 -1\n')
    marker_id = 1

    def region(name, start_bar, end_bar, lane, colour=0):
        nonlocal marker_id
        out(f'  MARKER {marker_id} {start_bar * SECS_PER_BAR:.6f} "{name}" 1 {colour} 1 B {guid()} 0 {lane}\n')
        out(f'  MARKER {marker_id} {end_bar * SECS_PER_BAR:.6f} "" 1\n')
        marker_id += 1

    def mark(name, bar, lane, colour=0):
        nonlocal marker_id
        out(f'  MARKER {marker_id} {bar * SECS_PER_BAR:.6f} "{name}" 0 {colour} 1 B {guid()} 0 {lane}\n')
        marker_id += 1

    def native(rgb):
        red, green, blue = (rgb >> 16) & 0xFF, (rgb >> 8) & 0xFF, rgb & 0xFF
        return 0x1000000 | (blue << 16) | (green << 8) | red

    region("SONG", 0, BARS, 1)
    for name, start, end, colour in [
        ("Intro", 0, 4, 0x4A6FA5),
        ("Verse 1", 4, 12, 0x3FA9A0),
        ("Chorus 1", 12, 20, 0xC94540),
        ("Verse 2", 20, 28, 0x3FA9A0),
        ("Chorus 2", 28, 36, 0xC94540),
        ("Bridge", 36, 44, 0x8CAA3A),
        ("Chorus 3", 44, 56, 0xC94540),
        ("Outro", 56, 64, 0x4A6FA5),
    ]:
        region(name, start, end, 2, native(colour))
    mark("SONGSTART", 0, 3)
    mark("SOLO", 40, 3)
    mark("SONGEND", BARS, 3)

    # Mixer strip widths, in the project's extension block — where
    # REAPER keeps what it does not model, and REAPER does not model
    # this. The same tracks that open at the minimum HEIGHT in the panel
    # open at the minimum WIDTH in the mixer: a trigger needs the same
    # amount of attention in both views, which is very little.
    narrow = [
        f"{g}={strip_width(name, is_folder, piece)}"
        for g, (name, _, _, is_folder, piece, _) in zip(guids, tracks)
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
        (i for i, (name, _, _, _, _, _) in enumerate(tracks) if name == "Kick"),
        next(
            (
                i
                for i, (name, _, _, is_folder, piece, _) in enumerate(tracks)
                if not is_folder and not piece and not is_auxiliary(name)
            ),
            -1,
        ),
    )

    # Who receives from whom: a send is written on the RECEIVING track
    # in REAPER, as the index of the track it comes from.
    index_of = {name: i for i, (name, *_rest) in enumerate(tracks)}
    receives = {}
    for i, (_name, _c, _d, _f, _p, opts) in enumerate(tracks):
        if opts.get("send"):
            receives.setdefault(opts["send"], []).append(i)

    for i, (name, colour, depth, is_folder, _piece, opts) in enumerate(tracks):
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
        # Where the audio goes: the folder above, unless the track
        # reaches the mix by a send instead — see `flatten`.
        dead_end = opts.get("no_parent") or (opts.get("send") and not opts.get("keep_parent"))
        out(f"    MAINSEND {0 if dead_end else 1} 0\n")
        if opts.get("group"):
            out(f"    GROUP_FLAGS {group_flags(opts['group'])}\n")
        for src in receives.get(name, []):
            out(f"    AUXRECV {src} 0 1 0 0 0 0 0 0 -1:U 0 -1 ''\n")
        out("    SHOWINMIX 1 0.6667 0.5 1 0.5 0 0 0 0\n")
        # The close mics are armed, the way a kit is before a take.
        # Folders are not — you do not record a bus — and neither are
        # the triggers and returns. Left unarmed across the board the
        # record arm was a ring that never lit, which is a control you
        # cannot tell from an ornament.
        armed = 0 if is_folder or is_auxiliary(name) or opts.get("bus") else 1
        # REAPER's second REC field is the input: a small integer is a
        # mono hardware input, counted from zero. 5088 was a value
        # copied from a real project and it decodes to nothing useful,
        # so every armed track's input field read "No input" — a label
        # that says the fixture forgot rather than that the track has
        # none.
        source = 0 if is_folder or is_auxiliary(name) else (i % 16)
        # A stereo track records from a stereo pair of inputs, which
        # REAPER numbers from 1024: 1024 + the pair's first channel.
        if opts.get("stereo"):
            source = 1024 + (source & ~1)
            out("    NCHAN 2\n")
        # One track selected, because a session is never opened with
        # nothing in focus — and because selection is what opens a mic's
        # strip to a working width in the Tone sub-mode. The kick's In is
        # the track you are on when you start on a kit.
        out(f"    SEL {1 if i == selected else 0}\n    REC {armed} {source} 1 0 0 0 0 0\n")
        height = MIN_HEIGHT if is_auxiliary(name) else 0
        out(f"    TRACKHEIGHT {height} 0 0 0 0 0 0\n")

        # Audio only on the leaves. A folder's items are its children's,
        # and a bus has none of its own.
        if not is_folder and not opts.get("bus"):
            bar = random.randrange(0, 8)
            while bar < BARS:
                length = random.choice([4, 8, 8, 16])
                out("    <ITEM\n")
                out(f"      POSITION {bar * SECS_PER_BAR:.6f}\n")
                out(f"      LENGTH {length * SECS_PER_BAR:.6f}\n")
                out(f"      NAME \"{name} {bar // 4 + 1}\"\n")
                # Fades the way a comped session has them: a short one
                # in, a longer one out, in a shape or two — so the
                # arrangement has fades to draw and handles to grab.
                out(f"      FADEIN {random.choice([0, 0, 1, 2, 5])} {random.choice([0.02, 0.05, 0.25, 0.5]):.4f} 0 0 0 0 0\n")
                out(f"      FADEOUT {random.choice([0, 0, 1, 2, 5])} {random.choice([0.1, 0.5, 1.0, 2.0]):.4f} 0 0 0 0 0\n")
                out("      IGUID " + guid() + "\n      GUID " + guid() + "\n")
                out("    >\n")
                bar += length + random.choice([0, 0, 4])
        out("  >\n")

    out(">\n")


if __name__ == "__main__":
    main()
