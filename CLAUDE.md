# Session — Claude Code Instructions

Session is the session/project management domain for FastTrackStudio.

## Architecture

This repo follows the **crate facade pattern**:
- `session` — the facade crate, the only public API surface
- `session-proto` — protocol/domain types (internal)
- `session-ui` — Dioxus UI components (public, feature-gated)
- `session-extension` — SHM guest process binary

Apps must depend only on `session` (facade) or `session-ui`, never on internal crates.

## Key Rules

### Async & Concurrency
- Use `moire::task::spawn` instead of `tokio::spawn`
- Use `moire::sync::Mutex` / `moire::sync::RwLock` instead of tokio/std equivalents
- Never hold std sync primitives across `.await`

### RPC Services
- Service traits use `#[roam::service]`
- Max 4 params per method (Facet constraint)
- Use `Tx<T>` / `Rx<T>` for streaming

## Build & Test

```bash
cargo check -p session           # Type-check facade
cargo check --workspace          # Type-check all
cargo test -p session            # Run tests
```

## Issue Tracking

Use `bd` (beads) for all task tracking. See AGENTS.md for workflow.
