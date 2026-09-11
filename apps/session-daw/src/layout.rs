//! How tall rows are drawn.
//!
//! The height itself comes from the track — `daw_proto::Track::height`,
//! which the standalone loader fills in from the project's
//! `TRACKHEIGHT` and the REAPER backend from `I_HEIGHTOVERRIDE`. What is
//! here is everything the TRACK does not say: what a track with no
//! stored height gets, and how small a row is allowed to become.
//!
//! Both are settings rather than constants, because both are preferences
//! and people disagree about them. A default track height is the first
//! thing anyone changes about a DAW's track panel, and the floor is the
//! whole argument of this module — see [`Layout::min`].

/// Row heights, as the user wants them.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    /// What a track with no stored height is drawn at.
    ///
    /// `Track::height` is `None` for a track nobody has resized, which
    /// is most of them in most projects, so this is the height of
    /// essentially every row in a fresh session. It belongs to the user,
    /// not to the file and not to this crate.
    ///
    /// The default is [`NAME_LEGIBLE`] rather than REAPER's 70: the
    /// resting state of a session should be as much of it as can be read
    /// at once, and a track gets opened up when it is being worked on.
    pub default: f64,
    /// The smallest a row is ever drawn, before the vertical zoom.
    ///
    /// One pixel, which is REAPER's own most-collapsed state. The panel
    /// sheds controls on the way down — see [`crate::tcp::Density`] —
    /// until a row is a single coloured line, because at that point the
    /// question being asked is "where is everything", and seeing the
    /// whole session answers it better than any one track's knobs.
    ///
    /// The divider lives inside the row, so a one-pixel row IS its
    /// divider: a line of the track's colour and nothing else.
    pub min: f64,
}

/// The smallest row that still prints a legible track name.
///
/// Not a taste: it is the name's own type size plus the space a line of
/// it needs, and it is why the number is here rather than inlined. Below
/// this the row stops carrying a name at all (see [`crate::tcp::Density`]),
/// so this is the boundary between "a list of tracks" and "a picture of
/// a session".
pub const NAME_LEGIBLE: f64 = 14.0;

impl Default for Layout {
    fn default() -> Self {
        Self {
            default: NAME_LEGIBLE,
            min: 1.0,
        }
    }
}

impl Layout {
    /// Read the overrides from the environment.
    ///
    /// A stand-in for the settings store this window does not have yet.
    /// It is here rather than at the call site so that when there IS a
    /// settings store, one function changes and no caller does.
    #[must_use]
    pub fn from_env() -> Self {
        let mut layout = Self::default();
        if let Some(height) = number("FTS_TRACK_HEIGHT") {
            layout.default = height;
        }
        if let Some(min) = number("FTS_TRACK_HEIGHT_MIN") {
            layout.min = min;
        }
        layout
    }

    /// How tall to draw a track.
    ///
    /// Anything at or below zero is treated as unset: a stored height of
    /// nought is not a row, and a negative one is a corrupt file rather
    /// than an instruction.
    #[must_use]
    pub fn height_of(self, stored: Option<u32>) -> f64 {
        stored
            .map(f64::from)
            .filter(|h| *h > 0.0)
            .unwrap_or(self.default)
            .max(self.min)
    }
}

/// A positive number from the environment, if it is one.
fn number(key: &str) -> Option<f64> {
    std::env::var(key)
        .ok()?
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|n| *n > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_height_takes_the_default() {
        let layout = Layout {
            default: 48.0,
            min: 1.0,
        };
        assert!((layout.height_of(None) - 48.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_stored_height_wins() {
        let layout = Layout::default();
        assert!((layout.height_of(Some(24)) - 24.0).abs() < f64::EPSILON);
    }

    /// Zero is REAPER's sentinel for automatic, not a row with no height.
    #[test]
    fn zero_is_unset_not_flat() {
        let layout = Layout {
            default: 70.0,
            min: 1.0,
        };
        assert!((layout.height_of(Some(0)) - 70.0).abs() < f64::EPSILON);
    }

    /// The floor applies to a stored height too — a project can ask for
    /// something smaller than anything can be drawn at.
    #[test]
    fn the_floor_holds() {
        let layout = Layout {
            default: 70.0,
            min: 6.0,
        };
        assert!((layout.height_of(Some(1)) - 6.0).abs() < f64::EPSILON);
    }
}
