# architect

A facet-native, vox-friendly entity framework for Rust. One
`#[derive(Entity)]` on a plain struct, and you get:

- A wasm-safe wire struct (with `facet::Facet`) usable from any
  client crate (Dioxus web, Dioxus desktop, future iOS via FFI).
- `<Entity>Create` / `<Entity>Update` / `<Entity>List` payload types.
- An auto-generated `<Entity>Repo` `#[vox::service]` trait — typed
  RPC over WebSocket, no JSON hand-rolling.
- Under `--features server`: the full SeaORM `Model` + `Entity` +
  `Column` + `Relation` + `ActiveModel`, plus a
  `<Entity>RepoStorage<C>` that implements the repo trait against a
  SeaORM connection.

```rust
#[derive(architect::Entity)]
#[architect(table_name = "examples", repo)]
pub struct Example {
    #[architect(primary_key, on_create = Uuid::new_v4())]
    pub id: Uuid,

    #[architect(filterable, sortable, fulltext)]
    pub name: String,

    #[architect(exclude(create, update), on_create = Utc::now())]
    pub created_at: DateTime<Utc>,
}
```

That's the whole entity. No `cfg_attr`. No parallel struct in another
crate. No manual `From<Model> for Example` glue.

## Layout

```
crates/architect/    the user-facing crate (re-exports the derive + runtime traits)
apps/architect/cli/  scaffolds new feature crate families (`architect feature new`)
features/macros/     architect-derive / architect-rpc-derive / architect-action-derive — the proc-macros
features/crdt/       crdt / crdt-seaorm / crdt-derive — the local-first layer

examples/app/        the reference full-stack demo
  features/example/  proto / db / memory / crdt / ui + facade for one entity
  server/            axum + vox; generic vox_router + the ExampleService impl
  cli/               native vox client (`app` binary)
  db/                sea-orm-migration CLI
  ui/                shared Dioxus shell — router + screens + vox client lifecycle
  web/  desktop/     thin Dioxus launchers (wasm / native) over `ui`
  tests/e2e/         real client ↔ real server over a vox socket
```

`examples/app/` is the **reference example** — a full Dioxus web +
desktop app (list/detail/create/edit/delete/search/duplicate) talking to
the server entirely over vox. Read it to learn the pattern, then template
it when spinning up a new project. The conventions it follows — and the
CI gates that enforce them — are written up in
[docs/architecture/idioms](docs/content/architecture/idioms.md).

## Why facet-only

vox uses facet for its wire encoding. By dropping serde derives
entirely, the wire format is one cohesive system — every architect
type is automatically Facet-able, which means vox can transport it
without any per-type glue. No parallel `serde` derives, no
`#[serde(rename = …)]` mismatches with `#[architect(...)]`.

## Learn

New here? Read **[Build a feature, end to end](docs/content/getting-started/walkthrough.md)**
— define an entity → pick a backend → serve it → consume it remote *or*
in-process, against the reference example. Then:

- [The architect pattern](docs/content/architecture/pattern.md) — what the
  `Entity` derive emits, field by field.
- [Idioms & enforcement](docs/content/architecture/idioms.md) — the
  conventions (vox-only RPC, the `Layer`/`Resource` DI engine, transport
  injection) and the CI gates behind them.
- [Reference](docs/content/reference/_index.md) — crate map, features, the
  runtime surface.

## Status

Working end to end. The `Entity` derive emits the full surface (wire types
+ `Create`/`Update`/`List` + the `<Entity>Repo` vox trait + a `Layer`
token, and the SeaORM storage under `server-seaorm`). The reference
example (`examples/app/`) runs as a Dioxus web app against an axum+vox
server, as a fully in-process desktop app, and as a CLI — all from the one
contract, covered by native + browser + in-process tests.
