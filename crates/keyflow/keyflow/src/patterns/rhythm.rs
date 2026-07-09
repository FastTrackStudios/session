//! Rhythm Patterns
//!
//! Patterns demonstrating rhythm and duration notation features.

use super::{Pattern, PatternCategory};

/// Basic note values demonstrating all standard durations.
pub static BASIC_NOTES: Pattern = Pattern::new(
    "basic-notes",
    "Basic Note Values",
    PatternCategory::Rhythm,
    r#"
#G

VS
C_1 | G_2 D_4 Em_8 G_16 D_32 r32
"#,
    "Demonstrates all standard note values from whole notes (_1) to thirty-second notes (_32). \
     Each note value is shown in progression within two measures.",
);

/// Rest values demonstrating all standard rest durations.
pub static REST_VALUES: Pattern = Pattern::new(
    "rest-values",
    "Rest Values",
    PatternCategory::Rhythm,
    r#"
#G

VS
r1 | r2 r4 r8 r16 r32 r32
"#,
    "Demonstrates all standard rest values from whole rests (r1) to thirty-second rests (r32). \
     Rests use the same duration suffixes as chords.",
);

/// Triplet eighth notes from Thriller intro.
pub static TRIPLET_EIGHTHS: Pattern = Pattern::new(
    "triplet-eighths",
    "Triplet Eighths (Thriller Intro)",
    PatternCategory::Triplets,
    r#"Thriller Intro
120bpm 4/4 #Ab

IN
r8t Ab9_8t r8t r8t r8t F9_8t r2 | s1
"#,
    "Demonstrates triplet eighth notes (_8t suffix). The pattern shows the iconic \
     Thriller intro rhythm with triplet eighths on beats 1 and 2, followed by a half rest.",
);

/// Triplet push chords from Thriller verse.
pub static TRIPLET_PUSH: Pattern = Pattern::new(
    "triplet-push",
    "Triplet Push Chords",
    PatternCategory::PushPull,
    r#"Thriller Verse
120bpm 4/4 #Ab
/push = triplet

VS
'F/C . | Cm . | 'F/C . | Cm . | 'F/C . | Cm . | 'F/C . | Cm // Gm7 // 'Abmaj7 / Abmaj7#5 / 'Db7#11/G //
"#,
    "Demonstrates triplet push timing with the ' prefix. When /push = triplet is set, \
     pushed chords are anticipated by one triplet eighth (160 ticks at 480 PPQ).",
);

/// Mixed triplet pattern from Thriller chorus.
pub static TRIPLET_MIXED: Pattern = Pattern::new(
    "triplet-mixed",
    "Mixed Triplet Pattern (Chorus)",
    PatternCategory::Triplets,
    r#"Thriller Chorus
120bpm 4/4 #Ab
/push = triplet

CH
Cm/Eb / 'Eb // | 'Eb / 'F/C / 'Cm // | 'F/A //// | 'Fm9 ////
"#,
    "A more complex pattern mixing regular slashes (/) with triplet-pushed chords ('). \
     Shows how push timing interacts with rhythmic subdivisions.",
);

/// Pure triplet rests pattern.
pub static TRIPLET_RESTS: Pattern = Pattern::new(
    "triplet-rests",
    "Triplet Rest Pattern",
    PatternCategory::Triplets,
    r#"Triplet Rests
120bpm 4/4 #C

IN
r8t r8t r8t r8t r8t r8t r8t r8t r8t r8t r8t r8t
"#,
    "Demonstrates triplet eighth rests (r8t). Each triplet eighth is 160 ticks \
     (one-third of a quarter note at 480 PPQ). Shows 12 triplet eighths filling one measure.",
);

/// All rhythm patterns.
pub static ALL: &[&Pattern] = &[
    &BASIC_NOTES,
    &REST_VALUES,
    &TRIPLET_EIGHTHS,
    &TRIPLET_PUSH,
    &TRIPLET_MIXED,
    &TRIPLET_RESTS,
];

/// Get all rhythm patterns.
pub fn patterns() -> &'static [&'static Pattern] {
    ALL
}
