//! The Tone rack — EQ, compressor and saturation, inside the strip.
//!
//! The mix phase after Balance is Tone, and it is three processors:
//! compression, EQ, saturation. The parallel filters and the parallel
//! compressors are a different phase and are deliberately not here.
//!
//! A channel strip that shows those three is a channel you can mix
//! without opening a plugin window, which is the whole point — a mix is
//! a comparison between tracks, and a comparison you have to make one
//! floating window at a time is not a comparison.
//!
//! # It draws the plugins' own curves
//!
//! Every response in this module comes from the function the plugin's
//! own editor draws with:
//!
//! | panel | curve from |
//! |---|---|
//! | EQ | `eq_ui::eq_graph_response::calculate_combined_response` |
//! | Compressor | `comp_ui::comp_graph_svg::compress_transfer` |
//! | Saturation | `saturate_dsp::preamp::analysis::transfer_curve` |
//!
//! None of the maths is here, and that is the point: a curve in the
//! strip that disagreed with the curve in the plugin window would be
//! worse than no curve at all, because it would be believed. This module
//! samples those functions into polylines and places them in a box.
//!
//! # What it does not do yet
//!
//! Move. The curves depend only on the parameters, so they are recorded
//! into the mixer's scene once and replayed under the transform like
//! everything else. The parts that DO move — gain reduction, the
//! spectrum behind the EQ — are metering, and metering arrives through
//! `daw_ui::widgets::trace` in a live pass over the top, so that a track
//! whose processing was rendered offline still animates.

use anyrender::{PaintScene, Scene};
use comp_ui::comp_graph_svg::compress_transfer;
use eq_ui::eq_graph_model::{EqBand, EqBandShape, StereoMode};
use eq_ui::eq_graph_response::calculate_combined_response;
use fts_audio_ui::axis::{DbAxis, FreqAxis};
use saturate_dsp::preamp::{ClassAPreamp, SideShaper};
use vello::kurbo::{Affine, BezPath, Line, Rect, Stroke};
use vello::peniko::{Color, Fill};

use crate::arrangement::Palette;
use crate::text::Font;

/// The sample rate the curves are drawn against.
///
/// A display constant, not an audio one: the EQ's response at 20 kHz
/// bends with the rate, and a strip that redrew its curve when the
/// device changed would be showing the device rather than the setting.
const DISPLAY_RATE: f64 = 48_000.0;

/// How many points a curve is sampled at.
///
/// One per ~2px of a 200-wide panel. Beyond that the polyline is finer
/// than the pixels and costs recording time for nothing — and this runs
/// once per track at project open, not per frame.
const SAMPLES: usize = 96;

/// The three processors of the Tone phase, for one track.
///
/// Deliberately the plugins' own parameter types rather than a summary
/// of them. A rack that held its own reduced idea of "an EQ" would have
/// to be widened every time a band gained a shape, and would be drawing
/// something the plugin cannot produce.
#[derive(Clone, Debug)]
pub struct Tone {
    pub eq: Vec<EqBand>,
    pub comp: Comp,
    pub sat: ClassAPreamp,
}

/// A compressor, as its transfer curve needs it.
#[derive(Clone, Copy, Debug)]
pub struct Comp {
    pub threshold: f32,
    pub ratio: f32,
    pub knee: f32,
}

impl Default for Comp {
    fn default() -> Self {
        Self {
            threshold: -18.0,
            ratio: 3.0,
            knee: 6.0,
        }
    }
}

/// What fits in a rack of a given width.
///
/// The same shed-in-order rule as `Squeeze` and `Collapse`: a panel that
/// cannot be read is worse than the space it took, because it still
/// looks like information.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Rack {
    /// All three panels, each with its grid and its label.
    Full,
    /// The curves alone, stacked. No grid, no labels — at this width a
    /// gridline is a third of the panel and the label is most of it.
    Curves,
    /// Nothing. The strip is a fader and a name, and the rack's height
    /// goes back to the strip.
    Off,
}

impl Rack {
    /// What fits in `width`.
    ///
    /// `Full` needs room for a decade of frequency to still read as a
    /// decade — below about 150 the EQ's three gridlines land on top of
    /// each other. `Curves` needs only enough width for the curve's
    /// shape to survive, which is far less.
    #[must_use]
    pub fn at(width: f64) -> Self {
        if width >= LEGIBLE {
            Self::Full
        } else if width >= 96.0 {
            Self::Curves
        } else {
            Self::Off
        }
    }

    /// Whether the rack draws at all.
    #[must_use]
    pub fn on(self) -> bool {
        self != Self::Off
    }
}

/// The narrowest rack whose EQ panel still reads as a frequency axis.
///
/// Three gridlines — 100, 1k, 10k — and below this they crowd into each
/// other, at which point the panel is a squiggle rather than a decision
/// you can check.
pub const LEGIBLE: f64 = 150.0;

/// The width a strip opens to when you go to WORK on it.
///
/// Above [`LEGIBLE`] rather than at it: the threshold is where the rack
/// stops being illegible, and a control you have deliberately opened
/// should not land on the edge of that. This is what a selected strip
/// expands to, and what the template stores for a track that carries
/// its piece's processing.
pub const WORKING: f64 = 195.0;

/// Where the rack goes.
#[derive(Clone, Copy, Debug)]
pub struct Panel {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Panel {
    const fn rect(self) -> Rect {
        Rect::new(self.x, self.y, self.x + self.width, self.y + self.height)
    }

    /// The panel inset by `by` on every side.
    const fn inset(self, by: f64) -> Self {
        Self {
            x: self.x + by,
            y: self.y + by,
            width: self.width - by * 2.0,
            height: self.height - by * 2.0,
        }
    }
}

/// The gap between two panels of the rack.
const GAP: f64 = 3.0;

/// How the rack's height is split between the three panels.
///
/// The EQ gets the most because it is the one with two axes worth
/// reading: a compressor's curve is a bent line and a saturator's is a
/// bent line, while an EQ's is the shape of the decision.
const SHARE: [f64; 3] = [0.44, 0.28, 0.28];

/// Record the rack into `scene`.
///
/// Returns nothing: like every other control in the mixer this is
/// recorded once into the strip's command range and replayed from there.
pub fn record(scene: &mut Scene, palette: &Palette, font: &Font, tone: &Tone, panel: Panel) {
    let rack = Rack::at(panel.width);
    if !rack.on() || panel.height < 24.0 {
        return;
    }

    let gaps = GAP * 2.0;
    let usable = (panel.height - gaps).max(0.0);
    let mut y = panel.y;
    for (share, which) in SHARE.iter().zip([Which::Eq, Which::Comp, Which::Sat]) {
        let h = usable * share;
        let at = Panel {
            x: panel.x,
            y,
            width: panel.width,
            height: h,
        };
        ground(scene, palette, at);
        let inner = at.inset(2.0);
        if inner.width > 0.0 && inner.height > 0.0 {
            match which {
                Which::Eq => eq(scene, palette, tone, inner, rack),
                Which::Comp => comp(scene, palette, tone.comp, inner, rack),
                Which::Sat => sat(scene, palette, &tone.sat, inner, rack),
            }
            if rack == Rack::Full {
                label(scene, palette, font, which.name(), inner);
            }
        }
        y += h + GAP;
    }
}

#[derive(Clone, Copy)]
enum Which {
    Eq,
    Comp,
    Sat,
}

impl Which {
    const fn name(self) -> &'static str {
        match self {
            Self::Eq => "EQ",
            Self::Comp => "COMP",
            Self::Sat => "SAT",
        }
    }
}

/// A panel's well — darker than the strip, so the rack reads as inset
/// rather than as three more controls on the strip's own surface.
fn ground(scene: &mut Scene, palette: &Palette, at: Panel) {
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        palette.tcp_meter_well,
        None,
        &at.rect(),
    );
}

/// The EQ's response across the audible band.
fn eq(scene: &mut Scene, palette: &Palette, tone: &Tone, at: Panel, rack: Rack) {
    let freq = FreqAxis::audible();
    // ±18 rather than the editor's ±30: a strip panel is thirty pixels
    // tall and a curve drawn to ±30 in it is a flat line with a wobble.
    let db = DbAxis::symmetric(18.0);
    let right = at.x + at.width;
    let bottom = at.y + at.height;

    if rack == Rack::Full {
        for hz in [100.0, 1_000.0, 10_000.0] {
            let x = freq.freq_to_x(hz, at.x, right);
            rule(scene, palette.grid_beat, Line::new((x, at.y), (x, bottom)));
        }
    }
    // Unity, always: without it a boost and a cut look the same.
    let zero = db.db_to_y(0.0, at.y, bottom);
    rule(
        scene,
        palette.grid,
        Line::new((at.x, zero), (right, zero)),
    );

    if tone.eq.is_empty() {
        return;
    }
    let points = (0..SAMPLES).map(|i| {
        let t = crate::num::coord(i) / crate::num::coord(SAMPLES.saturating_sub(1).max(1));
        let hz = freq.norm_to_freq(t);
        let gain = calculate_combined_response(&tone.eq, hz, DISPLAY_RATE);
        (
            freq.freq_to_x(hz, at.x, right),
            db.db_to_y(gain, at.y, bottom).clamp(at.y, bottom),
        )
    });
    curve(scene, palette.accent, points, 1.5);
}

/// The compressor's transfer curve, input dB across, output dB up.
fn comp(scene: &mut Scene, palette: &Palette, comp: Comp, at: Panel, rack: Rack) {
    let right = at.x + at.width;
    let bottom = at.y + at.height;
    // The comp editor's own window: −60 to 0 on both axes.
    let to_x = |db: f64| at.x + (db + 60.0) / 60.0 * at.width;
    let to_y = |db: f64| bottom - (db + 60.0) / 60.0 * at.height;

    // Unity, so the bend below threshold is visible as a departure from
    // it rather than as a line at an angle.
    rule(
        scene,
        palette.grid,
        Line::new((to_x(-60.0), to_y(-60.0)), (to_x(0.0), to_y(0.0))),
    );
    if rack == Rack::Full {
        let t = to_x(f64::from(comp.threshold));
        rule(scene, palette.grid_beat, Line::new((t, at.y), (t, bottom)));
    }

    let points = (0..SAMPLES).map(|i| {
        let t = crate::num::coord(i) / crate::num::coord(SAMPLES.saturating_sub(1).max(1));
        let input = t.mul_add(60.0, -60.0);
        let output = f64::from(compress_transfer(
            crate::tone::f64_to_f32(input),
            comp.threshold,
            comp.ratio,
            comp.knee,
        ));
        (
            to_x(input).clamp(at.x, right),
            to_y(output).clamp(at.y, bottom),
        )
    });
    curve(scene, palette.meter_warn, points, 1.5);
}

/// The saturator's static transfer curve over x ∈ [−1, 1].
fn sat(scene: &mut Scene, palette: &Palette, pre: &ClassAPreamp, at: Panel, rack: Rack) {
    let right = at.x + at.width;
    let bottom = at.y + at.height;
    let mid_y = at.y + at.height / 2.0;

    if rack == Rack::Full {
        rule(scene, palette.grid, Line::new((at.x, mid_y), (right, mid_y)));
    }

    let mut samples = [(0.0_f32, 0.0_f32); SAMPLES];
    saturate_dsp::preamp::analysis::transfer_curve(pre, &mut samples);
    let points = samples.iter().map(|(x, y)| {
        (
            at.x + (f64::from(*x) + 1.0) / 2.0 * at.width,
            (mid_y - f64::from(*y) * at.height / 2.0).clamp(at.y, bottom),
        )
    });
    curve(scene, palette.pan, points, 1.5);
}

/// One hairline.
fn rule(scene: &mut Scene, color: Color, line: Line) {
    scene.stroke(
        &Stroke::new(1.0),
        Affine::IDENTITY,
        color,
        None,
        &line,
    );
}

/// An open polyline through `points`.
fn curve(
    scene: &mut Scene,
    color: Color,
    points: impl Iterator<Item = (f64, f64)>,
    width: f64,
) {
    let mut path = BezPath::new();
    for (i, point) in points.enumerate() {
        if i == 0 {
            path.move_to(point);
        } else {
            path.line_to(point);
        }
    }
    if path.is_empty() {
        return;
    }
    scene.stroke(
        &Stroke::new(width).with_caps(vello::kurbo::Cap::Round),
        Affine::IDENTITY,
        color,
        None,
        &path,
    );
}

/// The panel's name, small and in the corner.
fn label(scene: &mut Scene, palette: &Palette, font: &Font, text: &str, at: Panel) {
    const SIZE: f32 = 7.0;
    crate::tcp::glyphs(
        scene,
        font,
        palette.text_faint,
        text,
        at.x + 2.0,
        at.y + f64::from(SIZE),
        SIZE,
    );
}

const fn f64_to_f32(value: f64) -> f32 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::as_conversions,
        reason = "a dB value in a display range of sixty; f32 holds it exactly"
    )]
    let narrowed = value as f32;
    narrowed
}

/// A plausible Tone chain, until tracks carry their own.
///
/// The rack has nowhere to read real settings from yet — the FX chain is
/// not in `daw_proto::Track` — so this stands in, varied by track index
/// so that a mixer full of racks looks like a mixer full of different
/// decisions rather than one curve repeated twenty times.
#[must_use]
pub fn placeholder(index: usize) -> Tone {
    let nudge = crate::num::coord(index % 7);
    Tone {
        eq: vec![
            band(0, 80.0 * 1.1_f64.powf(nudge), -3.0, 0.7, EqBandShape::LowShelf),
            band(1, 300.0 + nudge * 60.0, -2.5 - nudge * 0.4, 1.4, EqBandShape::Bell),
            band(2, 3_000.0 + nudge * 400.0, 2.0 + nudge * 0.5, 1.1, EqBandShape::Bell),
            band(3, 10_000.0, 2.5, 0.7, EqBandShape::HighShelf),
        ],
        comp: Comp {
            threshold: -14.0 - f64_to_f32(nudge),
            ratio: 2.0 + f64_to_f32(nudge) * 0.5,
            knee: 6.0,
        },
        sat: {
            let mut pre = ClassAPreamp::new(f64_to_f32(DISPLAY_RATE));
            // A single-ended stage: a triode grid above and iron below,
            // biased off centre. `Clean` is wire, and wire into the
            // output clamp draws a hard clipper's flat-ramp-flat — a
            // picture of a limiter, not of saturation.
            pre.positive = SideShaper::Tube;
            pre.negative = SideShaper::Transformer;
            pre.drive = 2.5 + f64_to_f32(nudge) * 0.8;
            pre.q_point = 0.25;
            pre
        },
    }
}

fn band(index: usize, frequency: f64, gain: f64, q: f64, shape: EqBandShape) -> EqBand {
    EqBand {
        index,
        used: true,
        enabled: true,
        frequency: f64_to_f32(frequency),
        gain: f64_to_f32(gain),
        q: f64_to_f32(q),
        shape,
        // The cut filters' slope order; `None` leaves the band on its Q,
        // which is what every shape the rack places uses.
        slope: None,
        focus: false,
        stereo_mode: StereoMode::default(),
        name: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tiers are ordered and the thresholds do not overlap.
    #[test]
    fn the_rack_sheds_in_order() {
        assert_eq!(Rack::at(300.0), Rack::Full);
        assert_eq!(Rack::at(120.0), Rack::Curves);
        assert_eq!(Rack::at(86.0), Rack::Off);
        assert!(Rack::Full < Rack::Curves && Rack::Curves < Rack::Off);
        assert!(Rack::at(300.0).on() && !Rack::at(86.0).on());
    }

    /// A rack with no room records nothing at all, rather than three
    /// slivers of well with no curve in them.
    #[test]
    fn a_flat_rack_draws_nothing() {
        let mut scene = Scene::new();
        let palette = Palette::from_theme(&daw_ui::theming::Theme::dark());
        let font = Font::embedded().expect("the embedded font");
        record(
            &mut scene,
            &palette,
            &font,
            &placeholder(0),
            Panel {
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 8.0,
            },
        );
        assert!(scene.commands.is_empty());
    }

    /// And a rack with room records all three panels.
    #[test]
    fn a_full_rack_draws_three_panels() {
        let mut scene = Scene::new();
        let palette = Palette::from_theme(&daw_ui::theming::Theme::dark());
        let font = Font::embedded().expect("the embedded font");
        record(
            &mut scene,
            &palette,
            &font,
            &placeholder(0),
            Panel {
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 180.0,
            },
        );
        // Three grounds, three curves, the rules and the labels.
        assert!(
            scene.commands.len() > 9,
            "a full rack should record more than its three grounds: {}",
            scene.commands.len()
        );
    }

    /// The EQ curve is the plugin's, so a boost must come back as one:
    /// the response at a boosted band's centre is above unity.
    #[test]
    fn the_curve_is_the_plugins_own() {
        let tone = placeholder(0);
        let at_3k = calculate_combined_response(&tone.eq, 3_000.0, DISPLAY_RATE);
        let at_300 = calculate_combined_response(&tone.eq, 300.0, DISPLAY_RATE);
        assert!(at_3k > 0.0, "the 3k bell boosts: {at_3k}");
        assert!(at_300 < 0.0, "the 300 bell cuts: {at_300}");
    }
}
