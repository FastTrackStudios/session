# Mixing Phases — Modes, Sub-Modes, and Visibility States

> Status: design draft (2026-06-04). Captures the phase concept so each phase
> can be refined individually — purpose, what you do in it, and what the
> session should *show* while you're in it.

## The idea

Modes (Record, Edit, Organize, Mix, …) are too coarse on their own. Inside a
mode there are **phases** — repeatable steps of the craft, each with its own
visibility preferences, tool set, and "what am I looking at" answer. We
already do this informally: Tempo Mapping is effectively a phase inside
Organize. This doc makes that pattern first-class for **Mix Mode**.

Key properties:

- **Phases are not strictly ordered.** There's a natural progression, but
  several can be active in one session, and you bounce between them. The
  system just needs fast switching (which-key / mode bar), not a wizard.
- **Each phase drives the visibility manager.** A phase is largely *a saved
  answer to "which tracks do I need to see, in TCP and MCP, at what level of
  detail."*
- **Phases have steps in between.** Like Tempo Mapping inside Organize, a
  phase may carry its own sub-steps. The structure is: Mode → Phase → Step.
- **MCP has two density states** that phases select between:
  - **Overview** — every top-level instrument folder collapsed to one strip
    (Kick, Snare, Toms, Cymbals, Rooms read as 5 strips; Drums could even be
    one). Bus-level decision making.
  - **Detailed** — everything in the active scope expanded: individual mics,
    SUMs, Fund/Verb/Parallel utility tracks.

## Phase list (working order)

`STAGING → RESCUE → BALANCE → TONE → POLISH → MIX → DEPTH → AUTOMATE → OVERVIEW`

Order is a default mental model, not a constraint.

| Phase | Working name(s) | Purpose | Typical visibility |
|---|---|---|---|
| **STAGING** | Gain Staging | Healthy levels into the chain; input gain, clip gain, phase/polarity, source cleanup | **All source tracks** visible & detailed; buses/FX/parallel hidden |
| **RESCUE** | Rescue EQ | Surgical repair — resonances, rumble, bleed, de-ess; fixing problems, not shaping tone | Source tracks detailed (like STAGING); spectrum-heavy workflow |
| **BALANCE** | Volume Balance | Static balance pass — faders only, full picture | **All tracks** visible; MCP detailed; volume-balancer groups active |
| **TONE** | Tonal EQ | Broad-stroke tone shaping per instrument | All tracks visible (same as BALANCE) |
| **POLISH** | Fix Dynamics → Enhance Dynamics | Two halves: fixing dynamics problems (comp for control) then enhancing (transients, parallel comp, saturation) | Instrument level; utility tracks (Parallel) become relevant |
| **MIX** | Relational EQ | Carving instruments against each other; masking decisions | **Overview density** — individual mics no longer needed; bus/instrument strips |
| **DEPTH** | Depth Matrix | Front-to-back placement: reverbs, delays, spatial sends | **All FX/verb/send channels visible** (Snare Verb, FX/Drum Verb, room mics); sources can collapse |
| **AUTOMATE** | Automation | Movement: rides, mutes, section contrast | Automation lanes; tracks-with-automation filter; arrange-centric |
| **OVERVIEW** | Mix Bus Processing / Print | Top-down listen, mix-bus chain, referencing, print | **Maximum collapse** — MIX BUS + top-level buses only |

### Per-phase notes (to refine)

#### STAGING — Gain Staging
- Show: every *source* track (things that record/contain audio). Hide: buses,
  FX returns, Parallel, Fund, Verb utility tracks.
- Candidate steps: input trim pass → polarity/phase check → clip-gain ride.
- Tooling hooks: peak/RMS readouts, normalize-to-target action.

#### RESCUE — Rescue EQ
- Same visibility as STAGING (you work the same tracks), different tools.
- Candidate steps: HP/LP pass → resonance hunt → bleed control.

#### BALANCE — Volume Balance
- Everything visible; this is where the **volume balancer** groups earn their
  keep (Parallel set, multi-mic SUMs).
- Candidate steps: static fader pass (mono?) → panorama pass.

#### TONE — Tonal EQ
- Full visibility like BALANCE; per-instrument tonal moves.

#### POLISH — Dynamics (fix → enhance)
- Two sub-steps explicitly: *Fixing Dynamics Issues* and *Enhancing
  Dynamics*. Parallel tracks (Clean/Tight/Punch/Smash/Crush) come into play.

#### MIX — Relational EQ
- Preference shift: stop showing individual mics — Overview density. Work at
  instrument/bus strips. Masking pairs (kick↔bass, vox↔guitars).

#### DEPTH — Depth Matrix
- Show all reverb/delay/send channels: Snare Verb (Short/Long), FX/Drum Verb,
  Rooms, future verb buses. Sources can collapse.
- The "matrix" framing: each element gets a depth assignment; the visible set
  is the *depth infrastructure*.

#### AUTOMATE — Automation
- Arrange-view-centric; show tracks with automation, envelope lanes.
- Candidate steps: section rides → lead rides → spot mutes/fills.

#### OVERVIEW — Mix Bus / Print
- MIX BUS chain processing, loudness/reference checks, print/render.
- Visibility: just the bus skeleton (MIX BUS, INSTRUMENTAL BUS, vocal buses).

## Relationship to existing systems

- **Visibility manager**: each phase maps to a visibility profile (like the
  existing `VISIBILITY_PROFILE_DRUM_EDITING` / `MIDI_EDITING`). Phases are
  the natural unit for saved TCP/MCP show-hide-collapse state. The
  overview/detailed MCP density is part of that profile.
- **Mode system**: Mix Mode hosts these phases the way Organize hosts Tempo
  Mapping. Mode bar / which-key should expose: enter Mix Mode → pick phase →
  (optional) step within phase.
- **Track taxonomy**: phases reference track *roles* (source, SUM, utility,
  FX/verb, bus, parallel) — the golden-template taxonomy already encodes
  most of this; visibility rules should be written against roles, not names.
- **Volume balancer**: BALANCE phase is where constant-sum groups are
  primarily active/exposed.

## Open questions

- Naming: POLISH vs. splitting Fix/Enhance Dynamics into two phases?
- Does OVERVIEW belong at the end, or is it a *density* you can apply inside
  any phase? (Current lean: both — it's a phase for the mix-bus work, and
  overview/detailed is an orthogonal MCP density toggle.)
- Keybinding shape: `M` enters Mix Mode; number keys 1–9 for phases? Or a
  which-key submenu mirroring the phase order?
- Per-phase FX-chain focus (auto-open the relevant FX window kind)?
- Where do per-phase *steps* live — hardcoded per phase, or user-defined
  workflows (styx config) like the existing workflow overlays?

## Next actions

1. Define track-role → visibility mapping per phase (table above, made
   precise against the taxonomy).
2. Add phase state to the mode system (Mix Mode sub-modes) + which-key.
3. Implement MCP overview/detailed density toggle in the visibility manager
   (B_SHOWINMIXER + folder compact in MCP via BUSCOMP chunk edit).
4. Wire 2–3 phases end-to-end first: STAGING, BALANCE, OVERVIEW — they have
   the clearest visibility semantics.
