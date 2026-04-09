# Project Instructions for AI Agents

This file provides instructions and context for AI coding agents working on this project.

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:b9766037 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

## Landing the Plane (Session Completion)

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **PUSH TO REMOTE** - This is MANDATORY:
   ```bash
   git pull --rebase
   bd dolt push
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Clean up** - Clear stashes, prune remote branches
6. **Verify** - All changes committed AND pushed
7. **Hand off** - Provide context for next session

**CRITICAL RULES:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing - that leaves work stranded locally
- NEVER say "ready to push when you are" - YOU must push
- If push fails, resolve and retry until it succeeds
<!-- END BEADS INTEGRATION -->


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
