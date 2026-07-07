---
name: architect
description: Enforces architect's monorepo directory structure and hygiene conventions (apps/, crates/, examples/, features/, libs/, macros/, xtask/) — what each top-level directory is for, naming rules, and what should never be committed. Use when deciding where new code/files belong, adding a crate or feature, reviewing a PR for structural drift, or auditing the repo for cruft.
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

crates/                        publishable, standalone libraries/tools.

examples/                      demo/reference consumers of this framework.

features/
  <feature>/                   one feature = one capability.
    <feature>/                 facade — selects backend via cargo features.
    <feature>-proto/           wire contract.
    <feature>-<backend>/       one impl per backend (db, memory, crdt, ...).
    <feature>-ui/              feature-scoped Dioxus components.
    spec/                      tracey-tracked rules for this feature.
    tests/{native,web}/        cargo test / wasm-bindgen browser tests.

libs/                          internal support libraries architect itself
                                is built on (not per-feature, not apps).

macros/                        proc-macros (architect, architect-derive, ...).

xtask/                         build tooling. in workspace `members` so
                                `cargo xtask` resolves, but never in
                                `default-members` — it's not production code.
```

### What each directory is *for*

- **`apps/`** — a runtime product: one or more binaries a user actually
  runs. **This repo (architect itself) has no `apps/`** — it's the
  framework, not a consumer. `examples/app/` plays the app role here so
  framework code and the demo that consumes it stay separated. A real
  consumer project puts `apps/` and `features/` directly at its repo
  root, per `docs/content/architecture/layout.md`.
- **`crates/`** — publishable libraries or CLI tools that are neither a
  framework internal (`libs/`) nor scoped to one feature
  (`features/<feature>/`). Example: `crates/architect-cli`, the
  `architect feature new` scaffolder.
- **`examples/`** — reference/demo code that shows how a consumer uses
  the framework. Not shipped, not depended on by `libs/`/`macros/`.
  `examples/app/` is the full reference app; `examples/custom-server`,
  `examples/external-stub`, `examples/layered-services` are narrow
  single-concept demos.
- **`features/`** — vertical slices of product capability, each
  independently backend-swappable and independently testable. If new
  code is a capability an app opts into (auth, atom, form, a DAW
  feature), it goes here, scaffolded with `just scaffold-feature <name>`
  — never hand-rolled.
- **`libs/`** — support code architect's *own* implementation depends on
  that isn't a proc-macro and isn't feature-scoped. Example: `libs/crdt`,
  `libs/crdt-seaorm`. If you're tempted to add something here, first ask
  whether it's really a `feature/` (product-facing, backend-swappable) or
  a `crates/` (standalone/publishable) instead.
- **`macros/`** — proc-macro crates only. `architect`, `architect-derive`,
  `architect-rpc-derive`, `architect-action-derive`, `crdt-derive`.

## Naming rules

- **Path prefix matches package name prefix.** `apps/<app>/<role>/` →
  package `<app>-<role>`. `features/<feature>/<feature>-<role>/` →
  package `<feature>-<role>`.
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

1. Is it a proc-macro? → `macros/`.
2. Is it one capability an app opts into, with a wire contract and
   swappable backends? → `features/<name>/`, via
   `just scaffold-feature <name>` (never hand-authored — it also wires
   the workspace `Cargo.toml` and `.config/tracey/config.styx`).
3. Is it a runtime binary suite (server/ui/web/desktop)? → `apps/<name>/`
   at a consumer's repo root (or `examples/app/` in this repo).
4. Is it a standalone publishable crate/tool that isn't feature-scoped?
   → `crates/`.
5. Is it internal plumbing architect's own implementation needs, that
   isn't a macro and isn't one feature's concern? → `libs/`.
6. Is it a demo/reference showing how to consume the framework, not
   itself part of the framework? → `examples/`.
7. Is it build tooling invoked via `cargo xtask`? → `xtask/`, in
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
- Before adding a new top-level directory, check whether it actually
  fits one of the seven categories above. A new top-level dir that
  doesn't map cleanly to apps/crates/examples/features/libs/macros/xtask
  is itself a hygiene smell — ask whether it should nest inside one of
  those instead.
