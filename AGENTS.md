# session — Repo Instructions

**session is the primitive at the bottom of FastTrackStudio's stack.** One
crate, `session`: the song structure, the generic timeline, per-extension
data, transport as data, and (feature `sync`) the realtime playback sync.
Everything else (`daw`, `fts-fx`, `signal`, Keyflow, Ignition, Studio)
builds on it. Read `docs/design/session.md` before changing the model.

## Rules

- **The session knows nothing about its extensions.** No audio, effect,
  adapter or UI crate is a dependency, ever. A field that only one
  extension understands belongs in that extension's data, not here.
- **wasm-clean.** `cargo check --target wasm32-unknown-unknown
  --all-features` must pass: the engine runs as wasm in an audio worklet on
  the web.
- **Realtime.** What `docs/design/session.md` lists as audio-thread safe
  (`sync::SnapshotCell`, `Transport::at`, `sync::Schedule` reads)
  never locks, allocates or blocks. Keep it that way.
- **Two kinds of sync, kept apart.** Project state is Architect's (the
  `architect` feature binds the model to a Loro document); playback is
  this crate's `sync`. Never put a playhead in the document.
- Architect idiom and pins follow Studio's root `Cargo.toml`: the same
  architect tag, Rust 1.99.1 (`rust-toolchain.toml`), the same lints.

## Build and test

Dev shell: `devenv shell` (or direnv: `.envrc` loads devenv's direnvrc,
then `use devenv`). It is the fleet's: `devenv.yaml` imports
devenv-fleet's `rust` module at the pins every fleet repo uses, so rustc
is the same store path as Studio's, Architect's and CI's, and kache shares
their builds. `.cargo/config.toml` carries the fleet's rustflags flag for
flag; change them in every fleet repo or in none. Move the pins only with
devenv-fleet's `scripts/fleet-pins --write .`; `devenv shell --
fleet-doctor` checks the shell. Never set `RUSTC_WRAPPER` or `CARGO_HOME`.

```bash
cargo test --workspace --all-features
cargo check -p session --target wasm32-unknown-unknown --all-features
cargo clippy --workspace --all-features --all-targets
```

## Process

Branch from `origin/main`, open a PR into `main` with a conventional
title. GPL-3.0-or-later; a new crate inherits `license.workspace = true`.
