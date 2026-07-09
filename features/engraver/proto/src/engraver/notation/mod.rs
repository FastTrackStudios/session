//! High-level notation API for automatic music layout.
//!
//! This module provides a simple, declarative API for creating music notation.
//! You specify the musical content (clef, time signature, rhythms) and the
//! system automatically handles:
//! - Segment creation with proper tick values
//! - Spring-based horizontal spacing
//! - Automatic beaming based on time signature
//! - Collision detection and minimum distances
//!
//! # Example
//!
//! ```ignore
//! use engraver::notation::{MeasureBuilder, NotationMode, Duration};
//!
//! let scene = MeasureBuilder::new()
//!     .clef(ClefType::Treble)
//!     .time_signature(4, 4)
//!     .mode(NotationMode::Rhythmic)
//!     .rhythm(vec![
//!         Duration::Quarter,
//!         Duration::Quarter,
//!         Duration::Eighth,
//!         Duration::Eighth,
//!         Duration::Quarter,
//!     ])
//!     .build(&ctx);
//! ```

mod builder;
mod mode;

pub use crate::engraver::layout::tlayout::TupletRatio;
pub use crate::engraver::model::DurationKind;
pub use builder::{MeasureBuilder, MeasureScene, RhythmEntry, SystemBuilder, TupletSpec};
pub use mode::NotationMode;

/// Duration values in ticks (480 ticks = quarter note, standard MIDI resolution).
///
/// This struct represents a musical duration with optional dots and tuplet modifier.
/// Use the associated constants (e.g., `Duration::Quarter`, `Duration::TripletEighth`)
/// for common durations, or construct custom durations with tuplet support.
///
/// # Examples
///
/// ```ignore
/// // Standard durations
/// let quarter = Duration::Quarter;
/// let dotted_half = Duration::DottedHalf;
///
/// // Triplet durations at any level
/// let triplet_eighth = Duration::triplet(DurationKind::Eighth);
/// let triplet_sixteenth = Duration::triplet(DurationKind::Sixteenth);
///
/// // Other tuplets
/// let quintuplet = Duration::quintuplet(DurationKind::Eighth);
/// let septuplet = Duration::septuplet(DurationKind::Sixteenth);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Duration {
    /// The base note value (whole, half, quarter, etc.)
    pub kind: DurationKind,
    /// Number of augmentation dots (0-2)
    pub dots: u8,
    /// Optional tuplet ratio (e.g., 3:2 for triplet)
    pub tuplet: Option<TupletRatio>,
}

#[allow(non_upper_case_globals)] // CamelCase constants for enum-like API backward compatibility
impl Duration {
    // ========== Standard duration constants ==========

    /// Whole note (4 beats in 4/4)
    pub const Whole: Self = Self::new(DurationKind::Whole);
    /// Half note (2 beats)
    pub const Half: Self = Self::new(DurationKind::Half);
    /// Dotted half note (3 beats)
    pub const DottedHalf: Self = Self::dotted(DurationKind::Half);
    /// Quarter note (1 beat)
    pub const Quarter: Self = Self::new(DurationKind::Quarter);
    /// Dotted quarter (1.5 beats)
    pub const DottedQuarter: Self = Self::dotted(DurationKind::Quarter);
    /// Eighth note (0.5 beats)
    pub const Eighth: Self = Self::new(DurationKind::Eighth);
    /// Dotted eighth (0.75 beats)
    pub const DottedEighth: Self = Self::dotted(DurationKind::Eighth);
    /// Sixteenth note (0.25 beats)
    pub const Sixteenth: Self = Self::new(DurationKind::Sixteenth);
    /// Dotted sixteenth
    pub const DottedSixteenth: Self = Self::dotted(DurationKind::Sixteenth);
    /// Thirty-second note
    pub const ThirtySecond: Self = Self::new(DurationKind::ThirtySecond);
    /// Sixty-fourth note
    pub const SixtyFourth: Self = Self::new(DurationKind::SixtyFourth);

    // ========== Triplet duration constants ==========

    /// Triplet whole note (2/3 of 1920 = 1280 ticks)
    pub const TripletWhole: Self = Self::triplet_const(DurationKind::Whole);
    /// Triplet half note (2/3 of 960 = 640 ticks)
    pub const TripletHalf: Self = Self::triplet_const(DurationKind::Half);
    /// Triplet quarter (2/3 of 480 = 320 ticks)
    pub const TripletQuarter: Self = Self::triplet_const(DurationKind::Quarter);
    /// Triplet eighth (2/3 of 240 = 160 ticks)
    pub const TripletEighth: Self = Self::triplet_const(DurationKind::Eighth);
    /// Triplet sixteenth (2/3 of 120 = 80 ticks)
    pub const TripletSixteenth: Self = Self::triplet_const(DurationKind::Sixteenth);
    /// Triplet thirty-second (2/3 of 60 = 40 ticks)
    pub const TripletThirtySecond: Self = Self::triplet_const(DurationKind::ThirtySecond);

    // ========== Constructors ==========

    /// Create a new duration without dots or tuplet.
    #[must_use]
    pub const fn new(kind: DurationKind) -> Self {
        Self {
            kind,
            dots: 0,
            tuplet: None,
        }
    }

    /// Create a dotted duration.
    #[must_use]
    pub const fn dotted(kind: DurationKind) -> Self {
        Self {
            kind,
            dots: 1,
            tuplet: None,
        }
    }

    /// Create a double-dotted duration.
    #[must_use]
    pub const fn double_dotted(kind: DurationKind) -> Self {
        Self {
            kind,
            dots: 2,
            tuplet: None,
        }
    }

    /// Create a triplet (3:2) duration at any note level.
    #[must_use]
    pub fn triplet(kind: DurationKind) -> Self {
        Self {
            kind,
            dots: 0,
            tuplet: Some(TupletRatio::triplet()),
        }
    }

    /// Const version of triplet() for use in const contexts.
    const fn triplet_const(kind: DurationKind) -> Self {
        Self {
            kind,
            dots: 0,
            tuplet: Some(TupletRatio::TRIPLET),
        }
    }

    /// Create a quintuplet (5:4) duration.
    #[must_use]
    pub fn quintuplet(kind: DurationKind) -> Self {
        Self {
            kind,
            dots: 0,
            tuplet: Some(TupletRatio::quintuplet()),
        }
    }

    /// Create a sextuplet (6:4) duration.
    #[must_use]
    pub fn sextuplet(kind: DurationKind) -> Self {
        Self {
            kind,
            dots: 0,
            tuplet: Some(TupletRatio::sextuplet()),
        }
    }

    /// Create a septuplet (7:8) duration (matches MuseScore).
    #[must_use]
    pub fn septuplet(kind: DurationKind) -> Self {
        Self {
            kind,
            dots: 0,
            tuplet: Some(TupletRatio::septuplet()),
        }
    }

    /// Create a duration with a custom tuplet ratio.
    #[must_use]
    pub const fn with_tuplet(kind: DurationKind, ratio: TupletRatio) -> Self {
        Self {
            kind,
            dots: 0,
            tuplet: Some(ratio),
        }
    }

    // ========== Query methods ==========

    /// Get the duration in ticks (480 = quarter note).
    #[must_use]
    pub const fn ticks(&self) -> i32 {
        let base = self.kind.base_ticks();

        // Apply dots
        let dotted_ticks = match self.dots {
            0 => base,
            1 => base + base / 2,     // 1.5x
            2 => base + base * 3 / 4, // 1.75x
            _ => base,
        };

        // Apply tuplet ratio
        match &self.tuplet {
            Some(ratio) => dotted_ticks * ratio.denominator as i32 / ratio.numerator as i32,
            None => dotted_ticks,
        }
    }

    /// Check if this duration is dotted.
    #[must_use]
    pub const fn is_dotted(&self) -> bool {
        self.dots > 0
    }

    /// Check if this is a tuplet duration.
    #[must_use]
    pub const fn is_tuplet(&self) -> bool {
        self.tuplet.is_some()
    }

    /// Check if this is a triplet duration (3:2 ratio).
    #[must_use]
    pub const fn is_triplet(&self) -> bool {
        match &self.tuplet {
            Some(ratio) => ratio.numerator == 3 && ratio.denominator == 2,
            None => false,
        }
    }

    /// Get the number of dots.
    #[must_use]
    pub const fn dots(&self) -> u8 {
        self.dots
    }

    /// Convert to NoteDuration enum (for glyph selection).
    /// The NoteDuration represents the visual appearance, not the actual duration.
    #[must_use]
    pub const fn to_note_duration(&self) -> crate::engraver::layout::tlayout::NoteDuration {
        use crate::engraver::layout::tlayout::NoteDuration;
        match self.kind {
            DurationKind::Whole => NoteDuration::Whole,
            DurationKind::Half => NoteDuration::Half,
            DurationKind::Quarter => NoteDuration::Quarter,
            DurationKind::Eighth => NoteDuration::Eighth,
            DurationKind::Sixteenth => NoteDuration::Sixteenth,
            DurationKind::ThirtySecond => NoteDuration::ThirtySecond,
            DurationKind::SixtyFourth => NoteDuration::SixtyFourth,
        }
    }

    /// Check if this duration needs a flag (when not beamed).
    /// Eighth notes and shorter need flags.
    #[must_use]
    pub const fn needs_flag(&self) -> bool {
        self.kind.flags() > 0
    }

    /// Get the number of beams needed for this duration.
    #[must_use]
    pub const fn beam_count(&self) -> u8 {
        self.kind.flags()
    }
}

/// Time signature representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSignature {
    pub numerator: u8,
    pub denominator: u8,
}

impl TimeSignature {
    /// Create a new time signature.
    #[must_use]
    pub const fn new(numerator: u8, denominator: u8) -> Self {
        Self {
            numerator,
            denominator,
        }
    }

    /// Common time (4/4).
    pub const COMMON: Self = Self::new(4, 4);
    /// Cut time (2/2).
    pub const CUT: Self = Self::new(2, 2);
    /// Waltz time (3/4).
    pub const WALTZ: Self = Self::new(3, 4);

    /// Get the number of ticks in one measure.
    #[must_use]
    pub const fn measure_ticks(&self) -> i32 {
        let beat_ticks = 1920 / self.denominator as i32; // 1920 = whole note
        beat_ticks * self.numerator as i32
    }

    /// Get the number of ticks per beat.
    #[must_use]
    pub const fn beat_ticks(&self) -> i32 {
        1920 / self.denominator as i32
    }

    /// Get beam groupings for this time signature.
    /// Returns a list of tick counts for each beam group.
    #[must_use]
    pub fn beam_groups(&self) -> Vec<i32> {
        let beat = self.beat_ticks();
        match (self.numerator, self.denominator) {
            // 4/4: beam in groups of 2 beats (or 1 beat for eighths)
            (4, 4) => vec![beat, beat, beat, beat],
            // 3/4: beam each beat separately
            (3, 4) => vec![beat, beat, beat],
            // 6/8: beam in groups of 3 eighths
            (6, 8) => vec![beat * 3, beat * 3],
            // 2/4: beam in groups of 1 beat
            (2, 4) => vec![beat, beat],
            // Default: beam each beat
            _ => vec![beat; self.numerator as usize],
        }
    }
}

impl Default for TimeSignature {
    fn default() -> Self {
        Self::COMMON
    }
}
