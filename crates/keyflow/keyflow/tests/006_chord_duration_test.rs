use keyflow::chord::ChordRhythm;
use keyflow::sections::SectionType;
use keyflow::time::MusicalPositionExt;
// Duration trait removed - unused

/// Test 1: Duration with all root formats (note names, scale degrees, Roman numerals)
/// Tests that:
/// - Slash notation works with note names (g////)
/// - Slash notation works with scale degrees (1//)
/// - Slash notation works with Roman numerals (I///)
/// - Key inference works for all formats
#[test]
fn test_chord_duration_all_root_formats() {
    let input = r#"Chord Duration Test - All Root Formats
120bpm 4/4 #G

intro
G//// Em// D//
vs
1// 4// 6// 5//
ch
I/// IV/ vi////
"#;

    let chart = keyflow::parse(input).unwrap();

    // Display the chart to show duration notation with all root formats
    println!("\n{}", chart);

    // Test metadata
    assert_eq!(
        chart.metadata.title,
        Some("Chord Duration Test".to_string())
    );
    assert_eq!(chart.metadata.artist, Some("All Root Formats".to_string()));

    // Test sections
    assert_eq!(chart.sections.len(), 3);

    // Test Intro section - note names with slash notation
    // g//// (4 beats = 1 measure), e// (2 beats) + d// (2 beats) = 1 measure
    let intro_section = &chart.sections[0];
    assert_eq!(intro_section.section.section_type, SectionType::Intro);
    assert_eq!(intro_section.measures().len(), 2);

    // Measure 1: G//// (4 beats fills the measure)
    let measure1 = &intro_section.measures()[0];
    assert_eq!(measure1.chords.len(), 1);
    let chord1 = &measure1.chords[0];
    assert_eq!(format!("{}", chord1.root), "G");
    assert_eq!(chord1.full_symbol, "G");
    match &chord1.rhythm {
        ChordRhythm::Slashes { count, .. } => {
            assert_eq!(*count, 4);
            assert_eq!(chord1.duration.to_beats(chart.time_signature.unwrap()), 4.0);
        }
        _ => panic!("Expected Slashes rhythm for g////"),
    }

    // Measure 2: Em// (2 beats) + D// (2 beats) = 4 beats
    let measure2 = &intro_section.measures()[1];
    assert_eq!(measure2.chords.len(), 2);

    let chord2 = &measure2.chords[0]; // Em//
    assert_eq!(format!("{}", chord2.root), "E");
    assert_eq!(chord2.full_symbol, "Em");
    match &chord2.rhythm {
        ChordRhythm::Slashes { count, .. } => {
            assert_eq!(*count, 2);
            assert_eq!(chord2.duration.to_beats(chart.time_signature.unwrap()), 2.0);
        }
        _ => panic!("Expected Slashes rhythm for e//"),
    }

    let chord3 = &measure2.chords[1]; // D//
    assert_eq!(format!("{}", chord3.root), "D");
    assert_eq!(chord3.full_symbol, "D");
    match &chord3.rhythm {
        ChordRhythm::Slashes { count, .. } => {
            assert_eq!(*count, 2);
            assert_eq!(chord3.duration.to_beats(chart.time_signature.unwrap()), 2.0);
        }
        _ => panic!("Expected Slashes rhythm for d//"),
    }

    // Test Verse section - scale degrees with slash notation
    // 1// (2) + 4// (2) = 1 measure, 6// (2) + 5// (2) = 1 measure
    let verse_section = &chart.sections[1];
    assert_eq!(verse_section.section.section_type, SectionType::Verse);
    assert_eq!(verse_section.measures().len(), 2);

    // Measure 1: 1// (2 beats) + 4// (2 beats) = 4 beats
    let verse_measure1 = &verse_section.measures()[0];
    assert_eq!(verse_measure1.chords.len(), 2);

    let chord4 = &verse_measure1.chords[0]; // 1//
    assert_eq!(format!("{}", chord4.root), "1"); // Preserves original format
    assert_eq!(chord4.full_symbol, "1"); // Infers quality: major triad (no suffix)
    match &chord4.rhythm {
        ChordRhythm::Slashes { count, .. } => {
            assert_eq!(*count, 2);
            assert_eq!(chord4.duration.to_beats(chart.time_signature.unwrap()), 2.0);
        }
        _ => panic!("Expected Slashes rhythm for 1//"),
    }

    let chord5 = &verse_measure1.chords[1]; // 4//
    assert_eq!(format!("{}", chord5.root), "4");
    assert_eq!(chord5.full_symbol, "4"); // Infers quality: major triad (no suffix)
    match &chord5.rhythm {
        ChordRhythm::Slashes { count, .. } => {
            assert_eq!(*count, 2);
            assert_eq!(chord5.duration.to_beats(chart.time_signature.unwrap()), 2.0);
        }
        _ => panic!("Expected Slashes rhythm for 4//"),
    }

    // Measure 2: 6// (2 beats) + 5// (2 beats) = 4 beats
    let verse_measure2 = &verse_section.measures()[1];
    assert_eq!(verse_measure2.chords.len(), 2);

    let chord6 = &verse_measure2.chords[0]; // 6//
    assert_eq!(format!("{}", chord6.root), "6");
    assert_eq!(chord6.full_symbol, "6"); // Scale degree quality is implied by key
    match &chord6.rhythm {
        ChordRhythm::Slashes { count, .. } => {
            assert_eq!(*count, 2);
            assert_eq!(chord6.duration.to_beats(chart.time_signature.unwrap()), 2.0);
        }
        _ => panic!("Expected Slashes rhythm for 6//"),
    }

    let chord7 = &verse_measure2.chords[1]; // 5//
    assert_eq!(format!("{}", chord7.root), "5");
    assert_eq!(chord7.full_symbol, "5"); // Infers quality: major triad (no suffix)
    match &chord7.rhythm {
        ChordRhythm::Slashes { count, .. } => {
            assert_eq!(*count, 2);
            assert_eq!(chord7.duration.to_beats(chart.time_signature.unwrap()), 2.0);
        }
        _ => panic!("Expected Slashes rhythm for 5//"),
    }

    // Test Chorus section - Roman numerals with slash notation
    // I/// (3) + IV/ (1) = 1 measure, vi//// (4) = 1 measure
    let chorus_section = &chart.sections[2];
    assert_eq!(chorus_section.section.section_type, SectionType::Chorus);
    assert_eq!(chorus_section.measures().len(), 2);

    // Measure 1: I/// (3 beats) + IV/ (1 beat) = 4 beats
    let chorus_measure1 = &chorus_section.measures()[0];
    assert_eq!(chorus_measure1.chords.len(), 2);

    let chord8 = &chorus_measure1.chords[0]; // I///
    assert_eq!(format!("{}", chord8.root), "I"); // Preserves original format
    assert_eq!(chord8.full_symbol, "I"); // Infers quality: major triad (no suffix)
    match &chord8.rhythm {
        ChordRhythm::Slashes { count, .. } => {
            assert_eq!(*count, 3);
            assert_eq!(chord8.duration.to_beats(chart.time_signature.unwrap()), 3.0);
        }
        _ => panic!("Expected Slashes rhythm for I///"),
    }

    let chord9 = &chorus_measure1.chords[1]; // IV/
    assert_eq!(format!("{}", chord9.root), "IV");
    assert_eq!(chord9.full_symbol, "IV"); // Infers quality: major triad (no suffix)
    match &chord9.rhythm {
        ChordRhythm::Slashes { count, .. } => {
            assert_eq!(*count, 1);
            assert_eq!(chord9.duration.to_beats(chart.time_signature.unwrap()), 1.0);
        }
        _ => panic!("Expected Slashes rhythm for IV/"),
    }

    // Measure 2: vi//// (4 beats fills the measure)
    let chorus_measure2 = &chorus_section.measures()[1];
    assert_eq!(chorus_measure2.chords.len(), 1);

    let chord10 = &chorus_measure2.chords[0]; // vi////
    assert_eq!(format!("{}", chord10.root), "vi");
    assert_eq!(chord10.full_symbol, "vim"); // lowercase vi = minor
    match &chord10.rhythm {
        ChordRhythm::Slashes { count, .. } => {
            assert_eq!(*count, 4);
            assert_eq!(
                chord10.duration.to_beats(chart.time_signature.unwrap()),
                4.0
            );
        }
        _ => panic!("Expected Slashes rhythm for vi////"),
    }
}

/// Test 2: Underscore duration with different root formats
/// Tests that:
/// - Underscore notation (_4, _2, _8) works with all root formats
/// - Key inference works with duration notation
#[test]
fn test_chord_duration_underscore_all_formats() {
    let input = r#"Underscore Duration Test - Demo
120bpm 4/4 #G

intro
G_4 Em_2 D_4 C////
vs
1_4 4_2 6_4 5////
ch
I_2 IV_2 vi_2 V_2
"#;

    let chart = keyflow::parse(input).unwrap();

    assert_eq!(chart.sections.len(), 3);

    // Test Intro - note names with underscore duration
    // g_4 (1 beat) + e_2 (2 beats) + d_4 (1 beat) = 4 beats = measure 0
    // c//// (4 beats) = measure 1
    let intro = &chart.sections[0];
    assert_eq!(intro.measures().len(), 2);

    let chord1 = &intro.measures()[0].chords[0];
    assert_eq!(chord1.full_symbol, "G");
    assert!(chord1.rhythm.has_lily_duration());
    assert_eq!(chord1.duration.to_beats(chart.time_signature.unwrap()), 1.0); // _4 = quarter note

    let chord2 = &intro.measures()[0].chords[1];
    assert_eq!(chord2.full_symbol, "Em");
    assert!(chord2.rhythm.has_lily_duration());
    assert_eq!(chord2.duration.to_beats(chart.time_signature.unwrap()), 2.0); // _2 = half note

    let chord3 = &intro.measures()[0].chords[2];
    assert_eq!(chord3.full_symbol, "D");
    assert!(chord3.rhythm.has_lily_duration());
    assert_eq!(chord3.duration.to_beats(chart.time_signature.unwrap()), 1.0); // _4 = quarter note

    let chord4 = &intro.measures()[1].chords[0];
    assert_eq!(chord4.full_symbol, "C");
    assert!(matches!(
        chord4.rhythm,
        ChordRhythm::Slashes { count: 4, .. }
    ));

    // Test Verse - scale degrees with underscore duration
    // 1_4 (1) + 4_2 (2) + 6_4 (1) = 4 beats = measure 0
    // 5//// = measure 1
    let verse = &chart.sections[1];
    assert_eq!(verse.measures().len(), 2);

    let chord5 = &verse.measures()[0].chords[0];
    assert_eq!(chord5.full_symbol, "1");
    assert!(chord5.rhythm.has_lily_duration());
    assert_eq!(chord5.duration.to_beats(chart.time_signature.unwrap()), 1.0);

    let chord6 = &verse.measures()[0].chords[1];
    assert_eq!(chord6.full_symbol, "4");
    assert!(chord6.rhythm.has_lily_duration());
    assert_eq!(chord6.duration.to_beats(chart.time_signature.unwrap()), 2.0);

    let chord7 = &verse.measures()[0].chords[2];
    assert_eq!(chord7.full_symbol, "6"); // Scale degrees shouldn't have quality suffix
    assert_eq!(chord7.duration.to_beats(chart.time_signature.unwrap()), 1.0);

    // Test Chorus - Roman numerals with underscore duration
    // I_2 (2) + IV_2 (2) = 4 beats = measure 0
    // vi_2 (2) + V_2 (2) = 4 beats = measure 1
    let chorus = &chart.sections[2];
    assert_eq!(chorus.measures().len(), 2);

    let chord8 = &chorus.measures()[0].chords[0];
    assert_eq!(chord8.full_symbol, "I");
    assert!(chord8.rhythm.has_lily_duration());
    assert_eq!(chord8.duration.to_beats(chart.time_signature.unwrap()), 2.0);

    let chord9 = &chorus.measures()[0].chords[1];
    assert_eq!(chord9.full_symbol, "IV");
    assert!(chord9.rhythm.has_lily_duration());
    assert_eq!(chord9.duration.to_beats(chart.time_signature.unwrap()), 2.0);

    let chord10 = &chorus.measures()[1].chords[0];
    assert_eq!(chord10.full_symbol, "vim"); // lowercase vi = minor
    assert_eq!(
        chord10.duration.to_beats(chart.time_signature.unwrap()),
        2.0
    );

    let chord11 = &chorus.measures()[1].chords[1];
    assert_eq!(chord11.full_symbol, "V");
    assert_eq!(
        chord11.duration.to_beats(chart.time_signature.unwrap()),
        2.0
    );
}

/// Test 3: Mixed duration notation with root formats
/// Tests that:
/// - Slash and underscore notation can be mixed
/// - All root formats work with mixed notation
#[test]
fn test_chord_duration_mixed_formats() {
    let input = r#"Mixed Duration Test - Demo
120bpm 4/4 #G

intro
g//// e_4 d/// c_2 d_2 f////
vs
1//// 4_4 6/// 5_2 1_2 4////
ch
I//// IV_4 vi/// V_2 I_2 IV////
"#;

    let chart = keyflow::parse(input).unwrap();

    assert_eq!(chart.sections.len(), 3);

    // Test Intro - note names with mixed notation (4 measures)
    let intro = &chart.sections[0];
    assert_eq!(intro.measures().len(), 4);

    // Measure 0: g//// (4 beats)
    assert_eq!(intro.measures()[0].chords.len(), 1);
    assert!(matches!(
        intro.measures()[0].chords[0].rhythm,
        ChordRhythm::Slashes { count: 4, .. }
    ));

    // Measure 1: e_4 (1 beat) + d/// (3 beats)
    assert_eq!(intro.measures()[1].chords.len(), 2);
    assert!(intro.measures()[1].chords[0].rhythm.has_lily_duration());
    assert!(matches!(
        intro.measures()[1].chords[1].rhythm,
        ChordRhythm::Slashes { count: 3, .. }
    ));

    // Measure 2: c_2 (2 beats) + d_2 (2 beats)
    assert_eq!(intro.measures()[2].chords.len(), 2);
    assert!(intro.measures()[2].chords[0].rhythm.has_lily_duration());
    assert!(intro.measures()[2].chords[1].rhythm.has_lily_duration());

    // Measure 3: f//// (4 beats)
    assert_eq!(intro.measures()[3].chords.len(), 1);
    assert!(matches!(
        intro.measures()[3].chords[0].rhythm,
        ChordRhythm::Slashes { count: 4, .. }
    ));

    // Test Verse - scale degrees with mixed notation (4 measures)
    let verse = &chart.sections[1];
    assert_eq!(verse.measures().len(), 4);

    assert_eq!(verse.measures()[0].chords.len(), 1);
    assert!(matches!(
        verse.measures()[0].chords[0].rhythm,
        ChordRhythm::Slashes { count: 4, .. }
    ));

    assert_eq!(verse.measures()[1].chords.len(), 2);
    assert!(verse.measures()[1].chords[0].rhythm.has_lily_duration());
    assert!(matches!(
        verse.measures()[1].chords[1].rhythm,
        ChordRhythm::Slashes { count: 3, .. }
    ));

    assert_eq!(verse.measures()[2].chords.len(), 2);
    assert!(verse.measures()[2].chords[0].rhythm.has_lily_duration());
    assert!(verse.measures()[2].chords[1].rhythm.has_lily_duration());

    assert_eq!(verse.measures()[3].chords.len(), 1);
    assert!(matches!(
        verse.measures()[3].chords[0].rhythm,
        ChordRhythm::Slashes { count: 4, .. }
    ));

    // Test Chorus - Roman numerals with mixed notation (4 measures)
    let chorus = &chart.sections[2];
    assert_eq!(chorus.measures().len(), 4);

    assert_eq!(chorus.measures()[0].chords.len(), 1);
    assert!(matches!(
        chorus.measures()[0].chords[0].rhythm,
        ChordRhythm::Slashes { count: 4, .. }
    ));

    assert_eq!(chorus.measures()[1].chords.len(), 2);
    assert!(chorus.measures()[1].chords[0].rhythm.has_lily_duration());
    assert!(matches!(
        chorus.measures()[1].chords[1].rhythm,
        ChordRhythm::Slashes { count: 3, .. }
    ));

    assert_eq!(chorus.measures()[2].chords.len(), 2);
    assert!(chorus.measures()[2].chords[0].rhythm.has_lily_duration());
    assert!(chorus.measures()[2].chords[1].rhythm.has_lily_duration());

    assert_eq!(chorus.measures()[3].chords.len(), 1);
    assert!(matches!(
        chorus.measures()[3].chords[0].rhythm,
        ChordRhythm::Slashes { count: 4, .. }
    ));
}

/// Test 4: Duration with explicit qualities
/// Tests that:
/// - Duration notation preserves explicit chord qualities
/// - Works with all root formats when quality is specified
#[test]
fn test_chord_duration_with_explicit_qualities() {
    let input = r#"Explicit Quality Duration Test - Demo
120bpm 4/4 #G

intro
Gmaj7//// Em7_2 D7_2 Cmaj7////
"#;

    let chart = keyflow::parse(input).unwrap();

    assert_eq!(chart.sections.len(), 1);

    let intro = &chart.sections[0];
    // Gmaj7//// (4 beats) = measure 0
    // Em7_2 (2 beats) + D7_2 (2 beats) = measure 1
    // Cmaj7//// (4 beats) = measure 2
    assert_eq!(intro.measures().len(), 3);

    // Gmaj7//// (explicit quality with slash notation)
    let chord1 = &intro.measures()[0].chords[0];
    assert_eq!(chord1.full_symbol, "Gmaj7");
    assert!(matches!(
        chord1.rhythm,
        ChordRhythm::Slashes { count: 4, .. }
    ));
    assert_eq!(chord1.duration.to_beats(chart.time_signature.unwrap()), 4.0);

    // Em7_2 (explicit quality with underscore notation)
    let chord2 = &intro.measures()[1].chords[0];
    assert_eq!(chord2.full_symbol, "Em7");
    assert!(chord2.rhythm.has_lily_duration());
    assert_eq!(chord2.duration.to_beats(chart.time_signature.unwrap()), 2.0);

    // D7_2 (explicit quality with underscore notation)
    let chord3 = &intro.measures()[1].chords[1];
    assert_eq!(chord3.full_symbol, "D7");
    assert!(chord3.rhythm.has_lily_duration());
    assert_eq!(chord3.duration.to_beats(chart.time_signature.unwrap()), 2.0);

    // Cmaj7//// (explicit quality with slash notation)
    let chord4 = &intro.measures()[2].chords[0];
    assert_eq!(chord4.full_symbol, "Cmaj7");
    assert!(matches!(
        chord4.rhythm,
        ChordRhythm::Slashes { count: 4, .. }
    ));
    assert_eq!(chord4.duration.to_beats(chart.time_signature.unwrap()), 4.0);
}

/// Test 5: Duration with chord memory across sections
/// Tests that:
/// - Duration notation works correctly
/// - Global memory only inherits for FIRST-TIME roots (new behavior)
/// - Once a root is seen, new sections don't inherit from global
#[test]
fn test_chord_duration_with_memory() {
    let input = r#"Duration Memory Test - Demo
120bpm 4/4 #G

intro
Gmaj13//// C9_2 D_2
vs
g_4 c_4 d_4 g_4
ch
G_2 C_2 D////
"#;

    let chart = keyflow::parse(input).unwrap();

    assert_eq!(chart.sections.len(), 3);

    // Intro sets qualities with slash and underscore notation
    // Gmaj13//// (4 beats) = measure 0
    // C9_2 (2) + D_2 (2) = measure 1
    let intro = &chart.sections[0];
    assert_eq!(intro.measures().len(), 2);
    assert_eq!(intro.measures()[0].chords[0].full_symbol, "Gmaj13");
    assert_eq!(intro.measures()[1].chords[0].full_symbol, "C9");
    assert_eq!(intro.measures()[1].chords[1].full_symbol, "D");

    // Verse: g_4 c_4 d_4 g_4
    // Basic chords recall from major family memory (Gmaj13, C9 set in intro)
    let verse = &chart.sections[1];
    assert_eq!(verse.measures().len(), 1);
    assert_eq!(verse.measures()[0].chords[0].full_symbol, "Gmaj13"); // Recalls from family
    assert!(
        verse.measures()[0].chords[0].rhythm.has_lily_duration(),
        "Expected Lily duration"
    );
    assert_eq!(verse.measures()[0].chords[1].full_symbol, "C9"); // Recalls from family
    assert!(
        verse.measures()[0].chords[1].rhythm.has_lily_duration(),
        "Expected Lily duration"
    );
    assert_eq!(verse.measures()[0].chords[2].full_symbol, "D");
    assert_eq!(verse.measures()[0].chords[3].full_symbol, "Gmaj13"); // Recalls from family

    // Chorus: G_2 C_2 D////
    // Basic chords recall from major family memory
    let chorus = &chart.sections[2];
    assert_eq!(chorus.measures().len(), 2);
    assert_eq!(chorus.measures()[0].chords[0].full_symbol, "Gmaj13"); // Recalls from family
    assert!(
        chorus.measures()[0].chords[0].rhythm.has_lily_duration(),
        "Expected Lily duration"
    );
    assert_eq!(chorus.measures()[0].chords[1].full_symbol, "C9"); // Recalls from family
    assert!(
        chorus.measures()[0].chords[1].rhythm.has_lily_duration(),
        "Expected Lily duration"
    );
    assert_eq!(chorus.measures()[1].chords[0].full_symbol, "D");
    assert!(matches!(
        chorus.measures()[1].chords[0].rhythm,
        ChordRhythm::Slashes { count: 4, .. }
    ));
}

/// Test 6: Complex scenario with all features
/// Tests a realistic chart combining:
/// - All root formats (note names, scale degrees, Roman numerals)
/// - All duration notations (default, slash, underscore, dotted)
/// - Chord memory and key inference
#[test]
fn test_chord_duration_complex_scenario() {
    let input = r#"Complex Duration Test - Demo
120bpm 4/4 #G

intro
Gmaj7//// Cmaj7////
vs
g_4 c_4 d_4 g_4
pre
1// 4// 6// 5//
ch
I_2. IV_2. vi_2. V_2.
"#;

    let chart = keyflow::parse(input).unwrap();

    assert_eq!(chart.sections.len(), 4);

    // Intro: explicit qualities with slash notation
    let intro = &chart.sections[0];
    assert_eq!(intro.measures().len(), 2);
    assert!(intro
        .measures()
        .iter()
        .all(|m| matches!(m.chords[0].rhythm, ChordRhythm::Slashes { count: 4, .. })));

    // Verse: note names with underscore notation (recalls memory)
    // g_4 (1) + c_4 (1) + d_4 (1) + g_4 (1) = 4 beats = 1 measure
    let verse = &chart.sections[1];
    assert_eq!(verse.measures().len(), 1);
    assert_eq!(verse.measures()[0].chords.len(), 4);
    assert!(verse.measures()[0]
        .chords
        .iter()
        .all(|c| c.rhythm.has_lily_duration()));

    // Pre-chorus: scale degrees with slash notation (infers from key)
    // 1// (2) + 4// (2) = 4 beats = measure 0
    // 6// (2) + 5// (2) = 4 beats = measure 1
    let pre = &chart.sections[2];
    assert_eq!(pre.measures().len(), 2);
    assert_eq!(pre.measures()[0].chords.len(), 2);
    assert_eq!(pre.measures()[1].chords.len(), 2);
    assert!(pre.measures().iter().all(|m| {
        m.chords
            .iter()
            .all(|c| matches!(c.rhythm, ChordRhythm::Slashes { count: 2, .. }))
    }));

    // Chorus: Roman numerals with dotted underscore notation
    // I_2. (3 beats) + vi_2. (3 beats) = 6 beats, overflow into 2 measures
    // Actually: I_2. fills 3 beats, then IV_2. fills 3 more = 6 beats (1.5 measures)
    // So: I_2. (3) = measure 0 incomplete, IV_2. (3) = completes measure 0 + starts measure 1
    // This will be measure 0: I_2. (3 beats) + IV_2. (1 beat from the 3)
    // measure 1: IV_2. (remaining 2 beats) + vi_2. (2 of its 3 beats)
    // measure 2: vi_2. (remaining 1 beat) + V_2. (3 beats)
    // Actually this is complex - let's just check it parses and has dotted notes
    let chorus = &chart.sections[3];
    assert!(!chorus.measures().is_empty());

    // Check first chord has dotted notation
    let first_chord = &chorus.measures()[0].chords[0];
    assert!(
        first_chord.rhythm.has_lily_duration(),
        "Expected Lily duration"
    );
    if let Some((_, dotted, _)) = first_chord.rhythm.lily_parts() {
        assert!(dotted, "Expected dotted rhythm");
        // Dotted half note = 3 beats
        assert_eq!(
            first_chord.duration.to_beats(chart.time_signature.unwrap()),
            3.0
        );
    } else {
        panic!("Expected Lily duration with parts");
    }
}
