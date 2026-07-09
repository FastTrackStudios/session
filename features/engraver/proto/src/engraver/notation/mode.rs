//! Notation modes for different types of music display.

use crate::engraver::layout::tlayout::{NoteHeadType, StemDirection};

/// Notation mode determines the visual style of the music.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NotationMode {
    /// Standard notation with pitched noteheads.
    #[default]
    Standard,
    /// Rhythmic slash notation (slash noteheads, stems down).
    /// Used for rhythm section parts, chord charts.
    Rhythmic,
    /// Percussion notation (X noteheads, specific line assignments).
    Percussion,
    /// Tablature (numbers on lines, no noteheads).
    Tablature,
}

impl NotationMode {
    /// Get the default notehead type for this mode.
    #[must_use]
    pub const fn notehead_type(self) -> NoteHeadType {
        match self {
            Self::Standard => NoteHeadType::Normal,
            Self::Rhythmic => NoteHeadType::Slash,
            Self::Percussion => NoteHeadType::X,
            Self::Tablature => NoteHeadType::Normal,
        }
    }

    /// Get the default stem direction for this mode.
    #[must_use]
    pub const fn default_stem_direction(self) -> StemDirection {
        match self {
            Self::Standard => StemDirection::Auto,
            Self::Rhythmic => StemDirection::Down, // Rhythmic always stems down
            Self::Percussion => StemDirection::Auto,
            Self::Tablature => StemDirection::Auto,
        }
    }

    /// Get the default staff line for notes in this mode.
    /// Rhythmic notation places all notes on the middle line.
    #[must_use]
    pub const fn default_line(self) -> i32 {
        match self {
            Self::Standard => 0, // Middle line (B4 in treble)
            Self::Rhythmic => 0, // Middle line (all slashes same position)
            Self::Percussion => 0,
            Self::Tablature => 0,
        }
    }

    /// Whether this mode uses fixed pitch (all notes on same line).
    #[must_use]
    pub const fn uses_fixed_pitch(self) -> bool {
        matches!(self, Self::Rhythmic)
    }

    /// Whether ledger lines should be drawn.
    #[must_use]
    pub const fn draw_ledger_lines(self) -> bool {
        matches!(self, Self::Standard)
    }
}
