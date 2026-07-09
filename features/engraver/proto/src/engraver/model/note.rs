//! Notehead representation for music notation.

use serde::{Deserialize, Serialize};

/// Notehead type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum NoteHead {
    /// Standard filled notehead (for quarter and shorter)
    #[default]
    Black,
    /// Open notehead (for half notes)
    Half,
    /// Open notehead (for whole notes)
    Whole,
    /// X-shaped notehead (for percussion/ghost notes)
    Cross,
    /// Diamond notehead (for harmonics)
    Diamond,
    /// Slash notehead for quarter/eighth notes (rhythmic notation)
    Slash,
    /// Slash notehead for half notes (rhythmic notation)
    SlashHalf,
    /// Slash notehead for whole notes (rhythmic notation)
    SlashWhole,
    /// Slash notehead for double whole notes (rhythmic notation)
    SlashDoubleWhole,
}

impl NoteHead {
    /// Get the SMuFL codepoint for this notehead.
    #[must_use]
    pub const fn smufl_codepoint(self) -> char {
        match self {
            Self::Black => '\u{E0A4}',            // noteheadBlack
            Self::Half => '\u{E0A3}',             // noteheadHalf
            Self::Whole => '\u{E0A2}',            // noteheadWhole
            Self::Cross => '\u{E0A9}',            // noteheadXBlack
            Self::Diamond => '\u{E0DB}',          // noteheadDiamondBlack
            Self::Slash => '\u{E101}',            // noteheadSlashHorizontalEnds
            Self::SlashHalf => '\u{E103}',        // noteheadSlashWhiteHalf
            Self::SlashWhole => '\u{E102}',       // noteheadSlashWhiteWhole
            Self::SlashDoubleWhole => '\u{E10A}', // noteheadSlashWhiteDoubleWhole
        }
    }

    /// Get the appropriate slash notehead for a given duration.
    #[must_use]
    pub const fn slash_for_duration(kind: super::DurationKind) -> Self {
        match kind {
            super::DurationKind::Whole => Self::SlashWhole,
            super::DurationKind::Half => Self::SlashHalf,
            // Quarter, eighth, sixteenth, etc. all use filled slash
            _ => Self::Slash,
        }
    }

    /// Returns true if this is any kind of slash notehead.
    #[must_use]
    pub const fn is_slash(self) -> bool {
        matches!(
            self,
            Self::Slash | Self::SlashHalf | Self::SlashWhole | Self::SlashDoubleWhole
        )
    }
}
