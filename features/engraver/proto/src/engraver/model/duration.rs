//! Duration representation for music notation.

use serde::{Deserialize, Serialize};

/// Duration kind (note value).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DurationKind {
    /// Whole note (semibreve)
    Whole,
    /// Half note (minim)
    Half,
    /// Quarter note (crotchet)
    Quarter,
    /// Eighth note (quaver)
    Eighth,
    /// Sixteenth note (semiquaver)
    Sixteenth,
    /// Thirty-second note (demisemiquaver)
    ThirtySecond,
    /// Sixty-fourth note (hemidemisemiquaver)
    SixtyFourth,
}

impl DurationKind {
    /// Get the duration in quarter notes (1.0 = quarter note).
    #[must_use]
    pub const fn quarters(self) -> f64 {
        match self {
            Self::Whole => 4.0,
            Self::Half => 2.0,
            Self::Quarter => 1.0,
            Self::Eighth => 0.5,
            Self::Sixteenth => 0.25,
            Self::ThirtySecond => 0.125,
            Self::SixtyFourth => 0.0625,
        }
    }

    /// Get the base duration in ticks (480 ticks = quarter note).
    /// This is the duration without dots or tuplet modification.
    #[must_use]
    pub const fn base_ticks(self) -> i32 {
        match self {
            Self::Whole => 1920,
            Self::Half => 960,
            Self::Quarter => 480,
            Self::Eighth => 240,
            Self::Sixteenth => 120,
            Self::ThirtySecond => 60,
            Self::SixtyFourth => 30,
        }
    }

    /// Get the number of flags/beams for this duration.
    #[must_use]
    pub const fn flags(self) -> u8 {
        match self {
            Self::Whole | Self::Half | Self::Quarter => 0,
            Self::Eighth => 1,
            Self::Sixteenth => 2,
            Self::ThirtySecond => 3,
            Self::SixtyFourth => 4,
        }
    }
}

/// Complete duration with dots.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Duration {
    /// The base duration kind
    pub kind: DurationKind,
    /// Number of dots (0-2)
    pub dots: u8,
}

impl Duration {
    /// Create a new duration.
    #[must_use]
    pub const fn new(kind: DurationKind) -> Self {
        Self { kind, dots: 0 }
    }

    /// Create a dotted duration.
    #[must_use]
    pub const fn dotted(kind: DurationKind) -> Self {
        Self { kind, dots: 1 }
    }

    /// Create a double-dotted duration.
    #[must_use]
    pub const fn double_dotted(kind: DurationKind) -> Self {
        Self { kind, dots: 2 }
    }

    /// Get the total duration in quarter notes.
    #[must_use]
    pub fn quarters(&self) -> f64 {
        let base = self.kind.quarters();
        match self.dots {
            0 => base,
            1 => base * 1.5,
            2 => base * 1.75,
            _ => base, // More than 2 dots is rare
        }
    }

    // Common durations
    pub const WHOLE: Self = Self::new(DurationKind::Whole);
    pub const HALF: Self = Self::new(DurationKind::Half);
    pub const QUARTER: Self = Self::new(DurationKind::Quarter);
    pub const EIGHTH: Self = Self::new(DurationKind::Eighth);
    pub const SIXTEENTH: Self = Self::new(DurationKind::Sixteenth);
}

impl Default for Duration {
    fn default() -> Self {
        Self::QUARTER
    }
}

/// Convert from notation::Duration to model::Duration.
///
/// Note: This drops tuplet information since model::Duration
/// doesn't support tuplets. Use notation::Duration directly
/// when tuplet information is needed.
impl From<crate::engraver::notation::Duration> for Duration {
    fn from(other: crate::engraver::notation::Duration) -> Self {
        Self {
            kind: other.kind,
            dots: other.dots,
        }
    }
}

// region:    --- crate::core::NoteValue Conversions

#[cfg(feature = "svg")]
impl From<crate::core::NoteValue> for DurationKind {
    fn from(note: crate::core::NoteValue) -> Self {
        match note {
            crate::core::NoteValue::Whole => Self::Whole,
            crate::core::NoteValue::Half => Self::Half,
            crate::core::NoteValue::Quarter => Self::Quarter,
            crate::core::NoteValue::Eighth => Self::Eighth,
            crate::core::NoteValue::Sixteenth => Self::Sixteenth,
            crate::core::NoteValue::ThirtySecond => Self::ThirtySecond,
            crate::core::NoteValue::SixtyFourth => Self::SixtyFourth,
        }
    }
}

#[cfg(feature = "svg")]
impl From<DurationKind> for crate::core::NoteValue {
    fn from(kind: DurationKind) -> Self {
        match kind {
            DurationKind::Whole => Self::Whole,
            DurationKind::Half => Self::Half,
            DurationKind::Quarter => Self::Quarter,
            DurationKind::Eighth => Self::Eighth,
            DurationKind::Sixteenth => Self::Sixteenth,
            DurationKind::ThirtySecond => Self::ThirtySecond,
            DurationKind::SixtyFourth => Self::SixtyFourth,
        }
    }
}

// endregion: --- crate::core::NoteValue Conversions
