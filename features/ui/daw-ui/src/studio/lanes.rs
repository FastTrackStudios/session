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
    /// The ruler's ground, and the ink its numbers are written in.
    pub ruler_bg: String,
    pub ruler_fg: String,
    /// The hairline under each of the ruler's rows.
    pub rule: String,
    /// What a row's own name is written in, beside its contents.
    pub faint: String,
    /// The accent, which a tempo change is marked with — and which a
    /// lit rail button takes as its face.
    pub accent: String,
    /// Ink that reads on the accent, which is black or near-white
    /// depending on how light the accent is. Resolved once here rather
    /// than guessed per control, because a theme with a pale accent and
    /// a theme with a deep one want opposite answers.
    pub ink_on_accent: String,
    /// The rails' own ground, which is the window's gutter.
    pub tcp_gutter: String,
    /// An unlit control's face.
    pub button: String,
    /// And the ink on it.
    pub text_dim: String,
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
            ruler_bg: c(theme.arrange.ruler_bg),
            ruler_fg: c(theme.arrange.ruler_fg),
            rule: c(theme.tokens.border),
            faint: c(theme.tokens.text_faint),
            accent: c(theme.tokens.accent),
            ink_on_accent: ink_on(theme.tokens.accent),
            tcp_gutter: c(theme.tokens.surface),
            button: c(theme.tokens.surface),
            text_dim: c(theme.tokens.text_dim),
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

/// Ink that reads on a background.
///
/// Black on anything light, near-white on anything dark, by relative
/// luminance. The floor is the one the painted window uses, so a lit
/// control letters the same way in both.
#[must_use]
pub fn ink_on(background: crate::theming::Color) -> String {
    /// Above this the background is light enough for black ink.
    ///
    /// Low, and deliberately so: these are saturated mid-tones and black
    /// on them reads as a number stamped on a colour, where a light ink
    /// reads as a second label floating over it. The painted window uses
    /// the same floor, so a lit control letters the same way in both.
    const FLOOR: f32 = 0.179;
    let at = |v: u8| f32::from(v) / 255.0;
    let luminance = 0.2126_f32.mul_add(
        at(background.r),
        0.7152_f32.mul_add(at(background.g), 0.0722 * at(background.b)),
    );
    if luminance > FLOOR {
        rgba(0, 0, 0, 1.0)
    } else {
        rgba(0xe8, 0xe8, 0xea, 1.0)
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
///
/// # One element, not three hundred
///
/// A grid is the same line repeated, which is what a repeating gradient
/// IS — so it is a background rather than a node per line. Two screens of
/// beats and bars is around two hundred and seventy divs, and style and
/// layout cost what the tree costs.
///
/// This is the move that was wrong in the WebView and is right here, for
/// a reason worth keeping: there, a gradient across a 24,000px element
/// was repainted by WebKit on every scroll frame and cost the window
/// nine tenths of its frames. Here the element is two screens wide, and
/// Vello re-encodes the whole scene every frame regardless — so a
/// gradient is one brush in that encoding rather than an extra repaint.
/// The same technique, opposite verdicts, because the renderers are not
/// the same renderer. Measured both ways.
#[component]
fn Lines(grid: Grid, view: View, colors: Colors, from: f64) -> Element {
    // Where the window's left edge falls inside a bar, so the first line
    // lands on a bar line and not wherever the window happened to start.
    let phase = |step: f64| -(from % (step * view.pps).max(1e-9));
    let mut images = Vec::new();
    let mut positions = Vec::new();
    // Beats first, so a bar line painted over the same pixel wins.
    if let Some(beat) = grid.beat.filter(|b| *b > 0.0) {
        let step = (beat * view.pps).max(1.0);
        images.push(format!(
            "repeating-linear-gradient(90deg, {} 0 1px, transparent 1px {step:.3}px)",
            colors.grid_beat
        ));
        positions.push(format!("{:.3}px 0", phase(beat)));
    }
    if grid.bar > 0.0 {
        let step = (grid.bar * view.pps).max(1.0);
        images.push(format!(
            "repeating-linear-gradient(90deg, {} 0 1px, transparent 1px {step:.3}px)",
            colors.grid
        ));
        positions.push(format!("{:.3}px 0", phase(grid.bar)));
    }
    if images.is_empty() {
        return rsx! {};
    }
    let (images, positions) = (images.join(", "), positions.join(", "));
    rsx! {
        div {
            style: "position:absolute; inset:0; pointer-events:none; \
                    background-image:{images}; background-position:{positions};",
        }
    }
}

/// How far the lanes are built past each edge of the screen, in screens.
///
/// The reason a pan is cheap. Items are built for a window WIDER than the
/// window they are seen through, and the scroll moves them inside it with
/// a transform — so panning re-renders nothing at all until the view
/// reaches the edge of what was built, and only then is a new window
/// built.
///
/// How much wider is a straight trade, and both ends of it were
/// measured on the golden session at 2129x1324:
///
/// | bleed | window | frame p99 | fps |
/// |---|---|---|---|
/// | 1.0 | 3 screens | 5.36 ms | 187 |
/// | 0.5 | 2 screens | 4.65 ms | 215 |
/// | 0.25 | 1.5 screens | 4.12 ms | 243 |
///
/// because style and layout cost what the tree costs, and a wider window
/// is a bigger tree. The cost of a NARROWER one is not in this table: it
/// is the margin a fast fling has before it reaches unbuilt session and
/// shows nothing. Half a screen is a thousand pixels of that, which is
/// tens of frames at any speed a hand can move; a quarter is one or two,
/// for a seventh more frames a second. So a half.
const BLEED: f64 = 0.5;

/// The lanes, and the items on them.
///
/// Renders the rows [`View`] can see, and on each of those the items
/// whose span reaches the built window. Everything else is not in the
/// tree — see rule 1.
///
/// `scroll` is a signal rather than a number because of rule 3: read as a
/// prop, every pan would rebuild every item's vnode, which measured at
/// 4.7 ms a frame against 0.2 ms. Nothing in this component reads it —
/// only [`Panner`] below does, and only [`Built`] reads how far the view
/// has travelled, through a memo that changes once a window rather than
/// once a frame.
#[component]
pub fn Lanes(
    project: ProjectRef,
    rows: RowsRef,
    view: View,
    colors: Colors,
    scroll: ReadSignal<f64>,
    #[props(default)] shapes: Shapes,
    #[props(default)] sizing: Rows,
    #[props(default)] grid: Grid,
) -> Element {
    // Which window of the session is built. A memo, so this component
    // re-renders when the window moves and not when the scroll does.
    let built = use_memo(move || Built::around(scroll(), view));
    let surface = colors.surface.clone();

    rsx! {
        div {
            style: "position:relative; width:{view.width}px; height:{view.height}px; \
                    overflow:hidden; background:{surface}; font-family:{FONT};",
            "data-testid": "studio-lanes",
            Panner {
                scroll,
                built: built(),
                children: rsx! {
                    Content {
                        project,
                        rows,
                        view,
                        colors,
                        shapes,
                        sizing,
                        grid,
                        built: built(),
                    }
                },
            }
        }
    }
}

/// The window of the session that is currently built, in pixels of
/// content.
#[derive(Clone, Copy, PartialEq)]
pub struct Built {
    pub from: f64,
    pub to: f64,
}

impl Built {
    /// The window around a scroll position.
    ///
    /// Snapped to whole windows rather than centred on the scroll, so
    /// that panning back and forth across one boundary does not rebuild
    /// on every frame it crosses — the window is the same window until
    /// the view leaves it entirely.
    #[must_use]
    pub fn around(scroll: f64, view: View) -> Self {
        let step = (view.width * BLEED).max(1.0);
        let index = (scroll / step).floor();
        let from = (index - BLEED) * step;
        Self {
            from: from.max(0.0),
            to: index.mul_add(step, view.width) + step * BLEED,
        }
    }
}

/// The node that moves.
///
/// Its children are built by the component above, which does not read the
/// scroll — so a pan re-runs exactly this function and hands the same
/// item vnodes straight back.
#[component]
fn Panner(scroll: ReadSignal<f64>, built: Built, children: Element) -> Element {
    let offset = scroll() - built.from;
    rsx! {
        div {
            style: "position:absolute; left:0; top:0; width:100%; height:100%; \
                    transform: translateX({-offset}px);",
            {children}
        }
    }
}

/// Everything inside the window, positioned relative to its left edge.
#[component]
#[expect(
    clippy::too_many_arguments,
    reason = "the lanes' own inputs, one level down so that a pan does               not re-render them"
)]
fn Content(
    project: ProjectRef,
    rows: RowsRef,
    view: View,
    colors: Colors,
    shapes: Shapes,
    sizing: Rows,
    grid: Grid,
    built: Built,
) -> Element {
    // Inside the window, the view is the window: everything is laid out
    // from its left edge, and the transform above puts that edge where
    // the scroll says it goes.
    let view = View {
        scroll_x: built.from,
        width: built.to - built.from,
        ..view
    };
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
            style: "position:absolute; left:0; top:0; width:{view.width}px; \
                    height:{view.height}px;",
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
            Lines { grid, view, colors: colors.clone(), from: built.from }
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

    // The lane's shapes, in the lane's own pixels. Built here rather than
    // inside each item because they share one `<svg>`.
    let shape_h = (body - inset * 2.0).max(0.5);
    let paths: Vec<(String, String)> = project
        .lane(&track.guid)
        .iter()
        .filter_map(|item| {
            let x0 = item.position.as_seconds();
            let x1 = x0 + item.length.as_seconds().max(0.001);
            if x1 < from || x0 > to {
                return None;
            }
            let left = x0.mul_add(view.pps, -view.scroll_x);
            let width = ((x1 - x0) * view.pps).max(1.0);
            let fill = item.color.map_or_else(
                || colour.clone(),
                |rgb| rgb24(rgb, if item.muted { 0.4 } else { 1.0 }),
            );
            let d = shapes.get(&item.guid)?.path(left, width, inset, shape_h)?;
            Some((d, fill))
        })
        .collect();

    rsx! {
        div {
            // The stripe and the divider are the lane itself: its own
            // background and its own bottom border, rather than two
            // rectangles inside it. Three nodes a row became one, and
            // style and layout cost what the tree costs.
            //
            // `border-box` so the row still occupies exactly its height
            // with the divider inside it — a divider added to the height
            // drifts the session a pixel a row. Absolutely positioned
            // children measure from the padding box, which the border
            // does not move, so every item stays where it was.
            style: "position:absolute; left:0; top:{top}px; width:100%; \
                    height:{height}px; box-sizing:border-box; background:{stripe}; \
                    border-bottom:{DIVIDER}px solid {colors.divider};",
            // Every shape on this lane, in ONE `<svg>`.
            //
            // A waveform was an `<svg>` and a `<path>` inside each item's
            // box: three nodes an item, two of which said nothing the
            // lane could not say once. Collapsed, an item costs one node
            // and its shape costs one — and the shapes of a lane move
            // together anyway, because they are on the same lane.
            if !paths.is_empty() {
                svg {
                    width: "{view.width:.0}",
                    height: "{body.max(1.0):.0}",
                    style: "position:absolute; left:0; top:0; pointer-events:none;",
                    view_box: "0 0 {view.width:.0} {body.max(1.0):.0}",
                    preserve_aspect_ratio: "none",
                    for (d, fill) in paths {
                        path { d: "{d}", fill: "{fill}" }
                    }
                }
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
                    let _ = (seen_from, seen_to);
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

    // The name is the item's OWN text rather than a box inside it: the
    // line box it needs is the same one either way, and a node per
    // titled item is a node per titled item.
    let ink = named.as_ref().map_or_else(String::new, |_| {
        format!(
            "padding-left:{TITLE_PAD}px; font-size:{TITLE_SIZE}px; \
             line-height:{TITLE_LINE}px; color:{text}; white-space:nowrap;"
        )
    });
    rsx! {
        div {
            style: "position:absolute; left:{left}px; top:{top}px; width:{width}px; \
                    height:{height}px; background:{body}; overflow:hidden; {ink}",
            "data-item": "{guid}",
            // What the item contains is drawn by its LANE — every shape
            // on a lane is one `<svg>` up there, because they share a
            // height and a transform and an element each said nothing
            // the lane could not say once.
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
                "{name}"
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

    /// The shape as one path, in the LANE's own pixels.
    ///
    /// One path rather than one node per peak or per note: a waveform is
    /// fifty amplitudes and a bar of sixteenths is sixteen blocks, and
    /// either as elements is thousands of nodes on a screen — which is
    /// the one thing the lanes may not spend.
    ///
    /// Lane pixels rather than a box of its own, because every shape on
    /// a lane shares one `<svg>`: they move together, they are the same
    /// height, and an element each said nothing the lane could not say
    /// once. It also takes the scaling out of the renderer — the
    /// amplitudes below are the pixels the recorded scene draws, rather
    /// than a fraction that a viewBox has to turn back into pixels.
    #[must_use]
    pub fn path(&self, left: f64, width: f64, top: f64, height: f64) -> Option<String> {
        match self {
            Self::Wave(peaks) => (!peaks.is_empty() && height >= 4.0)
                .then(|| envelope(peaks, left, width, top, height)),
            Self::Notes(notes) => {
                (!notes.is_empty() && height >= 2.0).then(|| roll(notes, left, width, top, height))
            }
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
fn roll(notes: &[Note], left: f64, width: f64, top: f64, height: f64) -> String {
    let mut path = String::with_capacity(notes.len() * 40);
    for note in notes {
        let x0 = f64::from(note.at).mul_add(width, left);
        // Every note gets a width, however short: a preview of a
        // sixteenth-note part at this zoom is otherwise nothing at all.
        let x1 = f64::from(note.len.max(0.004)).mul_add(width, x0);
        let h = f64::from(note.height.clamp(0.01, 1.0)) * height;
        let y = (height - h).mul_add(f64::from(note.from_top.clamp(0.0, 1.0)), top);
        let _ = write!(
            path,
            "M{x0:.2} {y:.2}L{x1:.2} {y:.2}L{x1:.2} {:.2}L{x0:.2} {:.2}Z",
            y + h,
            y + h
        );
    }
    path
}

/// A set of peaks as one closed path, in the lane's pixels.
///
/// Mirrored about the item's middle, the way a waveform is drawn: the
/// top edge out and the bottom edge back.
///
/// In pixels rather than in a box of its own units, because the two
/// parts of a waveform's height do not scale together. It fills its item
/// bar a pixel of margin, and it keeps a hair of amplitude at silence so
/// that a quiet item still has a line down its middle and reads as audio
/// rather than as a gap — and that hair is a PIXEL, not a fraction of
/// the row. Expressed as a fraction it shrinks with the row and a quiet
/// item on a short lane disappears.
fn envelope(peaks: &[f32], left: f64, width: f64, top: f64, height: f64) -> String {
    /// The margin left at full amplitude, in pixels.
    const MARGIN: f64 = 1.0;
    /// The amplitude silence still draws with, in pixels.
    const FLOOR: f64 = 0.6;
    let height = height.max(1.0);
    let middle = height.mul_add(0.5, top);
    let scale = height.mul_add(0.5, -MARGIN);
    let count = peaks.len().max(2);
    let last = f64::from(u32::try_from(count.saturating_sub(1)).unwrap_or(1)).max(1.0);
    let step = width / last;
    let amp = |p: f32| f64::from(p).clamp(0.0, 1.0).mul_add(scale, FLOOR);

    let mut path = String::with_capacity(count * 20);
    for (i, peak) in peaks.iter().enumerate() {
        let x = f64::from(u32::try_from(i).unwrap_or(0)).mul_add(step, left);
        let _ = write!(
            path,
            "{}{x:.2} {:.2}",
            if i == 0 { "M" } else { " L" },
            middle - amp(*peak)
        );
    }
    for (i, peak) in peaks.iter().enumerate().rev() {
        let x = f64::from(u32::try_from(i).unwrap_or(0)).mul_add(step, left);
        let _ = write!(path, " L{x:.2} {:.2}", middle + amp(*peak));
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
        // An item 200 wide and 100 tall, starting at x=40. A full peak
        // comes within the one-pixel margin of each edge, and silence
        // keeps six tenths of a pixel either side of the middle.
        let path = envelope(&[0.0, 1.0, 0.5], 40.0, 200.0, 0.0, 100.0);
        assert!(path.starts_with('M'), "{path}");
        assert!(path.ends_with('Z'), "{path}");
        assert!(
            path.contains("40.00"),
            "it did not start at the item: {path}"
        );
        assert!(
            path.contains("240.00"),
            "it did not reach the item's right edge: {path}"
        );
        // A full peak reaches (height/2 - 1) + 0.6 of amplitude, which
        // on a hundred-pixel item is 49.6 either side of the middle —
        // the same number the recorded scene draws.
        assert!(path.contains("0.40"), "a full peak fell short: {path}");
        assert!(path.contains("99.60"), "no bottom half: {path}");
        assert!(path.contains("49.40"), "silence lost its floor: {path}");
        assert!(path.contains("50.60"), "silence has one side only: {path}");
    }

    /// The floor and the margin are PIXELS, so a short row keeps both
    /// rather than scaling them away: a quiet item on a three-pixel lane
    /// still draws a line down its middle.
    #[test]
    fn the_floor_is_a_pixel_at_any_row_height() {
        for height in [8.0, 24.0, 96.0] {
            let path = envelope(&[0.0], 0.0, 100.0, 0.0, height);
            let y: f64 = path
                .trim_start_matches('M')
                .split(' ')
                .nth(1)
                .and_then(|y| y.trim_end_matches('Z').parse().ok())
                .expect("a y");
            let pixels = height / 2.0 - y;
            assert!(
                (pixels - 0.6).abs() < 0.01,
                "at {height}px the floor was {pixels} pixels"
            );
        }
    }

    /// A shape is drawn where its item is, not at the lane's origin.
    ///
    /// The whole point of lane pixels: a hundred items share one `<svg>`
    /// and each has to land on its own box.
    #[test]
    fn a_shape_is_drawn_where_its_item_is() {
        let at_left = envelope(&[0.5, 0.5], 0.0, 50.0, 0.0, 20.0);
        let further = envelope(&[0.5, 0.5], 300.0, 50.0, 0.0, 20.0);
        assert!(at_left.contains("M0.00"), "{at_left}");
        assert!(further.contains("M300.00"), "{further}");
        assert!(
            further.contains("350.00"),
            "it did not end at its item: {further}"
        );
    }

    /// One peak is still a shape rather than a division by zero.
    #[test]
    fn a_single_peak_does_not_divide_by_zero() {
        let path = envelope(&[0.7], 0.0, 10.0, 0.0, 24.0);
        assert!(path.ends_with('Z'));
        assert!(!path.contains("NaN"), "{path}");
        assert!(!path.contains("inf"), "{path}");
    }
}
