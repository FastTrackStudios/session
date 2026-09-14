//! The view settings the right rail switches.

/// How a focused track is sized, and what it costs its neighbours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Whether clicking a track opens it wide enough to edit a
    /// plugin's parameters on.
    pub focus_selected: bool,
    /// Whether a focused track takes its width FROM the other tracks or
    /// adds to the mixer's total.
    ///
    /// On, the mixer stays exactly as wide as it was and everything
    /// else gives up a few pixels — nothing moves off the screen you
    /// were looking at. Off, the other tracks keep their width and are
    /// simply pushed along, which is what you want when their widths
    /// are the thing you are comparing and must not change under you.
    pub take_focus_width: bool,
    /// How much of a 16:9 screen one focused track takes.
    pub focus_fraction: f64,
    /// Whether folding a phase's container folds it on every track.
    ///
    /// On, the chain stays in register across the mixer: fold Rescue
    /// away and every strip's Tone panels sit at the same height, which
    /// is what makes a row of racks comparable at all. Off, each track
    /// folds on its own — for when you are working one track rather
    /// than comparing them, and the other strips' chains are just in
    /// the way.
    pub fold_phases_together: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            focus_selected: true,
            take_focus_width: true,
            focus_fraction: 0.25,
            fold_phases_together: true,
        }
    }
}

impl Settings {
    /// How wide a focused track opens, for a panel `height` tall.
    ///
    /// A quarter of what a 16:9 screen of this height would leave
    /// between the rails: `(height * 16/9 - rails) / 4`. 618 at 1440p,
    /// 938 at 4K, 458 at 1080p.
    ///
    /// Derived from the HEIGHT rather than the actual width so a wider
    /// ASPECT gets more tracks instead of wider ones. A quarter of the
    /// real width would put four on a 32:9 screen too, each enormous,
    /// when the reason for that screen is that it holds eight. Physical
    /// size follows the display; how many fit follows its shape.
    ///
    /// And the rails come off first, because what has to hold four is
    /// the panel, not the window. Four of a window-quarter do not fit
    /// between rails that were never in the sum.
    #[must_use]
    pub fn focus_width(self, height: f64) -> f64 {
        let sixteen_by_nine = height * 16.0 / 9.0;
        let panel = (sixteen_by_nine - crate::rails::SIDE * 2.0).max(0.0);
        (panel * self.focus_fraction).max(crate::tone::LEGIBLE)
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;

    /// Four across the PANEL of a 16:9 screen, whatever its resolution.
    #[test]
    fn a_quarter_of_sixteen_by_nine() {
        let settings = Settings::default();
        for (w, h) in [(1920.0, 1080.0), (2560.0, 1440.0), (3840.0, 2160.0)] {
            let focus = settings.focus_width(h);
            let w = w - crate::rails::SIDE * 2.0;
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::as_conversions,
                reason = "a count of strips across a screen"
            )]
            let fits = (w / focus) as u32;
            assert_eq!(fits, 4, "{w}x{h} fits {fits} at {focus}px");
        }
    }

    /// And a wider aspect gets MORE of them, not wider ones — the whole
    /// reason this comes off the height.
    #[test]
    fn a_superwide_gets_more_tracks_not_bigger_ones() {
        let settings = Settings::default();
        let sixteen_nine = settings.focus_width(1440.0);
        let superwide = settings.focus_width(1440.0);
        assert!(
            (sixteen_nine - superwide).abs() < f64::EPSILON,
            "the same height must give the same width"
        );
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::as_conversions,
            reason = "a count of strips across a screen"
        )]
        let fits = ((5120.0 - crate::rails::SIDE * 2.0) / superwide) as u32;
        assert_eq!(fits, 8, "a 32:9 screen should hold eight at {superwide}px");
    }

    /// Never narrower than a rack can be read at, whatever the setting.
    #[test]
    fn a_focused_track_is_always_legible() {
        let tiny = Settings {
            focus_fraction: 0.01,
            ..Settings::default()
        };
        assert!(tiny.focus_width(1440.0) >= crate::tone::LEGIBLE);
    }
}
