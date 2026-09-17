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
    tempo: &[daw_ui::studio::project::TempoChange],
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

    let (from, to) = view.secs();
    // Every beat up to the right edge, counted through the tempo map
    // rather than multiplied from one tempo. A number has to sit on the
    // bar line it names, and after a tempo change a multiplied grid
    // does not.
    // How wide a bar is where the view starts. Asked BEFORE the walk,
    // because a zoom far enough out makes a bar narrower than a pixel
    // and there is then nothing to draw — and `view.secs()` at that
    // zoom asks for a range measured in centuries, which is a walk
    // that never returns.
    let start = Bars::at_time(tempo, from.max(0.0));
    let bar_px = start.secs_per_bar() * view.pps;
    if bar_px < 2.0 {
        return;
    }
    let beats = Timeline::new(tempo).beats(to, MAX_BEATS);
    let Some(first) = beats.first() else {
        return;
    };
    // Measured at the start rather than averaged: a ruler whose
    // spacing changed mid-screen would be harder to read than one
    // slightly too dense at one end.
    let every = step_beats(bar_px, f64::from(first.per_bar));

    // Which beats get a number. Counted in beats from the start so the
    // step lands on the same beats however far you scroll.
    let mut since = 0.0_f64;
    for (index, beat) in beats.iter().enumerate() {
        if index > 0 {
            since += 1.0;
        }
        // A downbeat is always written when whole bars are the step;
        // otherwise every `every` beats. Fractional steps subdivide
        // between the beats, which the sub-beat loop below handles.
        let on_step = if every >= 1.0 {
            (since % every).abs() < 1e-6 || (since % every - every).abs() < 1e-6
        } else {
            true
        };
        if !on_step {
            continue;
        }
        let ticks: Vec<(f64, f64)> = if every < 1.0 {
            // Inside the beat: the beat itself and the fractions of it
            // that fit, each measured with THIS beat's length.
            let mut out = Vec::new();
            let mut fraction = 0.0_f64;
            while fraction < 1.0 - 1e-9 {
                out.push((
                    fraction.mul_add(beat.secs_per_beat, beat.at),
                    f64::from(beat.beat - 1) + fraction,
                ));
                fraction += every;
            }
            out
        } else {
            vec![(beat.at, f64::from(beat.beat - 1))]
        };
        for (t, into_bar) in ticks {
            if t < from || t > to {
                continue;
            }
            let x = t.mul_add(view.pps, TCP_WIDTH - view.scroll_x);
            if x < TCP_WIDTH - 1.0 || x > view.width {
                continue;
            }
            fill(
                painter,
                palette.grid,
                Rect::new(ox + x, oy + BARS_H - 9.0, ox + x + 1.0, oy + BARS_H),
            );
            crate::tcp::glyphs(
                painter,
                font,
                palette.ruler_fg,
                &written(beat.measure, into_bar, f64::from(beat.per_bar), every),
                // Close to its own line, not floating between two. Four
                // pixels put the number nearer the NEXT tick than its
                // own at a beat-wide zoom, which is the one thing a
                // ruler must never be ambiguous about.
                ox + x + 2.0,
                oy + 14.0,
                11.0,
            );
        }
    }
}

/// How many beats a ruler will count before it gives up.
///
/// A tempo map cannot be trusted to be sane — a zero or a negative bpm
/// would make the walk stand still — and a ruler is not the place to
/// find that out by hanging.
///
/// Well above any real session: an hour of sixteenth notes at 200 bpm
/// is under fifty thousand. The guard above is what keeps an absurd
/// zoom from asking for them at all.
const MAX_BEATS: usize = 100_000;

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
/// Takes the measure it is IN rather than working it out, because after
/// a signature change the measure is no longer a division of the beat
/// count — the walk that produced it is the only thing that knows.
///
/// How much of it is written is decided by the STEP, not by the
/// position. A row reading 1, 1.2, 1.3, 1.4, 2 makes you notice that
/// the first of each bar is written differently from the rest and
/// wonder what that means; 1.1, 1.2, 1.3, 1.4, 2.1 is one column of
/// the same thing, which is what a row of beats is.
///
/// So: stepping in whole bars gives bar numbers, "5". Stepping in beats
/// gives "5.1" for every one of them, downbeat included. Stepping finer
/// carries the subdivision in THOUSANDTHS of a beat, zero-padded:
/// "5.2.250" is a quarter of the way through the second beat.
///
/// Thousandths because that is what a musical position IS here — the
/// DAW's own `MusicalPosition` carries measure, beat and a subdivision
/// of 0..999 — so a number read off this ruler is one you can type
/// back in.
#[must_use]
pub fn written(measure: u32, into_bar: f64, per_bar: f64, step: f64) -> String {
    let whole_beat = into_bar.floor();
    let fraction = into_bar - whole_beat;
    let beat_no = (whole_beat + 1.0).round() as i64;

    // A whole bar between numbers: bar numbers, nothing else to say.
    if step >= per_bar.max(1.0) - 1e-9 {
        return format!("{measure}");
    }
    // A whole beat or more: every label names its beat, downbeat
    // included, so the row is one column of the same thing.
    if step >= 1.0 - 1e-9 || fraction.abs() < 1e-9 {
        return format!("{measure}.{beat_no}");
    }
    // Thousandths of a beat, zero-padded so a column of them lines up
    // and .050 cannot be misread as .5.
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
#[must_use]
pub fn lane_row(lane: u32) -> usize {
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

/// The box an inline rename occupies for a mark at `at` in lane `row`.
///
/// Placed where the mark is drawn, not in a dialog: the name you are
/// typing has to be next to the thing it names, or two marks a bar
/// apart are indistinguishable while you rename one of them.
///
/// Clamped to the visible timeline at both ends, so a band starting off
/// screen is still renamed somewhere you can see.
#[must_use]
pub fn field(view: Viewport, origin: (f64, f64), row: usize, at: f64) -> Rect {
    const WIDTH: f64 = 140.0;
    let (ox, oy) = origin;
    let left = ox + TCP_WIDTH;
    let right = ox + view.width;
    let top = LANE_H.mul_add(crate::num::coord(row), oy);
    let x0 = at
        .mul_add(view.pps, left - view.scroll_x)
        .clamp(left, (right - WIDTH).max(left));
    Rect::new(x0, top + 1.0, (x0 + WIDTH).min(right), top + LANE_H - 1.0)
}

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

// ─── Counting through tempo and signature changes ───────────────────

/// One beat of the project, and where it falls.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Beat {
    /// When it sounds, in seconds.
    pub at: f64,
    /// Which measure it is in, counted from one.
    pub measure: u32,
    /// Which beat of that measure, counted from one.
    pub beat: u32,
    /// How many beats this measure has — the signature in force.
    pub per_bar: u32,
    /// How long a beat lasts here, in seconds. A grid subdividing the
    /// beat needs this; multiplying one project tempo would put every
    /// line after a change in the wrong place.
    pub secs_per_beat: f64,
}

impl Beat {
    /// The first beat of a measure.
    #[must_use]
    pub const fn is_downbeat(self) -> bool {
        self.beat == 1
    }
}

/// The project's beat grid, walked through its tempo map.
///
/// Every bar line is where the tempo and the signature BEFORE it put
/// it. Counting the whole timeline by multiplying one nominal tempo
/// puts every bar after the first change somewhere it is not, and a
/// ruler that is wrong about where bar forty is, is a ruler nobody can
/// edit against.
///
/// Two rules, and they are the ones that matter:
///
/// - A **tempo** change alters how long the following beats take. The
///   count carries on through it: beat three is still beat three.
/// - A **signature** change starts a NEW MEASURE at that point, because
///   a bar of four and a bar of three cannot share a bar line. Anything
///   else would leave a measure that is part one signature and part
///   another, which is not a measure.
#[derive(Clone, Copy)]
pub struct Timeline<'a> {
    changes: &'a [daw_ui::studio::project::TempoChange],
}

impl<'a> Timeline<'a> {
    #[must_use]
    pub const fn new(changes: &'a [daw_ui::studio::project::TempoChange]) -> Self {
        Self { changes }
    }

    /// Every beat from the start of the project up to `to`, in order.
    ///
    /// From the start rather than from the visible left edge, because a
    /// measure number is a COUNT from the beginning — there is no way
    /// to know what bar you are looking at without having counted the
    /// ones before it. Stopped by `to`, and by `limit` so a corrupt
    /// tempo map cannot spin.
    #[must_use]
    pub fn beats(self, to: f64, limit: usize) -> Vec<Beat> {
        let mut out = Vec::new();
        let Some(first) = self.changes.first() else {
            return out;
        };
        let mut at = first.at;
        let mut measure = 1u32;
        let mut beat = 1u32;
        let mut index = 0usize;

        while at <= to && out.len() < limit {
            let change = &self.changes[index];
            let per_bar = change.beats_per_bar.max(1);
            let secs_per_beat = 60.0 / if change.bpm > 0.0 { change.bpm } else { 120.0 };
            out.push(Beat {
                at,
                measure,
                beat,
                per_bar,
                secs_per_beat,
            });

            let next_at = at + secs_per_beat;
            // Does a change fall inside the beat just laid down? If it
            // does, the grid restarts there rather than carrying the
            // old beat length across it.
            let upcoming = self
                .changes
                .iter()
                .enumerate()
                .skip(index + 1)
                .find(|(_, c)| c.at > at + 1e-9);
            match upcoming {
                Some((i, c)) if c.at <= next_at + 1e-9 => {
                    let signature_moved = c.beats_per_bar != change.beats_per_bar;
                    index = i;
                    at = c.at;
                    if signature_moved {
                        measure = measure.saturating_add(1);
                        beat = 1;
                    } else {
                        (measure, beat) = step(measure, beat, per_bar);
                    }
                }
                _ => {
                    at = next_at;
                    (measure, beat) = step(measure, beat, per_bar);
                }
            }
        }
        out
    }
}

/// The next measure and beat after one of `per_bar` beats.
const fn step(measure: u32, beat: u32, per_bar: u32) -> (u32, u32) {
    if beat >= per_bar {
        (measure.saturating_add(1), 1)
    } else {
        (measure, beat + 1)
    }
}

/// The tempo strip: where the tempo or the signature changes, and to
/// what.
///
/// Its own row rather than a flag among the marks: a tempo change is
/// not a place in the song, it is a change to what every number below
/// it MEANS, and filing it with the marks would file it among the
/// things it reinterprets.
///
/// The reading repeats at the left edge when the change that set it is
/// off screen. A tempo you cannot see is a tempo you will assume, and
/// the assumption is always whatever the project started at.
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

#[cfg(test)]
mod tests {
    use super::{Bars, Timeline, step_beats, written};
    use daw_ui::studio::project::TempoChange;

    fn at(at: f64, bpm: f64, per_bar: u32) -> TempoChange {
        TempoChange {
            at,
            bpm,
            beats_per_bar: per_bar,
            beat_unit: 4,
        }
    }

    fn view() -> crate::arrangement::Viewport {
        crate::arrangement::Viewport {
            scroll_x: 0.0,
            scroll_y: 0.0,
            pps: 20.0,
            zoom_y: 1.0,
            width: 1000.0,
            height: 600.0,
        }
    }

    /// The name field opens on the mark, in the mark's own lane.
    #[test]
    fn a_name_opens_where_the_mark_is() {
        let field = super::field(view(), (0.0, 100.0), super::SECTIONS_ROW, 10.0);
        let expected = 10.0f64.mul_add(20.0, crate::arrangement::TCP_WIDTH);
        assert!((field.x0 - expected).abs() < 1e-9, "at {}", field.x0);
        let top = 100.0 + super::LANE_H;
        assert!(field.y0 >= top && field.y1 <= top + super::LANE_H);
    }

    /// A band that starts off the left of the view is still renamed
    /// somewhere you can see, and one near the right edge does not open
    /// its field off the end of the window.
    #[test]
    fn a_name_field_stays_on_screen() {
        let view = view();
        let left = crate::arrangement::TCP_WIDTH;
        let offscreen = super::field(view, (0.0, 0.0), 0, -30.0);
        assert!((offscreen.x0 - left).abs() < 1e-9, "at {}", offscreen.x0);
        let far = super::field(view, (0.0, 0.0), 0, 480.0);
        assert!(far.x1 <= view.width + 1e-9, "ran to {}", far.x1);
    }

    /// The ruler counts MEASURES, not seconds.
    ///
    /// Pinned because it is the question you cannot answer by looking:
    /// at 120 bpm in four four a bar is two seconds, so a ruler
    /// numbering seconds and one numbering bars both count 1, 2, 3.
    #[test]
    fn the_numbers_are_bars() {
        let beats = Timeline::new(&[at(0.0, 120.0, 4)]).beats(8.0, 100);
        let downbeats: Vec<&super::Beat> = beats.iter().filter(|b| b.is_downbeat()).collect();
        assert!(
            (downbeats[1].at - 2.0).abs() < 1e-9,
            "bar 2 is at two seconds"
        );
        assert_eq!(downbeats[1].measure, 2);
        // The tempo moves them, which a seconds ruler would not notice.
        let slow = Timeline::new(&[at(0.0, 60.0, 4)]).beats(8.0, 100);
        let bar2 = slow.iter().find(|b| b.measure == 2 && b.is_downbeat());
        assert!(
            (bar2.unwrap().at - 4.0).abs() < 1e-9,
            "at 60 a bar is four seconds"
        );
    }

    /// The signature decides how many beats make a bar.
    #[test]
    fn the_signature_decides_the_bar() {
        let beats = Timeline::new(&[at(0.0, 120.0, 3)]).beats(6.0, 100);
        let bar2 = beats.iter().find(|b| b.measure == 2 && b.is_downbeat());
        assert!(
            (bar2.unwrap().at - 1.5).abs() < 1e-9,
            "three beats at 120 is a bar and a half second"
        );
        assert_eq!(beats[2].beat, 3, "a three-four bar has a third beat");
        assert_eq!(beats[3].measure, 2, "and no fourth");
    }

    /// A tempo change moves every bar line AFTER it.
    ///
    /// The thing a multiplied grid gets wrong. Four bars at 120 take
    /// eight seconds; halve the tempo at eight and the next bar takes
    /// four, not two.
    #[test]
    fn a_tempo_change_moves_the_bars_after_it() {
        let map = [at(0.0, 120.0, 4), at(8.0, 60.0, 4)];
        let beats = Timeline::new(&map).beats(20.0, 200);
        let downs: Vec<f64> = beats
            .iter()
            .filter(|b| b.is_downbeat())
            .map(|b| b.at)
            .collect();
        assert!((downs[0] - 0.0).abs() < 1e-9);
        assert!((downs[1] - 2.0).abs() < 1e-9);
        assert!(
            (downs[4] - 8.0).abs() < 1e-9,
            "the change lands on a bar line"
        );
        assert!(
            (downs[5] - 12.0).abs() < 1e-9,
            "after the change a bar takes four seconds, got {}",
            downs[5]
        );
    }

    /// A signature change starts a new measure.
    ///
    /// A bar of four and a bar of three cannot share a bar line, and a
    /// measure that is part one signature and part another is not a
    /// measure.
    #[test]
    fn a_signature_change_starts_a_measure() {
        // Change mid-bar, two beats into the second bar.
        let map = [at(0.0, 120.0, 4), at(3.0, 120.0, 3)];
        let beats = Timeline::new(&map).beats(9.0, 200);
        let change = beats
            .iter()
            .find(|b| (b.at - 3.0).abs() < 1e-9)
            .expect("a beat at the change");
        assert_eq!(change.beat, 1, "the new signature starts on beat one");
        assert_eq!(change.per_bar, 3, "and counts in three from there");
        assert_eq!(change.measure, 3, "in a new measure, not the middle of one");
    }

    /// Zooming in keeps saying something new, down to the beat.
    #[test]
    fn zooming_in_subdivides_the_bar() {
        assert!(step_beats(400.0, 4.0) <= 1.0);
        assert!(step_beats(20.0, 4.0) >= 4.0);
        assert!(step_beats(400.0, 4.0) <= step_beats(100.0, 4.0));
    }

    /// measure.beat.subdivision, and only as much as the step needs.
    #[test]
    fn a_position_is_written_the_way_a_daw_writes_one() {
        // Stepping in bars: bar numbers.
        assert_eq!(written(1, 0.0, 4.0, 4.0), "1");
        assert_eq!(written(2, 0.0, 4.0, 4.0), "2");
        // Stepping in beats: every label names its beat, downbeat too.
        assert_eq!(written(1, 0.0, 4.0, 1.0), "1.1");
        assert_eq!(written(1, 1.0, 4.0, 1.0), "1.2");
        assert_eq!(written(2, 3.0, 4.0, 1.0), "2.4");
        // Thousandths of a beat, zero-padded.
        assert_eq!(written(1, 0.25, 4.0, 0.25), "1.1.250");
        assert_eq!(written(1, 0.5, 4.0, 0.25), "1.1.500");
        assert_eq!(written(1, 0.75, 4.0, 0.25), "1.1.750");
        assert_eq!(written(2, 0.5, 4.0, 0.25), "2.1.500");
        // A beat that lands whole still names itself at a fine step.
        assert_eq!(written(1, 1.0, 4.0, 0.25), "1.2");
    }

    /// A sane fallback when the project has no tempo at all.
    #[test]
    fn no_tempo_map_counts_nothing_rather_than_forever() {
        assert!(Timeline::new(&[]).beats(60.0, 100).is_empty());
        // And a nonsense tempo cannot spin the walk.
        let mad = [at(0.0, 0.0, 4)];
        let beats = Timeline::new(&mad).beats(60.0, 100);
        assert!(
            !beats.is_empty(),
            "a zero tempo should fall back, not stall"
        );
        assert!(beats.len() <= 100, "the limit holds");
    }

    /// `Bars` still answers for a single tempo, which the grid uses.
    #[test]
    fn bars_reads_the_map_at_a_time() {
        let map = [at(0.0, 120.0, 4), at(10.0, 60.0, 3)];
        assert!((Bars::at_time(&map, 0.0).secs_per_bar() - 2.0).abs() < 1e-9);
        assert!((Bars::at_time(&map, 9.9).secs_per_bar() - 2.0).abs() < 1e-9);
        assert!((Bars::at_time(&map, 10.0).secs_per_bar() - 3.0).abs() < 1e-9);
    }
}
