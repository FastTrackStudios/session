---
name: architect
description: Enforces architect's monorepo directory structure and hygiene conventions (apps/, crates/, examples/, features/, xtask/ — no top-level libs/ or macros/) — what each top-level directory is for, naming rules, and what should never be committed. Use when deciding where new code/files belong, adding a crate or feature, reviewing a PR for structural drift, or auditing the repo for cruft.
---

# architect repo hygiene

architect enforces one directory shape so a project can host multiple
apps and an unlimited number of features without naming collisions or
ambiguity about what a crate is. This skill is the enforcement checklist —
the full rationale lives in `docs/content/architecture/layout.md` and
`docs/content/reference/_index.md`; read those if a decision here feels
underspecified.

## The shape

```
apps/
  <app>/                       runtime apps. one app = one binary suite.
    server/                    package: <app>-server
    db/                        package: <app>-db   (migration CLI)
    ui/                        package: <app>-ui   (shell)
    web/                       package: <app>-web
    desktop/                   package: <app>-desktop
    tests/e2e/                 package: <app>-tests-e2e
    cli/                       tooling binaries that aren't proc-macros
                                (e.g. apps/architect/cli — the
                                `architect feature new` scaffolder)

crates/                        publishable, standalone libraries. Includes
                                architect's own user-facing facade
                                (crates/architect) — a regular lib, not a
                                proc-macro, so it lives here rather than
                                under a macros/ directory.

examples/                      demo/reference consumers of this framework.

features/
  <feature>/                   one feature = one capability.
    <feature>/                 facade — selects backend via cargo features.
    <feature>-proto/           wire contract.
    <feature>-<backend>/       one impl per backend (db, memory, crdt, ...).
    <feature>-ui/              feature-scoped Dioxus components.
    spec/                      tracey-tracked rules for this feature.
    tests/{native,web}/        cargo test / wasm-bindgen browser tests.
  macros/                      cross-cutting proc-macro crates that aren't
                                scoped to one feature (architect-derive,
                                architect-rpc-derive, architect-action-derive).
                                A proc-macro that DOES belong to one feature
                                nests inside that feature instead — see
                                features/crdt/crdt-derive.

xtask/                         build tooling. in workspace `members` so
                                `cargo xtask` resolves, but never in
                                `default-members` — it's not production code.
```

There is no top-level `libs/` **or** `macros/` directory. Anything that
would once have been "internal support code" is a `feature/` (see
`features/crdt/`), and every proc-macro crate lives under
`features/macros/` (cross-cutting) or inside the one feature it belongs
to (`features/crdt/crdt-derive`). If it's not one of the five top-level
categories above, it's a feature — don't invent a new top-level bucket
for "shared plumbing" or "codegen."

### What each directory is *for*

- **`apps/`** — a runtime product: one or more binaries a user actually
  runs, including tooling binaries (a scaffolder CLI counts — it's not
  a proc-macro, so it's not `features/macros/`, and it's a binary
  suite, not a library, so it's not `crates/`). **This repo has no
  general `apps/<app>/{server,ui,web,...}` product** — it's the
  framework, not a consumer, so `examples/app/` plays that role for the
  reference demo. `apps/architect/cli` is the one real exception: it's
  architect's own tool, not a demo. A real consumer project puts
  `apps/` and `features/` directly at its repo root, per
  `docs/content/architecture/layout.md`.
- **`crates/`** — publishable libraries that are neither proc-macros
  nor scoped to one feature. `crates/architect` is the reference case:
  it re-exports the derive macros from `features/macros/` plus the
  runtime traits (`Layer`, `Resource`, `local`, ...) — it has real
  logic and isn't itself `proc-macro = true`, so despite being
  "the architect crate," it belongs in `crates/`, not treated as a
  macro. This is what an app most directly depends on and assembles —
  a crate typically composes one or more `features/` underneath it.
- **`examples/`** — reference/demo code that shows how a consumer uses
  the framework. Not shipped, not depended on by anything under
  `crates/` or `features/`. `examples/app/` is the full reference app;
  `examples/custom-server`, `examples/external-stub`,
  `examples/layered-services` are narrow single-concept demos.
- **`features/`** — vertical slices of capability, meant to be
  *consumed* — think of a feature as a specialized, backend-swappable
  lib rather than a leaf app concern. This covers product-facing
  capabilities (auth, atom, form), shared infrastructure other
  features/crates build on (`features/crdt/`), **and** cross-cutting
  proc-macros (`features/macros/`). If new code is a capability
  something else opts into — including "a proc-macro every feature
  uses" — it goes here, scaffolded with `just scaffold-feature <name>`
  for the proto/backend/facade shape (never hand-rolled).
  - `features/crdt/` (`crdt`, `crdt-seaorm`, `crdt-derive`) is the
    reference case for infrastructure-as-feature: the local-first CRDT
    layer isn't itself app-visible, but it's swappable (persistence
    backend), independently tested, and consumed by other features
    (`example-crdt`) and apps directly (`app-ui`'s `use_synced_doc`).
  - `features/macros/` (`architect-derive`, `architect-rpc-derive`,
    `architect-action-derive`) is the reference case for cross-cutting
    proc-macros: none of the three is scoped to one feature — every
    feature and `crates/architect` itself depends on them — so they
    don't nest under a single feature, but they're still consumable
    capabilities, not framework-external tooling, so they live in
    `features/` rather than a bespoke `macros/` top-level.
  - A proc-macro that genuinely *is* scoped to one feature — like
    `crdt-derive`, which only `crdt` uses — nests inside that feature
    (`features/crdt/crdt-derive`) instead of `features/macros/`.

### Dependency direction

```
apps/  ──depends on──>  crates/  ──depends on──>  features/  ──depends on──>  features/macros/
  │                                                    │                            ^
  │                                                    └────────────────────────────┘
  └──────────────────── may also depend on directly ───┘
```

Apps *can* depend on `features/` directly, but the common shape is
apps depending on `crates/`, and `crates/` doing the work of composing
whichever `features/` it needs. Features can depend on *other*
features when one is infrastructure for another (`example-crdt` →
`features/crdt/crdt`) or when one is a cross-cutting proc-macro
(`crates/architect` → `features/macros/architect-derive`). Dependencies
only ever point rightward/downward in that chain — a `feature/` never
depends on a `crate/`, and `features/macros/` never depends on another
feature.

## Naming rules

- **Path prefix matches package name prefix.** `apps/<app>/<role>/` →
  package `<app>-<role>`. `features/<feature>/<feature>-<role>/` →
  package `<feature>-<role>`. This holds even for the exceptions above:
  `apps/architect/cli` → `architect-cli`; `features/crdt/crdt-derive` →
  `crdt-derive`. `features/macros/` is the one deliberate looseness —
  its members (`architect-derive`, `architect-rpc-derive`,
  `architect-action-derive`) share the `architect-` prefix with
  `crates/architect`, not with `macros`, because they're that crate's
  proc-macro half, not a `macros`-named feature in their own right.
- Cargo names use only `[a-z0-9_-]` — `<>` in docs is fill-in-the-blank
  notation, not literal syntax.
- App names and feature names are different namespaces; the `<app>-` /
  `<feature>-` prefix keeps packages disambiguated even if an app and a
  feature share a bare name.

## Workspace `members` vs `exclude`

Root `Cargo.toml` `members` lists everything that compiles for the host
target. Crates with target-cfg deps (wasm-only test crates, Dioxus apps
pulling `dioxus-web`/`dioxus-desktop`) go in `exclude` and are built by
`cd`-ing into their directory and invoking `cargo` directly — `just
check` wraps both halves into one command. Don't add a wasm/dioxus crate
to `members` and expect it to build on the host target; don't skip
`exclude` and hand-wave a separate CI step instead.

## Adding new code — decision checklist

1. Is it a proc-macro (`proc-macro = true` in its `Cargo.toml`)? Is it
   scoped to exactly one feature? → nest it inside that feature
   (`features/<feature>/<name>-derive`). Otherwise → `features/macros/`.
2. Is it one *consumable* capability — a wire contract with swappable
   backends, or shared infrastructure, meant to be pulled into a crate,
   app, or another feature? → `features/<name>/`, via
   `just scaffold-feature <name>` (never hand-authored — it also wires
   the workspace `Cargo.toml` and `.config/tracey/config.styx`).
3. Is it a runtime binary suite (server/ui/web/desktop), or a tooling
   binary that isn't a proc-macro? → `apps/<name>/` at a consumer's
   repo root (or `examples/app/` / `apps/architect/` in this repo). It
   should depend on `crates/` (which in turn compose the `features/` it
   needs), though depending on a `feature/` directly is fine for
   something small.
4. Is it a standalone publishable library that isn't a proc-macro and
   isn't feature-scoped — the thing an app actually reaches for? →
   `crates/`.
5. Is it a demo/reference showing how to consume the framework, not
   itself part of the framework? → `examples/`.
6. Is it build tooling invoked via `cargo xtask`? → `xtask/`, in
   `members` but never `default-members`.

## Hygiene — what does not belong in the tree

- **No runtime/generated data.** Databases (`*.db`), CRDT/collab state
  dirs (`collab-data/`), sqlite ingest stores — these are `.gitignore`d
  by pattern, not by path. If a new runtime-data location shows up
  (check env vars like `*_DATA_DIR` in server `main.rs` files), add its
  pattern to `.gitignore` rather than letting `git status` grow quiet
  cruft.
- **No unlinked root-level scratch docs.** One-off wave/session handoff
  notes (`HANDOFF.md`-style: dated, addressed to "whoever picks this up
  next", not linked from `README.md` or `docs/content/_index.md`) are
  session artifacts, not documentation. Durable content belongs in
  `docs/content/architecture/*.md` (discoverable, versioned with the
  code it describes) or in this project's persistent memory
  (`/home/cody/.claude/projects/-run-media-Development-architect/memory/`)
  — not as a floating root markdown file. If a root doc's status line
  says "locked"/"in progress" but nothing links to it, that's a signal
  it drifted out of the living-docs set; fold current content into
  `docs/content/` and delete the root file.
- **`nix build` symlinks** (`result`, `result-*`) are already
  `.gitignore`d — don't `git add -f` them back in.
- **No `libs/` or top-level `macros/` directory.** Both were folded into
  `features/` — internal support code is a feature
  (`features/crdt/`), and proc-macros are either a feature-scoped
  derive crate or live in `features/macros/` if cross-cutting.
- Before adding a new top-level directory, check whether it actually
  fits one of the five categories above (`apps/`, `crates/`,
  `examples/`, `features/`, `xtask/`). A new top-level dir that doesn't
  map cleanly onto those is itself a hygiene smell — ask whether it
  should nest inside one of those instead.
