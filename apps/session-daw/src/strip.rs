//! Where everything on a strip is — computed once.
//!
//! Every layout bug in this panel has been the same shape: a value that
//! has to match, worked out twice from different inputs. The button line
//! recomputed per strip instead of read. The strip's height taken from
//! the mixer instead of the strip. The bands resolved once for the
//! recorded chrome and again for the live controls, against different
//! heights.
//!
//! A layout engine removes that class by construction — you declare
//! once and it solves once — and the measurement said the DOM costs
//! four times the frame budget at these strip counts. So the panels
//! stay recorded, and this is the property taken from the idea without
//! the renderer: **one function computes the geometry, and drawing,
//! hit-testing and the live overlay all READ it.**
//!
//! The rule that follows is worth stating because it is what was being
//! broken: a control's position is not recalculated at the point of
//! use. If two places need it, they read the same [`Strip`].

use daw_theme_art::geometry::mcp as g;
use daw_ui::controls::{Collapse, VolumeWidget};
use vello::kurbo::Rect;

use daw_theme_art::paint::tcp as art;

use crate::mcp::{Columns, Control, Squeeze};

/// How far the pan knob sits in from the strip's left edge.
///
/// The coloured band holds two things and REAPER puts one at each end:
/// the pan knob left, the record arm right. A small inset rather than
/// flush, because the band's own edge is a colour boundary and a
/// control touching it reads as bleeding out of the strip.
const PAN_FROM_EDGE: f64 = 5.0;

/// How much further apart the button column's steps are than REAPER's
/// — added at each step, so the monitor comes down by one of these and
/// the routing by four.
const SPREAD: f64 = 4.0;

/// How far the monitor's cell reaches down into the arm's: the ring
/// starts five pixels into its cell, so the lamp sits close over it
/// without touching.
const MONITOR_TUCK: f64 = 3.0;

/// The side of the panel's bare record-arm ring — what a rail draws in
/// place of the housed arm (`art::Arm::Panel` is 20x20).
const ARM_RING: f64 = 20.0;

/// The ring's clearance: under the band, and to each edge of the
/// narrowest rail allowed to carry it.
const ARM_RING_GAP: f64 = 1.0;

/// The air between the bottom of the fader column and the name plate.
///
/// Small, but not nothing: a meter whose last pixel touches the plate
/// reads as part of it, and the floor of a meter is a value you look at.
const NAME_GAP: f64 = 3.0;

/// How a strip arranges itself: REAPER's stacked strip, or the focused
/// column layout.
///
/// A focused strip is wide, and stacking a rack over a strip that wide
/// wastes the one thing a focused track needs — height. The column
/// layout is REAPER's "layout C" idea: the strip's own controls in a
/// column at the left, every one of them on the same line as its
/// neighbour's — the fader included, which keeps its height — and the
/// rack as a second column beside it that spans the whole height. The
/// column above the coloured band is left empty: it is room for what
/// a focused track will want next. What a docked mixer wants too.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// The rack over the strip.
    Stacked,
    /// The strip beside the rack.
    Column,
}

/// The width of the strip's own column in the column layout: REAPER's
/// own strip width (`geometry::mcp::STRIP_W`), so the controls are
/// exactly the ones you know. Stated as an `f64` because the geometry
/// table is `f32` and a width is added to `f64` positions here.
pub const COLUMN_W: f64 = 86.0;

const _: () = assert!(g::STRIP_W == 86.0);

/// The strip's inset from the mixer's top and its rack's edges.
const EDGE: f64 = 2.0;

/// One strip's geometry, in the strip's own coordinates.
///
/// X is from the strip's left edge, Y from the mixer's top — the space
/// the recorded scene is in, so a rect here can be drawn without
/// translation and hit-tested by subtracting the strip's left edge.
#[derive(Clone, Copy, Debug)]
pub struct Strip {
    pub width: f64,
    /// This strip's own height. Shorter than the mixer's by one indent
    /// step per level of nesting.
    pub height: f64,
    pub squeeze: Squeeze,
    /// The sections resolved against the MIXER's height — everything
    /// anchored to the top, which is the same on every strip because
    /// nesting shortens from the bottom.
    shared: Collapse,
    /// Resolved against this strip's own height. Only the fader, which
    /// is what the indent shortens.
    own: Collapse,
    pub columns: Columns,
    pub rack_h: f64,
    pub buttons_top: f64,
    layout: Layout,
}

impl Strip {
    /// Resolve a strip.
    #[must_use]
    pub fn new(width: f64, height: f64, mixer_h: f64, rack_h: f64, buttons_top: f64) -> Self {
        Self::laid_out(width, height, mixer_h, rack_h, buttons_top, false)
    }

    /// Resolve a strip, saying whether its track WANTS the column
    /// layout — a channel with a chain taller than the rack's share
    /// does; a return with a short one does not.
    ///
    /// It gets the column only when it is also wide enough for one.
    /// Either way, past twice REAPER's strip width the controls stop
    /// centring and stay in the left 86 pixels: a fader drifting to
    /// the middle of a wide strip is a fader you have to go and find,
    /// and the box to the right of it is room for something else.
    #[must_use]
    pub fn laid_out(
        width: f64,
        height: f64,
        mixer_h: f64,
        rack_h: f64,
        buttons_top: f64,
        column: bool,
    ) -> Self {
        let layout = if column && rack_h > 0.0 && width >= crate::tone::FOCUSED + COLUMN_W {
            Layout::Column
        } else {
            Layout::Stacked
        };
        let own_w = if layout == Layout::Column || width >= COLUMN_W * 2.0 {
            COLUMN_W
        } else {
            width
        };
        Self {
            width,
            height,
            squeeze: Squeeze::at(own_w),
            shared: Collapse::at(crate::mcp::f64_to_f32((mixer_h - rack_h).max(1.0))),
            own: Collapse::at(crate::mcp::f64_to_f32((height - rack_h).max(1.0))),
            columns: Columns::at(0.0, own_w),
            rack_h,
            buttons_top,
            layout,
        }
    }

    /// Which layout this strip is in.
    #[must_use]
    pub const fn layout(&self) -> Layout {
        self.layout
    }

    /// How wide the strip's own chrome is — the band, the plate, the
    /// sections. The whole strip when stacked; the left column beside a
    /// rack.
    #[must_use]
    pub fn chrome_width(&self) -> f64 {
        match self.layout() {
            Layout::Stacked => self.width,
            Layout::Column => COLUMN_W,
        }
    }

    /// Where the fader column starts.
    ///
    /// Under the coloured band, in either layout — a focused strip's
    /// fader is the same height as its neighbours', so a level reads
    /// across the mixer whatever is focused. Except on a strip too
    /// narrow to put the buttons beside the fader: there the buttons
    /// sit over it, so the fader starts under the last of them rather
    /// than running up behind the mute and the solo.
    fn fader_top(&self) -> f64 {
        // Only the rail: a strip with a button column has the buttons
        // beside the fader, and its fader runs the full section like
        // its neighbours'. A rail has no column — the buttons are
        // centred, over the fader — so the fader starts under the last
        // of them.
        if self.squeeze.columns() {
            return self.band_bottom();
        }
        let under = self.arm_top() + self.column_step(Control::Solo) + f64::from(g::BUTTON_H);
        under.max(self.band_bottom()) + NAME_GAP
    }

    /// The top of the coloured band.
    #[must_use]
    pub fn band_top(&self) -> f64 {
        f64::from(daw_theme_art::collapse::FX_SECTION) + self.rack_h
    }

    /// Its bottom — what the record arm hangs from.
    #[must_use]
    pub fn band_bottom(&self) -> f64 {
        // One height on every strip, rail included: the band is the
        // line the mixer is read across, and a rail whose band was a
        // rule put its arm and its mute on a line of their own.
        self.band_top() + f64::from(self.shared.pan_band) + f64::from(self.shared.input_band)
    }

    /// How much travel the fader has, as the layout allots it.
    ///
    /// Not what the fader is DRAWN at — see [`Strip::travel`]. This is
    /// the section's own height, which the collapse layout hands out
    /// before it knows what else the strip is carrying.
    #[must_use]
    pub fn stretch(&self) -> f64 {
        f64::from(self.own.stretch)
    }

    /// Where the record arm's box starts.
    ///
    /// The anchor the whole column hangs from, because that is what
    /// `rtconfig` chains it to. It was hung off `buttons_top` — the
    /// line the FADER starts on, four pixels under the coloured band —
    /// which is about eighteen pixels lower than the arm, so every
    /// button sat that much below where REAPER draws it.
    fn arm_top(&self) -> f64 {
        if !self.squeeze.columns() {
            // A rail has no housing to sink into the band: its arm is
            // the panel's bare ring, and it sits UNDER the band, first
            // in the stack the monitor, mute and solo hang from.
            return self.band_bottom() + ARM_RING_GAP;
        }
        self.band_bottom() + f64::from(g::ARM_OVERHANG) - f64::from(g::ARM_CELL_H)
    }

    /// Whether the arm is drawn in its housing (the measured strip) or
    /// as the panel's bare ring (a rail). The layout decides, so the
    /// drawing and the hit test agree on which glyph is there.
    #[must_use]
    pub fn arm(&self) -> art::Arm {
        if self.squeeze.columns() {
            art::Arm::Mixer
        } else {
            art::Arm::Panel
        }
    }

    /// How far down the button column a control sits, from the arm.
    ///
    /// REAPER states this as a chain of offsets rather than a pitch,
    /// and the steps are deliberately unequal:
    ///
    /// ```text
    /// recmon  = recarm + 20
    /// mute    = recmon + 19
    /// solo    = mute   + 21
    /// io      = solo   + 23
    /// ```
    ///
    /// Our monitor is over the arm rather than under it (see
    /// [`Control::Monitor`] in [`Strip::rect`]), so the mute takes the
    /// monitor's step and the rest follow: the column under the arm
    /// is mute, solo, routing, with nothing in it that is sometimes
    /// not there.
    ///
    /// Plus [`SPREAD`] at every step: REAPER's chain packs the column
    /// into the top of a strip whose fader wants the height, and ours
    /// has the room — the column is beside the fader, not over it —
    /// so the buttons sit a little lower and a little further apart.
    fn column_step(&self, control: Control) -> f64 {
        // Everywhere but the head tier: a wide strip has the column
        // beside the fader, and a rail has the strip's taller share
        // (see `mcp::CONTROL_SHARE`) to spend on it. The head tier
        // keeps its pan in the band and its buttons over the fader,
        // so it keeps REAPER's pitch.
        let spread = if self.squeeze.columns() || !self.squeeze.head() {
            SPREAD
        } else {
            0.0
        };
        let mute = f64::from(g::RECMON_FROM_ARM) + spread;
        let solo = mute + f64::from(g::SOLO_FROM_MUTE) + spread;
        match control {
            Control::Mute => mute,
            Control::Solo => solo,
            _ => solo + f64::from(g::IO_FROM_SOLO) + spread,
        }
    }

    /// And how much of it the fader column may actually use.
    ///
    /// Clamped above the name plate. The allotted stretch runs past it
    /// on some strips, and a meter drawn into the name is a meter
    /// crossing out the one thing that says which track you are
    /// looking at — a track you cannot name is a track you cannot act
    /// on. Everything in the column reads this: the groove, the meter,
    /// the cap's travel and the dB scale beside them, so they cannot
    /// end at different heights.
    #[must_use]
    pub fn travel(&self) -> f64 {
        let floor = self
            .rect(Control::Name)
            .map_or(self.height, |plate| plate.y0 - NAME_GAP);
        (floor - self.fader_top()).clamp(0.0, self.stretch())
    }

    /// Whether the volume control is a fader rather than a knob.
    #[must_use]
    pub fn has_fader(&self) -> bool {
        matches!(self.own.volume, VolumeWidget::Fader)
    }

    /// The rack's box — everything above the REAPER strip.
    ///
    /// `None` when there is no rack, which is both "this phase asks for
    /// no panels" and "this strip is too narrow to draw one": the
    /// mixer collapses `rack_h` to zero in either case.
    #[must_use]
    pub fn rack_rect(&self) -> Option<Rect> {
        (self.rack_h > 0.0).then(|| match self.layout() {
            Layout::Stacked => Rect::new(EDGE, EDGE, self.width - EDGE, self.rack_h - EDGE),
            // Beside the strip's column, the whole height: what the
            // layout exists to give the chain.
            Layout::Column => Rect::new(
                COLUMN_W + EDGE,
                EDGE,
                self.width - EDGE,
                self.height - crate::mcp::INDENT_STEP - EDGE,
            ),
        })
    }

    /// The meter, which is the fader's own groove.
    ///
    /// Not a [`Control`]: a meter is read, never clicked. It shares its
    /// rect with the fader now — the groove is lit by the signal and
    /// the cap rides over it as glass — so the strip no longer has to
    /// find room for two columns and then draw neither when it cannot.
    /// Kept as its own accessor because the two are read for different
    /// reasons, and because a caller asking "is there a meter here"
    /// should not have to know it is asking about the fader.
    #[must_use]
    pub fn meter_rect(&self) -> Option<Rect> {
        self.squeeze.meter().then(|| self.fader_rect())?
    }

    /// The dB numbers beside the meter.
    ///
    /// Live, like the meter: each mark lights as the signal passes it,
    /// so the scale and the column it labels move together. That is
    /// also why it is geometry here rather than something the recording
    /// works out — the overlay needs to place it every frame.
    #[must_use]
    pub fn scale_rect(&self) -> Option<Rect> {
        self.squeeze.meter().then(|| {
            let top = self.fader_top();
            Rect::new(
                self.columns.scale_x,
                top,
                self.columns.scale_x + self.columns.scale_w,
                top + self.travel(),
            )
        })
    }

    /// The fader's whole column — groove, meter and cap.
    fn fader_rect(&self) -> Option<Rect> {
        let top = self.fader_top();
        let stretch = self.travel();
        (stretch > 0.0).then(|| {
            Rect::new(
                self.columns.fader_x,
                top,
                self.columns.fader_x + self.columns.fader_w,
                top + stretch,
            )
        })
    }

    /// Where a control is, or `None` if this strip does not show it.
    ///
    /// The single source both the drawing and the hit test read. A
    /// control that is not returned here is not drawn and cannot be
    /// clicked, which is the invariant that makes them agree.
    #[must_use]
    pub fn rect(&self, control: Control) -> Option<Rect> {
        let top = |y: f64, h: f64| Rect::new(0.0, y, self.chrome_width(), y + h);
        match control {
            Control::Fx => self.squeeze.head().then(|| {
                Rect::new(
                    7.0,
                    self.rack_h + f64::from(g::FX_PILL_TOP),
                    self.chrome_width() - 7.0,
                    self.rack_h + f64::from(daw_theme_art::collapse::FX_SECTION),
                )
            }),
            Control::Pan => (self.squeeze.head()
                && f64::from(self.shared.pan_band) + f64::from(self.shared.input_band) > 26.0)
                .then(|| {
                    // At the LEFT of the coloured band, which is where
                    // REAPER puts it — see `reference/mcp-zoom.png`.
                    // Centred, it collided with the record arm on the
                    // right of the same band and left the left half of
                    // the colour empty.
                    let x = PAN_FROM_EDGE;
                    Rect::new(
                        x,
                        self.band_top() + 2.0,
                        x + f64::from(g::PAN_KNOB_W),
                        self.band_bottom(),
                    )
                }),
            Control::RecArm => {
                let y = self.arm_top();
                if self.squeeze.columns() {
                    let x = self.columns.column_axis - f64::from(g::ARM_CELL_W) * 0.486;
                    return Some(Rect::new(
                        x,
                        y,
                        x + f64::from(g::ARM_CELL_W),
                        y + f64::from(g::ARM_CELL_H),
                    ));
                }
                // The bare ring, centred on the rail's button column —
                // the one control a rail used to drop that it has room
                // for. It was gated on the measured column, so a 60-px
                // return had a monitor and no arm over it.
                (self.width >= ARM_RING + 2.0 * ARM_RING_GAP).then(|| {
                    let x = self.columns.column_axis - ARM_RING / 2.0;
                    Rect::new(x, y, x + ARM_RING, y + ARM_RING)
                })
            }
            // Over the arm, tucked into the band beside the pan — a
            // lamp about recording, next to the ring that starts it.
            // Only where there is a band to sit in: a rail's band is a
            // rule, and a rail is read for its level, not its input.
            Control::Monitor => self.squeeze.columns().then(|| {
                let y = self.arm_top() + MONITOR_TUCK - f64::from(g::BUTTON_H);
                Rect::new(
                    self.columns.column_x,
                    y,
                    self.columns.column_x + f64::from(g::BUTTON_W),
                    y + f64::from(g::BUTTON_H),
                )
            }),
            Control::Mute | Control::Solo | Control::Routing => {
                if !self.squeeze.columns() && control == Control::Routing {
                    return None;
                }
                let y = self.arm_top() + self.column_step(control);
                // The routing's traced cell is padded a pixel around a
                // panel the width of a button, so its CELL goes a pixel
                // left of the column for its PANEL to land on it. The
                // rect is the cell, because that is what gets drawn and
                // therefore what should be hit — a caller doing this
                // arithmetic at the draw call is a caller that can
                // disagree with the hit test. See `ROUTING_CELL_V`.
                if control == Control::Routing {
                    let (cell_w, cell_h) = art::ROUTING_CELL_V;
                    let x = self.columns.column_x - art::ROUTING_INSET_V;
                    return Some(Rect::new(x, y, x + cell_w, y + cell_h));
                }
                Some(Rect::new(
                    self.columns.column_x,
                    y,
                    self.columns.column_x + f64::from(g::BUTTON_W),
                    y + f64::from(g::BUTTON_H),
                ))
            }
            // The fader IS its column — the same rect the meter and the
            // scale read — so it starts where they do: under the band,
            // or under the buttons where the buttons sit over it. It
            // used to start on the mixer's button line regardless,
            // which put the groove up behind the mute and the solo on
            // every narrow strip while the meter beside it had moved.
            Control::Volume => Some(Rect::new(
                self.columns.fader_x,
                self.fader_top(),
                self.columns.fader_x + self.columns.fader_w,
                self.fader_top() + self.travel(),
            )),
            Control::Clip => self.fader_rect().map(|fader| {
                Rect::new(
                    fader.x0,
                    fader.y0,
                    fader.x1,
                    fader.y0 + daw_theme_art::paint::tcp::CLIP_H,
                )
            }),
            // Sat directly on the colour band, not at the top of the
            // bottom section. REAPER's section is 47 high and holds a
            // 26-high name over a 12-high colour band, which leaves
            // nine pixels of nothing between them — a gap that reads as
            // the name floating rather than as anything separating two
            // things. Pushed down, the gap lands ABOVE the name where
            // `Strip::travel` hands it to the fader, which is the one
            // control on the strip that gets better with length.
            //
            // The section's own height is REAPER's measurement and is
            // shared with the Dioxus mixer, so it stays 47.
            Control::Name => {
                let plate = f64::from(g::NAME_PLATE);
                Some(top(self.height - crate::mcp::INDENT_STEP - plate, plate))
            }
            // The fold, at the left of the number band under the name.
            Control::Folder => Some(Rect::new(
                0.0,
                self.height - crate::mcp::INDENT_STEP,
                f64::from(crate::mcp::FOLD_W),
                self.height,
            )),
        }
    }

    /// Which control is at a point in the strip's own coordinates.
    ///
    /// The inverse of [`Strip::rect`] and built from it, so a control is
    /// hit exactly where it is drawn — not approximately, and not
    /// according to a second set of arithmetic that has to be kept in
    /// step.
    #[must_use]
    pub fn control_at(&self, x: f64, y: f64) -> Option<Control> {
        // Ordered by how specific each is: the small controls before
        // the large ones they sit inside, so the arm inside the band
        // wins over the band, and the fader does not swallow the
        // buttons beside it.
        [
            Control::RecArm,
            Control::Monitor,
            Control::Mute,
            Control::Solo,
            Control::Routing,
            Control::Fx,
            Control::Pan,
            // Before the fader it sits on top of, and reduced to the
            // fader by the window when nothing has clipped.
            Control::Clip,
            Control::Volume,
            Control::Folder,
            Control::Name,
        ]
        .into_iter()
        .find(|control| {
            self.rect(*control)
                .is_some_and(|r| x >= r.x0 && x < r.x1 && y >= r.y0 && y < r.y1)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip() -> Strip {
        // A piece strip in a Tone-racked mixer, at the numbers the
        // mixer actually builds — `buttons_top` included.
        //
        // It used to hardcode 1000, which its own bands contradicted:
        // the mixer puts the column four pixels under the coloured
        // band, and 1000 was seventy pixels ABOVE that band's floor. So
        // the fixture had the button column running up through the
        // record arm, and every test about the column was asking about
        // a strip the mixer never builds.
        let (width, height, rack_h) = (133.0, 1440.0, 950.0);
        let mut probe = Strip::new(width, height, height, rack_h, 0.0);
        probe.buttons_top = probe.band_bottom() + 4.0;
        Strip::new(width, height, height, rack_h, probe.buttons_top)
    }

    /// The invariant the whole module exists for: what is drawn is what
    /// is hit. Every control's own rect must hit that control.
    #[test]
    fn a_control_is_hit_where_it_is_drawn() {
        let strip = strip();
        for control in [
            Control::Fx,
            Control::Pan,
            Control::RecArm,
            Control::Monitor,
            Control::Mute,
            Control::Solo,
            Control::Volume,
            Control::Name,
        ] {
            let Some(rect) = strip.rect(control) else {
                continue;
            };
            let hit = strip.control_at(rect.center().x, rect.center().y);
            assert_eq!(
                hit,
                Some(control),
                "{control:?} is drawn at {rect:?} but {hit:?} was hit there"
            );
        }
    }

    /// A control the strip does not show cannot be clicked either. A
    /// hit on something invisible is the same bug as a control drawn
    /// with nothing behind it.
    #[test]
    fn what_is_not_drawn_is_not_hit() {
        let narrow = Strip::new(18.0, 1440.0, 1440.0, 0.0, 1000.0);
        assert!(narrow.rect(Control::RecArm).is_none());
        assert!(narrow.rect(Control::Pan).is_none());
        for y in [50.0, 300.0, 700.0] {
            assert_ne!(narrow.control_at(15.0, y), Some(Control::RecArm));
            assert_ne!(narrow.control_at(15.0, y), Some(Control::Pan));
        }
    }

    /// A rail keeps its record arm: the bare ring, first in the stack
    /// the mute hangs from, clear of the band and of both edges — and
    /// no monitor, which lives in a band the rail does not have.
    #[test]
    fn rail_stacks_the_arm_over_the_mute() {
        let rail = Strip::new(30.0, 1440.0, 1440.0, 0.0, 1000.0);
        assert_eq!(rail.arm(), art::Arm::Panel);
        let arm = rail
            .rect(Control::RecArm)
            .expect("a 30-px rail has room for the ring");
        assert!(rail.rect(Control::Monitor).is_none());
        let monitor = rail.rect(Control::Mute).expect("the mute");
        assert!(arm.y0 >= rail.band_bottom());
        assert!(arm.y1 <= monitor.y0);
        assert!(arm.x0 >= 0.0 && arm.x1 <= 30.0);
        assert!((arm.center().x - monitor.center().x).abs() < 1.0);
        assert!(rail.fader_top() >= rail.rect(Control::Solo).expect("solo").y1);
        // The rail's band is the strip's band: one line across the
        // mixer whatever the width.
        let full = Strip::new(86.0, 1440.0, 1440.0, 0.0, 1000.0);
        assert!((rail.band_bottom() - full.band_bottom()).abs() < f64::EPSILON);

        let full = Strip::new(86.0, 1440.0, 1440.0, 0.0, 1000.0);
        assert_eq!(full.arm(), art::Arm::Mixer);
    }

    /// On a strip with a band, the monitor sits in it, over the arm
    /// and on the column's axis, and the mute follows the arm directly.
    #[test]
    fn the_monitor_sits_in_the_band_over_the_arm() {
        let full = strip();
        let monitor = full.rect(Control::Monitor).expect("the monitor");
        let arm = full.rect(Control::RecArm).expect("the arm");
        let mute = full.rect(Control::Mute).expect("the mute");
        assert!(monitor.y0 >= full.band_top());
        assert!(monitor.y1 <= arm.y0 + MONITOR_TUCK + f64::EPSILON);
        assert!((monitor.center().x - mute.center().x).abs() < f64::EPSILON);
        assert!((mute.y0 - arm.y0 - f64::from(g::RECMON_FROM_ARM) - SPREAD).abs() < f64::EPSILON);
    }

    /// The top sections do NOT move with the strip's own height —
    /// nesting shortens from the bottom, so a deeper strip has its band
    /// and its arm in the same place as a shallow one.
    #[test]
    fn nesting_does_not_move_the_top() {
        let shallow = Strip::new(133.0, 1440.0, 1440.0, 950.0, 1000.0);
        let deep = Strip::new(133.0, 1440.0 - 36.0, 1440.0, 950.0, 1000.0);
        assert!((shallow.band_bottom() - deep.band_bottom()).abs() < f64::EPSILON);
        assert_eq!(shallow.rect(Control::RecArm), deep.rect(Control::RecArm));
        assert_eq!(shallow.rect(Control::Mute), deep.rect(Control::Mute));
    }

    /// But it DOES shorten the fader, which is what the indent costs.
    #[test]
    fn nesting_shortens_the_fader() {
        let shallow = Strip::new(133.0, 1440.0, 1440.0, 950.0, 1000.0);
        let deep = Strip::new(133.0, 1440.0 - 120.0, 1440.0, 950.0, 1000.0);
        assert!(
            deep.stretch() < shallow.stretch(),
            "the fader should give way: {} vs {}",
            deep.stretch(),
            shallow.stretch()
        );
    }

    /// Nothing escapes the strip it belongs to.
    #[test]
    fn every_control_stays_inside_the_strip() {
        for width in [30.0, 56.0, 86.0, 133.0, 195.0] {
            let strip = Strip::new(width, 900.0, 900.0, 400.0, 600.0);
            for control in [
                Control::Fx,
                Control::Pan,
                Control::RecArm,
                Control::Mute,
                Control::Solo,
                Control::Volume,
                Control::Name,
            ] {
                if let Some(r) = strip.rect(control) {
                    assert!(
                        r.x0 >= -0.01 && r.x1 <= width + 0.01,
                        "{control:?} at width {width} spans {}..{}",
                        r.x0,
                        r.x1
                    );
                }
            }
        }
    }
}
