use keyflow::chord::ChordRhythm;
use keyflow::sections::SectionType;
use keyflow::time::MusicalPositionExt;
// Duration trait removed - unused

/// Test 1: Basic duration syntax with underscore notation (_4, _2, _8)
/// Tests that:
/// - Lily-inspired duration syntax (_4 = quarter note, _2 = half note, _8 = eighth note)
/// - Durations are correctly parsed and stored
/// - Chord qualities are preserved with duration notation
#[test]
fn test_duration_underscore_syntax() {
    let input = r#"Duration Test - Demo
120bpm 4/4 #G

vs
Gmaj7_4 Cmaj7_2 Dm7_8 Em7_8
"#;

    let chart = keyflow::parse(input).unwrap();

    // Display the chart to show duration notation
    println!("\n{}", chart);

    // Test metadata
    assert_eq!(chart.metadata.title, Some("Duration Test".to_string()));
    assert_eq!(chart.metadata.artist, Some("Demo".to_string()));

    // Test sections
    assert_eq!(chart.sections.len(), 1);

    // Test Verse section
    // Gmaj7_4 (1) + Cmaj7_2 (2) + Dm7_8 (0.5) + Em7_8 (0.5) = 4 beats = 1 measure
    let verse_section = &chart.sections[0];
    assert_eq!(verse_section.section.section_type, SectionType::Verse);
    assert_eq!(verse_section.measures().len(), 1);

    // Single measure with all 4 chords
    let measure1 = &verse_section.measures()[0];
    assert_eq!(measure1.chords.len(), 4);

    // Test first chord: Gmaj7_4 (quarter note, 1 beat)
    let chord1 = &measure1.chords[0];
    assert_eq!(format!("{}", chord1.root), "G");
    assert_eq!(chord1.full_symbol, "Gmaj7");
    match &chord1.rhythm {
        ChordRhythm::Explicit(_) => {
            assert_eq!(chord1.duration.to_beats(chart.time_signature.unwrap()), 1.0);
        }
        _ => panic!("Expected Lily duration for Gmaj7_4"),
    }

    // Test second chord: Cmaj7_2 (half note, 2 beats)
    let chord2 = &measure1.chords[1];
    assert_eq!(format!("{}", chord2.root), "C");
    assert_eq!(chord2.full_symbol, "Cmaj7");
    match &chord2.rhythm {
        ChordRhythm::Explicit(_) => {
            assert_eq!(chord2.duration.to_beats(chart.time_signature.unwrap()), 2.0);
        }
        _ => panic!("Expected Lily duration for Cmaj7_2"),
    }

    // Test third chord: Dm7_8 (eighth note, 0.5 beats)
    let chord3 = &measure1.chords[2];
    assert_eq!(format!("{}", chord3.root), "D");
    assert_eq!(chord3.full_symbol, "Dm7");
    match &chord3.rhythm {
        ChordRhythm::Explicit(_) => {
            assert_eq!(chord3.duration.to_beats(chart.time_signature.unwrap()), 0.5);
        }
        _ => panic!("Expected Lily duration for Dm7_8"),
    }

    // Test fourth chord: Em7_8 (eighth note, 0.5 beats)
    let chord4 = &measure1.chords[3];
    assert_eq!(format!("{}", chord4.root), "E");
    assert_eq!(chord4.full_symbol, "Em7");
    match &chord4.rhythm {
        ChordRhythm::Explicit(_) => {
            assert_eq!(chord4.duration.to_beats(chart.time_signature.unwrap()), 0.5);
        }
        _ => panic!("Expected Lily duration for Em7_8"),
    }
}

/// Test 2: Slash notation for duration (/, //, ///, ////)
/// Tests that:
/// - Each slash represents one beat
/// - Slash notation works with explicit chord qualities
/// - Multiple chords can have different slash counts
#[test]
fn test_duration_slash_syntax() {
    let input = r#"Slash Duration Test - Demo
120bpm 4/4 #G

vs
Gmaj7//// Cmaj7// Dm7/ Em7/
"#;

    let chart = keyflow::parse(input).unwrap();

    // Test metadata
    assert_eq!(
        chart.metadata.title,
        Some("Slash Duration Test".to_string())
    );
    assert_eq!(chart.metadata.artist, Some("Demo".to_string()));

    // Test sections
    assert_eq!(chart.sections.len(), 1);

    // Test Verse section
    // Gmaj7//// (4 beats) = 1 measure, Cmaj7// (2) + Dm7/ (1) = 3 beats (partial measure)
    let verse_section = &chart.sections[0];
    assert_eq!(verse_section.section.section_type, SectionType::Verse);
    assert_eq!(verse_section.measures().len(), 2);

    // Measure 1: Gmaj7//// (4 beats fills the measure)
    let measure1 = &verse_section.measures()[0];
    assert_eq!(measure1.chords.len(), 1);
    let chord1 = &measure1.chords[0];
    assert_eq!(format!("{}", chord1.root), "G");
    assert_eq!(chord1.full_symbol, "Gmaj7");
    match &chord1.rhythm {
        ChordRhythm::Slashes { count, .. } => {
            assert_eq!(*count, 4);
            assert_eq!(chord1.duration.to_beats(chart.time_signature.unwrap()), 4.0);
        }
        _ => panic!("Expected Slashes rhythm for Gmaj7////"),
    }

    // Measure 2: Cmaj7// (2) + Dm7/ (1) + Em7/ (1) = 4 beats
    let measure2 = &verse_section.measures()[1];
    assert_eq!(measure2.chords.len(), 3);

    let chord2 = &measure2.chords[0]; // Cmaj7//
    assert_eq!(format!("{}", chord2.root), "C");
    assert_eq!(chord2.full_symbol, "Cmaj7");
    match &chord2.rhythm {
        ChordRhythm::Slashes { count, .. } => {
            assert_eq!(*count, 2);
            assert_eq!(chord2.duration.to_beats(chart.time_signature.unwrap()), 2.0);
        }
        _ => panic!("Expected Slashes rhythm for Cmaj7//"),
    }

    let chord3 = &measure2.chords[1]; // Dm7/
    assert_eq!(format!("{}", chord3.root), "D");
    assert_eq!(chord3.full_symbol, "Dm7");
    match &chord3.rhythm {
        ChordRhythm::Slashes { count, .. } => {
            assert_eq!(*count, 1);
            assert_eq!(chord3.duration.to_beats(chart.time_signature.unwrap()), 1.0);
        }
        _ => panic!("Expected Slashes rhythm for Dm7/"),
    }

    let chord4 = &measure2.chords[2]; // Em7/
    assert_eq!(format!("{}", chord4.root), "E");
    assert_eq!(chord4.full_symbol, "Em7");
    match &chord4.rhythm {
        ChordRhythm::Slashes { count, .. } => {
            assert_eq!(*count, 1);
            assert_eq!(chord4.duration.to_beats(chart.time_signature.unwrap()), 1.0);
        }
        _ => panic!("Expected Slashes rhythm for Em7/"),
    }
}

/// Test 3: Mixed duration syntax
/// Tests that:
/// - Underscore and slash notation can be mixed
/// - Different duration types work together
/// - Chord memory works with durations
#[test]
fn test_duration_mixed_syntax() {
    let input = r#"Mixed Duration Test - Demo
120bpm 4/4 #G

vs
Gmaj7_4 C//// Dm7_2 Em7/
"#;

    let chart = keyflow::parse(input).unwrap();

    // Test metadata
    assert_eq!(
        chart.metadata.title,
        Some("Mixed Duration Test".to_string())
    );
    assert_eq!(chart.metadata.artist, Some("Demo".to_string()));

    // Test sections
    assert_eq!(chart.sections.len(), 1);

    // Test Verse section
    // Gmaj7_4 (1) = partial, C//// (4) would exceed so m1=Gmaj7_4, m2=C////, Dm7_2(2)+Em7/(1)=m3
    let verse_section = &chart.sections[0];
    assert_eq!(verse_section.section.section_type, SectionType::Verse);
    assert_eq!(verse_section.measures().len(), 3);

    // Measure 1: Gmaj7_4 (1 beat, partial measure)
    let measure1 = &verse_section.measures()[0];
    assert_eq!(measure1.chords.len(), 1);
    let chord1 = &measure1.chords[0];
    assert_eq!(format!("{}", chord1.root), "G");
    assert_eq!(chord1.full_symbol, "Gmaj7");
    assert!(matches!(chord1.rhythm, ChordRhythm::Explicit(_)));
    assert_eq!(chord1.duration.to_beats(chart.time_signature.unwrap()), 1.0);

    // Measure 2: C//// (4 beats fills the measure)
    let measure2 = &verse_section.measures()[1];
    assert_eq!(measure2.chords.len(), 1);
    let chord2 = &measure2.chords[0];
    assert_eq!(format!("{}", chord2.root), "C");
    assert_eq!(chord2.full_symbol, "C");
    assert!(matches!(
        chord2.rhythm,
        ChordRhythm::Slashes { count: 4, .. }
    ));
    assert_eq!(chord2.duration.to_beats(chart.time_signature.unwrap()), 4.0);

    // Measure 3: Dm7_2 (2 beats) + Em7/ (1 beat) = 3 beats (partial measure)
    let measure3 = &verse_section.measures()[2];
    assert_eq!(measure3.chords.len(), 2);

    let chord3 = &measure3.chords[0]; // Dm7_2
    assert_eq!(format!("{}", chord3.root), "D");
    assert_eq!(chord3.full_symbol, "Dm7");
    assert!(matches!(chord3.rhythm, ChordRhythm::Explicit(_)));
    assert_eq!(chord3.duration.to_beats(chart.time_signature.unwrap()), 2.0);

    let chord4 = &measure3.chords[1]; // Em7/
    assert_eq!(format!("{}", chord4.root), "E");
    assert_eq!(chord4.full_symbol, "Em7");
    assert!(matches!(
        chord4.rhythm,
        ChordRhythm::Slashes { count: 1, .. }
    ));
    assert_eq!(chord4.duration.to_beats(chart.time_signature.unwrap()), 1.0);
}

/// Test 4: Dotted durations (_4., _2., _8.)
/// Tests that:
/// - Dotted notation extends duration by 1.5x
/// - Works with various note values
#[test]
fn test_duration_dotted_notes() {
    let input = r#"Dotted Duration Test - Demo
120bpm 4/4 #G

vs
Gmaj7_4. Cmaj7_2. Dm7_8.
"#;

    let chart = keyflow::parse(input).unwrap();

    // Test metadata
    assert_eq!(
        chart.metadata.title,
        Some("Dotted Duration Test".to_string())
    );

    // Test sections
    assert_eq!(chart.sections.len(), 1);

    // Test Verse section
    // Gmaj7_4. (1.5) alone, then Cmaj7_2. (3) + Dm7_8. (0.75) = 3.75
    let verse_section = &chart.sections[0];
    assert_eq!(verse_section.measures().len(), 2);

    // Measure 1: Gmaj7_4. (dotted quarter = 1.5 beats, partial measure)
    let measure1 = &verse_section.measures()[0];
    assert_eq!(measure1.chords.len(), 1);
    let chord1 = &measure1.chords[0];
    assert_eq!(chord1.full_symbol, "Gmaj7");
    if let Some((_, dotted, _)) = chord1.rhythm.lily_parts() {
        assert!(dotted);
        assert_eq!(chord1.duration.to_beats(chart.time_signature.unwrap()), 1.5);
    } else {
        panic!("Expected Lily duration with dot");
    }

    // Measure 2: Cmaj7_2. (3 beats) + Dm7_8. (0.75 beats) = 3.75 beats (partial measure)
    let measure2 = &verse_section.measures()[1];
    assert_eq!(measure2.chords.len(), 2);

    let chord2 = &measure2.chords[0]; // Cmaj7_2.
    assert_eq!(chord2.full_symbol, "Cmaj7");
    if let Some((_, dotted, _)) = chord2.rhythm.lily_parts() {
        assert!(dotted);
        assert_eq!(chord2.duration.to_beats(chart.time_signature.unwrap()), 3.0);
    } else {
        panic!("Expected Lily duration with dot");
    }

    let chord3 = &measure2.chords[1]; // Dm7_8.
    assert_eq!(chord3.full_symbol, "Dm7");
    if let Some((_, dotted, _)) = chord3.rhythm.lily_parts() {
        assert!(dotted);
        assert_eq!(
            chord3.duration.to_beats(chart.time_signature.unwrap()),
            0.75
        );
    } else {
        panic!("Expected Lily duration with dot");
    }
}

/// Test 5: Default duration (no notation = full measure)
/// Tests that:
/// - Chords without duration notation default to full measure
/// - Default duration respects time signature
#[test]
fn test_duration_default() {
    let input = r#"Default Duration Test - Demo
120bpm 4/4 #G

vs
Gmaj7 Cmaj7 Dm7 Em7
"#;

    let chart = keyflow::parse(input).unwrap();

    // Test metadata
    assert_eq!(
        chart.metadata.title,
        Some("Default Duration Test".to_string())
    );

    // Test sections
    assert_eq!(chart.sections.len(), 1);

    // Test Verse section
    let verse_section = &chart.sections[0];
    assert_eq!(verse_section.measures().len(), 4);

    // All chords should have default duration (full measure in 4/4 = 4 beats)
    for (i, measure) in verse_section.measures().iter().enumerate() {
        let chord = &measure.chords[0];
        assert!(
            matches!(chord.rhythm, ChordRhythm::Default),
            "Chord {} should have Default rhythm",
            i
        );
        assert_eq!(
            chord.duration.to_beats(chart.time_signature.unwrap()),
            4.0,
            "Chord {} should have 4 beats duration",
            i
        );
    }
}

/// Test 6: Duration with explicit chord quality
/// Tests that:
/// - Duration notation works with explicit chord qualities
/// - Both underscore and slash notation preserve chord quality
#[test]
fn test_duration_with_chord_quality() {
    let input = r#"Duration Quality Test - Demo
120bpm 4/4 #G

vs
Gmaj13_4 C9_2 Gmaj13// C9/
"#;

    let chart = keyflow::parse(input).unwrap();

    assert_eq!(chart.sections.len(), 1);

    let verse_section = &chart.sections[0];
    assert_eq!(verse_section.measures().len(), 2);

    // Measure 1: Gmaj13_4 (1) + C9_2 (2) = 3 beats (partial)
    let measure1 = &verse_section.measures()[0];
    assert_eq!(measure1.chords.len(), 2);

    let chord1 = &measure1.chords[0]; // Gmaj13_4
    assert_eq!(chord1.full_symbol, "Gmaj13");
    assert!(matches!(chord1.rhythm, ChordRhythm::Explicit(_)));
    assert_eq!(chord1.duration.to_beats(chart.time_signature.unwrap()), 1.0);

    let chord2 = &measure1.chords[1]; // C9_2
    assert_eq!(chord2.full_symbol, "C9");
    assert!(matches!(chord2.rhythm, ChordRhythm::Explicit(_)));
    assert_eq!(chord2.duration.to_beats(chart.time_signature.unwrap()), 2.0);

    // Measure 2: Gmaj13// (2) + C9/ (1) = 3 beats (partial)
    let measure2 = &verse_section.measures()[1];
    assert_eq!(measure2.chords.len(), 2);

    let chord3 = &measure2.chords[0]; // Gmaj13//
    assert_eq!(chord3.full_symbol, "Gmaj13");
    assert!(matches!(
        chord3.rhythm,
        ChordRhythm::Slashes { count: 2, .. }
    ));
    assert_eq!(chord3.duration.to_beats(chart.time_signature.unwrap()), 2.0);

    let chord4 = &measure2.chords[1]; // C9/
    assert_eq!(chord4.full_symbol, "C9");
    assert!(matches!(
        chord4.rhythm,
        ChordRhythm::Slashes { count: 1, .. }
    ));
    assert_eq!(chord4.duration.to_beats(chart.time_signature.unwrap()), 1.0);
}

/// Test 7: Different time signatures affect duration
/// Tests that:
/// - Default duration adapts to time signature
/// - Beat calculations respect time signature
#[test]
fn test_duration_different_time_signatures() {
    let input_4_4 = r#"4/4 Test - Demo
120bpm 4/4 #G
vs
Gmaj7
"#;

    let input_3_4 = r#"3/4 Test - Demo
120bpm 3/4 #G
vs
Gmaj7
"#;

    let input_6_8 = r#"6/8 Test - Demo
120bpm 6/8 #G
vs
Gmaj7
"#;

    // Test 4/4
    let chart_4_4 = keyflow::parse(input_4_4).unwrap();
    let chord_4_4 = &chart_4_4.sections[0].measures()[0].chords[0];
    assert_eq!(
        chord_4_4
            .duration
            .to_beats(chart_4_4.time_signature.unwrap()),
        4.0
    );

    // Test 3/4
    let chart_3_4 = keyflow::parse(input_3_4).unwrap();
    let chord_3_4 = &chart_3_4.sections[0].measures()[0].chords[0];
    assert_eq!(
        chord_3_4
            .duration
            .to_beats(chart_3_4.time_signature.unwrap()),
        3.0
    );

    // Test 6/8
    let chart_6_8 = keyflow::parse(input_6_8).unwrap();
    let chord_6_8 = &chart_6_8.sections[0].measures()[0].chords[0];
    assert_eq!(
        chord_6_8
            .duration
            .to_beats(chart_6_8.time_signature.unwrap()),
        6.0
    );
}

/// Test 8: Complex duration scenario with multiple sections
/// Tests a realistic chart with various duration notations
#[test]
fn test_duration_complex_chart() {
    let input = r#"Complex Chart - Demo
120bpm 4/4 #G

intro
Gmaj7//// Cmaj7////
vs
G_4 C_4 D_4 G_4
ch
Gmaj7_2 Cmaj7_2 Dm7_2 Em7_2
br
G// C// D/ Em7/
"#;

    let chart = keyflow::parse(input).unwrap();

    assert_eq!(chart.sections.len(), 4);

    // Intro: slash notation (4 beats each)
    let intro = &chart.sections[0];
    assert_eq!(
        intro.measures()[0].chords[0]
            .duration
            .to_beats(chart.time_signature.unwrap()),
        4.0
    );
    assert_eq!(
        intro.measures()[1].chords[0]
            .duration
            .to_beats(chart.time_signature.unwrap()),
        4.0
    );

    // Verse: 1 measure with 4 quarter notes (1 beat each)
    let verse = &chart.sections[1];
    assert_eq!(verse.measures().len(), 1);
    assert_eq!(verse.measures()[0].chords.len(), 4);
    assert_eq!(
        verse.measures()[0].chords[0]
            .duration
            .to_beats(chart.time_signature.unwrap()),
        1.0
    );

    // Chorus: 2 measures, each with 2 half notes (2 beats each)
    let chorus = &chart.sections[2];
    assert_eq!(chorus.measures().len(), 2);
    assert_eq!(chorus.measures()[0].chords.len(), 2);
    assert_eq!(
        chorus.measures()[0].chords[0]
            .duration
            .to_beats(chart.time_signature.unwrap()),
        2.0
    );

    // Bridge: 2 measures (G//+C//=4 beats, D/+Em7/=2 beats partial)
    let bridge = &chart.sections[3];
    assert_eq!(bridge.measures().len(), 2);
    assert_eq!(bridge.measures()[0].chords.len(), 2); // G// + C//
    assert_eq!(
        bridge.measures()[0].chords[0]
            .duration
            .to_beats(chart.time_signature.unwrap()),
        2.0
    ); // G//
    assert_eq!(
        bridge.measures()[0].chords[1]
            .duration
            .to_beats(chart.time_signature.unwrap()),
        2.0
    ); // C//
    assert_eq!(bridge.measures()[1].chords.len(), 2); // D/ + Em7/
    assert_eq!(
        bridge.measures()[1].chords[0]
            .duration
            .to_beats(chart.time_signature.unwrap()),
        1.0
    ); // D/
    assert_eq!(
        bridge.measures()[1].chords[1]
            .duration
            .to_beats(chart.time_signature.unwrap()),
        1.0
    ); // Em7/
}
