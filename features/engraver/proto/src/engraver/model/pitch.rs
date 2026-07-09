//! Pitch representation for music notation.

use serde::{Deserialize, Serialize};

/// Pitch class (note name without octave).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PitchClass {
    C,
    D,
    E,
    F,
    G,
    A,
    B,
}

impl PitchClass {
    /// Get the staff position offset from C (0-6).
    #[must_use]
    pub const fn staff_offset(self) -> i32 {
        match self {
            Self::C => 0,
            Self::D => 1,
            Self::E => 2,
            Self::F => 3,
            Self::G => 4,
            Self::A => 5,
            Self::B => 6,
        }
    }

    /// Get the MIDI note number for this pitch class in octave 0.
    #[must_use]
    pub const fn base_midi(self) -> u8 {
        match self {
            Self::C => 0,
            Self::D => 2,
            Self::E => 4,
            Self::F => 5,
            Self::G => 7,
            Self::A => 9,
            Self::B => 11,
        }
    }
}

/// Scientific pitch octave (0-9).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Octave(pub i8);

impl Octave {
    /// Middle C octave (C4).
    pub const MIDDLE: Self = Self(4);

    /// Create a new octave.
    #[must_use]
    pub const fn new(octave: i8) -> Self {
        Self(octave)
    }
}

impl Default for Octave {
    fn default() -> Self {
        Self::MIDDLE
    }
}

/// Complete pitch with pitch class and octave.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pitch {
    /// The pitch class (C, D, E, etc.)
    pub class: PitchClass,
    /// The octave
    pub octave: Octave,
    /// Chromatic alteration in semitones (-2 to +2 for double flat to double sharp)
    pub alteration: i8,
}

impl Pitch {
    /// Create a new pitch.
    #[must_use]
    pub const fn new(class: PitchClass, octave: Octave) -> Self {
        Self {
            class,
            octave,
            alteration: 0,
        }
    }

    /// Create a pitch with alteration.
    #[must_use]
    pub const fn with_alteration(class: PitchClass, octave: Octave, alteration: i8) -> Self {
        Self {
            class,
            octave,
            alteration,
        }
    }

    /// Middle C (C4).
    pub const MIDDLE_C: Self = Self::new(PitchClass::C, Octave::MIDDLE);

    /// Get the MIDI note number for this pitch.
    #[must_use]
    pub fn midi_note(&self) -> u8 {
        let base = self.class.base_midi();
        let octave_offset = (self.octave.0 + 1) * 12;
        (i16::from(base) + i16::from(octave_offset) + i16::from(self.alteration)) as u8
    }

    /// Get the staff position relative to middle C (0 = middle C line).
    #[must_use]
    pub fn staff_position(&self) -> i32 {
        let octave_offset = (i32::from(self.octave.0) - 4) * 7;
        self.class.staff_offset() + octave_offset
    }

    /// Transpose by semitones (returns new pitch, may change enharmonic spelling).
    #[must_use]
    pub fn transpose_semitones(&self, semitones: i8) -> Self {
        let new_midi = (i16::from(self.midi_note()) + i16::from(semitones)) as u8;
        Self::from_midi(new_midi)
    }

    /// Create a pitch from MIDI note number (uses sharp spelling for black keys).
    #[must_use]
    pub fn from_midi(midi: u8) -> Self {
        let octave = Octave::new((midi / 12) as i8 - 1);
        let pitch_in_octave = midi % 12;

        let (class, alteration) = match pitch_in_octave {
            0 => (PitchClass::C, 0),
            1 => (PitchClass::C, 1), // C#
            2 => (PitchClass::D, 0),
            3 => (PitchClass::D, 1), // D#
            4 => (PitchClass::E, 0),
            5 => (PitchClass::F, 0),
            6 => (PitchClass::F, 1), // F#
            7 => (PitchClass::G, 0),
            8 => (PitchClass::G, 1), // G#
            9 => (PitchClass::A, 0),
            10 => (PitchClass::A, 1), // A#
            11 => (PitchClass::B, 0),
            _ => unreachable!(),
        };

        Self::with_alteration(class, octave, alteration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_middle_c() {
        let middle_c = Pitch::MIDDLE_C;
        assert_eq!(middle_c.midi_note(), 60);
        assert_eq!(middle_c.staff_position(), 0);
    }

    #[test]
    fn test_from_midi() {
        let a4 = Pitch::from_midi(69);
        assert_eq!(a4.class, PitchClass::A);
        assert_eq!(a4.octave, Octave(4));
        assert_eq!(a4.alteration, 0);
    }
}
