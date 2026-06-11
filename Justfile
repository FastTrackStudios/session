# architect dev recipes
#
# Run from the repo root inside the Nix dev shell:
#   nix develop -c just <recipe>
#
# Most recipes delegate to `cargo xtask`, which owns the actual logic in
# Rust (see xtask/src/main.rs). Anything that needs to outlive the
# server process (the wasm e2e dance) still lives here as a shell
# recipe — xtask shells out to these.

# Default: full check across workspace + target-cfg crates.
default: check

# Type-check workspace + the target-cfg-only crates.
check:
    cargo xtask check
    cd examples/app/ui && cargo check
    cd examples/app/web && cargo check --target wasm32-unknown-unknown
    cd examples/app/desktop && cargo check

# nextest with the default profile.
test:
    cargo xtask test

# Workspace + clippy + fmt --check + nextest CI profile.
ci:
    cargo xtask ci

# Build + run the migration binary.
migrate:
    cargo run -p app-db -- up

# Run the axum + vox server. Migrations auto-apply on boot.
server:
    cargo run -p app-server

# Same, but with a 2s artificial latency on every write so the web
# client's optimistic create/update/delete sit visibly "pending" before
# reconciling. Test/demo only — see `LatencyRepo` in app-server.
server-slow:
    EXAMPLE_LATENCY_MS=2000 cargo run -p app-server

# The whole dev loop in one terminal: the vox server + `dx serve` for the
# web app (http://localhost:8765). Ctrl-C stops both. Open two browser
# windows to watch live events sync them.
dev:
    #!/usr/bin/env bash
    set -m
    trap 'kill 0' EXIT INT TERM
    cargo run -p app-server &
    (cd examples/app/web && dx serve --web --addr 0.0.0.0 --port 8765)

# ── Diagnostics (moiré) ───────────────────────────────────────────────

# Run the app-server with moiré instrumentation enabled. Connects to
# the dashboard at $MOIRE_DASHBOARD (default 127.0.0.1:9119); start
# `just moire-web` in another terminal first.
server-with-diagnostics:
    MOIRE_DASHBOARD="${MOIRE_DASHBOARD:-${MOIRE_DASHBOARD_DEFAULT:-127.0.0.1:9119}}" \
        cargo run -p app-server --features diagnostics

# Launch the moiré dashboard. $MOIRE_WEB_BIN is set by the flake's
# shellHook to the nix-built binary (frontend bundled). No install
# step, no per-user build cache, no cargo install.
#
# To rebuild the dashboard from a different moire rev, update the
# `moire` input in flake.nix and `nix flake update moire`.
moire-web:
    "$MOIRE_WEB_BIN"

# Run app-server + moire-web side-by-side. Server connects to the
# dashboard automatically; open http://127.0.0.1:9119 to view.
diagnostics:
    #!/usr/bin/env bash
    set -euo pipefail
    : "${MOIRE_DASHBOARD:=${MOIRE_DASHBOARD_DEFAULT:-127.0.0.1:9119}}"
    export MOIRE_DASHBOARD
    "$MOIRE_WEB_BIN" &
    DASHBOARD_PID=$!
    trap "kill $DASHBOARD_PID 2>/dev/null || true" EXIT
    sleep 1
    cargo run -p app-server --features diagnostics

# Run the wasm browser integration tests against an already-running server.
test-wasm:
    cd examples/app/features/example/tests/web && cargo test --target wasm32-unknown-unknown --release

# Browser e2e against the default (sqlite) backend.
test-e2e: (_e2e "")

# Same e2e against the in-memory backend — proves the contract is
# backend-agnostic (wasm tests don't change).
test-e2e-memory: (_e2e "--no-default-features --features backend-memory")

# Internal: build + run server with given cargo features, drive wasm tests.
_e2e features:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build -p app-server {{features}}
    rm -f example.db
    ./target/debug/app-server &
    server_pid=$!
    trap "kill $server_pid 2>/dev/null || true; rm -f example.db" EXIT
    for i in {1..30}; do
        if curl -fsS http://127.0.0.1:4040/api/health >/dev/null 2>&1; then break; fi
        sleep 0.2
    done
    cd examples/app/features/example/tests/web && cargo test --target wasm32-unknown-unknown --release

# `dx serve` the web app — connects to the server on 4040 by default.
web:
    cd examples/app/web && dx serve --web --addr 0.0.0.0 --port 8765

# `dx serve` the desktop app.
desktop:
    cd examples/app/desktop && dx serve --desktop

# ── CLI client ─────────────────────────────────────────────────────────

# Invoke the `app` CLI client. Pass subcommand + args after `--`.
#   just cli -- list
#   just cli -- create --name foo
cli *args:
    cargo run -p app-cli -- {{args}}

# ── Docs ──────────────────────────────────────────────────────────────

# Serve the dodeca docs site locally with live reload.
# Reads .config/dodeca.styx for paths; run from the repo root.
docs:
    ddc serve

# Build the dodeca docs site for production.
docs-build:
    ddc build

# Sync docs/content/ → the Forgejo wiki repo.
sync-wiki:
    cargo xtask wiki sync

sync-wiki-dry-run:
    cargo xtask wiki sync --dry-run

# ── Tracey ────────────────────────────────────────────────────────────

# Validate spec ↔ impl ↔ verify links. Fails on unmapped rules.
tracey-validate:
    cargo xtask tracey-validate

# Coverage overview (what's tested, what isn't).
tracey-status:
    tracey query status

# ── Scaffolding ───────────────────────────────────────────────────────

# Scaffold a new feature crate family at features/<name>/. Drops in
# proto + memory backend + facade + native tests + spec stub, wires
# everything into Cargo.toml + .config/tracey/config.styx.
scaffold-feature name:
    cargo run -q -p architect-cli -- feature new {{name}}

# ── Releases / changelog ──────────────────────────────────────────────

# Regenerate CHANGELOG.md from conventional commits.
changelog:
    git cliff -o CHANGELOG.md

# Preview release notes for the next bump (no file write).
changelog-preview:
    git cliff --unreleased

# Install git hooks (capn pre-commit + pre-push + tracey).
install-hooks:
    ./hooks/install.sh

# Run capn pre-commit checks manually (without committing).
capn-precommit:
    capn

# Run capn pre-push checks manually (without pushing).
capn-prepush:
    capn pre-push

# Format all Rust files in the workspace + target-cfg crates.
fmt:
    cargo fmt --all
    cd examples/app/ui && cargo fmt
    cd examples/app/web && cargo fmt
    cd examples/app/desktop && cargo fmt

# Headless-browser e2e for the web app: real server + `dx serve` + the
# playwright spec under examples/app/web/e2e (system chromium via
# playwright-core). This is the layer that catches wasm panics, render
# wedges, and main-thread livelocks that cargo tests can't see.
web-e2e:
    #!/usr/bin/env bash
    set -euo pipefail
    pids=()
    cleanup() { for p in "${pids[@]:-}"; do [ -n "$p" ] && kill "$p" 2>/dev/null || true; done; }
    trap cleanup EXIT INT TERM
    COLLAB_DATA_DIR=$(mktemp -d) cargo run -p app-server --no-default-features --features backend-memory &
    pids+=($!)
    until curl -sf http://127.0.0.1:4040/api/health -o /dev/null; do sleep 1; done
    rm -f /tmp/dx-web-e2e.log
    (cd examples/app/web && exec dx serve --web --addr 127.0.0.1 --port 8123 > /tmp/dx-web-e2e.log 2>&1) &
    pids+=($!)
    until grep -q "Build completed successfully" /tmp/dx-web-e2e.log 2>/dev/null; do sleep 2; done
    cd examples/app/web/e2e && pnpm install --silent && BASE_URL=http://127.0.0.1:8123 node collab.spec.mjs
