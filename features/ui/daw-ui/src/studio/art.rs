//! The theme's vector art, as SVG.
//!
//! `daw_theme_art` describes every control as a [`Drawing`]: a width, a
//! height, and a list of fills, strokes and labels over shapes that are
//! rectangles, ellipses, arcs, lines and polygons. The painted window
//! turns those into Vello commands. This turns the same ones into SVG,
//! so the controls render on Blitz, in a WebView and in a browser from
//! the same source — which is what makes them IDENTICAL rather than
//! similar. Nothing here invents a shape.
//!
//! # Why not a custom widget
//!
//! Because a widget exists on exactly one of the four renderers this has
//! to run on. Blitz's `Widget` trait can take a wgpu device and draw the
//! panel itself, which would be faster than any of this — and it would
//! not run under dioxus-desktop at all, where WRY renders through
//! WebKit and there is no device to take. The whole point of the
//! migration is one component tree on every target, so the art has to
//! arrive as something every target can draw. SVG is that thing.
//!
//! The art was built for this. Its own notes say gradient offsets are
//! `f32` "because every gradient API downstream — SVG's `offset`,
//! peniko's `ColorStop` — takes one", and that a label carries its
//! baseline rather than a font "because the DOM and the canvas resolve
//! fonts completely differently". It was waiting for a DOM backend.
//!
//! # One sheet, not one element per control
//!
//! Every control on screen goes into ONE `<svg>`, the way every waveform
//! on a lane goes into one. A panel is forty rows of six controls, and
//! an element each is five hundred nodes that say nothing the sheet
//! cannot say once — which is the lever this whole effort has been
//! pulling.
//!
//! Labels come back separately rather than as `<text>`. A label in the
//! sheet would be lettered by usvg's own font stack on Blitz and by the
//! page's on the web, which is two different pictures; returned, the
//! caller writes them as ordinary elements in the same face as every
//! other word in the window.

use std::fmt::Write as _;

use daw_theme::Color;
use daw_theme_art::paint::{Align, Brush, Drawing, Op, Shape, Stroke};

/// The palette the art prints in, off the window's theme.
///
/// Started from the art's own defaults and overridden rather than built
/// field by field: `Chrome` carries relationships that were measured
/// together — a hardware face, its edge and its mark — and a palette
/// assembled from scratch would silently lose the ones this theme has
/// nothing to say about.
#[must_use]
pub fn chrome(theme: &crate::theming::Theme) -> daw_theme::Chrome {
    let c = |col: crate::theming::Color| daw_theme::Color {
        r: col.r,
        g: col.g,
        b: col.b,
        a: col.a,
    };
    daw_theme::Chrome {
        surface: c(theme.tokens.surface),
        surface_raised: c(theme.tokens.surface_raised),
        surface_sunken: c(theme.tokens.surface_sunken),
        border: c(theme.tokens.border),
        text: c(theme.tokens.text),
        text_dim: c(theme.tokens.text_dim),
        text_faint: c(theme.tokens.text_faint),
        accent: c(theme.tokens.accent),
        // A control's face, its edge and the ink on it. The buttons read
        // as part of the panel when these come off the same ladder the
        // panel does, and as borrowed art when they do not.
        hardware: c(theme.tokens.surface_raised),
        hardware_edge: c(theme.tokens.border),
        // The art prints on this and rings with it — the record arm's
        // unlit ring is a control FACE, not a label, and mapped to
        // `text_dim` alone it came out dark enough to vanish against its
        // own housing.
        hardware_mark: c(theme.tokens.text_dim).mix(c(theme.tokens.text), 0.5),
        ..daw_theme::Theme::default().chrome
    }
}

/// The colours the ported controls light up in.
///
/// One place, so a control cannot pick a different blue from the one
/// beside it — and so that changing the theme changes the controls,
/// which was the whole reason the art stopped carrying its own hex.
#[must_use]
pub fn lit(theme: &crate::theming::Theme) -> daw_theme_art::paint::tcp::Lit {
    let c = |col: crate::theming::Color| daw_theme::Color {
        r: col.r,
        g: col.g,
        b: col.b,
        a: col.a,
    };
    daw_theme_art::paint::tcp::Lit {
        volume: c(theme.tokens.accent),
        pan: c(theme.tokens.route_send),
        rec: c(theme.tokens.rec),
        bypass: c(theme.tokens.mute),
    }
}

/// The colours the routing lanes light in.
///
/// The source art's own choices: the output lane is the accent, sends
/// the warn amber, receives the danger red — so "where does this go" is
/// answered by three colours that mean the same thing everywhere else
/// in the window.
#[must_use]
pub fn route_ink(theme: &crate::theming::Theme) -> daw_theme_art::paint::tcp::RouteInk {
    let c = |col: crate::theming::Color| daw_theme::Color {
        r: col.r,
        g: col.g,
        b: col.b,
        a: col.a,
    };
    daw_theme_art::paint::tcp::RouteInk {
        out: c(theme.tokens.accent),
        send: c(theme.tokens.meter_warn),
        recv: c(theme.tokens.meter_danger),
    }
}

/// A label the caller has to write, because the sheet will not.
#[derive(Clone, PartialEq, Debug)]
pub struct Label {
    pub body: String,
    /// Where it sits, already placed and scaled.
    pub x: f64,
    pub baseline: f64,
    pub size: f64,
    pub color: String,
    /// Whether `x` is the label's centre rather than its left edge.
    pub centred: bool,
}

/// Every control on screen, as one drawing.
#[derive(Default)]
pub struct Sheet {
    body: String,
    defs: String,
    labels: Vec<Label>,
    /// Gradients need ids, and an id has to be unique within the
    /// document rather than within the control it came from.
    gradients: usize,
}

impl Sheet {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Put a drawing at `(x, y)`, scaled.
    ///
    /// The drawing's own coordinates are its authored ones — a knob is
    /// eighteen across whatever row it lands on — so placing it is a
    /// translate and a scale rather than a re-derivation of every
    /// number in it.
    pub fn place(&mut self, drawing: &Drawing, x: f64, y: f64, scale: f64) {
        let _ = write!(
            self.body,
            "<g transform=\"translate({x:.2} {y:.2}) scale({scale:.4})\">"
        );
        for op in &drawing.ops {
            match op {
                Op::Fill(shape, brush) => {
                    let paint = self.paint(brush);
                    let _ = write!(self.body, "{}", element(shape, &paint, None));
                }
                Op::Stroke(shape, brush, stroke) => {
                    let paint = self.paint(brush);
                    let _ = write!(self.body, "{}", element(shape, &paint, Some(*stroke)));
                }
                Op::Text {
                    body,
                    x: at,
                    baseline,
                    size,
                    color,
                    align,
                } => self.labels.push(Label {
                    body: body.clone(),
                    x: at.mul_add(scale, x),
                    baseline: baseline.mul_add(scale, y),
                    size: f64::from(*size) * scale,
                    color: css(*color),
                    centred: *align == Align::Centre,
                }),
            }
        }
        self.body.push_str("</g>");
    }

    /// The sheet's markup, ready to go inside an `<svg>`.
    #[must_use]
    pub fn markup(&self) -> String {
        if self.defs.is_empty() {
            self.body.clone()
        } else {
            format!("<defs>{}</defs>{}", self.defs, self.body)
        }
    }

    /// The sheet as a whole SVG document, in a `data:` URI.
    ///
    /// Handed to an `<img>` rather than written as an inline `<svg>`,
    /// for a reason that cost an afternoon: Blitz renders an inline
    /// `<svg>` by walking its DOM subtree back into markup, so the
    /// shapes have to BE nodes. Setting them as raw inner HTML leaves
    /// the subtree empty and the sheet draws nothing at all — which
    /// looks exactly like a sheet that was never built, because
    /// everything visible in it was a label rendered separately.
    ///
    /// Making them real nodes would work and would cost a node per
    /// shape, which is the one thing this whole effort has been
    /// avoiding. A data URI is one node whatever the sheet holds, and an
    /// `<img>` with an SVG source is the most portable thing in this
    /// entire migration — every target has drawn one for twenty years.
    #[must_use]
    pub fn data_uri(&self, width: f64, height: f64) -> String {
        let document = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\"              height=\"{height:.0}\" viewBox=\"0 0 {width:.0} {height:.0}\">{}</svg>",
            self.markup()
        );
        format!("data:image/svg+xml,{}", escape(&document))
    }

    /// The labels the caller has to write itself.
    #[must_use]
    pub fn labels(&self) -> &[Label] {
        &self.labels
    }

    /// Whether anything was placed at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.body.is_empty()
    }

    /// A brush as something a `fill` or `stroke` attribute takes,
    /// declaring a gradient in `defs` if it needs one.
    fn paint(&mut self, brush: &Brush) -> String {
        match brush {
            Brush::Solid(color) => css(*color),
            Brush::Linear { from, to, stops } => {
                let id = self.next_id();
                let _ = write!(
                    self.defs,
                    "<linearGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" \
                     x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\">{}</linearGradient>",
                    from.0,
                    from.1,
                    to.0,
                    to.1,
                    stops_of(stops)
                );
                format!("url(#{id})")
            }
            Brush::Radial {
                centre,
                radius,
                stops,
            } => {
                let id = self.next_id();
                let _ = write!(
                    self.defs,
                    "<radialGradient id=\"{id}\" gradientUnits=\"userSpaceOnUse\" \
                     cx=\"{:.2}\" cy=\"{:.2}\" r=\"{:.2}\">{}</radialGradient>",
                    centre.0,
                    centre.1,
                    radius,
                    stops_of(stops)
                );
                format!("url(#{id})")
            }
        }
    }

    fn next_id(&mut self) -> String {
        self.gradients = self.gradients.saturating_add(1);
        format!("g{}", self.gradients)
    }
}

/// Percent-encode the few characters a `data:` URI cannot carry.
///
/// Not a general encoder: the sheet's own alphabet is known — tags,
/// attributes, numbers and `rgba(...)` colours — so this is the short
/// list that actually appears, and `#` is on it because a URI would take
/// everything after one as a fragment.
fn escape(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len() + svg.len() / 8);
    for c in svg.chars() {
        match c {
            '%' => out.push_str("%25"),
            '#' => out.push_str("%23"),
            '<' => out.push_str("%3C"),
            '>' => out.push_str("%3E"),
            '"' => out.push_str("%22"),
            '\n' | '\r' => out.push_str("%20"),
            _ => out.push(c),
        }
    }
    out
}

fn stops_of(stops: &[(f32, Color)]) -> String {
    let mut out = String::new();
    for (offset, color) in stops {
        let _ = write!(
            out,
            "<stop offset=\"{offset:.4}\" stop-color=\"{}\"/>",
            css(*color)
        );
    }
    out
}

/// A colour as CSS.
///
/// `rgba()` with the alpha as a fraction, because that is what both an
/// SVG attribute and a style attribute take — and the art carries alpha
/// on plenty of its shades.
#[must_use]
pub fn css(color: Color) -> String {
    format!(
        "rgba({}, {}, {}, {:.4})",
        color.r,
        color.g,
        color.b,
        f64::from(color.a) / 255.0
    )
}

/// One shape as an SVG element.
fn element(shape: &Shape, paint: &str, stroke: Option<Stroke>) -> String {
    let style = stroke.map_or_else(
        || format!("fill=\"{paint}\""),
        |stroke| {
            format!(
                "fill=\"none\" stroke=\"{paint}\" stroke-width=\"{:.3}\" \
                 stroke-linecap=\"{}\"",
                stroke.width,
                if stroke.round_cap { "round" } else { "butt" }
            )
        },
    );
    match shape {
        Shape::Rect { x, y, w, h, r } => format!(
            "<rect x=\"{x:.2}\" y=\"{y:.2}\" width=\"{w:.2}\" height=\"{h:.2}\" \
             rx=\"{r:.2}\" {style}/>"
        ),
        Shape::Ellipse { cx, cy, rx, ry } => format!(
            "<ellipse cx=\"{cx:.2}\" cy=\"{cy:.2}\" rx=\"{rx:.2}\" ry=\"{ry:.2}\" {style}/>"
        ),
        Shape::Arc {
            cx,
            cy,
            r,
            start,
            sweep,
        } => format!(
            "<path d=\"{}\" {style}/>",
            arc(*cx, *cy, *r, *start, *sweep)
        ),
        Shape::Line { from, to } => format!(
            "<line x1=\"{:.2}\" y1=\"{:.2}\" x2=\"{:.2}\" y2=\"{:.2}\" {style}/>",
            from.0, from.1, to.0, to.1
        ),
        Shape::Poly(points) => {
            let mut out = String::with_capacity(points.len() * 14);
            for (x, y) in points {
                let _ = write!(
                    out,
                    "{}{x:.2},{y:.2}",
                    if out.is_empty() { "" } else { " " }
                );
            }
            format!("<polygon points=\"{out}\" {style}/>")
        }
    }
}

/// An arc as a path.
///
/// The art measures angles in degrees CLOCKWISE FROM TWELVE O'CLOCK,
/// which is how a knob is read and is not how either trigonometry or
/// SVG measures them. So this is the one place in the conversion that is
/// arithmetic rather than transcription, and the one place with tests.
#[must_use]
pub fn arc(cx: f64, cy: f64, r: f64, start: f64, sweep: f64) -> String {
    // A full turn cannot be drawn as one arc — the two ends land on the
    // same point and SVG draws nothing at all — so it is two halves.
    if sweep.abs() >= 360.0 {
        let (a, b) = (point(cx, cy, r, start), point(cx, cy, r, start + 180.0));
        return format!(
            "M{:.3} {:.3}A{r:.3} {r:.3} 0 0 1 {:.3} {:.3}A{r:.3} {r:.3} 0 0 1 {:.3} {:.3}Z",
            a.0, a.1, b.0, b.1, a.0, a.1
        );
    }
    let from = point(cx, cy, r, start);
    let to = point(cx, cy, r, start + sweep);
    // An arc longer than half a turn is the "large" one of the two the
    // endpoints allow; the direction flag is which way round it goes.
    let large = u8::from(sweep.abs() > 180.0);
    let clockwise = u8::from(sweep > 0.0);
    format!(
        "M{:.3} {:.3}A{r:.3} {r:.3} 0 {large} {clockwise} {:.3} {:.3}",
        from.0, from.1, to.0, to.1
    )
}

/// Where an angle lands on a circle, in the art's own reckoning.
fn point(cx: f64, cy: f64, r: f64, degrees: f64) -> (f64, f64) {
    let radians = degrees.to_radians();
    (r.mul_add(radians.sin(), cx), r.mul_add(-radians.cos(), cy))
}

#[cfg(test)]
mod tests {
    use super::{Sheet, arc, css, point};
    use daw_theme::Color;
    use daw_theme_art::paint::{Drawing, Shape};

    /// Twelve o'clock is the top, and the clock runs the way a clock
    /// runs. Every knob in the panel depends on this and nothing else
    /// in the conversion can be wrong in a way that looks plausible.
    #[test]
    fn the_clock_starts_at_the_top_and_runs_clockwise() {
        let near = |a: (f64, f64), b: (f64, f64)| {
            assert!(
                (a.0 - b.0).abs() < 1e-9 && (a.1 - b.1).abs() < 1e-9,
                "{a:?} is not {b:?}"
            );
        };
        near(point(0.0, 0.0, 10.0, 0.0), (0.0, -10.0));
        near(point(0.0, 0.0, 10.0, 90.0), (10.0, 0.0));
        near(point(0.0, 0.0, 10.0, 180.0), (0.0, 10.0));
        near(point(0.0, 0.0, 10.0, 270.0), (-10.0, 0.0));
    }

    /// The flags say which of the four arcs between two points is meant.
    #[test]
    fn an_arc_says_which_way_round_and_how_far() {
        // A quarter turn clockwise: the short way, forwards.
        let quarter = arc(0.0, 0.0, 10.0, 0.0, 90.0);
        assert!(quarter.contains("A10.000 10.000 0 0 1"), "{quarter}");
        // Three quarters clockwise: the long way, forwards.
        let most = arc(0.0, 0.0, 10.0, 0.0, 270.0);
        assert!(most.contains("A10.000 10.000 0 1 1"), "{most}");
        // A quarter anticlockwise: the short way, backwards.
        let back = arc(0.0, 0.0, 10.0, 0.0, -90.0);
        assert!(back.contains("A10.000 10.000 0 0 0"), "{back}");
    }

    /// A full turn is two halves, because one arc whose ends meet draws
    /// nothing at all.
    #[test]
    fn a_full_ring_is_drawn_as_two_halves() {
        let ring = arc(5.0, 5.0, 4.0, 0.0, 360.0);
        assert_eq!(ring.matches('A').count(), 2, "{ring}");
        assert!(ring.ends_with('Z'), "{ring}");
    }

    /// Alpha survives the trip, because the art uses it.
    #[test]
    fn a_colour_keeps_its_alpha() {
        assert_eq!(css(Color::rgb(200, 40, 40)), "rgba(200, 40, 40, 1.0000)");
        let half = css(Color::rgba(0, 0, 0, 128));
        assert!(half.starts_with("rgba(0, 0, 0, 0.50"), "{half}");
    }

    /// A drawing is placed by translate and scale, not by re-deriving
    /// every number in it.
    #[test]
    fn a_drawing_is_placed_where_it_was_put() {
        let mut drawing = Drawing::new(10.0, 10.0);
        drawing.fill(
            Shape::Rect {
                x: 1.0,
                y: 2.0,
                w: 3.0,
                h: 4.0,
                r: 0.0,
            },
            Color::rgb(255, 0, 0),
        );
        let mut sheet = Sheet::new();
        sheet.place(&drawing, 100.0, 50.0, 0.5);
        let markup = sheet.markup();
        assert!(
            markup.contains("translate(100.00 50.00) scale(0.5000)"),
            "{markup}"
        );
        // The shape keeps its AUTHORED coordinates; the group moves it.
        assert!(markup.contains("x=\"1.00\" y=\"2.00\""), "{markup}");
        assert!(
            markup.contains("fill=\"rgba(255, 0, 0, 1.0000)\""),
            "{markup}"
        );
    }

    /// Labels come back rather than going in, so the caller letters them
    /// in the same face as the rest of the window.
    #[test]
    fn labels_are_handed_back_not_drawn() {
        let mut drawing = Drawing::new(20.0, 10.0);
        drawing.text(
            "FX",
            4.0,
            8.0,
            7.0,
            Color::rgb(0, 0, 0),
            daw_theme_art::paint::Align::Left,
        );
        let mut sheet = Sheet::new();
        sheet.place(&drawing, 10.0, 20.0, 2.0);
        assert!(
            !sheet.markup().contains("<text"),
            "a label went into the sheet: {}",
            sheet.markup()
        );
        let label = sheet.labels().first().expect("a label");
        assert_eq!(label.body, "FX");
        // Placed and scaled with everything else.
        assert!((label.x - 18.0).abs() < 1e-9, "{}", label.x);
        assert!((label.baseline - 36.0).abs() < 1e-9, "{}", label.baseline);
        assert!((label.size - 14.0).abs() < 1e-9, "{}", label.size);
    }

    /// A data URI carries the whole sheet and escapes what it must.
    #[test]
    fn a_data_uri_is_a_whole_document() {
        let mut drawing = Drawing::new(4.0, 4.0);
        drawing.fill(
            Shape::Rect {
                x: 0.0,
                y: 0.0,
                w: 4.0,
                h: 4.0,
                r: 0.0,
            },
            Color::rgb(1, 2, 3),
        );
        let mut sheet = Sheet::new();
        sheet.place(&drawing, 0.0, 0.0, 1.0);
        let uri = sheet.data_uri(100.0, 50.0);
        assert!(uri.starts_with("data:image/svg+xml,"), "{uri}");
        // The characters a URI cannot carry are gone.
        assert!(
            !uri.contains('<') && !uri.contains('>') && !uri.contains('"'),
            "{uri}"
        );
        // And the document is complete: a namespace, a viewBox, a shape.
        assert!(uri.contains("xmlns"), "{uri}");
        assert!(uri.contains("viewBox"), "{uri}");
        assert!(uri.contains("rect"), "{uri}");
    }

    /// A gradient becomes a def with an id, and the id is unique across
    /// the sheet rather than across the control it came from.
    #[test]
    fn gradients_get_their_own_ids() {
        use daw_theme_art::paint::Brush;
        let mut drawing = Drawing::new(10.0, 10.0);
        let brush = || Brush::Linear {
            from: (0.0, 0.0),
            to: (0.0, 10.0),
            stops: vec![(0.0, Color::rgb(0, 0, 0)), (1.0, Color::rgb(255, 255, 255))],
        };
        drawing.fill(
            Shape::Rect {
                x: 0.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
                r: 0.0,
            },
            brush(),
        );
        let mut sheet = Sheet::new();
        sheet.place(&drawing, 0.0, 0.0, 1.0);
        sheet.place(&drawing, 20.0, 0.0, 1.0);
        let markup = sheet.markup();
        assert!(
            markup.contains("id=\"g1\"") && markup.contains("id=\"g2\""),
            "{markup}"
        );
        assert!(
            markup.contains("url(#g1)") && markup.contains("url(#g2)"),
            "{markup}"
        );
    }
}
