# Two machines, one session: recordings and stills

Media for the session work (FastTrackStudios/session), from runs of real
Architect instances on two machines:

- **A**: voyager, macOS. It hosts the session.
- **B**: THEBATTLESHIP, Linux. It joins over the LAN.

Both run `architect-desktop` (`runtime-compositor` `51e50333`; for the
one-window picture, A runs `c2ac7fe`, the same line with macOS process views)
with development copies of Keyflow and of Ignition's Lighting extension.
Nothing here is mocked. Input and transport were driven through the same
actions the keys and buttons run, over each app's control socket, and every
step's result was read back from both machines (the logs).

No branch of code lives here; this is media only.

## `one-window-chart-room-cues/`: the picture

**One Architect window per machine.** Each holds:
- **Keyflow's chart** of "Drive To LA" (JT's recital);
- **Ignition's 3D room**, a process view drawn by Ignition's renderer;
- **Ignition's cue list** for the song.

All of it follows **one session**, playing. A (voyager, macOS) hosts the
session, and its window draws the room over IOSurface. B (THEBATTLESHIP,
Linux) has joined over the LAN, and its window draws the room from shared
memory.

The song is in the session through `fts.session`, as `ignition`:
- its 22 cues, resolved against the recital's Ableton Live set as the show
  resolves them;
- the set's tempo (160 bpm) as the session's tempo map.

The chart is Keyflow's data in the same session.

A's transport (A's `KEYFLOW_SESSION_PLAY`, the session host's clock, which B
estimates) does this:
1. plays from 0:48 into the first chorus;
2. seeks to 2:30;
3. stops.

On both machines the cue list lights the cue the playhead is in and the
room's lights follow:
- cue 5 "Chorus" at 0:54;
- cue 6 "· Chorus lift" at 1:00;
- after the seek, cue 15 "Bridge" (cue 16 is at 2:36).

| file | what it shows |
|---|---|
| `voyager-A.mp4` | A's window, 34 s, macOS `screencapture -v` (30 fps) |
| `thebattleship-B.mp4` | B's window, the same 34 s, KWin's `spectacle -a` (about 5 fps) |
| `A-voyager-playing-chorus.png`, `B-thebattleship-playing-chorus.png` | each window in the chorus, cue 5 lit |
| `A-moments.png`, `B-moments.png` | stopped, in the chorus, at the lift, after the seek |
| `transport.log` | A's transport commands and their times |

**Not in this window: Studio's arrangement view and mixer**
(`feat/arrangement-mixer-runtime-extensions`, studio#519). They run on
another Architect revision (`runtime-compositor-arrangement`), with their
own `daw` engine providing `fts.session` for its own session (the song
imported from the recital's REAPER project). In this window they would
follow their own engine, not this session. One app has one provider of
`fts.session`, and their engine does not yet publish its playhead to
presence. So they were not opened here.

## `typing-on-two-machines/`: one chart, typed into from both sides

Keyflow's chart lives in the session, Keyflow's data there, not a `.kf` file.
This run is keyflow#93.

1. A types chords in **syntax mode** into bars 2–3. They appear on B.
2. A enters **notes by note name** (C E G C) into bar 6. They appear on B as
   notes.
3. **Both type at once**: A into bar 8, B into bar 12. Both copies end
   identical, with both edits.
4. **A undoes**: only A's bar 8 goes back. B's bar 12 stays.
5. **B undoes**: only B's bar 12 goes back.

| file | what it shows |
|---|---|
| `voyager-A.mp4` | A's window through the run (62 s, the app's own frames, about 3 fps) |
| `thebattleship-B.mp4` | B's window through the same run (62 s, about 9 fps) |
| `A-both-edits-converged.png`, `B-both-edits-converged.png` | both windows at the moment the simultaneous edits converged: `B♭ C` in bar 8 (A's) and `Am Am/G` in bar 12 (B's) on both |
| `run.log` | the script's log: each step, and both machines' session text compared after it |

## `cues-follow-transport/`: Ignition's cues taken from the session's transport

Ignition's cue list for "Bye Bye Bye" is stored in the session through
`fts.session`, as the `ignition` extension (43 cues). On B it is followed from
A's transport, which is published in presence and stamped in A's clock (B
estimates A's clock). This run is Ignition#16/#18.

| file | what it shows |
|---|---|
| `A-voyager-chart.png`, `B-thebattleship-chart.png` | the session's chart engraved on both machines |
| `cues-taken.json` | what B's follower took and when, in session seconds: cues at 4.014, 12.009 and 20.005 s; a seek to 60 s landing on cue 13; cue 14 at 62.008 s; nothing after the stop. The clock round trip was 1.5 ms, so B's estimate of A's clock is within ±0.75 ms. |

No video was made of this run.

## `room-follows-transport/`: Ignition's 3D room beside the chart, following the session

On B, one Architect window holds the session's Keyflow chart ("Drive To LA",
from the recital) and Ignition's 3D room, a process view, side by side. The
room plays the recital's Drive To LA, its clock being the session host's
transport (`--clock session:…`, Ignition#19). A plays, seeks and stops from
the other machine.

| file | what it shows |
|---|---|
| `thebattleship-room-and-chart.mp4` | B's window (66 s, captured by the compositor, about 4.5 fps): dark until play; the intro wash; the 60 s seek's looks; held at the stop; the 30 s look after a stopped seek |
| `room-moments.png` | the room at each of those moments |
| `B-chart-and-room-playing.png` | the window just after the seek to 60 s |
| `transport.log` | A's transport commands and their times |

## What is not real yet

- **The transport** is the session host's software clock, not the DAW engine.
- **Sync** goes through a WebSocket host that the Keyflow extension starts, not
  through the org. Org sync for extension document kinds is in progress in
  Architect.
- **A read-only share link's guest** is designed (architect#847), not built.
