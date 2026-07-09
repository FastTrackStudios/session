//! Minimal pitch spelling helper for MIDI import.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Octave(pub i8);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pitch {
    pub class: PitchClass,
    pub octave: Octave,
    pub alteration: i8,
}

impl Pitch {
    pub const fn with_alteration(class: PitchClass, octave: Octave, alteration: i8) -> Self {
        Self {
            class,
            octave,
            alteration,
        }
    }

    pub fn from_midi(midi: u8) -> Self {
        let octave = Octave((midi / 12) as i8 - 1);
        let pitch_in_octave = midi % 12;

        let (class, alteration) = match pitch_in_octave {
            0 => (PitchClass::C, 0),
            1 => (PitchClass::C, 1),
            2 => (PitchClass::D, 0),
            3 => (PitchClass::D, 1),
            4 => (PitchClass::E, 0),
            5 => (PitchClass::F, 0),
            6 => (PitchClass::F, 1),
            7 => (PitchClass::G, 0),
            8 => (PitchClass::G, 1),
            9 => (PitchClass::A, 0),
            10 => (PitchClass::A, 1),
            11 => (PitchClass::B, 0),
            _ => unreachable!(),
        };

        Self::with_alteration(class, octave, alteration)
    }
}
