# session

The primitive FastTrackStudio's music software agrees on: the **song
structure** (tempo map, meters, bars and beats, markers and regions), a
**generic timeline** (lanes and timed items whose payload the session does
not interpret), **per-extension data**, **transport as data**, and the
**playback sync** primitives (`sync` feature).

A session assumes no audio tracks. The DAW's tracks, takes and FX chains
are extension data in a session, like Keyflow's chart, Ignition's cue lists
and Signal's patch changes. A session is an Architect document (feature
`architect`), so everything built on it syncs live between people and
devices.

Read [`docs/design/session.md`](docs/design/session.md).

## History

This repository held the Session app until 2026-10-12, when it was folded
into the Studio monorepo (FastTrackStudios/studio, `imported.txt`). The app
lives on in Studio. Since 2026-11-17 this repository is the `session`
primitive Studio is being split down to (FastTrackStudios/studio#510,
#509); the old tree is in this repository's history before that.

GPL-3.0-or-later.
