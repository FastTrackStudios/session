//! Example Patterns
//!
//! Full song examples demonstrating complete chart notation.
//! These serve as real-world examples of keyflow in practice.

use super::{Pattern, PatternCategory};

/// Thriller - Dirty Loops, Cory Wong cover arrangement.
///
/// Demonstrates triplet push/pull timing, complex rhythm notation,
/// section structure, and professional chart notation conventions.
pub static THRILLER: Pattern = Pattern::new(
    "thriller",
    "Thriller - Dirty Loops, Cory Wong",
    PatternCategory::Examples,
    r#"Thriller
Dirty Loops, Cory Wong
120bpm 4/4 #Ab
/push = triplet

COUNT 2

IN
r8t Ab9_8t r8t r8t r8t F9_8t r2 | s1

VS
'F/C . | Cm . | 'F/C . | Cm . | 'F/C . | Cm . | 'F/C . | Cm Cm9

CH
Cm/Eb / 'Eb /// | 'Eb / 'F/C / 'Cm // | 'F/A //// | 'Fm9  ////
Cm/Eb / 'Eb /// | 'Eb / 'F/C / 'Cm // | 'F/A | r8t Ab9_8t r8t r8t 'F9_8t r8t r4 Fm/Ab_4 | s1

BR
'_4F7 | . |  Abmaj9 //// | // r8t Abmaj9_8t r8t Bb_8t r8t Cm7_8t | Cm7 | Ebmaj7/Bb | Am7b5 | Abmaj7 | G7sus4 | 'G7

VS
'F/C . | Cm . | 'F/C . | Cm . | 'F/C . | Cm . | 'F/C . | Cm Cm9
"#,
    "A complete Thriller chart demonstrating triplet pushes ('chord), section structure \
     (VS, CH, BR), complex rhythms with explicit durations, and professional notation \
     conventions. Based on the Dirty Loops and Cory Wong cover arrangement.",
);

/// All example patterns.
pub static ALL: &[&Pattern] = &[&THRILLER];

/// Get all example patterns.
pub fn patterns() -> &'static [&'static Pattern] {
    ALL
}
