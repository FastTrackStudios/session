# The Maximal Session

A reference session that covers every base the dynamic template has to
cover, so that a real session — which always has a different shape — can
be checked against it. `scripts/ui-stress/make-template-rpp.py` writes
it; `just daw-scene <slug>` renders each scene of it.

Drums can be made perfect once: a kit is a kit. Guitars take a different
shape every song, so for them the reference is a checklist rather than a
fixed tree.

## Checklist

### Drums — `Drum Kit/`, `Process/`

- [x] Every piece a folder: **Kick, Snare, Toms, Cymbals, Rooms**, in one
      red family (folder crimson, pieces oxblood → scarlet → terracotta →
      rose → wine).
- [x] Each piece's close mics under a **Sum**; the piece's sends beside
      the Sum, not inside it.
- [x] **Kick**: In, Out, Trig; Sub; Verb.
- [x] **Snare**: Top, Bottom, Trig, Fund; **Verb/** Short, Long, Nonlin.
- [x] **Toms**: per tom T*n*, Trig, Fund; one shared Verb.
- [x] **Cymbals**: OH, Hi-Hat, Ride. **Rooms**: two stereo pairs, Rooms
      and Rooms Far.
- [x] One-note tracks (Fund, Sub) get Gate → band-pass at the fundamental
      → Sat; triggers get nothing.
- [x] **Process/Compress/**: Dry, Tight, Punch, Smash, Crunch — a balance
      group (`session-daw::balance`): one fader up, the others down by the
      same amount between them; a fader at silence is out.
- [x] **Process/FX/**: Room Sim (captured rooms, compressed fast on the
      way back) and a **Verb** bank: Wood Room, Music Club, Stadium,
      RMX 16, Nonlin, Brick Wall.
- [x] The kit and Process reach the mix by a send to **DRUM BUS**.
- [x] Scenes: Tracking, Mixing, Overview (pieces collapsed), Advanced,
      FX.

### Bass — `Bass/`

- [x] **Guitar/** DI, Amp (summed).
- [x] **Synth/** Sub, Synth.
- [x] To **BASS BUS** by send.
- [ ] Upright / DI-only variants when a song has them.

### Guitars — `Electric/`, `Acoustic/` (a different shape every song)

- [x] **No Sum folders**: each part is a track named for the part
      (Rhythm, Lead, Solo, …). A **stereo pair is one stereo track** —
      two channels, a stereo input — not a folder over an L and an R:
      its halves are almost never processed apart.
- [x] Electrics and acoustics are **separate top-level folders with
      their own buses** — no Guitars folder, no GUITAR BUS.
- [x] Every electric part goes to exactly one of **GTR RHYTHM, GTR LEAD,
      GTR SOLO** by a send — the three live under **ELECTRIC BUS** in the
      bus list, not in the folder. A part moves between them mid-song by
      automating the sends. Stems are the three buses.
- [x] **ELECTRIC BUS sends back into the Electric folder**, which is a
      dead end that meters the electrics after their bus, and is the
      **VCA lead of ELECTRIC BUS** (group 1, with mute and solo): the
      folder's fader is the electrics' fader. No VCA track.
- [x] **Acoustic/** Steel, Nylon, Nashville (the high-strung layer over
      the steel) → **ACOUSTIC BUS**, which returns to the Acoustic folder
      the same way (group 2).
- [ ] Per-part amp/DI pairs when both were captured (a part folder of
      two mics is the one case a Sum is right).

### Keys — `Keys/`, Synths — `Synths/`

- [x] Keys: Piano (a stereo track), Rhodes, Organ → **KEYS BUS**.
- [x] Synths: Pad, Lead Synth, Arp → **KEYS BUS**.
- [x] Colours, in session order and hue order: electrics blue,
      acoustics seafoam, keys green, synths lime.
- [ ] Clav, strings and pads as the song has them.

### Instrument FX — `Inst FX/` (one set for guitars, keys, synths, strings)

- [x] **Ambience**: Short Room, Slap Room, Early.
- [x] **Plate**: Fat, Dark, Gold. **Hall**: Large, Vienna, Atmosphere.
- [x] **Spring**: Big Sky, XL35. **Delay**: Slap, Tape, Echo Boy, Space
      Echo. **Mod**: Chorus, Flanger.
- [x] To **INST BUS**.
- [ ] Sends from the delay returns into the reverbs ("reverb the delay").

### Vocals — `Vocals/`

- [x] Lead (Close, Room, Verb), Doubles, Harmonies → **LEAD VOX BUS**.
- [ ] BGVs folder → **BGV BUS**; the vocal FX template (`make-vocal-fx-rpp.py`)
      folded in.

### Buses — `MIX BUS/`

- [x] `MIX BUS / INST BUS / {DRUM, BASS, ELECTRIC/{GTR RHYTHM, GTR LEAD,
      GTR SOLO}, ACOUSTIC, KEYS}` and `VOX BUS / {LEAD VOX, BGV}` —
      `dynamic_template::buses`' tree without GUITAR BUS, with the three
      electric buses added.
- [x] A bus is a track with no items, unarmed; its chain is EQ → Comp.
- [x] Scene: **Buses** — the tree alone.
- [ ] Monitor buses beside MIX BUS (Click + Guide, Headphones, Talkback,
      Utility).

## Routing rules the reference follows

1. An instrument folder reaches the mix by a **send to its bus**, parent
   send off (`to_bus`).
2. Where a folder's fader must still be the instrument's fader — the
   electrics, the acoustics — its bus sends back into it, the folder is
   a **dead end** that meters that return, and it is the **VCA lead** of
   the bus.
3. A **stereo pair** is one stereo track (`pair()` in the template:
   `NCHAN 2`, a stereo input). A folder over an `L` and an `R` is still
   read as one instrument by the rack (`tone::is_pair`) for sessions
   that arrive that way, with the halves as rails carrying nothing.
4. Buses nest as folders inside the bus they feed; a bus reaches its
   parent by the ordinary folder send.
5. A balance group is a folder named `Compress…`.
