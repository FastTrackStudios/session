//! The automation lanes on screen: where each is, what is drawn in it,
//! and what a point in it is over.
//!
//! One geometry ([`on_screen`]) for the drawing and the hit test, so a
//! point is grabbed where it is drawn. The lanes are drawn live every
//! frame rather than recorded with the items: a lane holds a handful of
//! points, and a drag reshapes it every frame.
//!
//! Each lane has two parts: its header in the track panel's column (its
//! name, the value playing under the cursor, its mode, a button that
//! hides it) and its body under the track's items (the curve as it plays,
//! the envelope's own points, its automation items as labelled windows
//! with their curves' points).

use anyrender::PaintScene;
use vello::kurbo::{Affine, BezPath, Circle, Line, Rect, Stroke};
use vello::peniko::{Color, Fill};

use daw_proto::automation::{EnvelopePoint, EnvelopeRef};

use crate::arrangement::{Arrangement, Palette, Viewport};
use crate::automation::{Kind, Lane};
use crate::text::Font;

/// The room above and below a lane's curve, in screen pixels, so a point
/// at the top or the bottom is still a whole square.
const PAD: f64 = 4.0;
/// Half a point's square.
const POINT: f64 = 3.0;
/// How near a point the pointer must be to take it.
const REACH: f64 = 6.0;
/// The hide button's width at the right of a header.
const HIDE_W: f64 = 16.0;
/// The strip along an automation item's top that carries its name and
/// moves it.
const ITEM_BAR: f64 = 11.0;
/// How far from a lane's bottom edge a press resizes it.
const EDGE: f64 = 3.0;

/// A lane where it is on screen.
#[derive(Clone, Debug)]
pub struct Placed {
    /// The arrangement row of its track.
    pub row: usize,
    pub lane: Lane,
    /// Its top and height on screen.
    pub top: f64,
    pub height: f64,
}

impl Placed {
    /// The y of `value` in this lane.
    #[must_use]
    pub fn y_of(&self, value: f64) -> f64 {
        let span = 2.0f64.mul_add(-PAD, self.height).max(1.0);
        (1.0 - self.lane.kind.y_of(value)).mul_add(span, self.top + PAD)
    }

    /// The value at screen `y` in this lane.
    #[must_use]
    pub fn value_at_y(&self, y: f64) -> f64 {
        let span = 2.0f64.mul_add(-PAD, self.height).max(1.0);
        self.lane.kind.value_of(1.0 - (y - self.top - PAD) / span)
    }
}

/// Every lane of the rows on screen, top to bottom. `below` is the screen
/// y of content y 0 (the ruler's foot less the scroll).
#[must_use]
pub fn on_screen(
    scene: &Arrangement,
    tracks: &[daw_proto::Track],
    view: Viewport,
    below: f64,
) -> Vec<Placed> {
    let zoom = if view.zoom_y > 0.0 { view.zoom_y } else { 1.0 };
    let mut out = Vec::new();
    for row in scene.visible_rows(view) {
        let Some(track) = tracks.get(row) else {
            continue;
        };
        let Some((top, _)) = scene.lanes_box(row) else {
            continue;
        };
        let mut y = top.mul_add(zoom, below);
        for lane in crate::automation::lanes_of(&track.guid) {
            let height = lane.height * zoom;
            out.push(Placed {
                row,
                lane,
                top: y,
                height,
            });
            y += height;
        }
    }
    out
}

/// What a point on screen is over in a lane.
#[derive(Clone, Debug, PartialEq)]
pub enum Spot {
    /// The header's hide button.
    Hide,
    /// The header's mode, which cycles.
    Mode,
    /// The rest of the header.
    Header,
    /// The lane's bottom edge, which resizes it.
    Edge,
    /// An automation item's name bar, which moves it.
    ItemBar { item: usize },
    /// The body: at `t` seconds and `value`, over the envelope's point
    /// `point` or, inside automation item `item`, over its curve's point
    /// `item_point`.
    Body {
        t: f64,
        value: f64,
        point: Option<usize>,
        item: Option<usize>,
        item_point: Option<usize>,
    },
}

/// A lane and what in it is under the pointer.
#[derive(Clone, Debug)]
pub struct Hit {
    pub placed: Placed,
    pub spot: Spot,
}

/// What is under `(x, y)`, if a lane is. `x` is from the window's left,
/// `y` from its top; `below` as for [`on_screen`].
#[must_use]
pub fn hit(
    scene: &Arrangement,
    tracks: &[daw_proto::Track],
    view: Viewport,
    below: f64,
    x: f64,
    y: f64,
) -> Option<Hit> {
    let placed = on_screen(scene, tracks, view, below)
        .into_iter()
        .find(|p| y >= p.top && y < p.top + p.height)?;
    let panel = view.panel_w;
    if x < panel {
        let spot = if x >= panel - HIDE_W {
            Spot::Hide
        } else if x >= panel - HIDE_W - 22.0 {
            Spot::Mode
        } else {
            Spot::Header
        };
        return Some(Hit { placed, spot });
    }
    if y >= placed.top + placed.height - EDGE {
        return Some(Hit {
            placed,
            spot: Spot::Edge,
        });
    }
    let x0 = panel - view.scroll_x;
    let t = (x - x0) / view.pps.max(1e-9);
    let x_of = |t: f64| t.mul_add(view.pps, x0);
    let lane = &placed.lane;
    if lane.kind == Kind::Leveler {
        // The leveled item under the pointer, and the ride's point near it.
        let item = lane.leveled.iter().position(|l| l.holds(t));
        let item_point = item.and_then(|i| lane.leveled.get(i)).and_then(|l| {
            l.lanes.ride.iter().position(|p| {
                (x_of(l.at(p.t_s)) - x).abs() <= REACH && (placed.y_of(p.db) - y).abs() <= REACH
            })
        });
        let value = placed.value_at_y(y);
        return Some(Hit {
            placed,
            spot: Spot::Body {
                t,
                value,
                point: None,
                item,
                item_point,
            },
        });
    }
    let item = lane.item_at(t);
    if let Some(i) = item
        && y < placed.top + ITEM_BAR
    {
        return Some(Hit {
            placed,
            spot: Spot::ItemBar { item: i },
        });
    }
    let near = |at: f64, value: f64| {
        (x_of(at) - x).abs() <= REACH && (placed.y_of(value) - y).abs() <= REACH
    };
    let (point, item_point) = match item.and_then(|i| lane.items.get(i)) {
        Some((ai, curve)) => {
            let found = curve
                .iter()
                .enumerate()
                .find(|(_, p)| near(item_time(ai, p), scaled(ai, p.value)))
                .map(|(k, _)| k);
            (None, found)
        }
        None => (
            lane.points
                .iter()
                .enumerate()
                .find(|(_, p)| near(p.time.as_seconds(), p.value))
                .map(|(k, _)| k),
            None,
        ),
    };
    let value = placed.value_at_y(y);
    Some(Hit {
        placed,
        spot: Spot::Body {
            t,
            value,
            point,
            item,
            item_point,
        },
    })
}

/// Where an automation item's source point lands on the timeline (its
/// first lap).
#[must_use]
pub fn item_time(item: &daw_proto::automation::AutomationItem, p: &EnvelopePoint) -> f64 {
    let rate = if item.play_rate > 0.0 {
        item.play_rate
    } else {
        1.0
    };
    item.position.as_seconds() + (p.time.as_seconds() - item.start_offset.as_seconds()) / rate
}

/// A source time for timeline `t` in an item (the inverse of
/// [`item_time`]).
#[must_use]
pub fn source_time(item: &daw_proto::automation::AutomationItem, t: f64) -> f64 {
    let rate = if item.play_rate > 0.0 {
        item.play_rate
    } else {
        1.0
    };
    (t - item.position.as_seconds()).mul_add(rate, item.start_offset.as_seconds())
}

/// A source value as an item plays it.
fn scaled(item: &daw_proto::automation::AutomationItem, v: f64) -> f64 {
    if (item.amplitude - 1.0).abs() < f64::EPSILON && (item.baseline - 0.5).abs() < f64::EPSILON {
        return v;
    }
    (v - 0.5).mul_add(item.amplitude, item.baseline)
}

/// A lane's colour, by what it automates.
#[must_use]
pub const fn color(palette: &Palette, kind: Kind) -> Color {
    match kind {
        Kind::Volume => palette.meter_safe,
        Kind::Pan => palette.pan,
        Kind::Mute => palette.mute,
        Kind::Width => palette.solo,
        Kind::Param => palette.accent,
        Kind::Leveler => palette.text,
    }
}

/// What is under the pointer, for the drawing to light up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lit {
    /// The lane (track, envelope) and the point in it, if one.
    pub lane: Option<(String, EnvelopeRef)>,
    pub point: Option<usize>,
    pub item: Option<usize>,
    pub item_point: Option<usize>,
    /// Where across, in seconds, the pointer is.
    pub t: Option<f64>,
}

/// The lanes' bodies: drawn after the items, before the panel.
pub fn bodies(
    painter: &mut impl PaintScene,
    palette: &Palette,
    font: &Font,
    placed: &[Placed],
    view: Viewport,
    lit: &Lit,
) {
    let x0 = view.panel_w - view.scroll_x;
    let x_of = |t: f64| t.mul_add(view.pps, x0);
    let left = view.panel_w;
    let right = view.width;
    for p in placed {
        if p.top > view.height || p.top + p.height < 0.0 {
            continue;
        }
        let lane = &p.lane;
        let ink = color(palette, lane.kind);
        let bottom = p.top + p.height;
        painter.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.row_b.multiply_alpha(0.92),
            None,
            &Rect::new(left, p.top, right, bottom),
        );
        painter.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.divider,
            None,
            &Rect::new(left, bottom - 1.0, right, bottom),
        );
        // The neutral value, faint: where the curve rests with nothing
        // drawn (0 dB, the centre).
        let rest_y = p.y_of(crate::automation::neutral(lane.kind));
        painter.stroke(
            &Stroke::new(1.0),
            Affine::IDENTITY,
            palette.grid.multiply_alpha(0.6),
            None,
            &Line::new((left, rest_y), (right, rest_y)),
        );
        let lit_here = lit
            .lane
            .as_ref()
            .is_some_and(|(track, target)| lane.is(track, target));
        if lane.kind == Kind::Leveler {
            leveler(painter, palette, p, view, lit_here.then_some(lit));
            continue;
        }
        windows(painter, palette, font, p, view, lit_here.then_some(lit));
        // The curve as it plays, every few pixels and at every point (a
        // jump is drawn as one).
        let path = curve(p, view, left, right);
        let mut under = path.clone();
        under.line_to((right, bottom - 1.0));
        under.line_to((left, bottom - 1.0));
        under.close_path();
        painter.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            ink.multiply_alpha(0.12),
            None,
            &under,
        );
        painter.stroke(
            &Stroke::new(if lit_here { 2.0 } else { 1.5 }),
            Affine::IDENTITY,
            ink,
            None,
            &path,
        );
        points(painter, palette, p, view, lit_here.then_some(lit));
        // The value under the pointer, beside it.
        if lit_here && let Some(t) = lit.t {
            let value = lane.value_at(t);
            let text = lane.say(value);
            let x = x_of(t) + 8.0;
            let y = p.y_of(value).clamp(p.top + 12.0, bottom - 3.0);
            crate::tcp::glyphs(painter, font, palette.text, &text, x, y, 10.0);
        }
    }
}

/// A lane's automation items: their windows and their names, under the
/// curve. `lit` when the pointer is in this lane.
fn windows(
    painter: &mut impl PaintScene,
    palette: &Palette,
    font: &Font,
    p: &Placed,
    view: Viewport,
    lit: Option<&Lit>,
) {
    let x0 = view.panel_w - view.scroll_x;
    let x_of = |t: f64| t.mul_add(view.pps, x0);
    let (left, right) = (view.panel_w, view.width);
    let lane = &p.lane;
    let ink = color(palette, lane.kind);
    let bottom = p.top + p.height;
    // Automation items: their windows, their names, under the curve.
    for (i, (item, _)) in lane.items.iter().enumerate() {
        let (a, b) = (
            x_of(item.position.as_seconds()),
            x_of(item.position.as_seconds() + item.length.as_seconds()),
        );
        if b < left || a > right {
            continue;
        }
        let (a, b) = (a.max(left), b.min(right));
        let lit_item = lit.is_some_and(|lit| lit.item == Some(i));
        painter.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            ink.multiply_alpha(if lit_item { 0.22 } else { 0.13 }),
            None,
            &Rect::new(a, p.top + 1.0, b, bottom - 1.0),
        );
        painter.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            ink.multiply_alpha(0.35),
            None,
            &Rect::new(a, p.top + 1.0, b, p.top + ITEM_BAR),
        );
        let label = if item.name.is_empty() {
            format!("Pool {}", item.pool_id)
        } else {
            item.name.clone()
        };
        let label = font.elide(&label, 9.0, (b - a - 6.0).max(0.0));
        crate::tcp::glyphs(
            painter,
            font,
            palette.text,
            &label,
            a + 3.0,
            p.top + 9.0,
            9.0,
        );
    }
}

/// A lane's points: the envelope's own (not under an item, where the item
/// plays), and the items' curves'. `lit` when the pointer is in this lane.
fn points(
    painter: &mut impl PaintScene,
    palette: &Palette,
    p: &Placed,
    view: Viewport,
    lit: Option<&Lit>,
) {
    let x0 = view.panel_w - view.scroll_x;
    let x_of = |t: f64| t.mul_add(view.pps, x0);
    let (left, right) = (view.panel_w, view.width);
    let lane = &p.lane;
    let ink = color(palette, lane.kind);
    // The envelope's own points (not under an item, where the item
    // plays), and the items' curves' points.
    for (k, point) in lane.points.iter().enumerate() {
        let t = point.time.as_seconds();
        if lane.item_at(t).is_some() {
            continue;
        }
        let x = x_of(t);
        if x < left - POINT || x > right + POINT {
            continue;
        }
        let lit_point = lit.is_some_and(|lit| lit.point == Some(k));
        square(painter, (x, p.y_of(point.value)), lit_point, ink, palette);
    }
    for (i, (item, curve)) in lane.items.iter().enumerate() {
        let end = item.position.as_seconds() + item.length.as_seconds();
        for (k, point) in curve.iter().enumerate() {
            let t = item_time(item, point);
            if t < item.position.as_seconds() || t > end {
                continue;
            }
            let x = x_of(t);
            if x < left - POINT || x > right + POINT {
                continue;
            }
            let lit_point = lit.is_some_and(|lit| lit.item == Some(i) && lit.item_point == Some(k));
            let r = if lit_point { POINT + 1.5 } else { POINT };
            let centre = (x, p.y_of(scaled(item, point.value)));
            painter.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                if lit_point { palette.text } else { ink },
                None,
                &Circle::new(centre, r),
            );
        }
    }
}

/// The vocal leveler's lane: under each leveled item, its gate, its
/// de-breath and its de-ess (each eased "ess" a bar along the foot), its
/// ride with its points, and over them their sum — what the item's volume
/// plays. `lit` when the pointer is in this lane.
fn leveler(painter: &mut impl PaintScene, palette: &Palette, p: &Placed, view: Viewport, lit: Option<&Lit>) {
    const STEP: f64 = 2.0;
    let x0 = view.panel_w - view.scroll_x;
    let pps = view.pps.max(1e-9);
    let (left, right) = (view.panel_w, view.width);
    let bottom = p.top + p.height;
    for (i, item) in p.lane.leveled.iter().enumerate() {
        let (a, b) = (
            item.position.mul_add(pps, x0),
            (item.position + item.length).mul_add(pps, x0),
        );
        if b < left || a > right {
            continue;
        }
        let (a, b) = (a.max(left), b.min(right));
        let lit_item = lit.is_some_and(|lit| lit.item == Some(i));
        painter.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.text.multiply_alpha(if lit_item { 0.07 } else { 0.035 }),
            None,
            &Rect::new(a, p.top + 1.0, b, bottom - 1.0),
        );
        // A curve of the item's, sampled across the stretch of it on
        // screen.
        let trace = |points: &[level_dsp::envelope::EnvPoint]| {
            let mut path = BezPath::new();
            let steps = num_traits::ToPrimitive::to_u32(&((b - a) / STEP).ceil()).unwrap_or(0);
            for k in 0..=steps {
                let x = f64::from(k).mul_add(STEP, a).min(b);
                let t = item.lane_time((x - x0) / pps);
                let at = (x, p.y_of(crate::automation::db_at(points, t)));
                if k == 0 {
                    path.move_to(at);
                } else {
                    path.line_to(at);
                }
            }
            path
        };
        let lanes = &item.lanes;
        for (points, ink, width) in [
            (&lanes.gate, palette.text_faint, 1.0),
            (&lanes.breath, palette.pan, 1.0),
            (&lanes.ride, palette.meter_safe, 1.4),
        ] {
            if !points.is_empty() {
                painter.stroke(&Stroke::new(width), Affine::IDENTITY, ink.multiply_alpha(0.8), None, &trace(points));
            }
        }
        for span in &lanes.sibilance {
            let (sa, sb) = (
                item.at(span.from_s).mul_add(pps, x0),
                item.at(span.to_s).mul_add(pps, x0),
            );
            if sb < a || sa > b {
                continue;
            }
            painter.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.solo.multiply_alpha(0.7),
                None,
                &Rect::new(sa.max(a), bottom - 5.0, sb.min(b).max(sa + 1.0), bottom - 2.0),
            );
        }
        painter.stroke(&Stroke::new(1.8), Affine::IDENTITY, palette.text, None, &trace(&item.composite));
        // The ride's points: what a hand moves.
        for (k, point) in lanes.ride.iter().enumerate() {
            let x = item.at(point.t_s).mul_add(pps, x0);
            if x < a - POINT || x > b + POINT {
                continue;
            }
            let lit_point = lit_item && lit.is_some_and(|lit| lit.item_point == Some(k));
            square(painter, (x, p.y_of(point.db)), lit_point, palette.meter_safe, palette);
        }
    }
}

/// A point's square.
fn square(
    painter: &mut impl PaintScene,
    (x, y): (f64, f64),
    lit: bool,
    ink: Color,
    palette: &Palette,
) {
    let r = if lit { POINT + 1.5 } else { POINT };
    let shape = Rect::new(x - r, y - r, x + r, y + r);
    painter.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        if lit { palette.text } else { ink },
        None,
        &shape,
    );
    painter.stroke(
        &Stroke::new(1.0),
        Affine::IDENTITY,
        palette.surface,
        None,
        &shape,
    );
}

/// A lane's curve across `left..right`: sampled every few pixels, with a
/// vertex at each point it plays through, so a step is a step.
fn curve(p: &Placed, view: Viewport, left: f64, right: f64) -> BezPath {
    const STEP: f64 = 3.0;
    let x0 = view.panel_w - view.scroll_x;
    let pps = view.pps.max(1e-9);
    let t_of = |x: f64| (x - x0) / pps;
    let lane = &p.lane;
    let (from, to) = (t_of(left), t_of(right));
    let steps = num_traits::ToPrimitive::to_u32(&((right - left) / STEP).ceil()).unwrap_or(0);
    let mut times: Vec<f64> = (0..=steps)
        .map(|k| t_of(f64::from(k).mul_add(STEP, left).min(right)))
        .collect();
    times.push(to);
    for point in &lane.played {
        let t = point.time.as_seconds();
        if t > from && t < to {
            // Just before and at: a jump draws straight up.
            times.push(t - 1e-6);
            times.push(t);
        }
    }
    times.sort_by(f64::total_cmp);
    let mut path = BezPath::new();
    for (i, t) in times.iter().enumerate() {
        let at = (t.mul_add(pps, x0), p.y_of(lane.value_at(*t)));
        if i == 0 {
            path.move_to(at);
        } else {
            path.line_to(at);
        }
    }
    path
}

/// The lanes' headers, in the track panel's column: drawn after the panel.
pub fn headers(
    painter: &mut impl PaintScene,
    palette: &Palette,
    font: &Font,
    placed: &[Placed],
    view: Viewport,
    lit: &Lit,
    at: f64,
) {
    let width = view.panel_w;
    for p in placed {
        if p.top > view.height || p.top + p.height < 0.0 {
            continue;
        }
        let lane = &p.lane;
        let ink = color(palette, lane.kind);
        let bottom = p.top + p.height;
        painter.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.tcp_gutter,
            None,
            &Rect::new(0.0, p.top, width, bottom),
        );
        painter.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            palette.tcp_rule,
            None,
            &Rect::new(0.0, bottom - 1.0, width, bottom),
        );
        // The rule in the lane's colour, down its left, after the track's
        // indent: whose lane it is, at a glance.
        painter.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            ink,
            None,
            &Rect::new(14.0, p.top + 3.0, 17.0, bottom - 4.0),
        );
        let lit_here = lit
            .lane
            .as_ref()
            .is_some_and(|(track, target)| lane.is(track, target));
        let room = (width - 24.0 - HIDE_W - 26.0).max(0.0);
        let name = font.elide(&lane.name, 10.5, room);
        let name_ink = if lit_here {
            palette.text
        } else {
            palette.text_dim
        };
        crate::tcp::glyphs(painter, font, name_ink, &name, 24.0, p.top + 14.0, 10.5);
        if p.height >= 30.0 {
            let value = lane.say(lane.value_at(lit.t.filter(|_| lit_here).unwrap_or(at)));
            crate::tcp::glyphs(
                painter,
                font,
                palette.text_faint,
                &value,
                24.0,
                p.top + 28.0,
                10.0,
            );
        }
        // Its mode, which a click cycles (the leveler's lanes have none:
        // they are the items' volume), and the hide button.
        let mode = if lane.kind == Kind::Leveler {
            ""
        } else {
            mode_letters(lane.mode)
        };
        let mx = width - HIDE_W - 22.0;
        if !mode.is_empty() {
            painter.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                palette.tcp_field,
                None,
                &Rect::new(mx, p.top + 4.0, mx + 20.0, p.top + 17.0),
            );
        }
        let w = font.width(mode, 9.0);
        crate::tcp::glyphs(
            painter,
            font,
            palette.text_dim,
            mode,
            mx + (20.0 - w) / 2.0,
            p.top + 14.0,
            9.0,
        );
        let hx = width - HIDE_W + 4.0;
        let cross = |a: (f64, f64), b: (f64, f64)| Line::new(a, b);
        for line in [
            cross((hx, p.top + 6.0), (hx + 7.0, p.top + 13.0)),
            cross((hx + 7.0, p.top + 6.0), (hx, p.top + 13.0)),
        ] {
            painter.stroke(
                &Stroke::new(1.2),
                Affine::IDENTITY,
                palette.text_faint,
                None,
                &line,
            );
        }
    }
}

/// A mode as two letters for the header.
#[must_use]
pub const fn mode_letters(mode: daw_proto::primitives::AutomationMode) -> &'static str {
    use daw_proto::primitives::AutomationMode as M;
    match mode {
        M::Off => "off",
        M::TrimRead => "Trm",
        M::Read => "Rd",
        M::Touch => "Tch",
        M::Write => "Wr",
        M::Latch => "Lt",
        M::LatchPreview => "LP",
    }
}

/// The mode a click on a lane's mode moves to: read, off, trim/read —
/// what is chosen while editing by hand (the writing modes are a fader's).
#[must_use]
pub const fn next_mode(
    mode: daw_proto::primitives::AutomationMode,
) -> daw_proto::primitives::AutomationMode {
    use daw_proto::primitives::AutomationMode as M;
    match mode {
        M::TrimRead => M::Read,
        M::Read => M::Off,
        _ => M::TrimRead,
    }
}
