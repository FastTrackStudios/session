+++
title = "Idioms & enforcement"
description = "The bearcove/vox + Dioxus conventions architect follows, and what keeps them honest."
weight = 50
+++

architect's client/server story is opinionated. These are the rules the
example app (`examples/app/`) follows, why they exist, and how each one
is kept from drifting. Copy them into your own project.

## 1. All client↔server data flows through vox services

Every read and write goes through a `#[vox::service]` trait — the
architect-emitted `<Entity>Repo` (CRUD) or a hand-written `<Entity>Service`
(domain ops like search/duplicate). The client establishes a typed
`<Trait>Client` over a WebSocket; the server mounts the matching
`Dispatcher`. There is **one** wire schema, generated from the trait.

**Never use Dioxus server functions** (`#[server]`, `use_server_future`,
`dioxus-fullstack`). They'd introduce a second, parallel RPC mechanism
with its own serialization, defeating the facet-only wire.

> Enforced: `cargo xtask idioms` greps `examples/app/` for `#[server]`,
> `use_server_future`, and `dioxus(_|::)fullstack` and fails CI on a hit.

## 2. Wire types are facet-only

`#[derive(architect::Entity)]` emits `facet::Facet` — no `serde` derives
on the wire struct or its `Create`/`Update`/`List` payloads. vox speaks
facet natively, so there's zero per-type glue and no
`#[serde(rename)]` ↔ `#[architect(...)]` drift.

(The CRDT layer's `JsonField` companions *do* use serde — that's a
storage-column concern, not the wire, so it's fine.)

## 3. The client is established once and shared via context

The app root (`examples/app/ui/src/client.rs`) establishes the typed
clients **once**, stores them in a `Signal<ConnState>`, and provides it
with `use_context_provider`. Screens pull it with `use_context` (via the
`use_conn` helper) and read data with `use_resource`, which re-runs when
the connection flips to `Ready` or when its inputs change. Don't
reconnect per-screen; don't block the first render on the socket.

```rust
let conn = use_context_provider(|| Signal::new(ConnState::Connecting));
use_future(move || async move { /* connect, then conn.set(Ready) */ });
// in a screen:
let results = use_resource(move || async move {
    let clients = match &*conn.read() { ConnState::Ready(c) => c.clone(), _ => return None };
    Some(clients.repo.list(/* … */).await)
});
```

## 4. Navigation is typed; UI primitives come from the catalog

- Navigate with the `Route` enum + `Link { to: Route::… }` / `use_navigator`,
  never a raw `<a href>` — the compiler then catches dead links.
- Reach for the Dioxus component catalog (`dx components add <name>`)
  before hand-rolling a widget (dialog, combobox, drag-to-reorder, …).

## 5. One coherent vox revision across server and clients

The server, CLI, web, and desktop must build against the **same** vox
revision — a mismatch yields a wire-schema translation error at the first
call, or (worse) a silent desync. The example pins one rev in the root
`Cargo.toml` and the client crates reference it verbatim.

> Lesson baked into the e2e suite: upstream vox v0.8.2 couldn't encode
> `Uuid` (no branch for opaque `Def::Scalar` shapes), which compiled fine
> but panicked the server on the first RPC. The
> `app-tests-e2e` suite — real client ↔ real server over a socket — is
> what catches this class of bug. Keep it green.

## What keeps these honest

| Rule | Gate |
| --- | --- |
| No Dioxus server fns (§1) | `cargo xtask idioms` (CI) |
| facet-only wire (§2) | the `Entity` derive emits facet; serde derive would be redundant |
| Excluded crates still build (§3–4) | `cargo xtask ci` checks `ui` / `web` (wasm) / `desktop` |
| Real wire round-trips (§5) | `app-tests-e2e` + `just test-e2e` (browser) |
| Hand-rolled widgets, signal/prop misuse | dioxus-MCP audits — run manually |

The dioxus-MCP audits aren't CLI-runnable, so they're a manual companion
to the automated gates. Run them against `examples/app/ui` when touching
the UI:

- `lint_project` — the full sweep (`check_rsx`, `dead_components`,
  `prop_drill`, `signal_lint`, `props_lint`, `reinvented_widget`,
  `components_audit`, …) in one call.
- `route_map` — sanity-check the route table after editing `Route`.

CI runs `cargo xtask ci`: `fmt --check` → `clippy -D warnings` →
`check --workspace` → target-cfg crate checks → `nextest` →
doctests → `app-ui` smoke test → `idioms` → `tracey validate`. The
browser e2e runs in a separate job (`just test-e2e` / `test-e2e-memory`).
