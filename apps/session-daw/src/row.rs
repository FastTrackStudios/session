//! Where everything on a track panel row is — computed once.
//!
//! The mixer's counterpart is [`crate::strip::Strip`], and this exists
//! for the same reason: drawing, hit testing and the live overlay must
//! read ONE layout rather than each working the geometry out again.
//!
//! The row is harder than the strip in one way. A strip's controls are
//! either shown or not; a row's CHANGE SHAPE — past a threshold the
//! volume knob becomes a flattened fader and the pan knob becomes a
//! line, because both are bars whose length is the value and squashing
//! that axis carries no meaning. So a control's rect is not simply
//! present or absent, and [`Row::indicator`] says which form it is in.

use daw_theme_art::geometry::tcp as g;
use vello::kurbo::Rect;

use crate::tcp::{BUTTON, BUTTON_GAP, Density, INDENT, KNOB_LEGIBLE, MAX_INDENT};

/// A control on a track panel row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Control {
    Mute,
    Solo,
    RecArm,
    Volume,
    Pan,
    Routing,
    Fx,
    Name,
    /// The folder mark in the rail — clicked to fold.
    Folder,
    /// Polarity, in the bottom corner of the gutter. The one control
    /// that changes the signal rather than a level, which is why the
    /// theme puts it on its own away from the button column.
    Phase,
}

impl Control {
    /// Set by dragging rather than by clicking.
    #[must_use]
    pub const fn is_continuous(self) -> bool {
        matches!(self, Self::Volume | Self::Pan)
    }
}

/// Which form a value indicator has taken.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Indicator {
    /// A knob, at rows tall enough to turn one.
    Knob,
    /// A flattened bar, on rows that are not.
    Bar,
}

/// One row's geometry, in the panel's coordinates.
#[derive(Clone, Copy, Debug)]
pub struct Row {
    pub y: f64,
    pub height: f64,
    pub indent: f64,
    pub density: Density,
    /// The row's control band — where the name, knobs and arm sit.
    field_top: f64,
    field_h: f64,
    band: f64,
}

impl Row {
    /// Resolve a row.
    #[must_use]
    pub fn new(y: f64, height: f64, depth: i32, is_folder: bool) -> Self {
        let indent = (f64::from(depth.max(0)) * INDENT).min(MAX_INDENT);
        let density = Density::at(height);
        // The control band, as `draw_row` lays it out: row one at its
        // authored height when there is room for two, the whole row
        // otherwise.
        let (field_top, field_h) = if density == Density::Full {
            (y + f64::from(g::ROW_ONE), 24.0)
        } else {
            (y + 1.0, (height - 2.0).max(1.0))
        };
        let _ = is_folder;
        Self {
            y,
            height,
            indent,
            density,
            field_top,
            field_h,
            band: field_h,
        }
    }

    /// Whether a value indicator is a knob or a flattened bar.
    ///
    /// The threshold is the row's own, not a guess: below it a knob is
    /// too small to turn and REAPER swaps it for a bar, so a hit test
    /// that assumed a knob would be testing a control that is not
    /// there.
    #[must_use]
    pub fn indicator(&self) -> Indicator {
        if self.band >= KNOB_LEGIBLE {
            Indicator::Knob
        } else {
            Indicator::Bar
        }
    }

    /// Where a control is, or `None` if this row does not show it.
    #[must_use]
    pub fn rect(&self, control: Control) -> Option<Rect> {
        if self.density == Density::Bar {
            // A row drawn as a band has no controls — it is two
            // rectangles saying which track this is and how it is
            // coloured. Nothing to hit.
            return None;
        }
        let rail = f64::from(g::COLUMN_RULE_X);
        let field_x = f64::from(g::NAME_FIELD_X) + self.indent;
        let volume_x = f64::from(g::NAME_FIELD_X) + f64::from(g::NAME_FIELD_W);
        match control {
            Control::Folder => Some(Rect::new(
                self.indent,
                self.y,
                self.indent + rail,
                self.y + self.height,
            )),
            // In the gutter at the right, in row one where there is a
            // row one, and lying down in the control band where there
            // is not: a compact row keeps its mute and its solo — they
            // are what a row is scanned for — and the routing and FX
            // that share the band at full height are the ones that go.
            Control::Mute | Control::Solo => {
                let x = f64::from(g::TINT_W)
                    + 2.0
                    + if control == Control::Mute {
                        0.0
                    } else {
                        BUTTON.0 + BUTTON_GAP
                    };
                let (top, h) = if self.density == Density::Full {
                    (
                        self.y + f64::from(g::ROW_ONE) + (24.0 - BUTTON.1) / 2.0,
                        BUTTON.1,
                    )
                } else {
                    let h = self.field_h.min(BUTTON.1);
                    (self.field_top + (self.field_h - h) / 2.0, h)
                };
                Some(Rect::new(x, top, x + BUTTON.0, top + h))
            }
            Control::RecArm => (self.indicator() == Indicator::Knob).then(|| {
                let x = field_x + 3.0;
                let top = self.field_top + (self.field_h - 20.0) / 2.0;
                Some(Rect::new(x, top, x + 20.0, top + 20.0))
            })?,
            Control::Volume => {
                // Scaled DOWN to a field too short to hold the knob,
                // never up past the size it was drawn at. A control is
                // the size it was authored: a taller track is a taller
                // lane with the same knob on it, which is what REAPER
                // does and what the pan knob beside it always did.
                let w = if self.indicator() == Indicator::Knob {
                    24.0 * (self.field_h / 22.0).min(1.0)
                } else {
                    24.0
                };
                Some(Rect::new(
                    volume_x - w / 2.0,
                    self.field_top,
                    volume_x + w / 2.0,
                    self.field_top + self.field_h,
                ))
            }
            Control::Pan => {
                let x = f64::from(g::PAN_KNOB_X);
                Some(Rect::new(
                    x,
                    self.field_top,
                    x + 25.0,
                    self.field_top + self.field_h,
                ))
            }
            Control::Routing => (self.density == Density::Full).then(|| {
                let x = f64::from(g::ROUTING_X);
                Some(Rect::new(
                    x,
                    self.field_top,
                    x + 26.0,
                    self.field_top + self.field_h,
                ))
            })?,
            // Hidden on rows too short for it, by the theme's own
            // formula — the row's shape must not depend on its height.
            Control::Phase => (self.height >= f64::from(g::PHASE_HIDE_H)).then(|| {
                let x = f64::from(g::TINT_W) + f64::from(g::GUTTER_BUTTON_X) + 3.0;
                let y = self.y + self.height - f64::from(g::PHASE_FROM_FLOOR);
                Some(Rect::new(
                    x,
                    y,
                    // The glyph's own width, which is one measurement
                    // shared by both panels — the phase button is the
                    // same art in the strip and in the row.
                    x + f64::from(daw_theme_art::geometry::mcp::PHASE_W),
                    y + f64::from(daw_theme_art::geometry::mcp::PHASE_W),
                ))
            })?,
            Control::Fx => (self.density == Density::Full).then(|| {
                let x = f64::from(g::FX_IN_X);
                Some(Rect::new(
                    x,
                    self.field_top,
                    x + 36.0,
                    self.field_top + self.field_h,
                ))
            })?,
            Control::Name => {
                let x = 58.0 + self.indent;
                Some(Rect::new(
                    x,
                    self.field_top,
                    (volume_x - 14.0).max(x),
                    self.field_top + self.field_h,
                ))
            }
        }
    }

    /// Which control is at a point in the panel's coordinates.
    #[must_use]
    pub fn control_at(&self, x: f64, y: f64) -> Option<Control> {
        // Small before large, so the arm inside the name field wins
        // over the field and the knobs win over the row.
        [
            Control::Phase,
            Control::Mute,
            Control::Solo,
            Control::RecArm,
            Control::Volume,
            Control::Pan,
            Control::Routing,
            Control::Fx,
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

    fn tall() -> Row {
        Row::new(100.0, 70.0, 0, false)
    }

    /// What is drawn is what is hit, for every control the row shows.
    #[test]
    fn a_control_is_hit_where_it_is_drawn() {
        let row = tall();
        for control in [
            Control::Mute,
            Control::Solo,
            Control::RecArm,
            Control::Volume,
            Control::Pan,
            Control::Routing,
            Control::Fx,
        ] {
            let Some(rect) = row.rect(control) else {
                continue;
            };
            assert_eq!(
                row.control_at(rect.center().x, rect.center().y),
                Some(control),
                "{control:?} at {rect:?}"
            );
        }
    }

    /// A row too short to turn a knob shows bars instead, and the arm
    /// goes entirely — a five-pixel ring is neither readable nor
    /// hittable, and claiming it is there would make a dead zone.
    #[test]
    fn a_short_row_swaps_knobs_for_bars_and_drops_the_arm() {
        let short = Row::new(0.0, 16.0, 0, false);
        assert_eq!(short.indicator(), Indicator::Bar);
        assert!(short.rect(Control::RecArm).is_none());
        // The value indicators are still there — they are what a
        // collapsed row is READ for.
        assert!(short.rect(Control::Volume).is_some());
        assert!(short.rect(Control::Pan).is_some());
    }

    /// A row drawn as a band has nothing to hit at all.
    #[test]
    fn a_band_has_no_controls() {
        let band = Row::new(0.0, 4.0, 0, false);
        assert_eq!(band.density, Density::Bar);
        for control in [
            Control::Mute,
            Control::Volume,
            Control::Name,
            Control::Folder,
        ] {
            assert!(band.rect(control).is_none(), "{control:?} on a band");
        }
        assert!(band.control_at(100.0, 2.0).is_none());
    }

    /// Indentation moves a row's own controls but not the columns
    /// REAPER fixes — the knobs and the routing widget are at measured
    /// offsets from the panel's right, not from the row's content.
    #[test]
    fn indent_moves_the_name_but_not_the_knobs() {
        let flat = Row::new(0.0, 70.0, 0, false);
        let deep = Row::new(0.0, 70.0, 3, false);
        assert!(
            deep.rect(Control::Name).unwrap().x0 > flat.rect(Control::Name).unwrap().x0,
            "the name should move with the indent"
        );
        assert_eq!(flat.rect(Control::Pan), deep.rect(Control::Pan));
        assert_eq!(flat.rect(Control::Volume), deep.rect(Control::Volume));
    }

    /// Knobs and faders are dragged; buttons are clicked.
    #[test]
    fn only_the_value_controls_are_dragged() {
        assert!(Control::Volume.is_continuous());
        assert!(Control::Pan.is_continuous());
        for click in [Control::Mute, Control::Solo, Control::RecArm, Control::Fx] {
            assert!(!click.is_continuous(), "{click:?}");
        }
    }
}
