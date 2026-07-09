# Migration: keyflow-ui → fts-ui

`keyflow-ui` now depends on the FastTrack Studio shared design system,
`fts-ui` (lives at `../fts-ui/crates/fts-ui`). The chart **renderer**
(Vello scene mounts in `chart_graphics.rs` / `chart_renderer.rs`) stays
raw — every other piece of UI chrome should compose `fts-ui` primitives
instead of hand-rolled `<button>` / `<div class="card">` markup.

## How to use it

Use the existing `keyflow_ui::prelude::*` import. It now also re-exports
`fts_ui::prelude::*` and the `cn!` macro, so callers never need to
`use fts_ui::…` directly.

```rust
use crate::prelude::*;

rsx! {
    Card {
        CardHeader { CardTitle { "Chart" } }
        CardContent {
            Button {
                variant: ButtonVariant::Primary,
                onclick: move |_| { /* … */ },
                "Save"
            }
        }
    }
}
```

## What's already migrated

- `components/highlighted_editor.rs` — status footer uses `Text` +
  `TextVariant::{Muted, Small}` instead of raw `<span style=…>` pairs.
- `panels/preview_panel.rs` — "Reset View" overlay button → `Button` +
  `lucide_dioxus::RotateCcw` icon.
- `layouts/chart_editor.rs` — full toolbar migration:
  - examples-dropdown trigger → `Button { variant: Secondary, size: Small }` + `lucide_dioxus::ChevronDown` icon
  - "Reset" button → `Button { variant: Ghost, size: Small }`
  - page-nav prev/next buttons → `Button` + `ChevronLeft` / `ChevronRight` icons
  - 4-segment "Page / Half / 3-Line / 2-Line" semantic-zoom toggle → `SegmentedControl { size: Small }`
  - "Snippet / Page (A4)" preview-mode toggle → `SegmentedControl`

Total: 13 raw `<button class="…tailwind…">` instances replaced; 2 ad-hoc
"buttons-in-a-pill-div" segmented controls collapsed onto the typed
`SegmentedControl` API.

### Round 2 — dropdowns, theme tokens, providers

- **Examples dropdown** (`layouts/chart_editor.rs`) — replaced ad-hoc
  `relative div` + manual `examples_open` open-state plumbing with
  `Dropdown` / `DropdownTrigger` / `DropdownContent` / `DropdownItem`.
  The fts-ui primitives carry their own keyboard nav, focus management,
  and click-outside dismissal.
- **Hard-coded color classes** swapped for theme tokens that resolve
  through `ThemeProvider`:
  - FPS counter: `text-green-400` / `text-yellow-400` / `text-red-400`
    → `text-success` / `text-warning` / `text-destructive`.
  - Cursor position: `text-blue-400` → `text-info`.
  - "Parse error" indicator: `text-red-400` → `text-destructive`.
- **Squiggle overlay** (`components/highlighted_editor.rs`) — inline
  `rgb(255, 92, 92)` / `rgb(255, 176, 64)` / `rgb(92, 168, 255)` /
  `rgb(160, 160, 160)` → `var(--destructive)` / `var(--warning)` /
  `var(--info)` / `var(--muted-foreground)` so squiggle colors track
  the active theme.
- **Provider wrap** — `ChartEditorLayout` is now a thin shell that
  wraps an inner `ChartEditorLayoutInner` in `ThemeProvider { state:
  default_theme_state }` + `toast::ToastProvider`. Descendants get
  `use_toast()` and theme-token CSS variables for free.

## What still needs migration

Run this to find raw HTML elements that should become `fts-ui` components:

```bash
grep -rn 'rsx! *{ *button\|class: "card"\|<input\|<select' \
  crates/keyflow-ui/src --include='*.rs'
```

### Per-file checklist

| File | What to migrate | Suggested fts-ui targets |
|---|---|---|
| `layouts/chart_editor.rs` | Toolbar buttons (lines ~174, 204, 227, 325, 354, 396, 406, 416, 426, 444, 456) — currently raw `button { class: "…tailwind…" }` | `Toolbar` + `ToolbarButton` for the row, `Button` for one-off CTAs, `Tooltip` for icon-only buttons |
| `panels/preview_panel.rs:568` | Single raw button | `Button { variant: Primary }` |
| `panels/render_stats.rs` | Stats card | `Card` + `KeyValueRow` |
| `panels/chart_view.rs` | View-switcher | `Tabs` (+ `TabList` / `TabTrigger` / `TabContent`) or `SegmentedControl` |
| `components/highlighted_editor.rs` | Editor textarea wrapper. Keep the `<textarea>` raw (it's a transparent input layer) but wrap the surrounding chrome in `Card` + `CardContent` | `Card` |
| `layouts/chart_editor.rs` (header / metadata fields) | Title / artist / key inputs | `Input` (+ `InputVariant`), `Field` for label-validated form rows |
| Any modals / "are you sure" prompts | Replace ad-hoc overlays | `Dialog` / `AlertDialog` |
| Toast-style notifications | Replace local timers | `toast::ToastProvider` + `use_toast()` |

### Theme

Wrap the top-level `App` (or `ChartEditorLayout`) in `ThemeProvider` once
so `theme/tokens` resolve. The default preset matches FTS apps; if a
keyflow-specific palette is needed, branch via `theme_preset(...)`.

### Color tokens

Stop hard-coding colors like `rgb(255, 92, 92)`. Use Tailwind theme
classes the design system already defines:

| Hard-coded | Theme class |
|---|---|
| `rgb(255, 92, 92)` (errors) | `text-destructive` / `bg-destructive` |
| `rgb(255, 176, 64)` (warnings) | `text-warning` / `bg-warning` |
| `rgb(92, 168, 255)` (info) | `text-info` |
| `rgb(160, 160, 160)` (hints / muted) | `text-muted-foreground` |

The `ThemeProvider` resolves these against the active preset, so light
/ dark / custom themes Just Work without per-component styling.

## Single-source-of-truth rule

`keyflow/Cargo.toml` has a `[patch."https://github.com/FastTrackStudios/fts-ui.git"]`
section that redirects every transitive `fts-ui` (e.g. the one
`session-ui` pulls in over git) to the local sibling checkout. This
keeps the workspace on a single `fts-ui` version so components composed
here interop with components composed elsewhere in the FTS workspace.

## Pre-existing build issues (unrelated)

`cargo build -p keyflow-ui` currently fails with 6 errors that exist on
`main` today and have nothing to do with this migration:

- `vello` 0.7 vs 0.8 version mismatch in `chart_graphics.rs`.
- `dioxus::desktop` not resolving in `chart_graphics.rs`,
  `chart_view.rs`, and `preview_panel.rs`.

Migration work can proceed in any **non-failing** file without making
those worse; once the dep-graph fixes land, the migrated components
light up automatically.
