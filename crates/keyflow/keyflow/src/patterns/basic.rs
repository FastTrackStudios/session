//! Basic Structure Patterns
//!
//! Patterns demonstrating fundamental chart structure features.

use super::{Pattern, PatternCategory};

/// Minimal chart structure with just a key and section.
pub static MINIMAL_CHART: Pattern = Pattern::new(
    "minimal-chart",
    "Minimal Chart",
    PatternCategory::BasicStructure,
    r#"#C

VS
C | G | Am | F
"#,
    "The simplest valid chart: a key signature (#C) followed by a section (VS for verse) \
     with four measures of whole-note chords.",
);

/// Chart with metadata (title, tempo, time signature, key).
pub static FULL_METADATA: Pattern = Pattern::new(
    "full-metadata",
    "Full Metadata",
    PatternCategory::BasicStructure,
    r#"My Song Title
120bpm 4/4 #G

VS
G | D | Em | C
"#,
    "A chart with complete metadata: title on the first line, tempo (120bpm), \
     time signature (4/4), and key (#G).",
);

/// Multiple sections with standard abbreviations.
pub static MULTIPLE_SECTIONS: Pattern = Pattern::new(
    "multiple-sections",
    "Multiple Sections",
    PatternCategory::Sections,
    r#"Section Example
100bpm 4/4 #D

IN
D | . | . | .

VS
D | G | Bm | A

CH
G | A | D | .

BR
Bm | G | A | .

OU
D | . | . | .
"#,
    "Demonstrates standard section types: IN (intro), VS (verse), CH (chorus), \
     BR (bridge), and OU (outro). The dot (.) repeats the previous chord.",
);

/// Chord slash notation for rhythmic subdivision.
pub static SLASH_NOTATION: Pattern = Pattern::new(
    "slash-notation",
    "Slash Notation",
    PatternCategory::Rhythm,
    r#"Slash Demo
120bpm 4/4 #A

VS
A | D / E / | A | D // E //
"#,
    "Demonstrates slash notation for rhythmic subdivision. A single slash (/) \
     divides the beat in half. Two slashes (//) divide into quarters.",
);

/// Chord memory with the dot shorthand.
pub static CHORD_MEMORY: Pattern = Pattern::new(
    "chord-memory",
    "Chord Memory",
    PatternCategory::Chords,
    r#"Memory Demo
90bpm 4/4 #E

VS
E | . | . | A
. | . | B | .
"#,
    "Demonstrates chord memory using the dot (.) shorthand. A dot repeats \
     the previous chord, reducing visual clutter in charts with sustained harmonies.",
);

/// All basic patterns.
pub static ALL: &[&Pattern] = &[
    &MINIMAL_CHART,
    &FULL_METADATA,
    &MULTIPLE_SECTIONS,
    &SLASH_NOTATION,
    &CHORD_MEMORY,
];

/// Get all basic patterns.
pub fn patterns() -> &'static [&'static Pattern] {
    ALL
}
