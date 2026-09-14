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
- [x] **Cymbals**: OH, Hi-Hat, Ride. **Rooms**: Mono, Stereo L/R.
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

### Guitars — `Guitars/` (a different shape every song)

- [x] **No Sum folders**: each part is a track named for the part
      (Rhythm L, Rhythm R, Lead, Solo, …).
- [x] Every electric part goes to exactly one of **GTR RHYTHM, GTR LEAD,
      GTR SOLO** by a send — the three live under **ELECTRIC BUS** in the
      bus list, not in the folder. A part moves between them mid-song by
      automating the sends. Stems are the three buses.
- [x] The parts keep their parent send, so the **Electric** folder still
      sums and meters everything — and it is a dead end (parent send off)
      so nothing is heard twice.
- [x] The Electric folder **leads** the three buses through group 1:
      volume, mute and solo for RHYTHM and LEAD; mute and solo only for
      SOLO, so the guitars can come down in a song without the solo.
      No VCA track.
- [x] **Acoustic/** Steel, Nylon → **ACOUSTIC BUS** the same way.
- [ ] Per-part amp/DI pairs when both were captured (a part folder of
      two mics is the one case a Sum is right).

### Keys — `Keys/`

- [x] Piano (L/R summed), Rhodes, Pad → **KEYS BUS**.
- [ ] Organ, clav, synth leads as the song has them.

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

- [x] `MIX BUS / INST BUS / {DRUM, BASS, GUITAR/{ACOUSTIC, ELECTRIC/{GTR
      RHYTHM, GTR LEAD, GTR SOLO}}, KEYS}` and `VOX BUS / {LEAD VOX, BGV}`
      — `dynamic_template::buses`' tree, with the three guitar buses added.
- [x] A bus is a track with no items, unarmed; its chain is EQ → Comp.
- [x] Scene: **Buses** — the tree alone.
- [ ] Monitor buses beside MIX BUS (Click + Guide, Headphones, Talkback,
      Utility).

## Routing rules the reference follows

1. An instrument folder reaches the mix by a **send to its bus**, parent
   send off (`to_bus`).
2. Where the folder must still **meter** what it holds — the electric
   guitars — the children keep their parent send and the folder is a
   **dead end** that **leads** its buses' faders through a track group.
3. Buses nest as folders inside the bus they feed; a bus reaches its
   parent by the ordinary folder send.
4. A balance group is a folder named `Compress…`.
