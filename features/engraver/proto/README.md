Engraver Proto API
==================

This crate contains the engraving pipeline and rendering types. The `api::`
module provides a small, ergonomic facade for common chart engraving workflows.

Quick start (layout a chart)
----------------------------

```rust
use engraver_proto::api::chart;
use engraver_proto::engraver::layout::chart::LayoutMode;

let layout = chart::layout_text(chart_text, &LayoutMode::default())?;
```

Prelude
-------

```rust
use engraver_proto::api::prelude::*;

let fonts = ChartFontBundle::new()?;
let style = leak_lead_sheet_style();
let engine = fonts.create_layout_engine(style);
```

Why `api::`?
-----------
- One-call helpers for chart layout
- Sensible defaults for fonts + style
- Avoids wiring the full engraver stack for simple use cases
