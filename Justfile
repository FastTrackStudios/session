# Justfile - Convenient commands for FastTrackStudio
# Install just: cargo install just
# Run commands: just <recipe-name>

# Default recipe - show help
_default:
    @just --list

# Run tracey dashboard server
@tracey:
    cargo xtask tracey check

# Generate traceability matrix
@tracey-matrix:
    cargo xtask tracey matrix

# Extract rules from specs
@tracey-rules:
    cargo xtask tracey rules

# Show impact analysis
@tracey-impact:
    cargo xtask tracey impact

# Build spec documentation
@dodeca:
    cargo xtask dodeca build

# Serve spec documentation locally
@dodeca-serve:
    cargo xtask dodeca serve

# Watch and rebuild spec documentation
@dodeca-watch:
    cargo xtask dodeca watch

# Run Rust tests
@test:
    cargo xtask test

# Run all tests (Rust + Playwright WASM integration)
@test-all:
    cargo xtask test
    cargo xtask playwright

# Run native integration tests (spawns test-extension)
@integration:
    cargo xtask integration

# Run WASM integration tests with Playwright
@playwright *args:
    cargo xtask playwright {{ args }}

# Run Playwright tests in UI mode (for debugging)
@playwright-ui:
    cargo xtask playwright --ui

# Install Playwright and run tests
@playwright-install:
    cargo xtask playwright --install

# Build all cells
@build:
    cargo xtask build

# Run DAW standalone cell
@run:
    cargo xtask run

# Quick development workflow: build and test
@dev:
    just build
    just test

# Clean build artifacts
@clean:
    cargo clean
    cd reference/tracey && cargo clean || true
    cd reference/dodeca && cargo clean || true
    cd reference/roam && cargo clean || true

# Full check: build, test, tracey
@check:
    just build
    just test
    just tracey

# Aliases for convenience
alias t := test
alias ta := test-all
alias i := integration
alias b := build
alias r := run
alias dc := dodeca
alias tr := tracey
alias pw := playwright
