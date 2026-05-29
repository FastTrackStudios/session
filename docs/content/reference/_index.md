+++
title = "Reference"
description = "Crate map, derive attributes, and macro outputs."
weight = 30
+++

## Crate map

architect itself lives in `macros/` + `libs/`; everything under
`examples/` is the reference demo a real project would copy.

| Crate | Path | Role |
|-------|------|------|
| `architect` | `macros/architect` | User-facing — re-exports the derive + runtime (layer / `Resource` / `local`). |
| `architect-derive` | `macros/architect-derive` | The `#[derive(Entity)]` / `#[derive(JsonField)]` proc-macro. |
| `architect-rpc-derive` | `macros/architect-rpc-derive` | `#[architect::rpc]` — sync/async trait → vox service + `Layer` token. |
| `crdt`, `crdt-seaorm` | `libs/` | Loro CRDT layer + its SeaORM persistence. |
| `architect-cli` | `crates/architect-cli` | `architect feature new <name>` scaffolder. |
| `example` | `examples/app/features/example/example` | Facade for the example feature. |
| `example-proto` | `…/example-proto` | Wire contract — `#[derive(architect::Entity)]` + `ExampleService`. |
| `example-db` | `…/example-db` | SeaORM/SQLite implementation. |
| `example-memory` | `…/example-memory` | In-memory implementation. |
| `example-crdt` | `…/example-crdt` | Loro CRDT implementation. |
| `example-ui` | `…/example-ui` | Per-feature Dioxus components. |
| `example-tests-native` / `-web` | `…/tests/{native,web}` | native + browser tests. |
| `app-server` | `examples/app/server` | axum + vox runtime (lib `service_router` + bin). |
| `app-cli` | `examples/app/cli` | native vox client (`app`). |
| `app-db` | `examples/app/db` | sea-orm-migration CLI. |
| `app-ui` / `app-web` / `app-desktop` | `examples/app/{ui,web,desktop}` | shared shell + wasm web + native desktop. |
| `app-tests-e2e` | `examples/app/tests/e2e` | native end-to-end (remote + in-process). |
| `example-stub-backend` | `examples/external-stub` | third-party backend pattern. |
| `example-custom-server` | `examples/custom-server` | server assembled by hand. |
| `example-layered-services` | `examples/layered-services` | `Layer` / `Resource` / planner walkthrough. |

## Cargo features

| Crate | Feature | Effect |
|-------|---------|--------|
| `architect` | `vox` | Enable the `#[vox::service]` surface + the `layer` module. |
| `architect` | `server-seaorm` (alias `server`) | SeaORM storage helpers (`storage::DbConn`); derive emits `<T>RepoStorage`. |
| `architect` | `server-axum` | axum adapter — `architect::axum_ws` (Link + `serve`). |
| `architect` | `local` | In-process transport — `architect::{LocalServer, serve_local}` (native). |
| `architect` | `platform` | Portable clock/sleep/spawn — `architect::platform` (native tokio ↔ wasm browser timers). |
| `architect` | `schedule` | Retry/repeat policies — `architect::{Schedule, retry, repeat}` (implies `platform`). |
| `architect` | `fake` | `#[derive(fake::Dummy)]` on emitted structs for seeding. |
| `example` | `backend-db` / `backend-memory` / `backend-crdt` | re-export the chosen backend at `example::backend_*`. |
| `example` | `server-axum` | re-export `architect::axum_ws`. |
| `example-proto` | `vox` (default) / `server` | the RPC surface / forward to `architect/server-seaorm`. |
| `app-server` | `backend-db` (default) / `backend-memory` | which storage the server binary builds with. |

## Vox dependency convention

`vox = { default-features = false, features = ["runtime"] }` is the
wasm-clean baseline used in every `*-proto` and shared library crate.
Native servers add `transport-websocket` if they go through
`vox::serve` directly (architect's bundled `axum_ws` adapter doesn't
need it). Wasm test/client crates depend on `vox-core` and
`vox-websocket` directly via `[target.'cfg(target_arch = "wasm32")']`.

## Layer / construction / transport surface

The runtime DI engine (see [idioms §6–7](@/architecture/idioms.md)):

| Item | Module | Role |
|------|--------|------|
| `Layer` / `Services` / `layers!` / `LayerRouter` | `architect::layer` | compose service tokens, bind a backend → router |
| `Resource<T, E>` / `Scope` / `SharedResource` | `architect::resource` | lazy backend builders — `and_then` / `zip` / `acquire_release` / `memoize`, LIFO teardown |
| `LayerGraph` / `LayerNode` / `LayerPlan` | `architect::plan` | declared-graph topological planner + diagnostics |
| `LocalServer` / `serve_local` | `architect::local` (feature `local`, native) | serve a router in-process over a vox in-memory link |
| `Schedule` / `retry` / `repeat` | `architect::schedule` (feature `schedule`) | composable retry/repeat policies — backoff, jitter, caps; see [scheduling](@/architecture/scheduling.md) |
| `Clock` / `SystemClock` / `TestClock` / `timeout` / `spawn` (`JoinHandle`) / `CancellationToken` / `Deferred` / `Semaphore` / `Queue` | `architect::platform` (feature `platform`) | the portable async-runtime seam — clock, tasks, timeouts, cancellation, concurrency primitives (native↔wasm) |

## What the derive emits

See [The architect pattern](@/architecture/pattern.md) for the full
list of fields and the input → output mapping.
