# Task workspace recipes
# Run commands: just <recipe-name>

# Default: check the core workspace
default: check

# ── CLI ──────────────────────────────────────────────────────────────────

# Run the task CLI
task *args:
    cargo run -p task-cli -- {{args}}

# ── Web (dev) ────────────────────────────────────────────────────────────

# ── Run the app ──────────────────────────────────────────────────────────
#
# Three recipes for three terminals (or `just dev` to run them all):
#   1. `just server` → task-server on :9090, sync relay
#   2. `just web`    → Dioxus dev server on :8765
#   3. `just db`     → run migrations + seed fake data
#
# Defaults to in-memory sqlite — `just server` and `just db` populate
# their own process's database. For persistent data across runs, set
# `SYNC_DEMO_DATABASE_URL=sqlite://./data.db?mode=rwc` first.

# Dioxus dev server for apps/web on port 8765. Binds 0.0.0.0 so the
# starcommand nginx reverse proxy reaches it via the 10G LAN.
# `--wasm-split` enables route-level lazy chunks.
#
# Assumes direnv/.envrc already loaded the `.#ui` dev shell. If you're
# running outside direnv, prefix with `nix develop .#ui --command`.
web:
    cd apps/web && dx serve --web --addr 0.0.0.0 --port 8765

# Regenerate apps/desktop/assets/tailwind.css from the source
# `tailwind.css` input. Run after touching `@source` paths or the
# `@layer components` block. `tailwindcss` comes from the nix
# devshell — `direnv` already loaded it, no explicit nix call.
desktop-css:
    tailwindcss -i apps/desktop/tailwind.css -o apps/desktop/assets/tailwind.css

# Native desktop window — the Logseq-like editor as a real app.
# Regenerates tailwind first so any new utility classes in
# touched sources actually exist in the bundled stylesheet, then
# hot-reloads on subsequent source changes.
desktop: desktop-css
    cd apps/desktop && dx serve --platform desktop

# Same as `desktop` but in release mode — slower compile, snappier
# runtime; use when smoke-testing a vault for actual editing.
desktop-release: desktop-css
    cd apps/desktop && dx serve --platform desktop --release

# Canonical server. Defaults: bind 0.0.0.0:9090, in-memory sqlite,
# seed-on-startup. Override via TASK_SERVER_{BIND,SEED} env vars.
server:
    TASK_SERVER_SEED=1 TASK_SERVER_BIND="0.0.0.0:9090" \
        cargo run --release -p task-server

# Run migrations + seed the workspace doc with fake data. Standalone
# CLI — useful for inspecting what `task-db` does without binding a
# port. Since the default sqlite is in-memory the snapshot dies when
# the process exits; set SYNC_DEMO_DATABASE_URL to a file URL for
# state that survives.
db:
    cargo run --release -p task-db -- all

# Launch server + web side-by-side; Ctrl+C kills both. Server lines
# prefixed [srv], web lines [web].
dev:
    #!/usr/bin/env bash
    set -euo pipefail
    trap 'kill 0' EXIT
    TASK_SERVER_SEED=1 TASK_SERVER_BIND="0.0.0.0:9090" \
        cargo run --release -p task-server 2>&1 | sed 's/^/[srv] /' &
    just web 2>&1 | sed 's/^/[web] /' &
    wait

# ── Build & Test ─────────────────────────────────────────────────────────

# All recipes assume `.#ui` dev shell is already loaded (direnv handles
# this automatically on `cd` into the repo). On hosts without direnv,
# run `nix develop .#ui` once first, or prefix any recipe with
# `nix develop .#ui --command just <recipe>`.
check:
    cargo check --workspace

build:
    cargo build --workspace

test:
    cargo test --workspace

# Browser tests — Playwright. Run inside the playwright dev
# shell so Chromium + node come from Nix:
#
#   nix develop .#playwright --command just test-browser
#
# First run does an `npm install` for @playwright/test (no
# browser download — the shell's `PLAYWRIGHT_BROWSERS_PATH`
# points at nixpkgs's playwright-driver.browsers). Then runs
# the suite, booting `task-server` (release) + `dx serve`
# automatically via playwright.config.js's webServer block.
#
# IMPORTANT: `dx serve` hot-patch DOES NOT pick up new RSX
# attribute additions (id=, data-testid=) — only function-body
# changes. If a UI selector test fails on "element not found"
# after you added a new attribute, use `just test-browser-fresh`
# below to force a clean dx-serve restart.
test-browser:
    cd tests/playwright && npm install --silent && npx playwright test

# Browser tests with a guaranteed-fresh `dx serve` + `task-server`.
# Sets `CI=1` so playwright.config.js's `reuseExistingServer` is
# false; any existing dev servers on :8765 / :9090 are killed
# first so the new ones boot from scratch. Use this when:
#   - you added a new RSX attribute (`id=`, `data-testid=`) and
#     `just test-browser` finds the old DOM (hot-patch gotcha)
#   - you're hunting a sync regression and want a known-clean
#     server doc per test run
# Takes ~3 minutes longer than `just test-browser` because it
# rebuilds task-server (release) + the wasm bundle from cold.
test-browser-fresh:
    pkill -f "dx serve" || true
    pkill -f "target/release/task-server" || true
    sleep 1
    CI=1 just test-browser

# Multiplayer conformance suites (tests/multiplayer/): 5-way editor
# convergence + 20-peer presence churn against an ISOLATED stack —
# its own task-server (port 18091, throwaway TASK_DATA_ROOT, seeded
# org + dev accounts) and a statically-served wasm bundle baked with
# TASK_VOX_URL_WEB pointing at it. Never touches the dev server on
# :18080. See tests/multiplayer/README.md for the suite status table
# and the current findings. ~5 min warm, longer on a cold build.
mp-test *ARGS:
    nix develop .#playwright --command tests/multiplayer/run.sh {{ARGS}}

# ── Lint / format / CI ───────────────────────────────────────────────────

fmt:
    cargo fmt --all

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

ci:
    cargo fmt --all -- --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo nextest run --workspace --profile ci

# ── Git hooks (capn) ─────────────────────────────────────────────────────

# Install the capn pre-commit + pre-push hooks. Run once per clone.
install-hooks:
    ./hooks/install.sh

# Run capn pre-commit checks manually (without committing).
capn-precommit:
    capn

# Run capn pre-push checks manually (without pushing).
capn-prepush:
    capn pre-push

# ── Releases / changelog ─────────────────────────────────────────────────

# Regenerate CHANGELOG.md from conventional commits.
changelog:
    git cliff -o CHANGELOG.md

# Preview release notes for the next bump (no file write).
changelog-preview:
    git cliff --unreleased

# ── Aliases ──────────────────────────────────────────────────────────────

alias c := check
alias b := build
alias t := test

# ── Deploy ───────────────────────────────────────────────────────────────

# Build task-cli (release) and ship it to starcommand for the
# task-email-watcher systemd service. The binary is placed at
# /var/lib/task-watcher/bin/task and the watcher is restarted.
#
# Called automatically from ~/.starcommand/justfile `deploy`, so
# `just deploy` in starcommand does the whole pipeline.
deploy-task-watcher host="root@192.168.0.106" remote="/var/lib/task-watcher/bin/task":
    cargo build --release -p task-cli
    scp target/release/task {{host}}:{{remote}}.new
    ssh {{host}} 'install -o task-watcher -g task-watcher -m 0755 {{remote}}.new {{remote}} && rm -f {{remote}}.new && systemctl restart task-email-watcher.service && sleep 2 && systemctl status task-email-watcher.service --no-pager | head -8'
