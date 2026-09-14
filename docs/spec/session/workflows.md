# Workflows — everything editing and mixing a project has to be able to do

The master list. A project goes through tracking, comping, editing and
mixing, instrument by instrument, and this is every flow the session
window and the expression editor directly support — each one with a
scene that can be rendered, a gesture that can be driven, and a test
that says it works. When every box here is ticked, the product is
ready. The list is far larger than this; when *this* list is done, we
are in a very good spot.

Tracey prefix `flow`. Each rule is one thing a user does. A rule is
ticked when there is an `r[impl]` in the code and an `r[verify]` in a
test; `tracey query uncovered` lists what is left.

Builds on: `maximal-template.md` (the reference session every scene is
rendered against), `track-organization.md`, `routing-project.md`, and
the expression editor's `drum-mode.md` (`drums.*`) and its alignment
and quantize specs.

## Checklist

| Instrument | Tracking | Comping | Editing | Mixing | More |
|---|---|---|---|---|---|
| Drums (audio) | [x] full · [ ] overview | [ ] folder items | [ ] edit scene · [x] stack · [x] slip/stretch · [x] quantize · [ ] align hits | [x] scenes · [x] balance | [ ] triggering samples |
| Bass | [ ] | [ ] | [ ] | [ ] | |
| Guitars | [ ] | [ ] | [ ] doubles aligned | [x] buses/VCA | |
| Vocals | [ ] | [ ] | [ ] doubles aligned | [x] lead vocal scenes | [ ] tuning · [ ] fx · [ ] automation |
| Keys | [ ] record MIDI | [ ] | [x] MIDI in the editor | [ ] | |

## Views a flow is done in

Every flow is looked at through a **scene**: which tracks are shown in
the arrangement's panel (the TCP) and the mixer (the MCP), how large,
and which folders are folded. A scene is a rule resolved against the
session's own taxonomy — never a list of track ids — so it fits a real
session whose shape differs from the reference.

r[flow.scenes.source-track]
A **source track** is a track that records: it has a record input and
is armed, or is the kind of track the session records to (audio input
or MIDI input). Whether a track is a source is read from the track's
record settings, not from its name. A trigger track is a source only
when it is being recorded as audio; a trigger fed by a plugin or a MIDI
conversion is present in the session and is not a source.

r[flow.scenes.per-piece-trigger]
Whether a kit piece's trigger is recorded is decided **per session and
per piece**: a session may record a kick trigger and no snare trigger,
or a snare trigger and no tom triggers. The tracking scene follows that
decision — the trigger is a source track where it is recorded, and only
there. Nothing in the template forces it either way.

r[flow.scenes.render]
Every scene named in this spec renders headless from the reference
session (`just daw-scene <slug>`) and the render is a test fixture: a
change to what a scene shows is a change to a picture, not a surprise.

r[flow.scenes.follow-mode]
Scenes are the **visibility manager**, and the visibility manager
follows the DAW mode: entering **Record** shows the instrument's
tracking scene, **Edit** its editing scene, **Mix** its mixing scene,
without a second choice being made. A mode has a default scene per
instrument, the number keys still recall any scene inside a mode, and
a scene chosen by hand stays until the mode changes. Comping is not a
mode; its view lives inside Record and Edit (see
`flow.drums.comping.folder-items`), because a kit is comped while it
is still being tracked as often as afterwards.

r[flow.scenes.two-audiences]
A flow has views for two audiences where they differ: the engineer's
full view, with every track the flow touches, and the player's overview
— what the person performing needs to see and nothing else. The
number keys recall scenes in the window; the same scenes are recipes
in the mixer.

## Drums — audio

The kit is the one instrument that can be made perfect once: a kit is
a kit. Every drum flow is rendered against `Drum Kit/` and `Process/`
of the reference session.

### Tracking

r[flow.drums.tracking.full]
**Drum Tracking** shows every source track of the kit in the TCP and
the MCP — every close mic, every overhead and room, and every trigger
that is being recorded — at working size, with the kit's folders open
and everything else in the session collapsed: the Process folder and
the bus tree present at their minimum. This is the engineer's view
while the kit is being dialled in.

r[flow.drums.tracking.overview]
**Drum Tracking Overview** shows the kit as the drummer reads it: one
strip per piece — Kick, Snare, each tom, Hi-Hat, Ride, OH as one
stereo strip, Rooms as one stereo strip — each a folder collapsed to
one row that can be expanded when something needs a look. The maximal
form is

```text
Kick
Snare
Tom 1
Tom 2
Tom 3
Tom 4
Hi-Hat
Ride
OH      (stereo)
Rooms   (stereo)
```

Everything else in the session is collapsed or hidden. This is what
the drummer and the rest of the band look at once the kit is dialled
in and tracking is one part of a whole-band session.

r[flow.drums.tracking.arm]
Arming, input monitoring and input selection are on the strip in both
scenes — the rail's arm on a thin strip, the arm and the monitor lamp
on a working one — and arming a piece's folder arms its sources.

r[flow.drums.tracking.meters]
Every source strip meters its input while armed and its playback
otherwise, at the engine's meter rate, in the TCP and the MCP alike.

### Comping

Twenty takes of a kit are twenty items on the kick, twenty on the
snare, twenty on each tom, and the arrangement becomes impossible to
read. Comping is done on the folder instead.

r[flow.drums.comping.folder-items]
The **Drum Kit folder shows folder items**, the way
[nvk folder items](https://nvk.tools/docs/workflow/folder_items/) do:
one item per take on the folder's row, rendered from the kit's main
sources — the kick, the snare and the toms summed — so a take reads as
one item with one waveform rather than a stack of items across twenty
tracks. The folder item is a **view** of the take's items on the source
tracks underneath; it owns no audio of its own.

r[flow.drums.comping.folder-item-colours]
The folder item's summed waveform is drawn **per piece in the piece's
colour**, not as one grey sum: the kick's contribution in the kick's
hue, the snare's in the snare's, the toms' in the toms' — the role
colours the expression editor's lanes already use (`LaneRole::color`),
so the two views agree. Layered in one item, the kick pattern reads on
its own from across the room, the snare's backbeat sits between it,
and a fill is visibly the toms. That is what makes a take readable as
one item: the pattern, not just the loudness. The same colouring is
used wherever a folder item carries the kit's summed waveform — the
edit scene's rows and the tracking overview's collapsed pieces.

r[flow.drums.comping.folder-lanes]
With the kit in comping, the folder shows **one lane per take**, each
lane the take's folder item, and a **comp lane** on top: the comp is
built by choosing regions of the take lanes, and the comp lane shows
the result as one item. The source tracks are collapsed under the
folder while this is done; expanding a piece shows its own take lanes
for the case a single mic needs a different choice.

r[flow.drums.comping.lanes]
Every take of a source track is a **take lane** under it, shown in the
arrangement when the track is in comping; the kit's take lanes line up
across the pieces because the takes were recorded together.

r[flow.drums.comping.group]
A comp on the kit is made **as a group**: choosing a take's region on
the folder's comp lane chooses the same region on every source track
of the kit, so the kit is never cut between mics. A piece can be taken
out of the group deliberately (a fixed snare hit from another take) on
its own take lanes and put back.

r[flow.drums.comping.crossfade]
Comp boundaries get crossfades of a session-wide default length, drawn
on the items and draggable at the top corners the way any fade is
(`arrangement.rs` fades), and a boundary snaps to a hit's onset when
the kit's hits are detected.

### Editing

r[flow.drums.editing.scene]
**Drum Editing** shows the kit the way the expression editor folds it:
**one row per source piece** — one Kick, one Snare, one row per tom,
one per source track — with the mics under each piece collapsed into
it, the pieces' folder items carrying the piece's summed waveform in
the piece's colour (`flow.drums.comping.folder-item-colours`), and
the Process folder and the buses hidden. It is the scene Edit mode
shows for the kit, and the arrangement's counterpart of the stack's
lanes: the same fold, so what is selected in one is what is edited in
the other.

r[flow.drums.editing.stack]
Editing the kit is done in the expression editor's stack: every mic
folded into the kick, snare, toms and other lanes, the mics' audio
summed per lane, and the detected hits marked on it. The other lane is
heard, not detected — hats, cymbals and rooms carry no hit list and
take no hit gesture. Docked under the arrangement, the stack draws
through the same renderer as the arrangement at the same rate.

r[flow.drums.editing.hands]
A hit is moved by hand: slip, stretch, snap to grid, nudge by the
grid, add and remove — the gestures of `drums.manual.*` — with the
edit written to every mic of the kit as one undo step.

r[flow.drums.editing.quantize]
The kit is quantized as a group with the transient quantizer
(`drums.quantize.*`): a grid, a strength, fills protected, previewed
before it is applied.

r[flow.drums.editing.align-hits]
A take of the kit can be aligned to another performance's hits — the
reference take, a click, a programmed part — by the hit-alignment
engine, so a punched-in section sits on the take around it.

### Mixing

r[flow.drums.mixing.scenes]
Mixing the kit has its scenes: **Drum Mixing** (the pieces as the
instrument, every mic a rail), **Drum Advanced** (the subs, funds,
triggers and verbs open), **Drum FX** (the Process folder's compress
and verb banks), and **Drum Overview** (five faders). Each is a scene
in the window and a recipe in the mixer.

r[flow.drums.mixing.balance]
The parallel compressors are a **balance group**: one fader up brings
the others down by the same amount between them, and a fader at
silence is out of the group until it is raised.

r[flow.drums.mixing.racks]
Every kit track's rack shows what its phase does to it — the tone
chain with its meters, the compressors reducing, the de-essers firing
— live, at the frame rate, from the engine's meters.

### Triggering — adding samples

r[flow.drums.trigger.per-piece]
A sample is added **per piece**: a kick sample on the kick, a snare
sample on the snare, each tom its own, from the piece's detected hits
— never from the audio in the other lane.

r[flow.drums.trigger.from-hits]
The trigger track is rendered from the hit list — one sample per hit
at the hit's velocity — so a hit slipped in the editor moves its
sample with it, and a hit removed removes its sample.

r[flow.drums.trigger.blend]
The sample and the mics are blended on the piece's Sum, with the
sample's level and its polarity against the close mic controls of the
piece, and the blend is what the Drum Mixing scene mixes.

## Bass

r[flow.bass.tracking]
**Bass Tracking** shows the bass's source tracks — DI and Amp, or the
synth's — at working size with everything else collapsed.

r[flow.bass.comping]
The bass comps like a two-mic drum: DI and Amp as a group, take lanes
lined up, crossfades at the boundaries.

r[flow.bass.editing]
The bass is edited in the stack with its DI as the detection source:
its hits are moved and quantized like a kit's, and the DI and Amp move
together.

r[flow.bass.mixing]
**Bass Mixing** shows the bass folder and BASS BUS with its rack, the
DI and Amp as rails.

## Guitars

Guitars take a different shape every song, so every guitar scene is a
rule over parts, not a fixed tree (`maximal-template.md`).

r[flow.guitars.tracking]
**Guitar Tracking** shows every part being recorded — each part's
track, or its DI and amp pair where both are captured — at working
size, the electrics and the acoustics as separate folders.

r[flow.guitars.comping]
A part comps on its own take lanes; a DI and amp pair comps as a group.

r[flow.guitars.editing.doubles]
Doubled parts are **aligned**: the second take of a rhythm part is
retimed to the first by the alignment engine (`align.rs`), with a
reference and a dub chosen by the user, the alignment previewed as
warp markers, and applied as one undo step. This is the whole
alignment system applied to guitars; the editor's stack shows both
takes on one timeline while it is done.

r[flow.guitars.mixing]
**Guitar Mixing** shows the electric and acoustic folders, their buses
(GTR RHYTHM, GTR LEAD, GTR SOLO under ELECTRIC BUS; ACOUSTIC BUS) and
the Inst FX returns, with the folder's fader as the VCA lead of its
bus.

## Vocals

r[flow.vocals.tracking]
**Vocal Tracking** shows the lead and every background vocal being
recorded at working size, with the vocal FX returns present for the
singer's headphone mix and everything else collapsed.

r[flow.vocals.comping]
The lead vocal comps on take lanes, one take chosen per phrase, with
crossfades that snap to phrase boundaries.

r[flow.vocals.editing.doubles]
Doubles and harmonies are **aligned** to the lead: reference the lead,
dub the double, retime the double, previewed and applied as one undo
step — the same engine as the guitars, on vocal features.

r[flow.vocals.tuning]
The lead is **tuned** in the expression editor: the take analysed to
pitch, drawn as blobs with pitch curves, corrected by hand or snapped
to the key, and written back as the retuned audio — docked under the
arrangement like the kit's stack.

r[flow.vocals.mixing.main]
**Lead Vocal** shows the lead's chain — the close mic, its room and
verb returns — at working size with its rack, and the lead vocal bus.

r[flow.vocals.mixing.fx]
**Lead Vocal FX Edit** opens the vocal's returns — delays, verbs,
widener, pitch — as wide focused strips with their racks, so an effect
is dialled in with its display in front of the engineer.

r[flow.vocals.mixing.automation]
The vocal's level, sends and effect parameters are automated in the
arrangement: envelopes drawn on the track's lanes, points dragged,
written from a fader move, and shown per parameter.

## Keys

r[flow.keys.midi-editing]
A MIDI part is edited in the expression editor: the roll and the
velocity strip, notes drawn, moved, trimmed and deleted, velocity set,
the grid and snap from the session's tempo, docked under the
arrangement.

r[flow.keys.recording]
A MIDI part is recorded to an armed track from a MIDI input, with the
count-in and the click from the session, and the take appears in the
arrangement as it is played.

r[flow.keys.comping]
MIDI takes comp on take lanes like audio takes, and a comp of a MIDI
part is one MIDI item.

## What proves it

r[flow.verify.scenarios]
Every flow above has at least one scenario test that drives the real
gesture — a press, a drag, a key through the profile — against a
project and reads the result back from the project, not from the
picture.

r[flow.verify.frame-rate]
The studio benchmark — a 5120x1440 arrangement with the editor docked
and a 2560x1440 mixer, both every frame — measures the views these
flows are done in against 240 Hz, and a change that costs a frame
shows in it before it ships.
