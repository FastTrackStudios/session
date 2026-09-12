//! The ruler, and the grid under it.
//!
//! # Why this is not in the recorded scene
//!
//! Everything else the window draws is recorded once in content space
//! and replayed under a transform, which is what makes scrolling and
//! zooming free. The grid cannot work that way: which lines exist
//! depends on the zoom. At one bar to the screen it draws sixteenths; at
//! five hundred bars it draws every sixteen bars, and a recorded scene
//! would have to hold every division at once and hide most of them.
//!
//! So this is built per frame — but only for what is on screen, which is
//! at most a screen's width divided by the closest two lines may sit,
//! and in practice a hundred or so lines. That is nothing next to the
//! twenty thousand items the recorded scene holds.
//!
//! # The division is not ours
//!
//! [`adaptive_grid`] picks it, and it already exists in this repo
//! because the expression editor needed exactly this. It follows Ilias
//! Poulakis's Adaptive Grid scripts for REAPER, so the grid coarsens and
//! refines the way a REAPER user expects it to rather than the way this
//! file would have invented.

use adaptive_grid::Adaptive;
use anyrender::PaintScene;
use vello::kurbo::{Affine, Rect};
use vello::peniko::{Color, Fill};

use crate::arrangement::{Palette, Viewport, TCP_WIDTH};
use crate::text::Font;

/// The ruler's height. REAPER's, measured.
pub const RULER_H: f64 = 28.0;

/// The project's bar grid.
///
/// One tempo and one time signature: true of most sessions and wrong for
/// some. A tempo map belongs here when the project model grows one — the
/// shape of this struct would change, not its callers.
#[derive(Clone, Copy, Debug)]
pub struct Bars {
    pub secs_per_beat: f64,
    pub beats_per_bar: f64,
}

impl Bars {
    /// From a tempo, in 4/4.
    #[must_use]
    pub fn at(bpm: f64) -> Self {
        Self {
            secs_per_beat: 60.0 / if bpm > 0.0 { bpm } else { 120.0 },
            beats_per_bar: 4.0,
        }
    }

    #[must_use]
    pub fn secs_per_bar(self) -> f64 {
        self.secs_per_beat * self.beats_per_bar
    }
}

/// Draw the grid lines across the lanes.
///
/// Drawn over the lanes rather than under them, at a low alpha. Under
/// would mean splitting the recorded scene into a layer below the items
/// and one above, and a grid that reads THROUGH an item is what REAPER's
/// looks like anyway.
pub fn grid(
    painter: &mut impl PaintScene,
    palette: &Palette,
    view: Viewport,
    bars: Bars,
    adaptive: &Adaptive,
    finest: f64,
    origin: (f64, f64),
) {
    let (ox, oy) = origin;
    let measure_px = bars.secs_per_bar() * view.pps;
    if measure_px <= 0.0 {
        return;
    }
    let (from, to) = view.secs();
    let from = from.max(0.0);

    // Beat and sub-beat lines, at whatever division still fits.
    if let Some(division) = adaptive.fit(finest, measure_px) {
        let step = division * 4.0 * bars.secs_per_beat;
        if step > 0.0 {
            line_every(painter, view, step, from, to, palette.grid_beat, 1.0, origin);
        }
    }
    // Bar lines always, and brighter: they are what the numbers above
    // count, so they have to survive whatever the division does.
    line_every(
        painter,
        view,
        bars.secs_per_bar(),
        from,
        to,
        palette.grid,
        1.0,
        origin,
    );
}

/// The ruler strip: its ground, its ticks and its bar numbers.
///
/// Painted last and opaque, so the lanes scrolled under it are covered
/// rather than clipped — one strip of overdraw against a clip layer per
/// frame, and the strip has to be drawn either way.
pub fn ruler(
    painter: &mut impl PaintScene,
    palette: &Palette,
    font: &Font,
    view: Viewport,
    bars: Bars,
    origin: (f64, f64),
) {
    // The rails frame the view, so the ruler starts where they leave
    // off. Drawn from the window's own corner it sat under the top
    // rail, and the strip of it that showed below read as a seam.
    let (ox, oy) = origin;
    fill(
        painter,
        palette.ruler_bg,
        Rect::new(ox, oy, ox + view.width, oy + RULER_H),
    );
    fill(
        painter,
        palette.tcp_rule,
        Rect::new(ox, oy + RULER_H - 1.0, ox + view.width, oy + RULER_H),
    );

    let secs_per_bar = bars.secs_per_bar();
    let bar_px = secs_per_bar * view.pps;
    if bar_px <= 0.0 {
        return;
    }
    // Number every bar while there is room, then every 4, 8, 16 — the
    // numbers must never collide, and a ruler that drops to "every 5"
    // stops being countable.
    let every = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0]
        .into_iter()
        .find(|n| bar_px * n >= 56.0)
        .unwrap_or(256.0);

    let (from, to) = view.secs();
    // Stepped by an integer count rather than by adding a float to
    // itself: over five hundred bars the accumulated error is visible,
    // and the bar a number sits on has to be the bar its line is on.
    let first = (from.max(0.0) / secs_per_bar / every).floor().max(0.0);
    for n in counts(MAX_LABELS) {
        let bar = (first + n) * every;
        let t = bar * secs_per_bar;
        if t > to {
            break;
        }
        let x = t.mul_add(view.pps, TCP_WIDTH - view.scroll_x);
        if x >= TCP_WIDTH - 1.0 && x <= view.width {
            fill(
                painter,
                palette.grid,
                Rect::new(ox + x, oy + RULER_H - 9.0, ox + x + 1.0, oy + RULER_H),
            );
            crate::tcp::glyphs(
                painter,
                font,
                palette.ruler_fg,
                // Bars are counted from one; only the arithmetic starts
                // at zero.
                &format!("{}", bar + 1.0),
                x + 4.0,
                14.0,
                11.0,
            );
        }
    }
}

/// How many bar numbers a ruler will ever print.
///
/// The spacing rule keeps them 56px apart, so a 5120-wide screen holds
/// about ninety. This is the guard against a degenerate tempo or zoom
/// turning the loop into a hang, not a layout decision.
const MAX_LABELS: usize = 512;

/// `0.0, 1.0, 2.0, …`, up to `max`.
///
/// A float counter compared against a float bound is how a grid loop
/// becomes either an infinite one or a drifting one; this keeps the
/// counting in integers and converts once. `u32` because a count that
/// did not fit one would have stopped at `max` long before.
fn counts(max: usize) -> impl Iterator<Item = f64> {
    (0..max).map_while(|i| u32::try_from(i).ok().map(f64::from))
}

/// One flat rectangle.
fn fill(painter: &mut impl PaintScene, color: Color, rect: Rect) {
    painter.fill(Fill::NonZero, Affine::IDENTITY, color, None, &rect);
}

/// Vertical lines every `step` seconds across the lane area.
fn line_every(
    painter: &mut impl PaintScene,
    view: Viewport,
    step: f64,
    from: f64,
    to: f64,
    color: Color,
    width: f64,
    origin: (f64, f64),
) {
    let (ox, oy) = origin;
    // A screen of lines is the most that can ever be visible; anything
    // beyond that is a division that should have coarsened, and drawing
    // it would cost a frame to produce a grey band.
    const MAX: usize = 2048;

    let first = (from / step).floor();
    for n in counts(MAX) {
        let t = (first + n) * step;
        if t > to {
            break;
        }
        let x = t.mul_add(view.pps, TCP_WIDTH - view.scroll_x);
        if x >= TCP_WIDTH {
            painter.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                color,
                None,
                &Rect::new(ox + x, oy + RULER_H, ox + x + width, oy + view.height),
            );
        }
    }
}
