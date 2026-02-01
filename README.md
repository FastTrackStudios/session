# FastTrackStudio

Distributed DAW (Digital Audio Workstation) control system using roam RPC framework with cross-process tracing.

## Quick Start

### 1. Enter the development environment

```bash
nix develop
```

Or with direnv:
```bash
direnv allow
```

### 2. Start Jaeger (for distributed tracing)

```bash
./start-jaeger.sh
```

Or manually:
```bash
docker context use desktop-linux
docker-compose up -d
```

This starts Jaeger on:
- UI: http://localhost:16686
- OTLP endpoint: localhost:4317

### 3. Build the project

```bash
cargo build
```

### 4. Run the host

```bash
cargo run -p fasttrackstudio
```

This will:
- Start the DAW host process
- Spawn the DAW standalone cell
- Spawn the Session cell
- Spawn the Gateway-WS cell (WebSocket server on port 3030)
- Export traces to Jaeger

### 5. View traces

Open http://localhost:16686 and select "daw-host" service to see distributed traces across all cells.

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        Host Process                          │
│                                                              │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐       │
│  │ Gateway-WS   │  │   Session    │  │     DAW      │       │
│  │   Cell       │  │    Cell      │  │  Standalone  │       │
│  │              │  │              │  │    Cell      │       │
│  │  WebSocket   │  │  Controls    │  │  Transport   │       │
│  │  :3030/ws    │  │    DAW       │  │   Control    │       │
│  └──────┬───────┘  └──────┬───────┘  └──────────────┘       │
│         │                 │                 ▲                │
│         │                 └─────────────────┘                │
│         │                                                    │
└─────────┼────────────────────────────────────────────────────┘
          │
          ▼
    ┌───────────┐
    │  Browser  │  (fts-control-web)
    │   WASM    │
    └───────────┘
```

### Components

- **Host** (`main/`): Orchestrates cells, routes RPC calls, aggregates tracing
- **Gateway-WS** (`cells/gateway/gateway-ws/`): WebSocket server for browser clients
- **DAW Standalone Cell** (`cells/daw/daw-standalone/`): Implements transport and project services
- **Session Cell** (`cells/session/session/`): Control surface that calls DAW methods
- **DAW Proto** (`cells/daw/daw-proto/`): Service definitions for DAW RPC
- **DAW Control** (`cells/daw/daw-control/`): Client library for calling DAW services
- **FTS Control Web** (`apps/fts-control/web/`): Browser-based DAW control UI

### Key Technologies

- **roam**: RPC framework with shared memory transport
- **roam-shm**: Shared memory communication layer
- **roam-websocket**: Binary WebSocket transport (WASM-compatible)
- **roam-tracing**: Cross-process distributed tracing
- **OpenTelemetry**: Trace export to Jaeger
- **Facet**: Schema-driven serialization
- **Dioxus**: Cross-platform UI framework (web/desktop)
- **Nix**: Reproducible development environment

## Web App

The FTS Control web app connects to the gateway via binary WebSocket and uses the same `daw-control` API as desktop apps.

### Build and serve

```bash
cd apps/fts-control/web
dx serve
```

Then open http://localhost:8080

## Distributed Tracing

All tracing events from cells are forwarded to the host and exported to Jaeger via OpenTelemetry.

See [JAEGER_SETUP.md](./JAEGER_SETUP.md) for detailed tracing documentation.

### Key Features

- **Automatic trace aggregation**: All cells forward traces to host
- **Peer tagging**: Each trace includes the source cell name
- **Structured fields**: Key-value pairs from tracing events
- **Zero-copy RPC**: Batched event forwarding via shared memory
- **Non-blocking**: Lossy buffers prevent cells from blocking on tracing

## Environment Variables

- `OTEL_EXPORTER_OTLP_ENDPOINT`: Jaeger endpoint (default: `http://localhost:4317`)
- `RUST_LOG`: Log level filter (default: `info`)
- `GATEWAY_WS_ADDR`: WebSocket gateway bind address (default: `0.0.0.0:3030`)
- `FTS_SOCKET`: Unix socket path for desktop app (default: `/tmp/fts-control.sock`)

## Development

### Running tests

```bash
cargo test
```

### Running WASM integration tests

```bash
cd tests/playwright
npm install
npx playwright test
```

### Running with debug logs

```bash
RUST_LOG=debug cargo run -p fasttrackstudio
```

### Stopping Jaeger

```bash
./stop-jaeger.sh
```

Or manually:
```bash
docker context use desktop-linux
docker-compose down
```

## Project Structure

```
.
├── main/                    # Host binary (fasttrackstudio)
├── apps/
│   ├── fts-control/
│   │   └── web/            # Browser UI (Dioxus/WASM)
│   └── tests/
│       └── wasm/           # WASM integration test app
├── cells/
│   ├── daw/
│   │   ├── daw-proto/      # Service definitions
│   │   ├── daw-control/    # Client library
│   │   ├── daw-standalone/ # Standalone cell
│   │   └── daw-reaper/     # Reaper integration (WIP)
│   ├── gateway/
│   │   ├── gateway-proto/  # Gateway protocol
│   │   └── gateway-ws/     # WebSocket gateway cell
│   ├── session/
│   │   └── session/        # Session cell
│   └── hello-world/        # Example cell
├── modules/
│   ├── cell-runtime/       # Cell process runtime
│   ├── host-client/        # Shared host connection library
│   └── integration-tests/  # Integration tests
├── tests/
│   └── playwright/         # WASM integration tests
├── flake.nix               # Nix development environment
├── docker-compose.yml      # Jaeger setup (full)
├── docker-compose.minimal.yml  # Jaeger setup (minimal)
└── JAEGER_SETUP.md         # Tracing documentation
```

## License

See LICENSE file.
