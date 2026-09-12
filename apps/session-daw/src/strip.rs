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

use crate::mcp::{Columns, Control, Squeeze};

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
}

impl Strip {
    /// Resolve a strip.
    #[must_use]
    pub fn new(width: f64, height: f64, mixer_h: f64, rack_h: f64, buttons_top: f64) -> Self {
        Self {
            width,
            height,
            squeeze: Squeeze::at(width),
            shared: Collapse::at(crate::mcp::f64_to_f32((mixer_h - rack_h).max(1.0))),
            own: Collapse::at(crate::mcp::f64_to_f32((height - rack_h).max(1.0))),
            columns: Columns::at(0.0, width),
            rack_h,
            buttons_top,
        }
    }

    /// The top of the coloured band.
    #[must_use]
    pub fn band_top(&self) -> f64 {
        f64::from(daw_theme_art::collapse::FX_SECTION) + self.rack_h
    }

    /// Its bottom — what the record arm hangs from.
    #[must_use]
    pub fn band_bottom(&self) -> f64 {
        self.band_top() + f64::from(self.shared.pan_band) + f64::from(self.shared.input_band)
    }

    /// How much travel the fader has.
    #[must_use]
    pub fn stretch(&self) -> f64 {
        f64::from(self.own.stretch)
    }

    /// Whether the volume control is a fader rather than a knob.
    #[must_use]
    pub fn has_fader(&self) -> bool {
        matches!(self.own.volume, VolumeWidget::Fader)
    }

    /// The meter well beside the fader.
    ///
    /// Not a [`Control`]: a meter is read, never clicked, and putting
    /// it in that enum would make it hit-testable and let it swallow
    /// presses meant for the fader it stands next to. It is here
    /// because it is GEOMETRY, and this module exists so that geometry
    /// is worked out once.
    ///
    /// `None` on a strip too narrow to hold one — REAPER draws no meter
    /// on an 86-wide strip either, because the scale and the fader have
    /// already taken the width.
    #[must_use]
    pub fn meter_rect(&self) -> Option<Rect> {
        (self.squeeze.meter() && self.columns.has_meter()).then(|| {
            let top = self.band_bottom();
            Rect::new(
                self.columns.meter_x,
                top,
                self.columns.meter_x + self.columns.meter_w,
                top + self.stretch(),
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
        let top = |y: f64, h: f64| Rect::new(0.0, y, self.width, y + h);
        match control {
            Control::Fx => self.squeeze.head().then(|| {
                Rect::new(
                    7.0,
                    self.rack_h + f64::from(g::FX_PILL_TOP),
                    self.width - 7.0,
                    self.rack_h + f64::from(daw_theme_art::collapse::FX_SECTION),
                )
            }),
            Control::Pan => (self.squeeze.head()
                && f64::from(self.shared.pan_band) + f64::from(self.shared.input_band) > 26.0)
                .then(|| {
                    let x = (self.width - f64::from(g::PAN_KNOB_W)) / 2.0;
                    Rect::new(
                        x,
                        self.band_top() + 2.0,
                        x + f64::from(g::PAN_KNOB_W),
                        self.band_bottom(),
                    )
                }),
            Control::RecArm => self.squeeze.columns().then(|| {
                let x = self.columns.column_axis - f64::from(g::ARM_CELL_W) * 0.486;
                let y = self.band_bottom() + f64::from(g::ARM_OVERHANG)
                    - f64::from(g::ARM_CELL_H);
                Rect::new(
                    x,
                    y,
                    x + f64::from(g::ARM_CELL_W),
                    y + f64::from(g::ARM_CELL_H),
                )
            }),
            Control::Mute | Control::Solo | Control::Routing => {
                let row = match control {
                    Control::Mute => 0.0,
                    Control::Solo => 1.0,
                    _ => 2.0,
                };
                if control == Control::Routing && !self.squeeze.columns() {
                    return None;
                }
                let y = self.buttons_top
                    + f64::from(g::RECMON_FROM_ARM)
                    + row * (f64::from(g::BUTTON_H) + 1.0);
                Some(Rect::new(
                    self.columns.column_x,
                    y,
                    self.columns.column_x + f64::from(g::BUTTON_W),
                    y + f64::from(g::BUTTON_H),
                ))
            }
            Control::Volume => Some(Rect::new(
                self.columns.fader_x,
                self.buttons_top,
                self.columns.fader_x + self.columns.fader_w,
                self.buttons_top + self.stretch(),
            )),
            Control::Name => {
                let plate = self.height - f64::from(daw_theme_art::collapse::BOTTOM_SECTION);
                Some(top(plate, self.height - plate))
            }
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
            Control::Mute,
            Control::Solo,
            Control::Routing,
            Control::Fx,
            Control::Pan,
            Control::Volume,
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
        // mixer actually builds.
        Strip::new(133.0, 1440.0, 1440.0, 950.0, 1000.0)
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
        let narrow = Strip::new(30.0, 1440.0, 1440.0, 0.0, 1000.0);
        assert!(narrow.rect(Control::RecArm).is_none());
        assert!(narrow.rect(Control::Pan).is_none());
        for y in [50.0, 300.0, 700.0] {
            assert_ne!(narrow.control_at(15.0, y), Some(Control::RecArm));
            assert_ne!(narrow.control_at(15.0, y), Some(Control::Pan));
        }
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
