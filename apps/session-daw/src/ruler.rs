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

use crate::arrangement::{Palette, TCP_WIDTH, Viewport};
use crate::text::Font;

/// The bar strip's height. REAPER's, measured.
pub const BARS_H: f64 = 28.0;

/// One ruler lane's height.
pub const LANE_H: f64 = 15.0;

/// How many ruler lanes there are, over the bars.
///
/// The FTS convention (REAPER 7.62's ruler lanes): lane 1 is the SONG,
/// one region over the whole song; lane 2 the SECTIONS, a region per
/// verse and chorus; lane 3 the MARKS — SONGSTART, SONGEND and the
/// like. Always three, so a session with fewer still lays its rows out
/// where every other session does.
pub const LANES: usize = 3;

/// What the lanes are called, top to bottom.
pub const LANE_NAMES: [&str; LANES] = ["SONG", "SECTIONS", "MARKS"];

/// The whole top strip: the lanes, then the bars under them.
pub const RULER_H: f64 = BARS_H + LANE_H * 3.0;

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
            line_every(
                painter,
                view,
                step,
                from,
                to,
                palette.grid_beat,
                1.0,
                origin,
            );
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
    // The bars are the bottom of the strip; the lanes sit over them.
    let oy = oy + RULER_H - BARS_H;

    let secs_per_bar = bars.secs_per_bar();
    let bar_px = secs_per_bar * view.pps;
    if bar_px <= 0.0 {
        return;
    }
    // How many bars between numbers. Halves and quarters first, so
    // zooming IN keeps saying something new — a ruler that stops at
    // every bar has nothing left to tell you once a bar is half the
    // window, and you are left counting beats by eye.
    //
    // Then whole bars, then every 4, 8, 16: the numbers must never
    // collide, and a ruler that drops to "every 5" stops being
    // countable.
    let every = STEPS
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
                Rect::new(ox + x, oy + BARS_H - 9.0, ox + x + 1.0, oy + BARS_H),
            );
            crate::tcp::glyphs(
                painter,
                font,
                palette.ruler_fg,
                // Bars are counted from one; only the arithmetic starts
                // at zero.
                &label(bar + 1.0),
                ox + x + 4.0,
                oy + 14.0,
                11.0,
            );
        }
    }
}

/// How many bars a ruler will put between two numbers.
///
/// Fractions first: zooming in has to keep saying something, and once a
/// bar is half the window a ruler numbering only whole bars has nothing
/// left to tell you. Then whole bars, then powers of two — the numbers
/// must never collide, and a ruler that drops to "every 5" stops being
/// countable.
const STEPS: [f64; 11] = [
    0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0, 256.0,
];

/// A bar number as it is written.
///
/// Whole bars have no decimal, because "1.0" in a row of bar numbers
/// reads as a measurement rather than a count. A half or a quarter
/// keeps just the digits it needs: 1.5, not 1.50.
#[must_use]
pub fn label(bar: f64) -> String {
    if (bar - bar.round()).abs() < 1e-9 {
        return format!("{}", bar.round() as i64);
    }
    let text = format!("{bar:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// The ruler's lanes: the song, its sections and its marks, over the
/// bars. Drawn per frame like the bars, and after them.
///
/// A region is a band across its lane with its name at the left; a
/// marker is a flag with its name after it. The lane names stand in
/// the panel's column, where the lanes have no timeline to be on.
pub fn lanes(
    painter: &mut impl PaintScene,
    palette: &Palette,
    font: &Font,
    view: Viewport,
    origin: (f64, f64),
    sections: &[daw_ui::studio::project::Section],
    markers: &[daw_ui::studio::project::Marker],
) {
    const SIZE: f32 = 8.0;
    let (ox, oy) = origin;
    let left = ox + TCP_WIDTH;
    let right = ox + view.width;
    let x_of = |t: f64| t.mul_add(view.pps, left - view.scroll_x);
    for (row, name) in LANE_NAMES.iter().enumerate() {
        let top = LANE_H.mul_add(crate::num::coord(row), oy);
        // A rule under each lane, and the lane's name in the column.
        fill(
            painter,
            palette.tcp_rule,
            Rect::new(ox, top + LANE_H - 1.0, right, top + LANE_H),
        );
        crate::tcp::glyphs(
            painter,
            font,
            palette.text_faint,
            name,
            ox + 8.0,
            top + LANE_H - 4.0,
            SIZE,
        );
    }
    // Regions: a band, clipped to the timeline, named where it starts
    // — or where the view starts, if the band began off screen, so a
    // long section still says what it is.
    for section in sections {
        let row = lane_row(section.lane);
        let top = LANE_H.mul_add(crate::num::coord(row), oy) + 2.0;
        let x0 = x_of(section.start).max(left);
        let x1 = x_of(section.end).min(right);
        if x1 <= x0 {
            continue;
        }
        let tint = section
            .color
            .as_deref()
            .and_then(css_hex)
            .unwrap_or(palette.accent);
        fill(
            painter,
            tint.multiply_alpha(0.45),
            Rect::new(x0, top, x1, top + LANE_H - 4.0),
        );
        fill(
            painter,
            tint,
            Rect::new(x0, top, (x0 + 2.0).min(x1), top + LANE_H - 4.0),
        );
        if x1 - x0 > 24.0 {
            let room = x1 - x0 - 8.0;
            if let Some(name) = fit(font, &section.name, SIZE, room) {
                crate::tcp::glyphs(
                    painter,
                    font,
                    palette.text,
                    name,
                    x0 + 5.0,
                    top + LANE_H - 6.0,
                    SIZE,
                );
            }
        }
    }
    // Markers: a flag on its lane, named to the right of it.
    for marker in markers {
        let row = lane_row(marker.lane);
        let top = LANE_H.mul_add(crate::num::coord(row), oy) + 2.0;
        let x = x_of(marker.at);
        if x < left || x > right {
            continue;
        }
        let tint = marker
            .color
            .as_deref()
            .and_then(css_hex)
            .unwrap_or(palette.ruler_fg);
        fill(
            painter,
            tint,
            Rect::new(x, top, x + 2.0, top + LANE_H - 4.0),
        );
        fill(
            painter,
            tint,
            Rect::new(x, top, (x + 8.0).min(right), top + 4.0),
        );
        if let Some(name) = fit(font, &marker.name, SIZE, right - x - 6.0) {
            crate::tcp::glyphs(
                painter,
                font,
                palette.text,
                name,
                x + 5.0,
                top + LANE_H - 6.0,
                SIZE,
            );
        }
    }
}

/// The lanes' lines, down through the arrangement: every region edge
/// and every marker, from under the ruler to the bottom of the view,
/// so the song's shape is read against the items and not only over
/// them.
///
/// Where two fall on one pixel the LOWEST lane wins — a section's
/// edge over the song's, a mark over a section — and a start beats an
/// end, so the chorus's first line is the chorus's and not the end of
/// the verse before it.
pub fn lane_lines(
    painter: &mut impl PaintScene,
    palette: &Palette,
    view: Viewport,
    origin: (f64, f64),
    sections: &[daw_ui::studio::project::Section],
    markers: &[daw_ui::studio::project::Marker],
    top: f64,
    bottom: f64,
) {
    let (ox, _) = origin;
    let left = ox + TCP_WIDTH;
    let right = ox + view.width;
    let x_of = |t: f64| t.mul_add(view.pps, left - view.scroll_x);
    // Every line, with what decides between two on one pixel: the lane
    // row first (lower wins), then whether it is a start.
    let mut lines: Vec<(f64, usize, bool, Color)> = Vec::new();
    for section in sections {
        let row = lane_row(section.lane);
        let tint = section
            .color
            .as_deref()
            .and_then(css_hex)
            .unwrap_or(palette.accent);
        lines.push((x_of(section.start), row, true, tint));
        lines.push((x_of(section.end), row, false, tint));
    }
    for marker in markers {
        let tint = marker
            .color
            .as_deref()
            .and_then(css_hex)
            .unwrap_or(palette.ruler_fg);
        lines.push((x_of(marker.at), lane_row(marker.lane), true, tint));
    }
    // Sorted so the winner of each pixel comes LAST, then drawn in that
    // order — the winner paints over the rest.
    lines.sort_by(|a, b| {
        a.1.cmp(&b.1)
            .then(a.2.cmp(&b.2))
            .then(a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
    });
    for (x, row, start, tint) in lines {
        if x < left || x > right {
            continue;
        }
        // The song's edges faint, a section's clearer, a mark's clearest:
        // the line's weight is the lane's.
        let alpha = match (row, start) {
            (0, _) => 0.22,
            (1, true) => 0.45,
            (1, false) => 0.3,
            (_, true) => 0.7,
            (_, false) => 0.5,
        };
        let x = x.round();
        fill(
            painter,
            tint.multiply_alpha(alpha),
            Rect::new(x, top, x + 1.0, bottom),
        );
    }
}

/// Which lane row a REAPER lane index lands on: lanes are numbered from
/// one, the default lane is the first, and anything past the last row
/// is drawn on it rather than off the strip.
fn lane_row(lane: u32) -> usize {
    usize::try_from(lane.saturating_sub(1))
        .unwrap_or(0)
        .min(LANES.saturating_sub(1))
}

/// As much of a name as fits a width, or nothing.
fn fit<'a>(font: &Font, name: &'a str, size: f32, room: f64) -> Option<&'a str> {
    let mut name = name;
    while !name.is_empty() && font.width(name, size) > room {
        let mut end = name.len().saturating_sub(1);
        while end > 0 && !name.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        name = name.get(..end).unwrap_or("");
    }
    (!name.is_empty()).then_some(name)
}

/// A `#rrggbb` string as a colour — what the studio project resolves
/// region and marker colours to.
fn css_hex(css: &str) -> Option<Color> {
    let digits = css.strip_prefix('#')?;
    if digits.len() != 6 {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(digits.get(at..at.saturating_add(2))?, 16).ok();
    Some(Color::from_rgba8(
        channel(0)?,
        channel(2)?,
        channel(4)?,
        0xff,
    ))
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

// ─── What the pointer is on ─────────────────────────────────────────

/// Which part of a region the pointer is over.
///
/// The same three the arrangement uses for an item, and for the same
/// reason: a band's ends mean "change where it stops" and its middle
/// means "move the whole thing", and a user expects that everywhere
/// something has a start and an end.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Zone {
    Start,
    Body,
    End,
}

/// What a press on the ruler landed on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum On {
    /// A marker's flag, by REAPER's marker number.
    Marker { id: u32 },
    /// A region band, by REAPER's region number.
    Region { id: u32, zone: Zone },
    /// A lane with nothing on it at that time. The LANE is what a press
    /// here will create in: the marks lane makes a marker, a region
    /// lane makes a region. That is the whole reason there is no tool
    /// to choose — the ruler already says what you meant.
    Lane { row: usize },
    /// The bars, under the lanes: the timeline itself.
    Bars,
}

/// How near a region's edge takes hold of the edge rather than the body.
///
/// In pixels, not seconds, because it is a question about the pointer
/// and not about the music — at a far zoom a whole bar can be a pixel,
/// and a grip measured in time would swallow the band.
const EDGE_GRIP: f64 = 4.0;

/// The lane a y falls in, or `None` if it is in the bars.
#[must_use]
pub fn lane_at(y: f64, top: f64) -> Option<usize> {
    let row = ((y - top) / LANE_H).floor();
    if row < 0.0 {
        return None;
    }
    let row = crate::num::index(row);
    (row < LANES).then_some(row)
}

/// What is under a point on the ruler.
///
/// Takes the same lists the drawing takes, so what you click is what
/// you see — a second geometry would drift from the first the day
/// either changed.
///
/// A marker is a flag, so it is hit by nearness in PIXELS rather than
/// by containing the time: it has no width to be inside of.
#[must_use]
pub fn on(
    x: f64,
    y: f64,
    top: f64,
    left: f64,
    pps: f64,
    scroll_x: f64,
    sections: &[daw_ui::studio::project::Section],
    markers: &[daw_ui::studio::project::Marker],
) -> On {
    let Some(row) = lane_at(y, top) else {
        return On::Bars;
    };
    let x_of = |t: f64| t.mul_add(pps, left - scroll_x);

    // Markers first: a flag drawn over a band is a flag you can take
    // hold of, and the drawing puts them on top.
    let mut nearest: Option<(f64, u32)> = None;
    for marker in markers {
        if lane_row(marker.lane) != row {
            continue;
        }
        let away = (x_of(marker.at) - x).abs();
        if away <= MARKER_GRIP && nearest.is_none_or(|(best, _)| away < best) {
            nearest = Some((away, marker.idx));
        }
    }
    if let Some((_, id)) = nearest {
        return On::Marker { id };
    }

    for section in sections {
        if lane_row(section.lane) != row {
            continue;
        }
        let (x0, x1) = (x_of(section.start), x_of(section.end));
        if x < x0 || x > x1 {
            continue;
        }
        // A band too narrow to have a middle is all body: offering an
        // edge grip on something four pixels wide means the user can
        // never move it.
        let zone = if x1 - x0 < EDGE_GRIP * 3.0 {
            Zone::Body
        } else if x - x0 <= EDGE_GRIP {
            Zone::Start
        } else if x1 - x <= EDGE_GRIP {
            Zone::End
        } else {
            Zone::Body
        };
        return On::Region {
            id: section.id,
            zone,
        };
    }

    On::Lane { row }
}

/// How near a marker's flag counts as being on it, in pixels.
///
/// Wider than the flag is drawn. A marker is a position, and a position
/// has no width — asking the user to hit two pixels is asking them to
/// miss.
const MARKER_GRIP: f64 = 6.0;

/// Which lane makes markers, by the FTS convention.
///
/// The third: SONG, SECTIONS, MARKS. A press in it creates a marker;
/// a press in either of the others creates a region.
pub const MARKS_ROW: usize = 2;

/// Which lane makes a song section.
///
/// The second. A region here IS a section — that is the rule the song
/// model reads the arrangement by, so it is the one this file has to
/// agree with.
pub const SECTIONS_ROW: usize = 1;

/// The lane number to store for a row.
///
/// The inverse of `lane_row`: REAPER counts ruler lanes from one, with
/// zero meaning the default lane, and the rows here count from zero.
#[must_use]
pub const fn lane_of(row: usize) -> u32 {
    row as u32 + 1
}

#[cfg(test)]
mod tests {
    use super::{Bars, STEPS, label};

    /// The ruler counts MEASURES, not seconds.
    ///
    /// Pinned because it is the question you cannot answer by looking:
    /// at 120 bpm in four four a bar is two seconds, so a ruler
    /// numbering seconds and one numbering bars both count 1, 2, 3 —
    /// they differ only in WHERE the numbers sit, and by then you are
    /// counting pixels.
    #[test]
    fn the_numbers_are_bars() {
        let bars = Bars::at(120.0);
        assert!(
            (bars.secs_per_bar() - 2.0).abs() < 1e-9,
            "a bar of four beats at 120 bpm is two seconds, got {}",
            bars.secs_per_bar()
        );
        // Bar 2 begins two seconds in, not two bars in.
        assert!((1.0 * bars.secs_per_bar() - 2.0).abs() < 1e-9);
        // And the tempo actually moves them: at 60 bpm a bar is twice
        // as long, which a seconds ruler would not notice.
        assert!((Bars::at(60.0).secs_per_bar() - 4.0).abs() < 1e-9);
    }

    /// Zooming in keeps saying something new.
    ///
    /// The steps run below a whole bar, so a bar wider than the window
    /// still gets numbered inside. Without the fractions the ruler goes
    /// quiet exactly when you have zoomed in to read it closely.
    #[test]
    fn zooming_in_subdivides_the_bar() {
        let pick = |bar_px: f64| {
            STEPS
                .into_iter()
                .find(|n| bar_px * n >= 56.0)
                .unwrap_or(256.0)
        };
        assert!(pick(400.0) < 1.0, "a wide bar should number inside it");
        assert_eq!(pick(60.0), 1.0, "a bar just wide enough numbers once");
        assert!(pick(10.0) > 1.0, "a narrow bar should number less often");
        // Monotone: zooming in never makes the numbers sparser.
        let (wide, narrow) = (pick(400.0), pick(100.0));
        assert!(wide <= narrow, "{wide} should be no coarser than {narrow}");
    }

    /// A whole bar is written as a count, a fraction as a fraction.
    ///
    /// "1.0" in a row of bar numbers reads as a measurement rather than
    /// a count, and "1.50" reads as a precision nobody asked for.
    #[test]
    fn a_bar_number_is_written_as_a_number() {
        assert_eq!(label(1.0), "1");
        assert_eq!(label(17.0), "17");
        assert_eq!(label(1.5), "1.5");
        assert_eq!(label(3.5), "3.5");
        assert_eq!(label(2.25), "2.25");
    }
}
