+++
title = "DAW"
description = "REAPER integration and DAW bridge for FastTrackStudio"
+++

DAW is the REAPER integration layer for FastTrackStudio.

It provides bidirectional transport control, marker-driven chart navigation, MIDI routing, and real-time state broadcast over the FTS protocol.

## Overview

- [Spec](/spec/) — Specifications and requirements
- [Getting Started](/getting-started/) — Building and running the REAPER extension

## Features

- **Bidirectional transport sync** — Play, stop, seek, and loop between REAPER and FTS tools
- **Marker and region mapping** — Chart sections linked to REAPER markers
- **MIDI routing** — Controller integration through REAPER's MIDI infrastructure
- **Session state broadcast** — Real-time state over the FTS protocol
- **RPP file parsing** — Read and manipulate REAPER project files programmatically
