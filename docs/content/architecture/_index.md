+++
title = "Architecture"
description = "The patterns the architect template is showcasing."
weight = 20
+++

architect's architecture is opinionated: contracts are the source of
truth, implementations are pluggable, and the monorepo layout makes the
distinction visible at a glance.

## Pages in this section

- [The architect pattern](@/architecture/pattern.md) — what
  `#[derive(architect::Entity)]` emits and how to write an entity.
- [Multi-backend features](@/architecture/backends.md) — one contract,
  multiple implementations behind one facade.
- [Extensibility](@/architecture/extensibility.md) — in-tree vs
  third-party implementations; how the contract crate makes both
  paths interchangeable.
- [Async diagnostics](@/architecture/diagnostics.md) — opt-in moiré
  instrumentation for the axum_ws adapter; see what every task is
  waiting on via the live dashboard.
- [Idioms & enforcement](@/architecture/idioms.md) — the vox + Dioxus
  conventions (data flows through vox, never Dioxus server functions)
  and the CI gates that keep them honest.
- [Testing strata](@/architecture/testing.md) — native unit, native
  integration, browser e2e: which lives where.
- [Monorepo layout](@/architecture/layout.md) — `<role>` crates per
  feature/app and why the prefixes are duplicated in both the directory
  and the package name.
- [Spec coverage](@/architecture/specs.md) — per-feature
  `features/<feature>/spec/*.md` tracked by tracey.
