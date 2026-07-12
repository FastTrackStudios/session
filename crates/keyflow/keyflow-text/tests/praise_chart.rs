//! Regression: the Elevation Worship "Praise" chart round-trips fully.
//!
//! Pins the three parser fixes together on a real worship chart:
//!   1. `Refrain` is a first-class section type (was silently dropped),
//!   2. a quoted label mid-marker keeps its trailing bar count
//!      (`Interlude "Breakdown" 8`),
//!   3. bare `PRE` / repeated sections inherit their bar count via memory.

use keyflow_text::chart::parse_chart;

const PRAISE: &str = "\
Praise - Elevation Worship
#A 127bpm 4/4

Count 2
In 4
Refrain 8
VS 8
VS
PRE 2
CH 8
VS
VS
PRE
CH
CH
Interlude \"Breakdown\" 8
BR \"Down\" 8
BR \"Build\"
CH
CH
CH
INST \"Guitar Lead\" 8
Refrain
Refrain";

#[test]
fn praise_chart_round_trips() {
    let chart = parse_chart(PRAISE).expect("Praise chart should parse");

    // Header.
    assert_eq!(chart.tempo.map(|t| t.bpm), Some(127.0));
    let ts = chart.time_signature.expect("time signature");
    assert_eq!((ts.numerator, ts.denominator), (4, 4));
    assert_eq!(
        chart.initial_key.as_ref().map(|k| k.root.name.clone()),
        Some("A".to_string())
    );

    // Print the parsed sections for eyeballing while iterating.
    for (i, cs) in chart.sections.iter().enumerate() {
        let s = &cs.section;
        println!(
            "[{i}] {:<11} measures={:?} comment={:?}",
            s.section_type.full_name(),
            s.measure_count,
            s.comment
        );
    }

    // All 21 chart lines become sections (the 3 Refrains are no longer dropped).
    assert_eq!(chart.sections.len(), 21, "every section line should parse");

    // Ordered (type, measures, comment) contract. `full_name()` keeps this
    // readable and independent of auto-numbering.
    let expected: &[(&str, usize, Option<&str>)] = &[
        ("Count-In", 2, None),
        ("Intro", 4, None),
        ("Refrain", 8, None),
        ("Verse", 8, None),
        ("Verse", 8, None),
        ("Pre-Chorus", 2, None),
        ("Chorus", 8, None),
        ("Verse", 8, None),
        ("Verse", 8, None),
        ("Pre-Chorus", 2, None), // inherited via memory (bug #3)
        ("Chorus", 8, None),
        ("Chorus", 8, None),
        ("Interlude", 8, Some("Breakdown")), // trailing 8 kept (bug #2)
        ("Bridge", 8, Some("Down")),         // trailing 8 kept (bug #2)
        ("Bridge", 8, Some("Build")),        // inherited via memory (bug #3)
        ("Chorus", 8, None),
        ("Chorus", 8, None),
        ("Chorus", 8, None),
        ("Instrumental", 8, Some("Guitar Lead")),
        ("Refrain", 8, None), // inherited via memory (bug #1 + #3)
        ("Refrain", 8, None),
    ];

    for (i, (ty, measures, comment)) in expected.iter().enumerate() {
        let s = &chart.sections[i].section;
        assert_eq!(s.section_type.full_name(), *ty, "section {i} type");
        assert_eq!(s.measure_count, Some(*measures), "section {i} measures");
        assert_eq!(
            s.comment.as_deref(),
            *comment,
            "section {i} comment"
        );
    }
}
