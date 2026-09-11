//! Controls as SHAPES, so a third surface can draw them.
//!
//! The crate's premise is that artwork is authored once and consumed
//! twice — live SVG on the web, rasterised PNG for REAPER. Both of those
//! consume *markup*, which was fine while both were SVG consumers.
//!
//! A GPU canvas is not. `anyrender` wants fills and strokes of shapes,
//! and it cannot be handed an `<svg>` string. Drawing the controls a
//! second time for it is exactly the drift this crate exists to prevent:
//! the numbers here were measured off REAPER pixel by pixel — a knob's
//! 11 rim, its 3.5 ring and its 7.5 body — and a canvas that re-invented
//! them would be a second, worse set of the same controls.
//!
//! So the geometry moves one layer down, into data:
//!
//! ```text
//!              paint::volume_knob(value, at) -> Drawing
//!                          │
//!            ┌─────────────┼──────────────┐
//!            ▼             ▼              ▼
//!        to SVG        to anyrender    rasterised
//!      (web, REAPER)   (GPU canvas)     via SVG
//! ```
//!
//! A [`Drawing`] is pure data: no Dioxus, no kurbo, no colour library
//! beyond the theme's own. Coordinates are `f64` — SVG writes them into
//! strings and takes any precision, while a GPU canvas is `f64`
//! throughout, so `f64` is the one choice that makes neither consumer
//! convert on the way in. That is deliberate — the crate is consumed by
//! the PNG rasteriser, which should not grow a GPU stack, and by the
//! canvas, which should not grow a DOM.
//!
//! # Coordinates
//!
//! Every drawing is in its own box, origin at the top-left, sized by
//! [`Drawing::w`] / [`Drawing::h`]. Placing it on a row is the caller's
//! transform. Angles are **degrees clockwise from twelve o'clock**,
//! which is the convention the knobs were measured in.

use daw_theme::Color;

/// How a shape is painted.
#[derive(Clone, PartialEq, Debug)]
pub enum Brush {
    Solid(Color),
    /// Axis-aligned linear gradient, in the drawing's own coordinates.
    Linear {
        from: (f64, f64),
        to: (f64, f64),
        /// Offsets are `f32` because every gradient API downstream —
        /// SVG's `offset`, peniko's `ColorStop` — takes one.
        stops: Vec<(f32, Color)>,
    },
    /// Radial gradient centred in the drawing's own coordinates.
    Radial {
        centre: (f64, f64),
        radius: f64,
        /// Offsets are `f32` because every gradient API downstream —
        /// SVG's `offset`, peniko's `ColorStop` — takes one.
        stops: Vec<(f32, Color)>,
    },
}

/// A shape, in the drawing's own coordinates.
///
/// Deliberately a small closed set rather than a path language. Every
/// control in this crate is built from these five, and a shape nobody
/// draws is a shape nobody has to port to a new backend.
#[derive(Clone, PartialEq, Debug)]
pub enum Shape {
    /// `r` is the corner radius; zero for a square corner.
    Rect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        r: f64,
    },
    Ellipse {
        cx: f64,
        cy: f64,
        rx: f64,
        ry: f64,
    },
    /// An arc of a circle, for the knobs' rings.
    Arc {
        cx: f64,
        cy: f64,
        r: f64,
        /// Degrees clockwise from twelve o'clock.
        start: f64,
        sweep: f64,
    },
    Line {
        from: (f64, f64),
        to: (f64, f64),
    },
    /// A closed polygon — carets, triangles, the folder arrow.
    Poly(Vec<(f64, f64)>),
}

/// How a stroke is drawn.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Stroke {
    pub width: f64,
    /// Round caps on an arc read as a soft ring; butt caps as a gauge.
    /// REAPER's knob uses butt, which is why it is the default.
    pub round_cap: bool,
}

impl Stroke {
    #[must_use]
    pub const fn new(width: f64) -> Self {
        Self {
            width,
            round_cap: false,
        }
    }

    #[must_use]
    pub const fn round(width: f64) -> Self {
        Self {
            width,
            round_cap: true,
        }
    }
}

/// Where a text run sits horizontally within its box.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    Left,
    Centre,
}

/// One drawing operation.
#[derive(Clone, PartialEq, Debug)]
pub enum Op {
    Fill(Shape, Brush),
    Stroke(Shape, Brush, Stroke),
    /// A label. The backend owns the font — this says what to draw and
    /// where the baseline is, not which face to use, because the DOM and
    /// the canvas resolve fonts completely differently.
    Text {
        body: String,
        x: f64,
        baseline: f64,
        /// In points, `f32` because that is what every text API takes.
        size: f32,
        color: Color,
        align: Align,
    },
}

/// A control, drawn.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Drawing {
    pub w: f64,
    pub h: f64,
    pub ops: Vec<Op>,
}

impl Drawing {
    #[must_use]
    pub const fn new(w: f64, h: f64) -> Self {
        Self {
            w,
            h,
            ops: Vec::new(),
        }
    }

    pub fn fill(&mut self, shape: Shape, brush: impl Into<Brush>) -> &mut Self {
        self.ops.push(Op::Fill(shape, brush.into()));
        self
    }

    pub fn stroke(&mut self, shape: Shape, brush: impl Into<Brush>, stroke: Stroke) -> &mut Self {
        self.ops.push(Op::Stroke(shape, brush.into(), stroke));
        self
    }

    pub fn text(
        &mut self,
        body: impl Into<String>,
        x: f64,
        baseline: f64,
        size: f32,
        color: Color,
        align: Align,
    ) -> &mut Self {
        self.ops.push(Op::Text {
            body: body.into(),
            x,
            baseline,
            size,
            color,
            align,
        });
        self
    }
}

impl From<Color> for Brush {
    fn from(color: Color) -> Self {
        Self::Solid(color)
    }
}

/// Parse one of the theme's hex constants.
///
/// The measured colours are stated as hex in `daw_theme::defaults`
/// because that is how they were sampled out of REAPER's images. A
/// malformed one is a typo in a constant, not a runtime condition, so it
/// falls back to black rather than returning a `Result` nobody could act
/// on.
#[must_use]
pub fn hex(text: &str) -> Color {
    let digits = text.trim_start_matches('#');
    let byte = |i: usize| {
        digits
            .get(i..i.saturating_add(2))
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            .unwrap_or(0)
    };
    Color {
        r: byte(0),
        g: byte(2),
        b: byte(4),
        a: 255,
    }
}

/// The track panel's controls, as drawings.
///
/// Every number in here came out of the corresponding component in
/// [`crate::vector_controls`], which got it by measuring REAPER. Nothing
/// is re-derived and nothing is rounded on the way through: a knob's rim
/// is 11 here because it is 11 there, and if REAPER is re-measured both
/// change together.
pub mod tcp {
    use super::{hex, Align, Brush, Drawing, Shape, Stroke};
    use crate::vector_controls::{ink_in, Interaction, VOLUME_KNOB_START, VOLUME_KNOB_SWEEP};
    use daw_theme::{Chrome, Color};

    /// The volume knob: a rim, a track, a value arc and a moulded cap.
    ///
    /// `value` is 0..1 across the knob's 320° sweep.
    ///
    /// The detail matters more than it looks. REAPER's knob is not a
    /// circle with a pointer — it is ringed in near-black, casts a soft
    /// drop below itself, and its ring runs from the lit token at the
    /// bottom to a darker one at the top because this theme's hardware
    /// is lit from below. Drawn flat it reads as a printed circle.
    #[must_use]
    pub fn volume_knob(chrome: &Chrome, value: f64, at: Interaction) -> Drawing {
        // 24, the height of the field it straddles. A knob shorter than
        // its box reads as sunk into it rather than seated on its edge.
        let (w, h) = (24.0_f64, 24.0_f64);
        let (cx, cy) = (w * 0.5, h * 0.5);
        // Measured across the knob's centre row and down its centre
        // column: an 11 rim, a 3.5 ring on a radius of 9, a 7.5 body.
        let (rim, r, stroke, body) = (11.0_f64, 9.0_f64, 3.5_f64, 7.5_f64);

        let value = value.clamp(0.0, 1.0);
        let ink = ink_in(chrome, None, at, true, 0.35);
        let mut drawing = Drawing::new(w, h);

        // The drop it casts, which is most of what makes it read as
        // hardware rather than as a filled circle.
        drawing.fill(
            Shape::Ellipse {
                cx,
                cy: h.mul_add(0.055, cy),
                rx: rim * 1.07,
                ry: rim * 1.23,
            },
            Brush::Radial {
                centre: (cx, h.mul_add(0.055, cy)),
                radius: rim * 1.23,
                stops: vec![
                    (0.90_f32, Color { r: 0, g: 0, b: 0, a: 38 }),
                    (1.0, Color { r: 0, g: 0, b: 0, a: 0 }),
                ],
            },
        );
        // The black outline the ring is inset into. Without it the ring
        // runs to the cell's edge and the knob reads a size larger than
        // the one beside it.
        drawing.fill(circle(cx, cy, rim), hex("#0d0d0d"));
        // The unlit track, all the way round.
        drawing.stroke(
            Shape::Arc {
                cx,
                cy,
                r,
                start: f64::from(VOLUME_KNOB_START),
                sweep: f64::from(VOLUME_KNOB_SWEEP),
            },
            hex(daw_theme::defaults::VOLUME_RING_UNLIT),
            Stroke::new(stroke),
        );
        // The body: #303030 at the top to #2d2d2d at the bottom. Three
        // units, which is the difference between a moulded cap and a
        // filled circle.
        drawing.fill(
            circle(cx, cy, body),
            Brush::Linear {
                from: (cx, cy - body),
                to: (cx, cy + body),
                stops: vec![(0.0, hex("#303030")), (1.0, hex("#2d2d2d"))],
            },
        );
        drawing.stroke(circle(cx, cy, body), ink.border, Stroke::new(0.8));
        // The value, over the track. Last, so its end sits on top of the
        // unlit stroke rather than under it.
        if value > 0.0 {
            drawing.stroke(
                Shape::Arc {
                    cx,
                    cy,
                    r,
                    start: f64::from(VOLUME_KNOB_START),
                    sweep: f64::from(VOLUME_KNOB_SWEEP) * value,
                },
                Brush::Linear {
                    from: (cx, cy + r),
                    to: (cx, cy - r),
                    stops: vec![
                        (0.0, hex(daw_theme::defaults::VOLUME_RING_LIT)),
                        (1.0, hex(daw_theme::defaults::VOLUME_RING_LIT_TOP)),
                    ],
                },
                Stroke::new(stroke),
            );
        }
        drawing
    }

    /// The pan knob: a rim, a face, a pointer and a cap.
    ///
    /// `position` is -1..1, and the pointer sweeps 135° either side of
    /// twelve o'clock — REAPER's range, not a full rotation.
    #[must_use]
    pub fn pan_knob(chrome: &Chrome, position: f64) -> Drawing {
                let (w, h) = (24.0_f64, 25.0_f64);
        let (cx, cy, r) = (12.0_f64, 12.08_f64, 9.37_f64);
        let (cap_cy, cap_r) = (12.05_f64, 4.06_f64);

        let pos = position.clamp(-1.0, 1.0);
        let sweep = pos * 135.0;
        let point_w = w * 0.083;
        let point_top = h.mul_add(0.045, cy - r);
        let point_bot = h.mul_add(-0.01, cy - cap_r);

        let point = chrome.hardware_mark.shade(0.11);
        let rim = chrome.hardware_edge.shade(-0.45);

        let mut drawing = Drawing::new(w, h);
        // Rim first, face inset — a stroke would centre itself on the
        // boundary and eat half of each.
        drawing.fill(circle(cx, cy, r), rim);
        drawing.fill(circle(cx, cy, r - 0.35), chrome.hardware);
        // The pointer, as a rotated quad rather than a rotation applied
        // to the whole drawing: the canvas replays these under a
        // transform that may scale the axes differently, and a rotation
        // baked into the shape survives that where a rotated coordinate
        // system would shear.
        drawing.fill(
            Shape::Poly(
                [
                    (-point_w / 2.0, point_top - cy),
                    (point_w / 2.0, point_top - cy),
                    (point_w / 2.0, point_bot - cy),
                    (-point_w / 2.0, point_bot - cy),
                ]
                .into_iter()
                .map(|(px, py)| rotate(px, py, sweep, cx, cy))
                .collect(),
            ),
            point,
        );
        drawing.fill(circle(cx, cap_cy, cap_r), chrome.hardware_mark);
        drawing
    }

    /// A gutter button — mute or solo — lit in its own colour.
    #[must_use]
    pub fn gutter_button(
        chrome: &Chrome,
        label: &str,
        on: bool,
        lit: Color,
        at: Interaction,
    ) -> Drawing {
        let (w, h) = (21.0_f64, 20.0_f64);
        let ink = ink_in(chrome, on.then_some(lit), at, false, 0.25);
        let mut drawing = Drawing::new(w, h);
        drawing.fill(rect(0.0, 0.0, w, h, 3.0), ink.face);
        drawing.stroke(rect(0.5, 0.5, w - 1.0, h - 1.0, 3.0), ink.border, Stroke::new(1.0));
        drawing.text(label, w / 2.0, h / 2.0 + 3.5, 10.0, ink.text, Align::Centre);
        drawing
    }

    /// What a track's routing button has to say.
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub struct Routing {
        /// The track feeds its parent.
        pub parent_send: bool,
        /// It has sends to other tracks.
        pub sends: bool,
        /// Other tracks send to it.
        pub receives: bool,
    }

    /// The three lit routing colours.
    ///
    /// Supplied by the caller rather than taken from the art's default
    /// theme, so a REAPER theme's own accent and meter colours reach the
    /// button. The art's choices are `accent`, `meter_warn` and
    /// `meter_danger`, in that order.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub struct RouteInk {
        /// The output lane. Blue in the source art.
        pub out: Color,
        /// Sends. Amber.
        pub send: Color,
        /// Receives. Red — the source uses a brighter red for a lit lane
        /// than the record ring's.
        pub recv: Color,
    }

    /// The routing widget: three lanes in a row, on a scrim.
    ///
    /// Traced off the source cells rather than composed from fractions:
    /// the track panel's button is three 4x10 bars across a 28x22 cell
    /// at x = 5, 12 and 19, on a plate that is 28x20 inset from the top.
    /// Guessing those as "56% of the width, centred" put every lane a
    /// pixel and a bit left of the art.
    ///
    /// The top lane is the track's OUTPUT and is lit in every source
    /// cell — a track always has one — so it greys only when the parent
    /// send is cut. Colouring it by whether anything is routed made an
    /// unrouted track look broken rather than merely unrouted.
    #[must_use]
    pub fn routing(chrome: &Chrome, state: Routing, ink: RouteInk, at: Interaction) -> Drawing {
        // The source cell, which the caller scales by placing it.
        let (vw, vh) = (28.0, 22.0);
        let plate = ink_in(chrome, None, at, true, 0.35);

        // Traced: 28x20 in a 28x22 cell, inset by half the stroke, which
        // straddles the edge it is drawn on. Filling the cell made the
        // button visibly chunkier than the art beside it.
        let edge = vh * 0.03;
        let (box_y, box_h) = (vh / 22.0, vh * 20.0 / 22.0);

        let mut drawing = Drawing::new(vw, vh);
        // Black at 35%, not an opaque grey: a scrim that lets the track
        // colour through, which is why the button looks near-black on a
        // dark row and tinted on a coloured one.
        drawing.fill(
            rect(
                edge / 2.0,
                box_y + edge / 2.0,
                vw - edge,
                box_h - edge,
                vw.min(vh) * 0.16,
            ),
            Color { r: 0, g: 0, b: 0, a: 89 },
        );
        drawing.stroke(
            rect(
                edge / 2.0,
                box_y + edge / 2.0,
                vw - edge,
                box_h - edge,
                vw.min(vh) * 0.16,
            ),
            plate.border,
            Stroke::new(edge),
        );
        // The lip along the top.
        drawing.fill(
            rect(vw * 0.08, box_y + edge, vw.mul_add(-0.16, vw), vh * 0.04, 0.0),
            Color { r: 255, g: 255, b: 255, a: 18 },
        );

        // An unlit lane is plain grey at half alpha over the strip. It
        // was a blue-grey once, which made a track with nothing routed
        // look faintly lit.
        let dim = alpha(chrome.hardware_mark.shade(-0.29), 0.49);
        let lanes = [
            // Disabled greys the OUTPUT lane and nothing else: compared
            // cell for cell, the `_dis` variant differs in exactly one
            // place.
            if state.parent_send { ink.out } else { dim },
            if state.sends { ink.send } else { dim },
            if state.receives { ink.recv } else { dim },
        ];
        let (bar_w, bar_h) = (vw * 4.0 / 28.0, vh * 10.0 / 22.0);
        let cross = (vh - bar_h) / 2.0;
        // The traced lane positions, stated rather than stepped from an
        // index: they came off the source cells as 5, 12 and 19, and a
        // pitch computed from a loop counter is a number nobody measured.
        for (at_x, lit) in [5.0, 12.0, 19.0].into_iter().zip(lanes) {
            let x = vw * at_x / 28.0;
            drawing.fill(rect(x, cross, bar_w, bar_h, bar_w.min(bar_h) / 2.0), lit);
        }
        drawing
    }

    /// `c` at `a` of its opacity.
    fn alpha(color: Color, amount: f64) -> Color {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::as_conversions,
            reason = "an 0..1 opacity scaled into a u8 channel"
        )]
        let scaled = (f64::from(color.a) * amount.clamp(0.0, 1.0)).round() as u8;
        Color { a: scaled, ..color }
    }

    /// What a track's FX chain is doing.
    #[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
    pub enum Chain {
        /// Nothing in it.
        #[default]
        Empty,
        /// Loaded and running.
        Active,
        /// Loaded and bypassed.
        Bypassed,
    }

    /// The FX control: a bypass lamp and the labelled button, one pill.
    ///
    /// REAPER blits this as two images side by side — `track_fx_norm` at
    /// 20 wide and `track_fxempty_h` at 16 — which is why the pill is 36
    /// across with a split in it. Drawn as one element rather than two
    /// positioned halves, because composing them put the toggle's
    /// rounded end through the letters.
    ///
    /// The plate is black at 35%, not an opaque grey: on the track panel
    /// it is a scrim that lets the row's colour through.
    #[must_use]
    pub fn fx_pill(chrome: &Chrome, chain: Chain, at: Interaction) -> Drawing {
        // Traced: 36x22, body rows 1..21 of 22, split at 20 — the
        // labelled half first and the bypass toggle after it, which is
        // the order REAPER blits `track_fx_norm` (20 wide) and
        // `track_fxon_h` (16) in.
        let (w, h) = (36.0, 22.0);
        let split = 20.0;
        let plate = ink_in(chrome, None, at, true, 0.35);
        let (body_y, body_h) = (h / 22.0, h * 20.0 / 22.0);
        let radius = h * 0.12;

        let mut drawing = Drawing::new(w, h);
        drawing.fill(
            rect(0.0, body_y, w, body_h, radius),
            Color { r: 0, g: 0, b: 0, a: 89 },
        );
        drawing.stroke(
            rect(0.5, body_y + 0.5, w - 1.0, body_h - 1.0, radius),
            plate.border,
            Stroke::new(1.0),
        );
        // The seam between the halves.
        drawing.fill(
            rect(split, body_y + 2.0, 1.0, body_h - 4.0, 0.0),
            plate.border,
        );

        // The lamp: a vertical capsule, measured straight off
        // `track_fxon_h` and `track_fxoff_h` in the installed ReaperTips
        // theme — 4 wide by 10 tall in a 16x22 cell, fully rounded, and
        // #60c2fe lit against #ff6975 bypassed. It is a capsule rather
        // than a square because that is what the art is; drawn as a dot
        // it reads as an LED on a different control.
        let lamp = match chain {
            Chain::Active => hex("#60c2fe"),
            Chain::Bypassed => hex("#ff6975"),
            // Empty shows a dark slug: there is nothing to bypass, and
            // the slot still has to read as a slot.
            Chain::Empty => chrome.hardware_mark.shade(-0.45),
        };
        drawing.fill(
            rect(
                split + (w - split) / 2.0 - 2.0,
                (h - 10.0) / 2.0,
                4.0,
                10.0,
                2.0,
            ),
            lamp,
        );

        // Neutral letters, printed on the scrim rather than lit: the
        // source is #c1 at 0.61 empty and #eb at 0.87 active, and the
        // chrome ramp's blues read as an indicator instead of as print.
        let (ink, ink_alpha) = match chain {
            Chain::Empty => (chrome.hardware_mark.shade(0.33), 0.61),
            Chain::Active => (chrome.hardware_mark.shade(0.78), 0.87),
            Chain::Bypassed => (hex(daw_theme::defaults::MUTE), 1.0),
        };
        drawing.text(
            "FX",
            split / 2.0,
            h / 2.0 + 3.5,
            10.5,
            alpha(ink, ink_alpha),
            Align::Centre,
        );
        drawing
    }

    /// The envelope button: the ramp it stands for.
    #[must_use]
    pub fn envelope(chrome: &Chrome, at: Interaction) -> Drawing {
        let (w, h) = (22.0_f64, 20.0_f64);
        let ink = ink_in(chrome, None, at, false, 0.25);
        let mut drawing = Drawing::new(w, h);
        drawing.fill(rect(0.0, 0.0, w, h, 2.0), ink.face);
        drawing.stroke(rect(0.5, 0.5, w - 1.0, h - 1.0, 2.0), ink.border, Stroke::new(1.0));
        // A rising ramp with a handle at each end, which is what an
        // automation lane looks like at this size.
        drawing.stroke(
            Shape::Line {
                from: (4.0, h - 5.0),
                to: (w - 4.0, 5.0),
            },
            ink.text,
            Stroke::round(1.4),
        );
        drawing.fill(circle(4.0, h - 5.0, 1.8), ink.text);
        drawing.fill(circle(w - 4.0, 5.0, 1.8), ink.text);
        drawing
    }

    /// Phase: the slashed circle, lit when inverted.
    #[must_use]
    pub fn phase(chrome: &Chrome, inverted: bool, at: Interaction) -> Drawing {
                let (w, h) = (16.0_f64, 16.0_f64);
        let ink = ink_in(chrome, inverted.then_some(chrome.accent), at, false, 0.25);
        let (cx, cy) = (w / 2.0, h / 2.0);
        let mut drawing = Drawing::new(w, h);
        drawing.fill(circle(cx, cy, 8.0), ink.face);
        drawing.stroke(circle(cx, cy, 5.5), ink.text, Stroke::new(1.3));
        drawing.stroke(
            Shape::Line {
                from: (cx - 5.5, cy + 5.5),
                to: (cx + 5.5, cy - 5.5),
            },
            ink.text,
            Stroke::round(1.3),
        );
        drawing
    }

    /// The record arm: a ring when idle, a lit disc when armed.
    #[must_use]
    pub fn record_arm(chrome: &Chrome, armed: bool, at: Interaction) -> Drawing {
        let (w, h) = (18.0_f64, 18.0_f64);
        let ink = ink_in(chrome, armed.then_some(hex(daw_theme::defaults::REC)), at, true, 0.25);
        let (cx, cy) = (w / 2.0, h / 2.0);
        let mut drawing = Drawing::new(w, h);
        drawing.fill(circle(cx, cy, 9.0), ink.border);
        drawing.fill(circle(cx, cy, 7.5), ink.face);
        if armed {
            drawing.fill(circle(cx, cy, 3.2), hex(daw_theme::defaults::REC));
        }
        drawing
    }

    /// Turn `(x, y)` about `(cx, cy)` by `deg` clockwise.
    fn rotate(x: f64, y: f64, deg: f64, cx: f64, cy: f64) -> (f64, f64) {
        let angle = deg.to_radians();
        let (sin, cos) = (angle.sin(), angle.cos());
        (
            y.mul_add(-sin, x.mul_add(cos, cx)),
            y.mul_add(cos, x.mul_add(sin, cy)),
        )
    }

    const fn circle(cx: f64, cy: f64, r: f64) -> Shape {
        Shape::Ellipse { cx, cy, rx: r, ry: r }
    }

    const fn rect(x: f64, y: f64, width: f64, height: f64, radius: f64) -> Shape {
        Shape::Rect {
            x,
            y,
            w: width,
            h: height,
            r: radius,
        }
    }
}
