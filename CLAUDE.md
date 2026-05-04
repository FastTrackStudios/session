# Project Instructions for AI Agents

This file provides instructions and context for AI coding agents working on this project.

## Issue Tracking

Use GitHub Issues for follow-up work. File a GitHub issue for any remaining
known bug, missing test, or implementation task that should survive the current
session.


## Build & Test

```bash
cargo check -p daw              # Type-check facade
cargo check --workspace         # Type-check all
cargo test -p daw-audio-graph   # Run audio graph tests
```

## Architecture

This repo follows the **crate facade pattern**:
- `daw` — facade, the only public API surface
- `daw-proto` — protocol/domain types (internal)
- `daw-control` — high-level control API (internal)
- `daw-reaper` — REAPER-specific implementation (internal)
- `daw-standalone` — reference/standalone implementation (internal)
- `daw-audio-graph` — audio processing DAG engine (internal)
- `daw-builtin-fx` — FTS DSP crates as AudioNode wrappers (internal)
- `daw-plugin-host` — CLAP/VST3 external plugin hosting (internal, Phase 6)

Apps must depend only on `daw` (facade), never on internal crates.

## Platform Targets

The processing-core crates (`daw-audio-graph`, `daw-builtin-fx`) must run
in all three environments. Only I/O adapter crates are platform-specific.

| Target | Notes |
|---|---|
| **Native** (Linux/macOS/Pi) | Full `std`, JACK/ALSA/CoreAudio via `cpal` |
| **WASM / Browser** | AudioWorklet drives `AudioGraph::process()`; no `cpal` |
| **Embedded `no_std`** | `#![no_std]` + `alloc`; no OS, no threads |

### Processing-core crate rules (`daw-audio-graph`, `daw-builtin-fx`)

- `#![no_std]` compatible — depend only on `core` and `alloc`, never `std`
  directly. Gate `std`-only code behind `#[cfg(feature = "std")]`; keep
  the `std` feature additive/default.
- **No heap allocation on the hot path** — pre-allocate in `reset()`; the
  `process()` path must never call `Vec::push`, `Box::new`, or any allocator.
- **No threads** — the graph is driven synchronously by whatever callback
  owns it (cpal, AudioWorklet, bare-metal ISR). Never spawn tasks or threads
  inside processing crates.
- **No platform I/O** — no `cpal`, no `web-sys`, no MIDI drivers inside the
  graph core. I/O lives only in adapter crates.
- **`AudioNode: Send`** — keep the bound; auto-satisfied in single-threaded
  WASM, required for multi-threaded WASM / native.

## Key Rules

### Async & Concurrency
- Use `moire::task::spawn` instead of `tokio::spawn`
- Use `moire::sync::Mutex` / `moire::sync::RwLock` instead of tokio/std equivalents
- Never hold std sync primitives across `.await`
- Processing-core crates (`daw-audio-graph`, `daw-builtin-fx`) must never use async

### RPC Services
- Service traits use `#[vox::service]`
- Max 4 params per method (Facet constraint)
- Use `Tx<T>` / `Rx<T>` for streaming
