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
        } else if width >= SHAPE {
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

    /// The narrowest width still in this tier.
    ///
    /// What a strip may be shrunk to without changing what it shows —
    /// see `mcp::lending_floor`, which is the only caller and the
    /// reason this exists. Returning the tier's own lower bound rather
    /// than a fixed floor is what keeps a strip from being lent out of
    /// the rack it was drawing.
    #[must_use]
    pub const fn floor(self) -> Option<f64> {
        match self {
            Self::Full => Some(LEGIBLE),
            Self::Curves => Some(SHAPE),
            Self::Off => None,
        }
    }
}

/// The narrowest rack whose EQ panel still reads as a frequency axis.
///
/// Three gridlines — 100, 1k, 10k — and below this they crowd into each
/// other, at which point the panel is a squiggle rather than a decision
/// you can check.
pub const LEGIBLE: f64 = 96.0;

/// The narrowest rack that still says anything.
///
/// Below this a curve is a few pixels of wiggle — it reads as ornament
/// rather than as a setting, and ornament in a mixer is worse than
/// space.
pub const SHAPE: f64 = 90.0;

/// The width a strip opens to when you go to WORK on it.
///
/// Above [`LEGIBLE`] rather than at it: the threshold is where the rack
/// stops being illegible, and a control you have deliberately opened
/// should not land on the edge of that.
///
/// One number for two jobs, and they have to be the same number: it is
/// what a selected strip expands to, AND what the template stores for a
/// track that carries its piece's processing. If the stored width were
/// larger, selecting a piece would SHRINK it; if smaller, selecting one
/// would widen it and shove every strip to its right. Either way the
/// mixer moves under you at the moment you click a track, which is
/// exactly when it must not.
///
/// It does not have to be paid for twice. An opened strip BORROWS its
/// extra width from the others rather than adding to the total — see
/// `mcp::widths` — so this is bounded by what the resting layout can
/// afford, not by what the worst selection would cost on top of it.
pub const WORKING: f64 = 133.0;

/// Where the rack goes.
#[derive(Clone, Copy, Debug)]
pub struct Panel {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Panel {
    /// From a rect in strip coordinates, moved to where the strip is.
    ///
    /// The rack's box comes from [`crate::strip::Strip::rack_rect`], in
    /// the strip's own space; the recording is in the mixer's. This is
    /// the one conversion between them, so a grip and a curve cannot
    /// disagree about where the rack starts.
    #[must_use]
    pub fn of(rect: Rect, left: f64) -> Self {
        Self {
            x: rect.x0 + left,
            y: rect.y0,
            width: rect.width(),
            height: rect.height(),
        }
    }

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

    /// `by` off the top, and what is left.
    ///
    /// The header is a strip of the panel rather than an overlay on it:
    /// a value printed over a curve is unreadable exactly when the
    /// curve is interesting, which is the moment you want the number.
    const fn split_top(self, by: f64) -> (Self, Self) {
        let head = Self {
            height: by,
            ..self
        };
        let body = Self {
            y: self.y + by,
            height: self.height - by,
            ..self
        };
        (head, body)
    }
}

/// How tall a panel's header is.
///
/// The type is 7pt and this is the line it sits on plus a pixel of air
/// under it. Small, because it is a readout and not a title — you look
/// at the curve and read the number to confirm what you saw.
const HEAD: f64 = 10.0;

/// The gap between two panels of the rack.
const GAP: f64 = 3.0;

/// How the rack's height is split between the three panels.
///
/// The EQ gets the most because it is the one with two axes worth
/// reading: a compressor's curve is a bent line and a saturator's is a
/// bent line, while an EQ's is the shape of the decision.
const SHARE: [f64; 3] = [0.44, 0.28, 0.28];

/// Which panels a mix phase asks for.
///
/// A phase is a pass over the session with one question in it, and the
/// rack should be showing the processing that answers it. Rescue is a
/// surgical pass, so it wants the EQ and nothing else; Balance is the
/// fader pass and wants no rack at all, which hands its height back to
/// the strip — which is what "every track visible and detailed" means
/// when the thing being compared is levels.
///
/// The three panels this rack can draw are the three the Tone phase is
/// made of, so the phases past Polish come back empty rather than
/// borrowing a curve that is not about them. An empty rack is honest;
/// a saturation graph over a Depth pass is not.
#[must_use]
pub fn panels_for(phase: session::mix_phases::MixPhase) -> &'static [Which] {
    use session::mix_phases::MixPhase as P;
    match phase {
        P::Rescue => &[Which::Eq],
        P::Tone => &[Which::Eq, Which::Comp, Which::Sat],
        P::Polish => &[Which::Comp, Which::Sat],
        // Balance is the fader pass; Relational, Depth and Creative are
        // phases whose processing this rack has no panel for yet; and
        // Overview is the one view that is deliberately only a shape.
        P::Balance | P::Relational | P::Depth | P::Creative | P::Overview => &[],
    }
}

/// Record the rack into `scene`.
///
/// Returns nothing: like every other control in the mixer this is
/// recorded once into the strip's command range and replayed from there.
pub fn record(
    scene: &mut Scene,
    palette: &Palette,
    font: &Font,
    tone: &Tone,
    panels: &[Which],
    panel: Panel,
) {
    draw(scene, palette, font, tone, panels, panel, None);
}

/// The same, with one grip lit.
///
/// Only the live pass has a pointer to report, so the recording calls
/// [`record`] and this is what the overlay reaches for. A lit grip is
/// the difference between "there is a handle here" and "this is the
/// handle you will move", which on a curve with four of them is the
/// whole question.
pub fn draw(
    scene: &mut Scene,
    palette: &Palette,
    font: &Font,
    tone: &Tone,
    panels: &[Which],
    panel: Panel,
    lit: Option<Grip>,
) {
    let rack = Rack::at(panel.width);
    if !rack.on() || panel.height < 24.0 || panels.is_empty() {
        return;
    }

    for (which, at) in layout(panels, panel) {
        ground(scene, palette, at);
        let inner = at.inset(2.0);
        // The header is only taken at `Full`. At `Curves` the panel is
        // a shape and nothing else fits; giving up a tenth of its
        // height for a number nobody can read would cost the shape too.
        let body = body_of(at, rack);
        let head = (body.y > inner.y).then(|| inner.split_top(HEAD).0);
        if body.width > 0.0 && body.height > 0.0 {
            match which {
                Which::Eq => eq(scene, palette, font, tone, body, rack, lit),
                Which::Comp => comp(scene, palette, tone.comp, body, rack, lit),
                Which::Sat => sat(scene, palette, &tone.sat, body, rack),
            }
            if let Some(head) = head {
                header(scene, palette, font, which.name(), &which.summary(tone), head);
            }
        }
    }
}

/// Where each panel of the rack lands.
///
/// The one place the rack's vertical division is worked out. Drawing
/// reads it and so does the hit test, which is the same rule
/// [`crate::strip::Strip`] exists for: a grip that is not where its
/// curve is drawn is a grip that moves the wrong thing.
///
/// The shares are authored for the full three; a shorter rack
/// renormalises them rather than leaving a gap at the bottom, so two
/// panels fill the height three did and keep their proportions to each
/// other.
#[must_use]
pub fn layout(panels: &[Which], panel: Panel) -> Vec<(Which, Panel)> {
    let total: f64 = panels.iter().map(|which| which.share()).sum();
    if total <= 0.0 || panels.is_empty() {
        return Vec::new();
    }
    let gaps = GAP * crate::num::coord(panels.len().saturating_sub(1));
    let usable = (panel.height - gaps).max(0.0);
    let mut y = panel.y;
    panels
        .iter()
        .copied()
        .map(|which| {
            let height = usable * which.share() / total;
            let at = Panel {
                x: panel.x,
                y,
                width: panel.width,
                height,
            };
            y += height + GAP;
            (which, at)
        })
        .collect()
}

/// The body of a panel — what is left once its header is taken.
///
/// Shared by the drawing and the hit test for the same reason
/// [`layout`] is: the curve is drawn in the body, so a grip has to be
/// measured against the body.
#[must_use]
pub fn body_of(at: Panel, rack: Rack) -> Panel {
    let inner = at.inset(2.0);
    if rack == Rack::Full && inner.height > HEAD * 2.0 {
        inner.split_top(HEAD).1
    } else {
        inner
    }
}

/// One panel of the rack.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Which {
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

    /// What this panel's settings come to, in one short line.
    ///
    /// A curve says the SHAPE of a decision and a number says the
    /// decision. Both, because they answer different questions: you
    /// scan the curves across a mixer to find the track that is
    /// different, and you read the number to know what to type into the
    /// one you opened.
    fn summary(self, tone: &Tone) -> String {
        match self {
            Self::Eq => {
                let live = tone.eq.iter().filter(|band| band.enabled && band.used).count();
                let range = tone
                    .eq
                    .iter()
                    .filter(|band| band.enabled && band.used)
                    .map(|band| band.gain.abs())
                    .fold(0.0_f32, f32::max);
                if live == 0 {
                    "flat".to_owned()
                } else {
                    format!("{live} · {range:.1}dB")
                }
            }
            Self::Comp => format!("{:.0}dB · {:.1}:1", tone.comp.threshold, tone.comp.ratio),
            Self::Sat => format!("x{:.1}", tone.sat.drive),
        }
    }

    /// How much of the rack this panel wants, relative to the others.
    ///
    /// The EQ gets most of it because a frequency response needs
    /// horizontal AND vertical room to be read; a transfer curve is a
    /// line through a square and survives being short.
    const fn share(self) -> f64 {
        match self {
            Self::Eq => SHARE[0],
            Self::Comp => SHARE[1],
            Self::Sat => SHARE[2],
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
fn eq(
    scene: &mut Scene,
    palette: &Palette,
    font: &Font,
    tone: &Tone,
    at: Panel,
    rack: Rack,
    lit: Option<Grip>,
) {
    let freq = FreqAxis::audible();
    let db = DbAxis::symmetric(EQ_RANGE);
    let right = at.x + at.width;
    let bottom = at.y + at.height;

    if rack == Rack::Full {
        for (hz, name) in DECADES {
            let x = freq.freq_to_x(hz, at.x, right);
            rule(scene, palette.grid_beat, Line::new((x, at.y), (x, bottom)));
            // Labelled along the floor, so a bump can be described to
            // someone else without opening the plugin.
            const SIZE: f32 = 6.0;
            let w = font.width(name, SIZE);
            if x - w / 2.0 >= at.x && x + w / 2.0 <= right {
                crate::tcp::glyphs(
                    scene,
                    font,
                    palette.text_faint,
                    name,
                    x - w / 2.0,
                    bottom - 2.0,
                    SIZE,
                );
            }
        }
        // The dB ladder. A strip panel is tall and a gentle EQ uses
        // little of it, so most of the box is empty — and empty space
        // with no marks in it makes a 3 dB decision look the same as a
        // 12 dB one. These are what turn the height into a scale.
        for step in [6.0, 12.0] {
            for gain in [step, -step] {
                let y = db.db_to_y(gain, at.y, bottom);
                rule(scene, palette.grid_beat, Line::new((at.x, y), (right, y)));
            }
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

    // The bands themselves, as handles on the curve.
    //
    // Without them the panel is a line: you can see that something was
    // done and not how many decisions it took or where they sit. A
    // four-band cut and one wide shelf can draw the same curve, and
    // they are not the same setting.
    //
    // Only at `Full`. At `Curves` a handle is three pixels of dot on a
    // curve two pixels wide, which reads as a kink in the line.
    if rack != Rack::Full {
        return;
    }
    for (index, band) in tone.eq.iter().enumerate() {
        if !(band.enabled && band.used) {
            continue;
        }
        let x = freq.freq_to_x(f64::from(band.frequency), at.x, right);
        let y = db.db_to_y(f64::from(band.gain), at.y, bottom);
        if x < at.x || x > right || y < at.y || y > bottom {
            continue;
        }
        // The one under the pointer grows rather than changing colour:
        // a handle is already the accent, and a second accent would be
        // a colour nobody could name. Size is what a hand reads.
        let grown = lit == Some(Grip::Band(index));
        let r = if grown { HANDLE + 1.6 } else { HANDLE };
        dot(scene, palette.tcp_meter_well, (x, y), r + 1.0);
        dot(scene, palette.accent, (x, y), r);
    }
}

/// How big an EQ band's handle is.
///
/// Small enough that four of them on one curve do not merge, large
/// enough to be a target: this is the radius a pointer will have to
/// find once bands are draggable.
const HANDLE: f64 = 2.6;

/// The decades the frequency axis is labelled at.
///
/// Three, not five: at a strip's width the labels are 6pt and four of
/// them collide. 100, 1k and 10k are the ones a tone decision is
/// described in — "take out some 300", "lift the 3k" — and they bracket
/// the two that are not.
const DECADES: [(f64, &str); 3] = [(100.0, "100"), (1_000.0, "1k"), (10_000.0, "10k")];

/// The compressor's transfer curve, input dB across, output dB up.
fn comp(
    scene: &mut Scene,
    palette: &Palette,
    comp: Comp,
    at: Panel,
    rack: Rack,
    lit: Option<Grip>,
) {
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

    // The threshold, where the curve leaves unity. The vertical rule
    // says which input it is; the dot says which OUTPUT, which is the
    // half a transfer curve is read for.
    if rack == Rack::Full {
        let db = f64::from(comp.threshold);
        let out = f64::from(compress_transfer(
            comp.threshold,
            comp.threshold,
            comp.ratio,
            comp.knee,
        ));
        let at_point = (to_x(db).clamp(at.x, right), to_y(out).clamp(at.y, bottom));
        let r = if lit == Some(Grip::Threshold) {
            HANDLE + 1.6
        } else {
            HANDLE
        };
        dot(scene, palette.tcp_meter_well, at_point, r + 1.0);
        dot(scene, palette.meter_warn, at_point, r);
    }
}

/// The saturator's static transfer curve over x ∈ [−1, 1].
fn sat(scene: &mut Scene, palette: &Palette, pre: &ClassAPreamp, at: Panel, rack: Rack) {
    let right = at.x + at.width;
    let bottom = at.y + at.height;
    let mid_y = at.y + at.height / 2.0;

    if rack == Rack::Full {
        rule(scene, palette.grid, Line::new((at.x, mid_y), (right, mid_y)));
        // Unity, so the curve's departure from it IS the saturation.
        // Without it a gentle drive and a hard one are both "an S", and
        // the thing you are looking for is how far from straight it
        // has gone.
        rule(
            scene,
            palette.grid_beat,
            Line::new((at.x, bottom), (right, at.y)),
        );
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

/// A filled circle — a handle, or a marker on a curve.
fn dot(scene: &mut Scene, color: Color, at: (f64, f64), r: f64) {
    scene.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        color,
        None,
        &vello::kurbo::Circle::new(at, r),
    );
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
/// A panel's header: what it is on the left, what it is set to on the
/// right.
///
/// The value is right-aligned so the three panels' numbers line up
/// down the rack — which is what lets you compare two strips by
/// running your eye down them rather than reading six numbers.
///
/// The value is dropped rather than elided when the panel is too narrow
/// for both: a truncated "−14d…" is a number you have to open the
/// plugin to check, which is worse than one you know is not shown.
fn header(
    scene: &mut Scene,
    palette: &Palette,
    font: &Font,
    name: &str,
    value: &str,
    at: Panel,
) {
    const SIZE: f32 = 7.0;
    let baseline = at.y + f64::from(SIZE);
    crate::tcp::glyphs(scene, font, palette.text_faint, name, at.x, baseline, SIZE);

    let name_w = font.width(name, SIZE);
    let value_w = font.width(value, SIZE);
    if name_w + value_w + 6.0 > at.width {
        return;
    }
    crate::tcp::glyphs(
        scene,
        font,
        palette.text_dim,
        value,
        at.x + at.width - value_w,
        baseline,
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
/// The rack has nowhere to read real settings from yet. Not because the
/// chain is unreachable — `FxChain::parameters` is right there — but
/// because what comes back is unusable: daw-standalone adds an FX entry
/// by name without loading a binary, so its parameters fall through to
/// the stored path and arrive as `Param 1..N` at 0.5. `bin/chain-probe`
/// demonstrates it against a real session.
///
/// A rack fed `Param 3 = 0.5` would draw a curve that looks like
/// information and is not, which is worse than one that is honestly
/// made up. So this stands in, varied by track index so that a mixer
/// full of racks looks like a mixer full of different decisions rather
/// than one curve repeated twenty times.
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

/// Something in the rack you can take hold of.
///
/// Not a control in the strip's sense: these live inside a curve's own
/// axes, so what a drag means depends on which curve it started in. A
/// band moves in frequency AND gain at once, which is the gesture an
/// EQ is actually used with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Grip {
    /// An EQ band, by its index in the chain's list.
    Band(usize),
    /// The compressor's threshold — dragged along the input axis.
    Threshold,
    /// The saturator's drive.
    Drive,
}

/// How close a pointer has to be to take hold of something.
///
/// Generous next to [`HANDLE`], because a handle is drawn at the size
/// it reads best and grabbed at the size a hand can hit. Six pixels is
/// about a millimetre at these densities.
const GRAB: f64 = 6.0;

/// What is under a point in the rack, if anything.
///
/// `panel` is the rack's whole box in the same coordinates as `x` and
/// `y` — the strip's, not the window's.
#[must_use]
pub fn grip_at(
    panels: &[Which],
    tone: &Tone,
    panel: Panel,
    x: f64,
    y: f64,
) -> Option<Grip> {
    let rack = Rack::at(panel.width);
    if !rack.on() {
        return None;
    }
    for (which, at) in layout(panels, panel) {
        let body = body_of(at, rack);
        if body.width <= 0.0 || body.height <= 0.0 {
            continue;
        }
        if y < body.y || y > body.y + body.height {
            continue;
        }
        match which {
            // Bands are only grabbable where they are DRAWN, which is
            // only at `Full` — a handle you cannot see is a handle you
            // cannot aim at, and grabbing one by accident moves a
            // setting you did not know was there.
            Which::Eq if rack == Rack::Full => {
                // The nearest band within reach, not the first: four
                // bands on one curve overlap at their skirts, and the
                // one you meant is the one you are closest to.
                let freq = FreqAxis::audible();
                let db = DbAxis::symmetric(EQ_RANGE);
                let right = body.x + body.width;
                let bottom = body.y + body.height;
                let mut best: Option<(f64, usize)> = None;
                for (index, band) in tone.eq.iter().enumerate() {
                    if !(band.enabled && band.used) {
                        continue;
                    }
                    let bx = freq.freq_to_x(f64::from(band.frequency), body.x, right);
                    let by = db.db_to_y(f64::from(band.gain), body.y, bottom);
                    let away = (bx - x).hypot(by - y);
                    if away <= GRAB && best.is_none_or(|(nearest, _)| away < nearest) {
                        best = Some((away, index));
                    }
                }
                if let Some((_, index)) = best {
                    return Some(Grip::Band(index));
                }
            }
            // The whole panel is the grip for these two: there is one
            // number in each, and hunting for a three-pixel dot to
            // change it would be worse than useless on a strip. They
            // stay grabbable at `Curves`, where the curve still shows
            // what the drag is doing.
            Which::Comp => return Some(Grip::Threshold),
            Which::Sat => return Some(Grip::Drive),
            Which::Eq => {}
        }
    }
    None
}

/// The EQ panel's dB range, top to bottom.
///
/// ±18 rather than the editor's ±30: a strip panel is a few hundred
/// pixels tall at most and a curve drawn to ±30 in it is a flat line
/// with a wobble. Stated once because the drawing and the hit test both
/// have to read the same scale.
pub const EQ_RANGE: f64 = 18.0;

/// Put a grip back where it started.
///
/// Double-clicking a control to default it is the gesture every DAW
/// has, and it is the one that makes a control safe to explore: you can
/// drag something to see what it does knowing the way back is one
/// gesture rather than a memory of the number.
///
/// A band goes FLAT rather than to some authored frequency — its
/// frequency is where you put it and its gain is the decision, so
/// undoing the decision is undoing the gain.
pub fn reset(tone: &mut Tone, grip: Grip) {
    match grip {
        Grip::Band(index) => {
            if let Some(band) = tone.eq.get_mut(index) {
                band.gain = 0.0;
            }
        }
        Grip::Threshold => tone.comp.threshold = Comp::default().threshold,
        // Unity: a preamp at drive one is the wire it is modelled on.
        Grip::Drive => tone.sat.drive = 1.0,
    }
}

/// Move a grip by a pixel delta.
///
/// Pixels rather than fractions because these axes are not linear in
/// the same way: a band's frequency is logarithmic and its gain is not,
/// so the conversion has to happen against the panel the drag is in.
pub fn drag(tone: &mut Tone, grip: Grip, panels: &[Which], panel: Panel, dx: f64, dy: f64) {
    let rack = Rack::at(panel.width);
    let Some((_, at)) = layout(panels, panel)
        .into_iter()
        .find(|(which, _)| match grip {
            Grip::Band(_) => *which == Which::Eq,
            Grip::Threshold => *which == Which::Comp,
            Grip::Drive => *which == Which::Sat,
        })
    else {
        return;
    };
    let body = body_of(at, rack);
    if body.width <= 0.0 || body.height <= 0.0 {
        return;
    }
    match grip {
        Grip::Band(index) => {
            let Some(band) = tone.eq.get_mut(index) else {
                return;
            };
            let freq = FreqAxis::audible();
            let db = DbAxis::symmetric(EQ_RANGE);
            let right = body.x + body.width;
            let bottom = body.y + body.height;
            let x = freq.freq_to_x(f64::from(band.frequency), body.x, right) + dx;
            let y = db.db_to_y(f64::from(band.gain), body.y, bottom) + dy;
            let t = ((x - body.x) / body.width).clamp(0.0, 1.0);
            band.frequency = f64_to_f32(freq.norm_to_freq(t));
            let gain = db.y_to_db(y.clamp(body.y, bottom), body.y, bottom);
            band.gain = f64_to_f32(gain.clamp(-EQ_RANGE, EQ_RANGE));
        }
        Grip::Threshold => {
            // Up is a HIGHER threshold, which is less compression —
            // the same direction a fader moves for more level, and the
            // opposite of following the dot down its curve.
            let per_db = body.height / 60.0;
            let moved = f64::from(tone.comp.threshold) - dy / per_db.max(f64::EPSILON);
            tone.comp.threshold = f64_to_f32(moved.clamp(-60.0, 0.0));
        }
        Grip::Drive => {
            // A quarter of the panel's height is the whole range, so a
            // short drag is a real change — drive is the parameter you
            // nudge, not the one you sweep.
            let per_unit = body.height / 4.0;
            let moved = f64::from(tone.sat.drive) - dy / per_unit.max(f64::EPSILON);
            tone.sat.drive = f64_to_f32(moved.clamp(0.0, 10.0));
        }
    }
}

/// The Tone settings for every track the window has opened.
///
/// Keyed by GUID, not by row: a preset can hide a track and a fold can
/// move one, and a rack that followed a row index would show the
/// neighbour's EQ the moment either happened.
///
/// This is where a real chain's parameters will land. Until then it is
/// seeded from [`placeholder`] — which means a rack is editable NOW,
/// against state the window owns, in exactly the shape the plugin's
/// parameters will take. The binding is the piece that is missing; the
/// UI above it is not waiting on anything.
#[derive(Clone, Debug, Default)]
pub struct Store {
    by_guid: std::collections::HashMap<String, Tone>,
}

impl Store {
    /// Make sure every track has settings, without disturbing the ones
    /// that already do.
    ///
    /// Called before a record rather than lazily inside one, so that
    /// recording can take `&self` — a `&mut` threaded through the
    /// drawing would put a lock between the mixer and its strips.
    pub fn seed(&mut self, rows: &[(daw_proto::Track, u32)]) {
        for (index, (track, _)) in rows.iter().enumerate() {
            self.by_guid
                .entry(track.guid.clone())
                .or_insert_with(|| placeholder(index));
        }
    }

    /// A track's settings, if it has any.
    #[must_use]
    pub fn get(&self, guid: &str) -> Option<&Tone> {
        self.by_guid.get(guid)
    }

    /// The same, to change.
    pub fn edit(&mut self, guid: &str) -> Option<&mut Tone> {
        self.by_guid.get_mut(guid)
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

    /// The tiers are ordered, and each threshold is where its tier
    /// starts — written against the constants, because pasted widths go
    /// stale the moment a threshold moves and then test nothing.
    #[test]
    fn the_rack_sheds_in_order() {
        assert!(SHAPE < LEGIBLE, "the tiers must not overlap");
        assert_eq!(Rack::at(LEGIBLE), Rack::Full);
        assert_eq!(Rack::at(LEGIBLE - 0.5), Rack::Curves);
        assert_eq!(Rack::at(SHAPE), Rack::Curves);
        assert_eq!(Rack::at(SHAPE - 0.5), Rack::Off);
        assert!(Rack::Full < Rack::Curves && Rack::Curves < Rack::Off);
        assert!(Rack::at(LEGIBLE).on() && !Rack::at(SHAPE - 0.5).on());
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
            &[Which::Eq, Which::Comp, Which::Sat],
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
            &[Which::Eq, Which::Comp, Which::Sat],
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

#[cfg(test)]
mod phase_tests {
    use super::{Which, panels_for};
    use session::mix_phases::MixPhase as P;

    /// The phase decides what the rack is showing, which is the whole
    /// reason the left rail's lower half is a rail and not a label.
    #[test]
    fn each_phase_asks_for_its_own_processing() {
        assert_eq!(panels_for(P::Tone), &[Which::Eq, Which::Comp, Which::Sat]);
        assert_eq!(panels_for(P::Rescue), &[Which::Eq]);
        assert_eq!(panels_for(P::Polish), &[Which::Comp, Which::Sat]);
    }

    /// Balance is the fader pass: no rack, so its height goes back to
    /// the strip. That is what "every track visible and detailed"
    /// means when the thing being compared is levels.
    #[test]
    fn balance_hands_the_height_back() {
        assert!(panels_for(P::Balance).is_empty());
        assert!(panels_for(P::Overview).is_empty());
    }

    /// A phase whose processing has no panel yet draws nothing rather
    /// than borrowing a curve that is not about it.
    #[test]
    fn an_unmodelled_phase_is_empty_not_wrong() {
        for phase in [P::Relational, P::Depth, P::Creative] {
            assert!(panels_for(phase).is_empty(), "{phase:?} borrowed a panel");
        }
    }

    /// Every phase answers — a `match` that grew a hole would be a
    /// phase button that silently kept the previous rack.
    #[test]
    fn every_phase_answers() {
        for phase in P::ALL {
            let panels = panels_for(phase);
            assert!(panels.len() <= 3, "{phase:?} asked for {panels:?}");
        }
    }
}

#[cfg(test)]
mod grip_tests {
    use super::{Grip, Panel, Store, Which, drag, grip_at, placeholder};

    const ALL: [Which; 3] = [Which::Eq, Which::Comp, Which::Sat];

    fn rack() -> Panel {
        Panel {
            x: 0.0,
            y: 0.0,
            width: 133.0,
            height: 600.0,
        }
    }

    /// A band is grabbed where it is DRAWN. The layout is shared by the
    /// two, so this asserts they have not drifted apart — which is the
    /// only way a grip can move the wrong band.
    #[test]
    fn a_band_is_grabbed_where_it_is_drawn() {
        let tone = placeholder(0);
        let (which, at) = super::layout(&ALL, rack())
            .into_iter()
            .find(|(which, _)| *which == Which::Eq)
            .expect("an EQ panel");
        assert_eq!(which, Which::Eq);
        let body = super::body_of(at, super::Rack::at(rack().width));
        let freq = fts_audio_ui::axis::FreqAxis::audible();
        let db = fts_audio_ui::axis::DbAxis::symmetric(super::EQ_RANGE);
        let band = &tone.eq[2];
        let x = freq.freq_to_x(f64::from(band.frequency), body.x, body.x + body.width);
        let y = db.db_to_y(f64::from(band.gain), body.y, body.y + body.height);
        assert_eq!(grip_at(&ALL, &tone, rack(), x, y), Some(Grip::Band(2)));
    }

    /// And a point well away from every band grabs none of them, rather
    /// than the nearest one at any distance.
    #[test]
    fn empty_space_in_the_eq_grabs_nothing() {
        let tone = placeholder(0);
        // The EQ panel's top-left corner: inside the panel, far from
        // any band, which all sit near the middle at these settings.
        assert_eq!(grip_at(&ALL, &tone, rack(), 3.0, 14.0), None);
    }

    /// Dragging a band up raises its gain and dragging it right raises
    /// its frequency — the two axes it is drawn against.
    #[test]
    fn a_band_follows_the_pointer() {
        let mut tone = placeholder(0);
        let (before_f, before_g) = (tone.eq[1].frequency, tone.eq[1].gain);
        drag(&mut tone, Grip::Band(1), &ALL, rack(), 12.0, -20.0);
        assert!(tone.eq[1].frequency > before_f, "right is higher");
        assert!(tone.eq[1].gain > before_g, "up is more gain");
    }

    /// A band cannot be dragged out of its own axes.
    #[test]
    fn a_band_stays_inside_the_panel() {
        let mut tone = placeholder(0);
        for _ in 0..50 {
            drag(&mut tone, Grip::Band(0), &ALL, rack(), 400.0, -400.0);
        }
        assert!(tone.eq[0].gain <= super::f64_to_f32(super::EQ_RANGE));
        assert!(tone.eq[0].frequency <= 24_000.0);
        for _ in 0..50 {
            drag(&mut tone, Grip::Band(0), &ALL, rack(), -400.0, 400.0);
        }
        assert!(tone.eq[0].gain >= -super::f64_to_f32(super::EQ_RANGE));
        assert!(tone.eq[0].frequency > 0.0);
    }

    /// Up is a higher threshold — less compression — which is the way
    /// a fader moves for more level.
    #[test]
    fn dragging_the_threshold_up_compresses_less() {
        let mut tone = placeholder(0);
        let before = tone.comp.threshold;
        drag(&mut tone, Grip::Threshold, &ALL, rack(), 0.0, -10.0);
        assert!(tone.comp.threshold > before);
        for _ in 0..200 {
            drag(&mut tone, Grip::Threshold, &ALL, rack(), 0.0, 40.0);
        }
        assert!(tone.comp.threshold >= -60.0, "the threshold clamps");
    }

    /// Drive clamps at zero rather than going negative, which would
    /// invert the curve.
    #[test]
    fn drive_stays_positive() {
        let mut tone = placeholder(0);
        for _ in 0..200 {
            drag(&mut tone, Grip::Drive, &ALL, rack(), 0.0, 40.0);
        }
        assert!(tone.sat.drive >= 0.0);
    }

    /// The store hands a track its own settings and keeps them apart —
    /// keyed by GUID, so a preset hiding a track cannot shuffle them.
    #[test]
    fn the_store_keeps_tracks_apart() {
        let rows: Vec<(daw_proto::Track, u32)> = ["a", "b"]
            .into_iter()
            .map(|guid| {
                (
                    daw_proto::Track {
                        guid: guid.to_owned(),
                        ..daw_proto::Track::default()
                    },
                    0,
                )
            })
            .collect();
        let mut store = Store::default();
        store.seed(&rows);
        store.edit("a").expect("a's settings").comp.threshold = -3.0;
        assert!((store.get("a").expect("a").comp.threshold + 3.0).abs() < f32::EPSILON);
        assert!(store.get("b").expect("b").comp.threshold < -3.0);
        assert!(store.get("nonesuch").is_none());
    }

    /// Seeding twice does not reset what was edited in between — the
    /// rows are re-derived on every fold and every preset.
    #[test]
    fn seeding_again_keeps_edits() {
        let rows: Vec<(daw_proto::Track, u32)> = vec![(
            daw_proto::Track {
                guid: "a".to_owned(),
                ..daw_proto::Track::default()
            },
            0,
        )];
        let mut store = Store::default();
        store.seed(&rows);
        store.edit("a").expect("a").sat.drive = 9.0;
        store.seed(&rows);
        assert!((store.get("a").expect("a").sat.drive - 9.0).abs() < f32::EPSILON);
    }
}

#[cfg(test)]
mod tier_tests {
    use super::{Grip, Panel, Which, grip_at, placeholder};

    const ALL: [Which; 3] = [Which::Eq, Which::Comp, Which::Sat];

    fn rack_of(width: f64) -> Panel {
        Panel {
            x: 0.0,
            y: 0.0,
            width,
            height: 600.0,
        }
    }

    /// A handle you cannot see is a handle you cannot aim at. Below the
    /// tier that draws them, the EQ panel grabs nothing — grabbing a
    /// band by accident moves a setting you did not know was there.
    #[test]
    fn bands_are_only_grabbable_where_they_are_drawn() {
        let tone = placeholder(0);
        let wide = rack_of(133.0);
        let narrow = rack_of(super::SHAPE + 1.0);
        assert_eq!(super::Rack::at(narrow.width), super::Rack::Curves);

        // The same band, in both tiers.
        let (_, at) = super::layout(&ALL, wide)
            .into_iter()
            .find(|(which, _)| *which == Which::Eq)
            .expect("an EQ panel");
        let body = super::body_of(at, super::Rack::Full);
        let freq = fts_audio_ui::axis::FreqAxis::audible();
        let db = fts_audio_ui::axis::DbAxis::symmetric(super::EQ_RANGE);
        let band = &tone.eq[2];
        let y = db.db_to_y(f64::from(band.gain), body.y, body.y + body.height);

        let x_wide = freq.freq_to_x(f64::from(band.frequency), body.x, body.x + body.width);
        assert_eq!(grip_at(&ALL, &tone, wide, x_wide, y), Some(Grip::Band(2)));

        let narrow_body = super::body_of(
            super::layout(&ALL, narrow)
                .into_iter()
                .find(|(which, _)| *which == Which::Eq)
                .expect("an EQ panel")
                .1,
            super::Rack::Curves,
        );
        let x_narrow = freq.freq_to_x(
            f64::from(band.frequency),
            narrow_body.x,
            narrow_body.x + narrow_body.width,
        );
        assert_eq!(grip_at(&ALL, &tone, narrow, x_narrow, y), None);
    }

    /// The compressor and the saturator stay grabbable when the rack
    /// narrows: their curves still show what the drag is doing.
    #[test]
    fn the_single_value_panels_survive_the_narrow_tier() {
        let tone = placeholder(0);
        let narrow = rack_of(super::SHAPE + 1.0);
        let comp = super::layout(&ALL, narrow)
            .into_iter()
            .find(|(which, _)| *which == Which::Comp)
            .expect("a comp panel")
            .1;
        let inside = comp.y + comp.height / 2.0;
        assert_eq!(
            grip_at(&ALL, &tone, narrow, narrow.width / 2.0, inside),
            Some(Grip::Threshold)
        );
    }

    /// A rack too narrow to draw at all grabs nothing.
    #[test]
    fn an_absent_rack_grabs_nothing() {
        let tone = placeholder(0);
        let off = rack_of(30.0);
        assert_eq!(super::Rack::at(off.width), super::Rack::Off);
        assert_eq!(grip_at(&ALL, &tone, off, 15.0, 300.0), None);
    }
}

#[cfg(test)]
mod reset_tests {
    use super::{Comp, Grip, Which, drag, placeholder, reset};

    const ALL: [Which; 3] = [Which::Eq, Which::Comp, Which::Sat];

    fn rack() -> super::Panel {
        super::Panel {
            x: 0.0,
            y: 0.0,
            width: 133.0,
            height: 600.0,
        }
    }

    /// Reset undoes a drag, whatever the drag was — which is the whole
    /// promise: you can move something to find out what it does.
    #[test]
    fn reset_undoes_a_drag() {
        let mut tone = placeholder(0);
        drag(&mut tone, Grip::Band(1), &ALL, rack(), 0.0, -40.0);
        assert!(tone.eq[1].gain.abs() > 0.5);
        reset(&mut tone, Grip::Band(1));
        assert!(tone.eq[1].gain.abs() < f32::EPSILON, "the band went flat");

        drag(&mut tone, Grip::Threshold, &ALL, rack(), 0.0, -30.0);
        reset(&mut tone, Grip::Threshold);
        assert!(
            (tone.comp.threshold - Comp::default().threshold).abs() < f32::EPSILON,
            "the threshold went back to its default"
        );

        drag(&mut tone, Grip::Drive, &ALL, rack(), 0.0, -60.0);
        reset(&mut tone, Grip::Drive);
        assert!((tone.sat.drive - 1.0).abs() < f32::EPSILON, "drive is unity");
    }

    /// A band reset keeps its FREQUENCY: where you put it is not the
    /// decision, how much you did there is.
    #[test]
    fn resetting_a_band_keeps_where_it_sits() {
        let mut tone = placeholder(0);
        drag(&mut tone, Grip::Band(2), &ALL, rack(), 20.0, -20.0);
        let moved = tone.eq[2].frequency;
        reset(&mut tone, Grip::Band(2));
        assert!((tone.eq[2].frequency - moved).abs() < f32::EPSILON);
    }

    /// A band that is not there is not a panic.
    #[test]
    fn resetting_a_missing_band_is_harmless() {
        let mut tone = placeholder(0);
        reset(&mut tone, Grip::Band(99));
    }
}
