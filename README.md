# DAW

**REAPER integration, transport control, and DAW abstraction layer.**

DAW provides the unified interface between
[FastTrackStudio](https://github.com/FastTrackStudios/FastTrackStudio) and
REAPER. It handles transport control, track management, project files, and the
extension runtime that allows domain crates to run as hot-reloadable SHM guest
processes inside REAPER.

## Architecture

DAW follows a layered design separating protocol, control API, and
implementation:

```
daw-proto (service definitions + types)
     ↓
daw-control (ergonomic Rust API, reaper-rs style)
     ↓
daw-reaper / daw-standalone (implementations)
     ↓
daw (facade — public API)
```

**Protocol** defines the services. **Control** wraps them in an ergonomic API
with lightweight handles and hierarchical navigation. **Implementations**
connect to the actual DAW (REAPER) or provide a standalone mock for development.

## Workspace Crates

```
daw/
├── daw-proto              Service definitions — Transport, Track, Project, FX,
│                          and streaming update types.
├── daw-control            Ergonomic API — global singleton, lightweight handles,
│                          hierarchical navigation (project.transport().play()).
├── daw-control-sync       Sync-aware control variant.
├── daw-reaper             REAPER implementation via reaper-rs.
├── daw-standalone         Standalone mock implementation for development.
├── dawfile-reaper         REAPER project file handling.
├── daw-ui                 Dioxus UI components.
├── audio-controls         Standalone audio widget library.
├── daw-extension-runtime  Hot-reloadable SHM extension framework.
├── daw-bridge             Extension communication bridge.
├── daw-allocator          RT-aware memory allocator.
├── fts-audio-proto        Audio protocol definitions.
├── fts-devtools           Development utilities.
└── daw                    Facade crate — the only public API surface.
```

## Quick Start

```bash
# Build
cargo build

# Run tests
cargo test

# Type-check the facade
cargo check -p daw
```

## Part of FastTrackStudio

DAW is the shared abstraction layer used by all domain projects in the
[FastTrackStudio](https://github.com/FastTrackStudios/FastTrackStudio)
ecosystem:
[Signal](https://github.com/FastTrackStudios/signal),
[Session](https://github.com/FastTrackStudios/session),
[Keyflow](https://github.com/FastTrackStudios/keyflow), and
[Sync](https://github.com/FastTrackStudios/sync).

## License

See [LICENSE.md](./LICENSE.md)
