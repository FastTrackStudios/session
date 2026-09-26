//! The bar grid down the lanes and the bar numbers along the ruler's foot:
//! which divisions a zoom shows, and how a measure is written.

use super::*;

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
    // The main toolbar's grid-lines switch.
    if !crate::options::GRID.get() {
        return;
    }
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
        Rect::new(ox, oy, ox + view.width, oy + ruler_h()),
    );
    fill(
        painter,
        palette.tcp_rule,
        Rect::new(ox, oy + ruler_h() - 1.0, ox + view.width, oy + ruler_h()),
    );
    // The bars are the bottom of the strip; the tempo sits just above
    // them and the lanes over that.
    let oy = oy + ruler_h() - BARS_H;

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
            let x = t.mul_add(view.pps, view.panel_w - view.scroll_x);
            if x < view.panel_w - 1.0 || x > view.width {
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
pub(super) const MAX_BEATS: usize = 100_000;

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
pub(super) const BEAT_STEPS: [f64; 3] = [0.25, 0.5, 1.0];

/// How many bars between numbers once a beat is too fine to label.
pub(super) const BAR_STEPS: [f64; 8] = [1.0, 2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0];

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
pub(super) const LABEL_ROOM: f64 = 56.0;

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
