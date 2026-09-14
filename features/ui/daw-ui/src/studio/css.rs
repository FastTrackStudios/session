//! The studio's stylesheet.
//!
//! One sheet, embedded as a string and mounted once. Not a linked file:
//! this crate is consumed across a repo boundary, where a relative
//! `href` has no stable path on disk, and the panels also have to render
//! under a renderer that does not fetch external CSS.
//!
//! ## Why so much of the layout lives here rather than in `rsx!`
//!
//! Two quantities change constantly in a DAW window and neither should
//! cost a render:
//!
//! - **Zoom.** Every item, marker, region, grid line and the playhead is
//!   positioned from `--pps` (pixels per second) in `calc()`. Zooming
//!   writes that one custom property on the root; the browser re-lays
//!   out from its own style engine and dioxus is never told. The
//!   alternative — recomputing pixel positions in Rust — re-renders the
//!   entire timeline for a gesture that changes no data at all.
//! - **Scrolling.** The TCP column is `position: sticky` in the left
//!   column and the ruler `position: sticky` in the top row of the SAME
//!   scroll container, so panning is the compositor's job end to end.
//!   Nothing subscribes to scroll offset, so nothing re-renders on it.
//!
//! `content-visibility: auto` on each lane row is what makes mounting
//! the whole session affordable: the engine skips style, layout and
//! paint for rows that are off screen, without any of the mount/unmount
//! churn a hand-written virtualiser needs. `contain-intrinsic-size`
//! gives the skipped rows their real height so the scrollbar does not
//! lie.

/// The sheet. Mount with [`super::StudioStyles`].
pub const STUDIO_CSS: &str = r#"
.studio {
  /* Fallbacks only — `Studio` writes these from
     `daw_theme_art::geometry::tcp`, which is where the panel's measured
     numbers live. Keep them in step so a bare mount still lines up. */
  --row-h: 70px;
  --row-pitch: 71px;
  --tcp-w: 343px;
  --ruler-h: 26px;
  --region-h: 18px;
  --marker-h: 15px;
  /* The opening zoom, and the source of truth for it — the page reads
     this on its first frame. Keep in step with `clock::DEFAULT_PPS`. */
  --pps: 40;
  --secs-per-beat: 0.5;
  --secs-per-bar: 2;

  /* Colours are NOT declared here. `studio::theme` writes the whole
     custom-property block onto this element from `theming::Theme`, which
     is what a `.ReaperTheme` import re-colours. The `var(..., fallback)`
     forms below exist only so the window is legible if it is ever
     mounted with no theme provided at all. */

  position: fixed;
  inset: 0;
  display: flex;
  flex-direction: column;
  background: var(--surface, #1b1b1f);
  color: var(--ink, #d8d8dc);
  font: 12px/1.35 ui-sans-serif, system-ui, "Inter", sans-serif;
  overflow: hidden;
}

/* ── The transport band ─────────────────────────────────────────── */

.studio-transport {
  flex: 0 0 auto;
  display: flex;
  align-items: center;
  gap: 10px;
  height: 40px;
  padding: 0 10px;
  background: var(--surface-raised, #232329);
  border-bottom: 1px solid var(--rule, #2e2e35);
}
.studio-transport button {
  /* The symbols inside are SVG, not characters — see `TransportGlyph`.
     `currentColor` in the drawing means they inherit this rule's colour,
     so they follow the theme like any other text would. */
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: 24px;
  min-width: 30px;
  padding: 0 8px;
  border: 1px solid var(--rule, #2e2e35);
  border-radius: var(--radius, 4px);
  background: var(--surface-sunken, #141417);
  color: var(--ink, #d8d8dc);
  font: inherit;
  cursor: pointer;
}
.studio-transport button:hover { filter: brightness(1.35); }
.studio-transport button[aria-pressed="true"] {
  background: var(--accent, #38bdf8);
  border-color: var(--accent, #38bdf8);
  /* Readable ON the accent whatever the accent is — a fixed ink here is
     what makes an imported theme's bright accent unreadable. */
  color: var(--surface, #17170c);
}
.studio-clock {
  font-variant-numeric: tabular-nums;
  font-weight: 600;
  font-size: 13px;
  padding: 0 10px;
  min-width: 128px;
  text-align: right;
}
.studio-stat { color: var(--ink-dim, #8a8a92); font-size: 11px; }

/* ── The one scroll container ───────────────────────────────────── */

.studio-scroll {
  flex: 1 1 auto;
  overflow: auto;
  position: relative;
  /* The pane paints its own layer, so a pan never repaints the
     transport bar above it. */
  will-change: scroll-position;
  contain: paint;
}

/* Two columns, two rows: corner, ruler, TCP, lanes. `sticky` pins the
   first of each inside the scroller, which is exactly a DAW's shape and
   costs nothing per frame. */
.studio-grid {
  display: grid;
  grid-template-columns: var(--tcp-w) max-content;
  grid-template-rows: max-content max-content;
  width: max-content;
}

.studio-corner {
  position: sticky;
  top: 0;
  left: 0;
  z-index: 4;
  background: var(--ruler-bg, #232329);
  border-right: 1px solid var(--rule, #2e2e35);
  border-bottom: 1px solid var(--rule, #2e2e35);
}

/* The ruler and the lanes span the same time axis, so they take their
   width from the same two variables — the song's length and the zoom. */
.studio-ruler, .studio-lanes {
  width: calc(var(--len, 60) * var(--pps) * 1px);
}

.studio-ruler {
  position: sticky;
  top: 0;
  z-index: 3;
  background: var(--ruler-bg, #232329);
  border-bottom: 1px solid var(--rule, #2e2e35);
}

.studio-tcp {
  display: flex;
  flex-direction: column;
  position: sticky;
  left: 0;
  z-index: 2;
  /* Only what shows THROUGH the rows — each one paints its own surface,
     tinted with the track's colour, at the panel's measured width. */
  background: var(--surface, #1b1b1f);
  border-right: 1px solid var(--rule, #2e2e35);
}
.studio-tcp .studio-row { contain-intrinsic-size: var(--tcp-w) var(--row-h); }

.studio-lanes {
  position: relative;
  z-index: 1;
  /* NO gradient grid.
     Two stacked `repeating-linear-gradient`s across this element — which
     is 24,000px wide at a normal zoom — was the single most expensive
     thing in the window: 8.7 fps with them, 61.7 without, measured on a
     steady box. The grid is drawn by the canvas now, which paints only
     the bars that are actually on screen.
     This element draws nothing else either. It exists to carry the
     content's SIZE so the scrollbars stay honest, and the canvas
     overlays it; its height is set from the row count in
     `arrange::Lanes`. */
  background-color: var(--arrange-bg, #16161a);
}

/* ── Ruler contents ─────────────────────────────────────────────── */

.studio-region-lane {
  position: relative;
  height: var(--region-h);
  background: var(--region-lane-bg, #232329);
}
.studio-marker-lane {
  position: relative;
  height: var(--marker-h);
  background: var(--marker-lane-bg, #232329);
  border-top: 1px solid var(--rule, #2e2e35);
}
.studio-bar-lane {
  position: relative;
  height: var(--ruler-h);
  background: var(--ruler-bg, #232329);
  border-top: 1px solid var(--rule, #2e2e35);
}

.studio-region {
  position: absolute;
  top: 1px;
  height: calc(var(--region-h) - 2px);
  left: calc(var(--t0) * var(--pps) * 1px);
  width: calc(var(--dur) * var(--pps) * 1px);
  border-radius: 3px;
  padding-left: 5px;
  font-size: 9px;
  line-height: calc(var(--region-h) - 2px);
  color: var(--region-lane-text, #17170c);
  overflow: hidden;
  white-space: nowrap;
  cursor: pointer;
}

.studio-marker {
  position: absolute;
  top: 1px;
  left: calc(var(--t0) * var(--pps) * 1px);
  height: calc(var(--marker-h) - 2px);
  padding: 0 5px 0 4px;
  border-radius: 0 3px 3px 0;
  border-left: 2px solid #0000004d;
  font-size: 9px;
  line-height: calc(var(--marker-h) - 2px);
  color: var(--marker-lane-text, #17170c);
  white-space: nowrap;
  cursor: pointer;
}

.studio-bar {
  position: absolute;
  top: 0;
  left: calc(var(--t0) * var(--pps) * 1px);
  height: 100%;
  padding-left: 4px;
  border-left: 1px solid var(--ruler-fg2, #2e2e35);
  font-size: 9px;
  line-height: var(--ruler-h);
  color: var(--ruler-fg, #8a8a92);
  white-space: nowrap;
  pointer-events: none;
}

/* ── Track rows ─────────────────────────────────────────────────── */

/* Both columns' rows. The pitch is shared, which is the whole alignment
   contract between the TCP and its lanes: one constant, two columns. */
/* REAPER alternates lane backgrounds per track. Only the LANES: a TCP row
   paints its own surface, tinted with the track's colour, so striping
   underneath it would be invisible where it showed at all. */
.studio-lanes .studio-row:nth-child(odd) { background: var(--row-bg-a, #1b1b1f); }
.studio-lanes .studio-row:nth-child(even) { background: var(--row-bg-b, #1e1e23); }

.studio-row {
  height: var(--row-h);
  /* The pitch is authoritative, in BOTH columns.
     A TCP row draws a real track panel and can render a pixel or two
     taller than the height it was given; a lane next to it draws
     nothing. Let either stretch its own slot and the two columns drift
     apart by that much per track — invisible at the top of the window
     and a whole row out by the bottom of a sixty-five track session. */
  box-sizing: border-box;
  overflow: hidden;
  flex: 0 0 auto;
  border-bottom: 1px solid var(--row-divider, #2e2e35);
  /* NO `content-visibility: auto`.
     It reads like free culling and measured like the opposite: the
     property defers realising a row until it scrolls into view, so a
     sweep pays style, layout and paint for row after row at exactly the
     wrong moment, and pays it again on the way back. The control page
     that holds a locked 62 fps with MORE content than this window does
     not use it. */
}

/* ── Lanes and items ────────────────────────────────────────────── */

.studio-lane { position: relative; }
.studio-lanes .studio-row { contain-intrinsic-size: auto var(--row-h); }

.studio-item {
  position: absolute;
  top: 2px;
  height: calc(var(--row-h) - 5px);
  left: calc(var(--t0) * var(--pps) * 1px);
  width: calc(var(--dur) * var(--pps) * 1px);
  box-sizing: border-box;
  border: 1px solid var(--item-edge, #00000066);
  border-radius: 3px;
  background: var(--item-color, var(--item-fallback, #46506a));
  overflow: hidden;
  contain: strict;
  cursor: default;
}
.studio-item[data-muted="true"] { opacity: 0.4; }
/* The `items:px` probe supplies left/width itself — see `arrange`. */
.studio-item-px { left: auto; width: auto; }
/* `items:bare`: no containment, no overflow clip — the control page's
   block, for comparison. */
.studio-item-bare { contain: none; overflow: visible; }

/* `lanes:noflex` and `lanes:nohead` measured innocent when the gradient
   was found (7.8 and 7.6 fps against a 8.7 baseline, all inside the
   noise). Kept only so that finding can be re-checked cheaply. */
.studio.lanes-noflex .studio-lanes { display: block; }
.studio.lanes-nohead .studio-playhead { display: none; }
.studio-item-name {
  display: block;
  padding: 1px 4px;
  font-size: 9px;
  color: var(--item-label, #e8e8ee);
  text-shadow: 0 1px 1px #0009;
  white-space: nowrap;
  overflow: hidden;
  pointer-events: none;
}
/* The peak canvas fills what is left under the label. Sized in device
   pixels by the drawing pass; stretched by CSS so a zoom is free until
   the next redraw catches up. */
.studio-item canvas {
  display: block;
  width: 100%;
  height: calc(100% - 12px);
}

/* The drawn arrangement. An overlay, positioned from the layout by the
   renderer itself — it has to line up with the lanes exactly, and the
   layout is the only thing that knows where they ended up. Not a child
   of the scroller: it is viewport-sized and must not scroll. */
.studio-canvas {
  position: absolute;
  z-index: 1;
  pointer-events: none;   /* until the interaction model lands */
  display: block;
}

/* ── The playhead ───────────────────────────────────────────────── */

/* Spans the lanes, moved by `transform` from a rAF loop in the page.
   `translate3d` keeps it on the compositor: playback moves a layer and
   never touches style, layout or dioxus. */
.studio-playhead {
  position: absolute;
  top: 0;
  left: 0;
  width: 1px;
  height: 100%;
  background: var(--play-cursor, #f6f6fa);
  pointer-events: none;
  z-index: 5;
  /* Promoted, and then left alone: the line is moved by a Web Animation
     that WebKit runs on the compositor (see `studio::clock`), not by a
     transform written from script. That is what keeps it on the path
     WebKit does NOT cap near 60fps. */
  will-change: transform;
}

/* ── States ─────────────────────────────────────────────────────── */

.studio-empty {
  padding: 18px;
  color: var(--ink-dim, #8a8a92);
}

/* ── A/B probe ──────────────────────────────────────────────────────
   `FTS_STUDIO_NO_STICKY=1` unpins the TCP column and the ruler. The
   window is WRONG like this — the whole point of both is that they stay
   put — but it isolates what `position: sticky` costs on a subtree of a
   few thousand nodes over a surface many screens wide, which is the
   one thing both scroll axes have in common. */
.studio.no-sticky .studio-tcp,
.studio.no-sticky .studio-ruler,
.studio.no-sticky .studio-corner {
  position: static;
}
"#;