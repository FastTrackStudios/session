//! The arrangement's lanes, as components.
//!
//! The same picture the recorded scene draws, built out of elements
//! instead of paint commands, so that one `rsx!` renders it on
//! dioxus-native (Blitz on wgpu), in a WebView and in a browser — with
//! hot reload and AccessKit for free, and with the session's performance
//! view able to live in the same window and the same signals.
//!
//! # The three rules this is built to
//!
//! All three come from measurement, not taste
//! (`apps/session-daw/src/bin/blitz_arrange.rs` and `blitz_controls.rs`):
//!
//! 1. **Only what is on screen may be in the tree.** Blitz has no
//!    `content-visibility`, so an off-screen item is a node that style
//!    and layout both walk. The cost of a frame tracks the NUMBER of
//!    nodes, not how many of them changed — one dirty node in a tree of
//!    1389 costs 6.7 ms, and the same tree cut to what a 1440px window
//!    shows costs under 1 ms. So [`Lanes`] renders the rows and items
//!    the [`View`] can see and nothing else.
//! 2. **Flat fills, square corners.** A `conic-gradient` and a
//!    `border-radius` cost a full millisecond a frame MORE than the
//!    vector art they replace, all of it in paint, because Vello
//!    re-encodes every gradient brush and turns every rounded corner
//!    into a path on every frame whether or not it changed. Paint is the
//!    one pass that is never incremental.
//! 3. **A pan is read where it is applied.** Reading the scroll in the
//!    component that OWNS the items rebuilds every item's vnode: 5.0 ms
//!    a frame against 0.17 ms. So the offset is applied by a transform
//!    on the one node that moves, and nothing above it subscribes.
//!
//! # Why an item's shape is an `<svg>` and its box is not
//!
//! A waveform is fifty amplitudes; as elements that is fifty divs an
//! item and thousands on screen, which rule 1 forbids outright. As one
//! `<svg>` path it is a single node whose markup Blitz hands to usvg
//! once, at construction — measured at near zero for an item that is not
//! changing, which a waveform never is. Everything that IS a rectangle
//! stays a rectangle, because a rectangle is cheaper than a path and
//! participates in layout and hit testing on its own.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use crate::prelude::*;

use super::{ProjectRef, RowsRef};

/// The divider under every row, inside its height.
///
/// Part of the row rather than added to it, so a track set to 24 in
/// REAPER occupies 24 here too — adding it would drift the session a
/// pixel a row.
pub const DIVIDER: f64 = 1.0;

/// One note in a MIDI item, as fractions of the item.
///
/// Normalised rather than in seconds or semitones so the shape survives
/// a zoom without being rebuilt: `at` and `len` are fractions of the
/// item's length, `from_top` a fraction of its pitch range, already
/// resolved by whoever read the notes.
#[derive(Clone, Copy, PartialEq)]
pub struct Note {
    pub at: f32,
    pub len: f32,
    pub from_top: f32,
    /// How tall to draw it, as a fraction of the item's height.
    pub height: f32,
}

/// What an item CONTAINS.
///
/// An item drawn from a waveform it does not have is why a chord track
/// once looked like a shaker, so the two are different things here
/// rather than one thing with a flag.
#[derive(Clone, PartialEq)]
pub enum Shape {
    /// Peak amplitudes in 0..1, evenly spaced across the item.
    ///
    /// Where they come from is the host's business — real peaks off the
    /// media, or a stand-in — and keeping that out of here is what lets
    /// the same component draw a take that has been analysed and one
    /// that has not.
    Wave(Arc<[f32]>),
    /// The notes, for an item that holds MIDI.
    Notes(Arc<[Note]>),
}

/// Every item's shape, by item guid.
///
/// Behind an `Arc` and compared by pointer, like [`ProjectRef`]: a map
/// this size deep-compared on every render would cost more than the
/// thing it is memoising.
#[derive(Clone)]
pub struct Shapes(pub Arc<HashMap<String, Shape>>);

impl PartialEq for Shapes {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Default for Shapes {
    fn default() -> Self {
        Self(Arc::new(HashMap::new()))
    }
}

impl Shapes {
    /// The shape of an item, if it has been read.
    #[must_use]
    pub fn get(&self, guid: &str) -> Option<&Shape> {
        self.0.get(guid)
    }
}

/// What part of the session the lanes can see.
///
/// The same quantities the recorded scene's viewport holds, so a window
/// can hand the identical numbers to either renderer and get the
/// identical framing.
#[derive(Clone, Copy, PartialEq)]
pub struct View {
    pub scroll_x: f64,
    pub scroll_y: f64,
    /// Pixels per second, zoom included.
    pub pps: f64,
    pub zoom_y: f64,
    pub width: f64,
    pub height: f64,
}

impl Default for View {
    fn default() -> Self {
        Self {
            scroll_x: 0.0,
            scroll_y: 0.0,
            pps: 40.0,
            zoom_y: 1.0,
            width: 1280.0,
            height: 720.0,
        }
    }
}

/// How tall a row is drawn, from what the project stored.
///
/// Anything at or below zero is unset: a stored height of nought is not
/// a row, and a negative one is a corrupt file rather than an
/// instruction.
#[derive(Clone, Copy, PartialEq)]
pub struct Rows {
    pub default: f64,
    pub min: f64,
}

impl Default for Rows {
    fn default() -> Self {
        Self {
            default: 24.0,
            min: 3.0,
        }
    }
}

impl Rows {
    #[must_use]
    pub fn height_of(self, stored: Option<u32>) -> f64 {
        stored
            .map(f64::from)
            .filter(|h| *h > 0.0)
            .unwrap_or(self.default)
            .max(self.min)
    }
}

/// The colours the lanes draw in, as CSS.
///
/// Resolved once from the theme so no render parses a token, and held as
/// strings because that is what a style attribute takes — the conversion
/// would otherwise happen per item per frame.
#[derive(Clone, PartialEq)]
pub struct Colors {
    pub surface: String,
    pub row_a: String,
    pub row_b: String,
    pub divider: String,
    pub grid: String,
    pub grid_beat: String,
    /// What a track with no colour of its own lends its items.
    pub uncoloured: String,
    /// The shade a fade lays over the part of an item it takes away.
    pub fade: String,
    pub text: String,
}

impl Colors {
    /// The arrangement's own colours, off the theme.
    #[must_use]
    pub fn from_theme(theme: &crate::theming::Theme) -> Self {
        let c = |col: crate::theming::Color| rgba(col.r, col.g, col.b, f64::from(col.a) / 255.0);
        Self {
            surface: c(theme.arrange.bg),
            row_a: c(theme.arrange.row_bg[0]),
            row_b: c(theme.arrange.row_bg[1]),
            divider: c(theme.arrange.row_divider[0]),
            grid: c(theme.arrange.grid_measure),
            grid_beat: c(theme.arrange.grid_beat),
            uncoloured: c(theme.tokens.text_faint),
            // The recorded scene's own fade shade, as CSS.
            fade: rgba(0, 0, 0, 0.45),
            text: c(theme.tokens.text),
        }
    }
}

impl Default for Colors {
    fn default() -> Self {
        Self::from_theme(&crate::theming::Theme::default())
    }
}

/// `rgba(r, g, b, a)`, which is what a style attribute wants.
fn rgba(r: u8, g: u8, b: u8, a: f64) -> String {
    format!("rgba({r}, {g}, {b}, {a:.3})")
}

/// A track's colour, or what an uncoloured track lends its items.
fn track_color(colors: &Colors, track: &daw_proto::Track) -> String {
    track
        .color
        .map_or_else(|| colors.uncoloured.clone(), |rgb| rgb24(rgb, 1.0))
}

/// A packed `0xRRGGBB` as CSS, at an alpha.
fn rgb24(rgb: u32, alpha: f64) -> String {
    let r = u8::try_from((rgb >> 16) & 0xff).unwrap_or(0);
    let g = u8::try_from((rgb >> 8) & 0xff).unwrap_or(0);
    let b = u8::try_from(rgb & 0xff).unwrap_or(0);
    rgba(r, g, b, alpha)
}

/// Where every row starts and how tall it is, cumulatively.
///
/// Rows are whatever height the project says, so "which row is at this
/// y" is a lookup rather than a division — there is no pitch.
#[derive(Clone, PartialEq)]
pub struct Offsets(Vec<f64>);

impl Offsets {
    /// The offsets for a row list.
    #[must_use]
    pub fn of(rows: &RowsRef, sizing: Rows) -> Self {
        let mut offsets = Vec::with_capacity(rows.len().saturating_add(1));
        let mut y = 0.0;
        for (track, _) in rows.iter() {
            offsets.push(y);
            y += sizing.height_of(track.height);
        }
        offsets.push(y);
        Self(offsets)
    }

    /// How tall the whole session is.
    #[must_use]
    pub fn content_height(&self) -> f64 {
        self.0.last().copied().unwrap_or(0.0)
    }

    /// A row's top and its height.
    #[must_use]
    pub fn row(&self, row: usize) -> Option<(f64, f64)> {
        let top = *self.0.get(row)?;
        let bottom = *self.0.get(row.checked_add(1)?)?;
        Some((top, bottom - top))
    }

    /// The rows that intersect `view`, with one row of bleed each side.
    ///
    /// The bleed is not politeness: a row scrolled half off the top
    /// still has to paint its visible half, and dropping it tears the
    /// edge of the screen during exactly the fast scroll this is for.
    #[must_use]
    pub fn visible(&self, view: View) -> std::ops::Range<usize> {
        let rows = self.0.len().saturating_sub(1);
        if rows == 0 {
            return 0..0;
        }
        let zoom = if view.zoom_y > 0.0 { view.zoom_y } else { 1.0 };
        let top = view.scroll_y / zoom;
        let bottom = (view.scroll_y + view.height) / zoom;
        let first = self.0.partition_point(|&y| y <= top).saturating_sub(1);
        let last = self.0.partition_point(|&y| y < bottom);
        first.min(rows)..last.min(rows)
    }
}

/// The bar grid: how far apart the lines are, in seconds.
///
/// Held as two spacings rather than a tempo map because that is all the
/// drawing needs, and because it keeps the adaptive division — which
/// line still fits at this zoom — with the caller that knows the zoom.
/// A `beat` of `None` is a zoom too far out for anything under a bar.
#[derive(Clone, Copy, PartialEq, Default)]
pub struct Grid {
    pub bar: f64,
    pub beat: Option<f64>,
}

/// The grid, over the lanes.
///
/// Over rather than under, the way the recorded scene draws it: a bar
/// line you cannot see across an item is a bar line that stops existing
/// exactly where the session is densest.
#[component]
fn Lines(grid: Grid, view: View, colors: Colors) -> Element {
    let pps = view.pps.max(1e-9);
    let from = (view.scroll_x / pps).max(0.0);
    let to = (view.scroll_x + view.width) / pps;
    rsx! {
        div {
            style: "position:absolute; inset:0; pointer-events:none;",
            // Beats first, so a bar line drawn at the same x wins.
            if let Some(beat) = grid.beat.filter(|b| *b > 0.0) {
                for x in every(beat, from, to, view) {
                    div {
                        style: "position:absolute; top:0; bottom:0; left:{x:.1}px; \
                                width:1px; background:{colors.grid_beat};",
                    }
                }
            }
            if grid.bar > 0.0 {
                for x in every(grid.bar, from, to, view) {
                    div {
                        style: "position:absolute; top:0; bottom:0; left:{x:.1}px; \
                                width:1px; background:{colors.grid};",
                    }
                }
            }
        }
    }
}

/// Where a line lands on screen, every `step` seconds across the view.
///
/// Capped, because a zoom far enough out asks for a line a pixel — and
/// a grid denser than the screen is a flat wash that costs a node for
/// every stripe in it.
fn every(step: f64, from: f64, to: f64, view: View) -> Vec<f64> {
    /// The most lines worth drawing across one screen.
    const MOST: usize = 400;
    if step <= 0.0 || to <= from {
        return Vec::new();
    }
    let first = (from / step).floor().max(0.0);
    let count = ((to - from) / step).ceil().max(0.0);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::as_conversions,
        reason = "a count of lines across one screen, clamped below"
    )]
    let count = (count as usize).min(MOST);
    (0..=count)
        .map(|i| {
            let at = (first + f64::from(u32::try_from(i).unwrap_or(0))) * step;
            at.mul_add(view.pps, -view.scroll_x)
        })
        .filter(|x| *x >= 0.0 && *x <= view.width)
        .collect()
}

/// The lanes, and the items on them.
///
/// Renders the rows [`View`] can see, and on each of those the items
/// whose span reaches the screen. Everything else is not in the tree —
/// see rule 1.
#[component]
pub fn Lanes(
    project: ProjectRef,
    rows: RowsRef,
    view: View,
    colors: Colors,
    #[props(default)] shapes: Shapes,
    #[props(default)] sizing: Rows,
    #[props(default)] grid: Grid,
) -> Element {
    // The cumulative offsets are a function of the row list alone, so
    // they are computed when THAT changes rather than on every render —
    // a fold is a discrete act, a pan is not.
    let offsets = use_memo({
        let rows = rows.clone();
        move || Offsets::of(&rows, sizing)
    });
    let offsets = offsets();
    let visible = offsets.visible(view);
    // Which slice of the timeline is on screen, with a screen of bleed
    // either side for the same reason the rows get one.
    let (from, to) = {
        let pps = view.pps.max(1e-9);
        let left = view.scroll_x / pps;
        let right = (view.scroll_x + view.width) / pps;
        (left - 1.0, right + 1.0)
    };

    rsx! {
        div {
            style: "position:relative; width:{view.width}px; height:{view.height}px; \
                    overflow:hidden; background:{colors.surface}; font-family:{FONT};",
            "data-testid": "studio-lanes",
            for row in visible {
                if let Some((top, height)) = offsets.row(row) {
                    if let Some((track, _)) = rows.get(row) {
                        Lane {
                            key: "{track.guid}",
                            row,
                            top: top.mul_add(view.zoom_y, -view.scroll_y),
                            height: height * view.zoom_y,
                            track: track.clone(),
                            project: project.clone(),
                            view,
                            colors: colors.clone(),
                            shapes: shapes.clone(),
                            from,
                            to,
                        }
                    }
                }
            }
            Lines { grid, view, colors: colors.clone() }
        }
    }
}

/// One lane: its stripe, its divider, and the items on it.
#[component]
#[expect(
    clippy::too_many_arguments,
    reason = "a lane is its geometry, its data and its palette; bundling them \
              into a struct would make every render clone what it already has"
)]
fn Lane(
    row: usize,
    top: f64,
    height: f64,
    track: daw_proto::Track,
    project: ProjectRef,
    view: View,
    colors: Colors,
    shapes: Shapes,
    from: f64,
    to: f64,
) -> Element {
    let body = (height - DIVIDER).max(0.5);
    let stripe = if row % 2 == 0 {
        &colors.row_a
    } else {
        &colors.row_b
    };
    // Items sit inside their lane, but a two-pixel inset on a
    // three-pixel row leaves nothing to see — so it scales down as the
    // row does, and a collapsed session still shows its items as bands
    // rather than as empty lanes.
    let inset = (body * 0.05).clamp(0.0, 2.0);
    let colour = track_color(&colors, &track);

    rsx! {
        div {
            style: "position:absolute; left:0; top:{top}px; width:100%; height:{height}px;",
            // The stripe and the divider under it. Two rectangles, flat.
            div { style: "position:absolute; inset:0 0 {DIVIDER}px 0; background:{stripe};" }
            div {
                style: "position:absolute; left:0; right:0; bottom:0; height:{DIVIDER}px; \
                        background:{colors.divider};",
            }
            for item in project.lane(&track.guid) {
                {
                    let x0 = item.position.as_seconds();
                    let x1 = x0 + item.length.as_seconds().max(0.001);
                    // Off screen is not in the tree.
                    if x1 < from || x0 > to {
                        return rsx! {};
                    }
                    let left = x0.mul_add(view.pps, -view.scroll_x);
                    let width = ((x1 - x0) * view.pps).max(1.0);
                    // Where the screen cuts this item, as fractions of
                    // it — the shape and the box it is drawn in are both
                    // cut to this.
                    let seen_from = ((-left) / width).clamp(0.0, 1.0);
                    let seen_to = ((view.width - left) / width).clamp(0.0, 1.0);
                    let colour = item.color.map_or_else(
                        || colour.clone(),
                        |rgb| rgb24(rgb, if item.muted { 0.4 } else { 1.0 }),
                    );
                    rsx! {
                        Item {
                            key: "{item.guid}",
                            guid: item.guid.clone(),
                            left,
                            top: inset,
                            width,
                            height: (body - inset * 2.0).max(0.5),
                            colour,
                            title: project.title(item).map(str::to_owned),
                            clipped: shapes
                                .get(&item.guid)
                                .map(|shape| shape.clip(seen_from, seen_to)),
                            item_width: width,
                            row_height: height,
                            fade_in: item.fade_in_length.as_seconds().max(0.0) * view.pps,
                            fade_out: item.fade_out_length.as_seconds().max(0.0) * view.pps,
                            fade: colors.fade.clone(),
                            text: colors.text.clone(),
                        }
                    }
                }
            }
        }
    }
}

/// How tall a ROW has to be before a title is worth writing on it.
///
/// The row rather than the item, because that is what the name has to
/// fit inside once the inset has taken its share.
const TITLE_MIN_H: f64 = 12.0;

/// How much room a name needs beside the padding, in pixels.
///
/// Below this there is space for a letter and an ellipsis, which is not
/// a name — and a title that overflows its item reads as belonging to
/// the one beside it.
const TITLE_MIN_W: f64 = 12.0;

/// The gap between an item's edge and its name.
const TITLE_PAD: f64 = 3.0;

/// How big a title is drawn.
const TITLE_SIZE: f64 = 8.0;

/// The line box that puts an 8px baseline ten pixels down.
///
/// A title's baseline sits at the item's top plus the type size plus
/// two, which is where the recorded scene puts it. CSS has no way to
/// say "baseline here", so it is said in the only units a line box
/// takes: `(line-height - (ascent + descent)) / 2 + ascent` is the
/// baseline inside the box, and this is the height that makes that ten.
const TITLE_LINE: f64 = 14.5;

/// The face the arrangement is lettered in.
///
/// Named rather than left to the default so that every renderer letters
/// it the same: the recorded scene embeds this face, and a component
/// tree that takes whatever the system calls `sans-serif` is a different
/// picture on every machine it opens on.
pub const FONT: &str = "'DejaVu Sans', 'Bitstream Vera Sans', sans-serif";

/// One item: the box, what it contains, and its fades.
#[component]
#[expect(
    clippy::too_many_arguments,
    reason = "the same reason as `Lane` — these are the item's own \
              geometry and colours, already computed by its lane"
)]
fn Item(
    guid: String,
    left: f64,
    top: f64,
    width: f64,
    height: f64,
    colour: String,
    title: Option<String>,
    /// The part of the shape that is on screen, and the part of the item
    /// it covers.
    clipped: Option<Clipped>,
    /// How wide the whole item is, which is what those fractions are of.
    item_width: f64,
    /// How tall the ROW is, which is what decides whether a name fits.
    row_height: f64,
    fade_in: f64,
    fade_out: f64,
    fade: String,
    text: String,
) -> Element {
    // The body dimmed and the shape over it in full colour: an item is
    // read by its waveform, and a solid block of colour is a waveform
    // you cannot see through.
    let body = dim(&colour, 0.42);
    let named =
        title.filter(|_| row_height >= TITLE_MIN_H && width - TITLE_PAD * 2.0 >= TITLE_MIN_W);

    rsx! {
        div {
            style: "position:absolute; left:{left}px; top:{top}px; width:{width}px; \
                    height:{height}px; background:{body}; overflow:hidden;",
            "data-item": "{guid}",
            // What the item CONTAINS. One node, drawn once: a path is
            // the only shape a waveform can be without a div per peak.
            if let Some((d, left, wide)) = clipped.as_ref().and_then(|clipped| {
                let d = clipped.shape.path(height)?;
                let left = clipped.from * item_width;
                Some((d, left, (clipped.to - clipped.from) * item_width))
            }) {
                svg {
                    // Sized by ATTRIBUTE as well as by style: Blitz
                    // renders an inline `<svg>` by handing its markup to
                    // usvg, and usvg reads the root element's own
                    // width/height. Sized only in CSS it parses to
                    // nothing and draws nothing.
                    // Whole pixels, because that is what Blitz lays the
                    // element out at: a fractional attribute sizes the
                    // usvg tree to one box and the layout to another, and
                    // the shape is then scaled by the difference. Tried
                    // both and measured it — fractional is worse.
                    width: "{wide.max(1.0):.0}",
                    height: "{height.max(1.0):.0}",
                    style: "position:absolute; left:{left:.1}px; top:0;",
                    view_box: "0 0 1000 100",
                    preserve_aspect_ratio: "none",
                    path { d: "{d}", fill: "{colour}" }
                }
            }
            // The fades, as the part of the item they take away. Two
            // triangles, which `clip-path` draws with no node of their
            // own beyond the box they are in.
            if fade_in > 0.5 {
                div {
                    style: "position:absolute; left:0; top:0; height:100%; \
                            width:{fade_in.min(width)}px; background:{fade}; \
                            clip-path: polygon(0 0, 100% 0, 0 100%);",
                }
            }
            if fade_out > 0.5 {
                div {
                    style: "position:absolute; right:0; top:0; height:100%; \
                            width:{fade_out.min(width)}px; background:{fade}; \
                            clip-path: polygon(100% 0, 100% 100%, 0 0);",
                }
            }
            if let Some(name) = named {
                div {
                    style: "position:absolute; left:{TITLE_PAD}px; top:0; \
                            font-size:{TITLE_SIZE}px; line-height:{TITLE_LINE}px; \
                            color:{text}; white-space:nowrap; pointer-events:none;",
                    "{name}"
                }
            }
        }
    }
}

/// A colour at a fraction of its opacity.
///
/// The colours here are always `rgba(...)` because [`rgba`] made them,
/// so this rewrites the last component rather than parsing anything.
fn dim(colour: &str, alpha: f64) -> String {
    colour.rsplit_once(',').map_or_else(
        || colour.to_owned(),
        |(head, tail)| {
            let existing: f64 = tail.trim().trim_end_matches(')').parse().unwrap_or(1.0);
            format!("{head}, {:.3})", existing * alpha)
        },
    )
}

impl Shape {
    /// The part of the shape between two fractions of the item,
    /// renormalised to fill the result.
    ///
    /// An item can be minutes long while the screen shows seconds of it,
    /// and the whole of its shape would then be an `<svg>` thousands of
    /// pixels wide holding a path whose visible part is a sliver. Blitz
    /// renders an inline `<svg>` as an image, so that is not merely
    /// wasteful — past a certain width it stops drawing at all, which is
    /// how a long item came to be a flat block where the recorded scene
    /// drew a waveform.
    ///
    /// So the shape is cut to what is on screen, and the box it is drawn
    /// in is cut with it.
    #[must_use]
    pub fn clip(&self, from: f64, to: f64) -> Clipped {
        let (from, to) = (from.clamp(0.0, 1.0), to.clamp(0.0, 1.0));
        if to - from >= 1.0 || to <= from {
            return Clipped {
                shape: self.clone(),
                from: 0.0,
                to: 1.0,
            };
        }
        let span = to - from;
        match self {
            Self::Wave(peaks) => {
                let n = peaks.len();
                if n < 2 {
                    return Clipped {
                        shape: self.clone(),
                        from: 0.0,
                        to: 1.0,
                    };
                }
                let last = n.saturating_sub(1);
                let places = f64::from(u32::try_from(last).unwrap_or(u32::MAX));
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    clippy::as_conversions,
                    reason = "an index into a peak list, clamped to it"
                )]
                let at = |f: f64| ((f * places).floor() as usize).min(last);
                // One peak of overhang each side, so the cut edge is a
                // continuation of the shape rather than a cliff.
                let start = at(from).saturating_sub(1);
                let end = at(to).saturating_add(2).min(n);
                let end = end.max(start.saturating_add(2)).min(n);
                // The span the KEPT peaks actually cover, which is not
                // the span that was asked for: they are whole peaks and
                // the request was a fraction. Reporting the request
                // instead would stretch the slice across a box it does
                // not fill, and every point in it would land a little
                // off — a waveform in the right place with the wrong
                // phase, which reads as a different recording.
                let place =
                    |i: usize| f64::from(u32::try_from(i).unwrap_or(u32::MAX)) / places.max(1.0);
                Clipped {
                    shape: Self::Wave(peaks[start..end].iter().copied().collect()),
                    from: place(start),
                    to: place(end.saturating_sub(1)),
                }
            }
            Self::Notes(notes) => Clipped {
                from,
                to,
                shape: Self::Notes(
                    notes
                        .iter()
                        .filter(|note| {
                            let (a, b) = (f64::from(note.at), f64::from(note.at + note.len));
                            b >= from && a <= to
                        })
                        .map(|note| {
                            #[expect(
                                clippy::cast_possible_truncation,
                                reason = "a fraction of an item, and the shape is f32"
                            )]
                            let (at, len) = (
                                ((f64::from(note.at) - from) / span) as f32,
                                (f64::from(note.len) / span) as f32,
                            );
                            Note { at, len, ..*note }
                        })
                        .collect(),
                ),
            },
        }
    }

    /// The shape as one path, in a 1000x100 box, or nothing if the item
    /// is too short to show one.
    ///
    /// One path rather than one node per peak or per note: a waveform is
    /// fifty amplitudes and a bar of sixteenths is sixteen blocks, and
    /// either as elements is thousands of nodes on a screen — which is
    /// the one thing the lanes may not spend.
    #[must_use]
    pub fn path(&self, height: f64) -> Option<String> {
        match self {
            Self::Wave(peaks) => {
                (!peaks.is_empty() && height >= 4.0).then(|| envelope(peaks, height))
            }
            Self::Notes(notes) => (!notes.is_empty() && height >= 2.0).then(|| roll(notes)),
        }
    }
}

/// A shape cut down to part of its item, and the part it covers.
///
/// The fractions come back with it because a cut does not land where it
/// was asked to: peaks are whole, so the slice that covers a request
/// covers a little more than it, and the box the shape is drawn in has
/// to be the one it actually fills.
#[derive(Clone, PartialEq)]
pub struct Clipped {
    pub shape: Shape,
    pub from: f64,
    pub to: f64,
}

/// The notes as one path: a block each, stacked by pitch.
///
/// A piano roll squeezed into a lane. What a preview is for is the SHAPE
/// of the part, which is why the pitch range is the item's own rather
/// than the full 0..127 — a bass part pressed into the bottom eighth of
/// a scale it never plays in has no shape at all.
fn roll(notes: &[Note]) -> String {
    let mut path = String::with_capacity(notes.len() * 40);
    for note in notes {
        // Not clamped to the box: a clipped shape has notes that begin
        // before its left edge and end past its right, and cutting them
        // to the edge would redraw every one of them as a block starting
        // exactly there.
        let x0 = f64::from(note.at) * 1000.0;
        // Every note gets a width, however short: a preview of a
        // sixteenth-note part at this zoom is otherwise nothing at all.
        let x1 = f64::from(note.len.max(0.004)).mul_add(1000.0, x0);
        if x1 < 0.0 || x0 > 1000.0 {
            continue;
        }
        let h = f64::from(note.height.clamp(0.01, 1.0)) * 100.0;
        let y = (100.0 - h) * f64::from(note.from_top.clamp(0.0, 1.0));
        let _ = write!(
            path,
            "M{x0:.1} {y:.1}L{x1:.1} {y:.1}L{x1:.1} {:.1}L{x0:.1} {:.1}Z",
            y + h,
            y + h
        );
    }
    path
}

/// A set of peaks as one closed path, in a 1000x100 box.
///
/// Mirrored about the middle, the way a waveform is drawn: the top edge
/// out and the bottom edge back. A viewBox so the item's own width does
/// the horizontal scaling and the path survives a horizontal zoom
/// unchanged.
///
/// The VERTICAL scaling is not left to the box, because the two parts of
/// it do not scale together. A waveform fills its lane bar a pixel of
/// margin, and it keeps a hair of amplitude at silence so that a quiet
/// item still has a line down its middle and reads as audio rather than
/// as a gap — and that hair is a PIXEL, not a fraction of the row. Left
/// to the viewBox it shrinks with the row and a quiet item on a short
/// lane disappears. So the row's height comes in here and the two are
/// converted into the box's units separately.
fn envelope(peaks: &[f32], height: f64) -> String {
    /// The margin left at full amplitude, in pixels.
    const MARGIN: f64 = 1.0;
    /// The amplitude silence still draws with, in pixels.
    const FLOOR: f64 = 0.6;
    let height = height.max(1.0);
    // A pixel is this much of the box.
    let per_px = 100.0 / height;
    let scale = MARGIN.mul_add(-per_px, 50.0);
    let floor = FLOOR * per_px;
    let count = peaks.len().max(2);
    let step = 1000.0 / f64::from(u32::try_from(count.saturating_sub(1)).unwrap_or(1)).max(1.0);
    let amp = |p: f32| f64::from(p).clamp(0.0, 1.0).mul_add(scale, floor);

    let mut path = String::with_capacity(count * 16);
    for (i, peak) in peaks.iter().enumerate() {
        let x = f64::from(u32::try_from(i).unwrap_or(0)) * step;
        let _ = write!(
            path,
            "{}{x:.1} {:.1}",
            if i == 0 { "M" } else { " L" },
            50.0 - amp(*peak)
        );
    }
    for (i, peak) in peaks.iter().enumerate().rev() {
        let x = f64::from(u32::try_from(i).unwrap_or(0)) * step;
        let _ = write!(path, " L{x:.1} {:.1}", 50.0 + amp(*peak));
    }
    path.push('Z');
    path
}

#[cfg(test)]
mod tests {
    use super::{Offsets, Rows, View, dim, envelope};

    /// The offsets are cumulative and every row has a `..end`.
    #[test]
    fn a_row_knows_its_top_and_its_height() {
        let offsets = Offsets(vec![0.0, 24.0, 60.0, 84.0]);
        assert_eq!(offsets.row(0), Some((0.0, 24.0)));
        assert_eq!(offsets.row(1), Some((24.0, 36.0)));
        assert_eq!(offsets.row(2), Some((60.0, 24.0)));
        assert_eq!(offsets.row(3), None, "there is no fourth row");
        assert!((offsets.content_height() - 84.0).abs() < f64::EPSILON);
    }

    /// Only the rows on screen, with one of bleed each side.
    #[test]
    fn only_what_is_on_screen_is_visible() {
        let offsets = Offsets((0..=100).map(|i| f64::from(i) * 24.0).collect());
        let view = View {
            scroll_y: 240.0,
            height: 240.0,
            zoom_y: 1.0,
            ..View::default()
        };
        let visible = offsets.visible(view);
        assert!(visible.start <= 10, "no bleed above: {visible:?}");
        assert!(visible.end >= 20, "the last visible row was dropped");
        assert!(
            visible.end - visible.start < 15,
            "far more than the screen: {visible:?}"
        );
    }

    /// An empty row list asks for nothing rather than panicking.
    #[test]
    fn nothing_is_visible_in_an_empty_session() {
        assert_eq!(Offsets(vec![0.0]).visible(View::default()), 0..0);
        assert_eq!(Offsets(Vec::new()).visible(View::default()), 0..0);
    }

    /// A stored height of nought is not a row.
    #[test]
    fn an_unset_height_falls_back_rather_than_collapsing() {
        let rows = Rows {
            default: 24.0,
            min: 3.0,
        };
        assert!((rows.height_of(Some(48)) - 48.0).abs() < f64::EPSILON);
        assert!((rows.height_of(Some(0)) - 24.0).abs() < f64::EPSILON);
        assert!((rows.height_of(None) - 24.0).abs() < f64::EPSILON);
        assert!(
            (rows.height_of(Some(1)) - 3.0).abs() < f64::EPSILON,
            "under the floor"
        );
    }

    /// Dimming multiplies the alpha it was given rather than replacing
    /// it — a muted item is already half transparent and its body must
    /// not come back to full.
    #[test]
    fn dimming_multiplies_the_alpha() {
        assert_eq!(
            dim("rgba(200, 40, 40, 1.000)", 0.42),
            "rgba(200, 40, 40, 0.420)"
        );
        assert_eq!(
            dim("rgba(200, 40, 40, 0.400)", 0.42),
            "rgba(200, 40, 40, 0.168)"
        );
    }

    /// The path closes, spans the box, and is mirrored about the middle.
    #[test]
    fn an_envelope_is_a_closed_mirrored_shape() {
        // At a hundred pixels tall a box unit IS a pixel, which is what
        // makes these numbers readable: a full peak comes within the
        // one-pixel margin of each edge, and silence keeps six tenths of
        // a pixel either side of the middle.
        let path = envelope(&[0.0, 1.0, 0.5], 100.0);
        assert!(path.starts_with('M'), "{path}");
        assert!(path.ends_with('Z'), "{path}");
        assert!(
            path.contains("1000.0"),
            "it did not reach the right edge: {path}"
        );
        assert!(path.contains("0.4"), "a full peak fell short: {path}");
        assert!(path.contains("99.6"), "no bottom half: {path}");
        assert!(path.contains("49.4"), "silence lost its floor: {path}");
        assert!(path.contains("50.6"), "silence has one side only: {path}");
    }

    /// The floor and the margin are PIXELS, so a short row keeps both
    /// rather than scaling them away: a quiet item on a three-pixel lane
    /// still draws a line down its middle.
    #[test]
    fn the_floor_is_a_pixel_at_any_row_height() {
        for height in [8.0, 24.0, 96.0] {
            let path = envelope(&[0.0], height);
            let middle = 50.0;
            // The first y in the path, as the box has it.
            let y: f64 = path
                .trim_start_matches('M')
                .split(' ')
                .nth(1)
                .and_then(|y| y.trim_end_matches('Z').parse().ok())
                .expect("a y");
            // Back into pixels: the floor is six tenths of one, whatever
            // the row does.
            let pixels = (middle - y) * height / 100.0;
            assert!(
                (pixels - 0.6).abs() < 0.05,
                "at {height}px the floor was {pixels} pixels"
            );
        }
    }

    /// One peak is still a shape rather than a division by zero.
    #[test]
    fn a_single_peak_does_not_divide_by_zero() {
        let path = envelope(&[0.7], 24.0);
        assert!(path.ends_with('Z'));
        assert!(!path.contains("NaN"), "{path}");
        assert!(!path.contains("inf"), "{path}");
    }
}
