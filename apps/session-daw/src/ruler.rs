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

/// How tall the tempo strip is, above the bar numbers.
///
/// Its own row rather than a flag among the marks: a tempo change is
/// not a place in the song, it is a change to what every number below
/// it MEANS. Putting it in the marks lane would file it with the things
/// it reinterprets.
pub const TEMPO_H: f64 = 13.0;

/// The whole top strip: the lanes, then the bars under them.
pub const RULER_H: f64 = BARS_H + TEMPO_H + LANE_H * 3.0;

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
        Self::in_time(bpm, 4)
    }

    /// From a tempo and a signature's top number.
    ///
    /// The top number is what a bar is counted in: three beats to a bar
    /// in three four, seven in seven eight. A ruler that assumed four
    /// put every bar line after the first in the wrong place for
    /// anything else — and a grid that disagrees with the music is one
    /// you edit against at your peril.
    #[must_use]
    pub fn in_time(bpm: f64, beats_per_bar: u32) -> Self {
        Self {
            secs_per_beat: 60.0 / if bpm > 0.0 { bpm } else { 120.0 },
            beats_per_bar: f64::from(beats_per_bar.max(1)),
        }
    }

    /// The bars at a time, from the project's tempo map.
    ///
    /// The last change at or before `seconds` wins, which is what a
    /// tempo map means.
    #[must_use]
    pub fn at_time(tempo: &[daw_ui::studio::project::TempoChange], seconds: f64) -> Self {
        tempo
            .iter()
            .take_while(|change| change.at <= seconds + 1e-9)
            .last()
            .map_or_else(
                || Self::at(120.0),
                |change| Self::in_time(change.bpm, change.beats_per_bar),
            )
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
    // The bars are the bottom of the strip; the tempo sits just above
    // them and the lanes over that.
    let oy = oy + RULER_H - BARS_H;

    let secs_per_bar = bars.secs_per_bar();
    let bar_px = secs_per_bar * view.pps;
    if bar_px <= 0.0 {
        return;
    }
    // Stepped in BEATS, because that is what the numbering counts: a
    // bar is however many beats the signature says, and a step measured
    // in bars can never land on beat three of four.
    let every = step_beats(bar_px, bars.beats_per_bar);

    let (from, to) = view.secs();
    // Stepped by an integer count rather than by adding a float to
    // itself: over five hundred bars the accumulated error is visible,
    // and the bar a number sits on has to be the bar its line is on.
    let first = (from.max(0.0) / bars.secs_per_beat / every)
        .floor()
        .max(0.0);
    for n in counts(MAX_LABELS) {
        let beat = (first + n) * every;
        let t = beat * bars.secs_per_beat;
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
                &label(beat, bars.beats_per_bar, every),
                // Close to its own line, not floating between two.
                // Four pixels of gap put the number nearer the NEXT
                // tick than its own at a beat-wide zoom, which is the
                // one thing a ruler must never be ambiguous about.
                ox + x + 2.0,
                oy + 14.0,
                11.0,
            );
        }
    }
}

/// How many BEATS a ruler will put between two numbers.
///
/// Beats, not bars, because that is what the numbering counts: a bar is
/// however many beats the signature says, and a step measured in bars
/// cannot land on beat three of four. Quarters and halves of a beat
/// first, so zooming in keeps saying something; then a beat, then whole
/// bars by way of the signature.
///
/// The bar-sized steps are filled in from the signature at use, because
/// four beats is a bar in four four and three in three four.
const BEAT_STEPS: [f64; 3] = [0.25, 0.5, 1.0];

/// How many bars between numbers once a beat is too fine to label.
const BAR_STEPS: [f64; 8] = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0];

/// The step between numbers, in beats, for a bar this wide.
#[must_use]
pub fn step_beats(bar_px: f64, beats_per_bar: f64) -> f64 {
    let beat_px = bar_px / beats_per_bar.max(1.0);
    if let Some(step) = BEAT_STEPS.into_iter().find(|n| beat_px * n >= LABEL_ROOM) {
        return step;
    }
    BAR_STEPS
        .into_iter()
        .find(|n| bar_px * n >= LABEL_ROOM)
        .unwrap_or(256.0)
        * beats_per_bar
}

/// How much room a number needs before the next one, in pixels.
const LABEL_ROOM: f64 = 56.0;

/// A position written the way a DAW writes one: measure.beat.subdivision.
///
/// Counted from `beat`, the number of beats from the start of the
/// project, against a bar of `beats_per_bar`.
///
/// How much of it is written is decided by the STEP, not by the
/// position. A row reading 1, 1.2, 1.3, 1.4, 2 makes you notice that
/// the first of each bar is written differently from the rest and
/// wonder what that means; 1.1, 1.2, 1.3, 1.4, 2.1 is one column of
/// the same thing, which is what a row of beats is.
///
/// So: stepping in bars gives bar numbers, "5". Stepping in beats gives
/// "5.1" for every one of them, downbeat included. Stepping finer
/// carries the subdivision in THOUSANDTHS of a beat, zero-padded:
/// "5.2.250" is a quarter of the way through the second beat of the
/// fifth bar.
///
/// Thousandths because that is what a musical position IS here — the
/// DAW's own `MusicalPosition` carries measure, beat and a subdivision
/// of 0..999 — so a number read off this ruler is a number you can
/// type back in.
#[must_use]
pub fn label(beat: f64, beats_per_bar: f64, step: f64) -> String {
    let per_bar = beats_per_bar.max(1.0);
    let bar = (beat / per_bar).floor();
    let into_bar = beat - bar * per_bar;
    let whole_beat = into_bar.floor();
    let fraction = into_bar - whole_beat;

    let measure = (bar + 1.0).round() as i64;
    let beat_no = (whole_beat + 1.0).round() as i64;
    // A whole bar between numbers: bar numbers, nothing else to say.
    if step >= per_bar - 1e-9 {
        return format!("{measure}");
    }
    // A whole beat or more: every label names its beat, the downbeat
    // included, so the row is one column of the same thing.
    if step >= 1.0 - 1e-9 || fraction.abs() < 1e-9 {
        return format!("{measure}.{beat_no}");
    }
    // Thousandths of a beat, zero-padded so the numbers line up in a
    // row and .050 cannot be misread as .5.
    let sub = (fraction * 1000.0).round() as i64;
    format!("{measure}.{beat_no}.{sub:03}")
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
    use super::{Bars, label, step_beats};

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
        // The tempo actually moves them: at 60 a bar is twice as long,
        // which a seconds ruler would not notice.
        assert!((Bars::at(60.0).secs_per_bar() - 4.0).abs() < 1e-9);
    }

    /// The signature decides how many beats make a bar.
    ///
    /// A ruler that assumed four put every bar line after the first in
    /// the wrong place for anything else, and a grid that disagrees
    /// with the music is one you edit against at your peril.
    #[test]
    fn the_signature_decides_the_bar() {
        let three = Bars::in_time(120.0, 3);
        assert!(
            (three.secs_per_bar() - 1.5).abs() < 1e-9,
            "three beats at 120 is a bar and a half second, got {}",
            three.secs_per_bar()
        );
        // And the numbering follows it: the fourth beat of a three-four
        // project is the first beat of bar two.
        assert_eq!(label(3.0, 3.0, 3.0), "2");
        assert_eq!(
            label(3.0, 4.0, 1.0),
            "1.4",
            "four four should still be in bar one"
        );
    }

    /// Zooming in keeps saying something new, down to the beat.
    #[test]
    fn zooming_in_subdivides_the_bar() {
        // A wide bar numbers inside itself, at a beat or finer.
        assert!(step_beats(400.0, 4.0) <= 1.0);
        // A narrow one numbers whole bars or less often.
        assert!(step_beats(20.0, 4.0) >= 4.0);
        // Monotone: zooming in never makes the numbers sparser.
        assert!(step_beats(400.0, 4.0) <= step_beats(100.0, 4.0));
    }

    /// measure.beat.subdivision, and only as much of it as is needed.
    ///
    /// A whole bar is a bar number: "5.1.000" in a row of them is noise
    /// that makes the bar lines harder to pick out, which is the one
    /// thing the number is there for.
    #[test]
    fn a_position_is_written_the_way_a_daw_writes_one() {
        // Stepping in bars: bar numbers.
        assert_eq!(label(0.0, 4.0, 4.0), "1");
        assert_eq!(label(4.0, 4.0, 4.0), "2");
        // Stepping in beats: every label names its beat, downbeat too.
        assert_eq!(label(0.0, 4.0, 1.0), "1.1");
        assert_eq!(label(1.0, 4.0, 1.0), "1.2");
        assert_eq!(label(2.0, 4.0, 1.0), "1.3");
        assert_eq!(label(4.0, 4.0, 1.0), "2.1");
        assert_eq!(label(7.0, 4.0, 1.0), "2.4");
        // Thousandths of a beat, zero-padded so a column of them lines
        // up and .050 cannot be misread as .5.
        assert_eq!(label(0.25, 4.0, 0.25), "1.1.250");
        assert_eq!(label(0.5, 4.0, 0.25), "1.1.500");
        assert_eq!(label(0.75, 4.0, 0.25), "1.1.750");
        assert_eq!(label(4.5, 4.0, 0.25), "2.1.500");
        // A beat that lands whole still names itself at a fine step.
        assert_eq!(label(1.0, 4.0, 0.25), "1.2");
    }

    /// The tempo at a time is the last change at or before it.
    #[test]
    fn the_tempo_map_decides_the_bar() {
        use daw_ui::studio::project::TempoChange;
        let changes = [
            TempoChange {
                at: 0.0,
                bpm: 120.0,
                beats_per_bar: 4,
                beat_unit: 4,
            },
            TempoChange {
                at: 10.0,
                bpm: 60.0,
                beats_per_bar: 3,
                beat_unit: 4,
            },
        ];
        assert!((Bars::at_time(&changes, 0.0).secs_per_bar() - 2.0).abs() < 1e-9);
        assert!((Bars::at_time(&changes, 9.9).secs_per_bar() - 2.0).abs() < 1e-9);
        // After the change: 60 bpm, three to a bar — three seconds.
        assert!((Bars::at_time(&changes, 10.0).secs_per_bar() - 3.0).abs() < 1e-9);
    }
}

/// The tempo strip: where the tempo or the signature changes, and to
/// what.
///
/// Drawn between the lanes and the bar numbers, because that is what it
/// governs. Its own row rather than a flag among the marks: a tempo
/// change is not a place in the song, it is a change to what every
/// number below it MEANS, and filing it with the marks would file it
/// among the things it reinterprets.
///
/// The reading is repeated at the left edge when the change that set it
/// is off screen. A tempo you cannot see is a tempo you will assume,
/// and the assumption is always whatever the project started at.
pub fn tempo(
    painter: &mut impl PaintScene,
    palette: &Palette,
    font: &Font,
    view: Viewport,
    origin: (f64, f64),
    changes: &[daw_ui::studio::project::TempoChange],
) {
    const SIZE: f32 = 8.0;
    let (ox, oy) = origin;
    let top = oy + RULER_H - BARS_H - TEMPO_H;
    let left = ox + TCP_WIDTH;
    let right = ox + view.width;
    fill(
        painter,
        palette.tcp_rule,
        Rect::new(ox, top + TEMPO_H - 1.0, right, top + TEMPO_H),
    );
    crate::tcp::glyphs(
        painter,
        font,
        palette.text_faint,
        "TEMPO",
        ox + 8.0,
        top + TEMPO_H - 3.0,
        SIZE,
    );

    let x_of = |t: f64| t.mul_add(view.pps, left - view.scroll_x);
    let (from, to) = view.secs();

    if let Some(current) = changes
        .iter()
        .take_while(|change| change.at <= from + 1e-9)
        .last()
        && changes.iter().any(|change| change.at > from)
    {
        crate::tcp::glyphs(
            painter,
            font,
            palette.text_faint,
            &reading(current),
            left + 4.0,
            top + TEMPO_H - 3.0,
            SIZE,
        );
    }

    for change in changes {
        if change.at < from || change.at > to {
            continue;
        }
        let x = x_of(change.at);
        if x < left - 1.0 || x > right {
            continue;
        }
        fill(
            painter,
            palette.accent,
            Rect::new(x, top + 1.0, x + 1.0, top + TEMPO_H - 1.0),
        );
        crate::tcp::glyphs(
            painter,
            font,
            palette.text,
            &reading(change),
            x + 4.0,
            top + TEMPO_H - 3.0,
            SIZE,
        );
    }
}

/// A tempo change as it reads: "120 4/4".
///
/// Both halves always, even when only one of them moved. A strip that
/// showed the tempo at one change and the signature at the next would
/// make you look back through the project to answer either question.
fn reading(change: &daw_ui::studio::project::TempoChange) -> String {
    let bpm = if (change.bpm - change.bpm.round()).abs() < 0.05 {
        format!("{}", change.bpm.round() as i64)
    } else {
        format!("{:.1}", change.bpm)
    };
    format!("{bpm} {}/{}", change.beats_per_bar, change.beat_unit)
}
