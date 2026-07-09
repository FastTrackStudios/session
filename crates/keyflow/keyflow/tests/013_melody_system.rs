//! Test 013: Melody System
//!
//! Tests the melody notation system including:
//! - m{ } melody blocks
//! - $varName melody references
//! - Scale degrees (1-7)
//! - Octave modifiers (' and ,)
//! - Relative pitch mode

#[test]
fn test_melody_variable_definition() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
mainRiff = m{ C_8 D_8 E_4 }
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // Should have the melody variable defined
    assert!(chart.melody_variables.get("mainRiff").is_some());

    let melody = chart.melody_variables.get("mainRiff").unwrap();
    assert_eq!(melody.notes.len(), 3);
    assert_eq!(melody.notes[0].pitch, "C");
    assert_eq!(melody.notes[1].pitch, "D");
    assert_eq!(melody.notes[2].pitch, "E");
}

#[test]
fn test_inline_melody_in_chord_line() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// m{ C_8 D_8 E_4 F_4 } Dm7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // The measure should have a melody attached
    let first_measure = &chart.sections[0].measures()[0];
    assert_eq!(first_measure.melodies.len(), 1);

    let melody = &first_measure.melodies[0];
    assert_eq!(melody.notes.len(), 4);
}

#[test]
fn test_parallel_measure_container_parses() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 4
<< Cmaj7/// ; m{ r2 r4t Bb4t B4t } >> | Dm7///
"#;

    let chart = keyflow::parse(input).unwrap();
    let first_measure = &chart.sections[0].measures()[0];

    assert_eq!(first_measure.chords.len(), 1);
    assert_eq!(first_measure.melodies.len(), 1);
    assert_eq!(first_measure.chords[0].full_symbol, "Cmaj7");
    // Post-processing resolves relative melody notes to absolute octaves (via
    // `resolve_melody_octaves`), which Display renders with the `:` octave form.
    // From the default C4 reference, Bb and B both resolve to octave 3.
    assert_eq!(
        format!("{}", first_measure.melodies[0]),
        "m{ r2 r4t Bb:34t B:34t }"
    );
}

#[test]
#[ignore = "lineage divergence: MIDI-import/chart-Display canonicality undecided"]
fn test_parallel_measure_container_round_trips() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 4
<< Cmaj7/// ; m{ r2 r4t Bb4t B4t } >> | Dm7///
"#;

    let chart = keyflow::parse(input).unwrap();
    let rendered = format!("{}", chart);

    // Octaves are resolved to absolute during post-processing and render via `:`.
    assert!(rendered.contains("<< Cmaj7/// ; m{ r2 r4t Bb:34t B:34t } >>"));
}

#[test]
fn test_melody_variable_reference() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
mainRiff = m{ C_8 D_8 E_4 }
Cmaj7/// $mainRiff Dm7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // The melody variable should be defined
    assert!(chart.melody_variables.get("mainRiff").is_some());

    // The first measure should have the melody attached via reference
    let first_measure = &chart.sections[0].measures()[0];
    assert_eq!(first_measure.melodies.len(), 1);
}

#[test]
fn test_melody_with_scale_degrees() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 4
riff = m{ 1_4 3_4 5_4 1'_4 }
Cmaj7/// $riff
"#;

    let chart = keyflow::parse(input).unwrap();

    let melody = chart.melody_variables.get("riff").unwrap();
    assert_eq!(melody.notes.len(), 4);

    // 1 = C, 3 = E, 5 = G
    assert_eq!(melody.notes[0].pitch, "C");
    assert_eq!(melody.notes[0].scale_degree, Some(1));

    assert_eq!(melody.notes[1].pitch, "E");
    assert_eq!(melody.notes[1].scale_degree, Some(3));

    assert_eq!(melody.notes[2].pitch, "G");
    assert_eq!(melody.notes[2].scale_degree, Some(5));

    // 1' should be C with octave up modifier
    assert_eq!(melody.notes[3].pitch, "C");
    assert_eq!(melody.notes[3].scale_degree, Some(1));
    assert_eq!(
        melody.notes[3].octave_modifier,
        keyflow::chart::OctaveModifier::Up
    );
}

#[test]
fn test_melody_with_octave_modifiers() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 4
riff = m{ C_4 G'_4 C,_4 E_4 }
Cmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();

    let melody = chart.melody_variables.get("riff").unwrap();
    assert_eq!(melody.notes.len(), 4);

    // G' should have octave up modifier
    assert_eq!(melody.notes[1].pitch, "G");
    assert_eq!(
        melody.notes[1].octave_modifier,
        keyflow::chart::OctaveModifier::Up
    );

    // C, should have octave down modifier
    assert_eq!(melody.notes[2].pitch, "C");
    assert_eq!(
        melody.notes[2].octave_modifier,
        keyflow::chart::OctaveModifier::Down
    );
}

#[test]
fn test_melody_with_accidentals() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 4
blues = m{ C_4 Eb_4 F_4 F#_4 G_4 Bb_4 C'_4 }
Cmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();

    let melody = chart.melody_variables.get("blues").unwrap();
    assert_eq!(melody.notes.len(), 7);

    assert_eq!(melody.notes[1].pitch, "Eb");
    assert_eq!(melody.notes[3].pitch, "F#");
    assert_eq!(melody.notes[5].pitch, "Bb");
}

#[test]
fn test_melody_with_rests() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 4
phrase = m{ C_4 r_4 E_4 r_4 }
Cmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();

    let melody = chart.melody_variables.get("phrase").unwrap();
    assert_eq!(melody.notes.len(), 4);

    assert!(!melody.notes[0].is_rest());
    assert!(melody.notes[1].is_rest());
    assert!(!melody.notes[2].is_rest());
    assert!(melody.notes[3].is_rest());
}

#[test]
fn test_melody_with_dotted_notes() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 4
swing = m{ C_4. D_8 E_4. F_8 }
Cmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();

    let melody = chart.melody_variables.get("swing").unwrap();
    assert_eq!(melody.notes.len(), 4);

    assert!(melody.notes[0].dotted);
    assert!(!melody.notes[1].dotted);
    assert!(melody.notes[2].dotted);
    assert!(!melody.notes[3].dotted);
}

#[test]
fn test_multiple_melodies_in_chart() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
intro = m{ C_4 D_4 E_4 F_4 }
outro = m{ G_4 F_4 E_4 D_4 }
Cmaj7/// $intro Dm7/// $outro
"#;

    let chart = keyflow::parse(input).unwrap();

    // Both melodies should be defined
    assert!(chart.melody_variables.get("intro").is_some());
    assert!(chart.melody_variables.get("outro").is_some());

    // First measure should have intro melody
    let first_measure = &chart.sections[0].measures()[0];
    assert_eq!(first_measure.melodies.len(), 1);

    // Second measure should have outro melody
    let second_measure = &chart.sections[0].measures()[1];
    assert_eq!(second_measure.melodies.len(), 1);
}

#[test]
fn test_melody_round_trip() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
mainRiff = m{ C_8 D_8 E_4 F_4 }
Cmaj7/// $mainRiff Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("Original parsed chart:");
    println!("{}", chart);

    // Serialize the chart
    let syntax = chart.to_syntax();
    println!("Serialized syntax:");
    println!("{}", syntax);

    // Re-parse the serialized syntax
    let reparsed = keyflow::parse(&syntax).unwrap();
    println!("Re-parsed chart:");
    println!("{}", reparsed);

    // Verify the melody variable exists in both
    assert!(chart.melody_variables.get("mainRiff").is_some());
    assert!(reparsed.melody_variables.get("mainRiff").is_some());

    // Verify the melody content matches
    let original_melody = chart.melody_variables.get("mainRiff").unwrap();
    let reparsed_melody = reparsed.melody_variables.get("mainRiff").unwrap();
    assert_eq!(original_melody.notes.len(), reparsed_melody.notes.len());
}

#[test]
fn test_inline_melody_serialization() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 4
Cmaj7/// m{ C_4 E_4 G_4 C'_4 } Dm7///
"#;

    let chart = keyflow::parse(input).unwrap();
    let syntax = chart.to_syntax();
    println!("Serialized: {}", syntax);

    // The syntax should contain the inline melody block
    assert!(syntax.contains("m{"));
    assert!(syntax.contains("}"));
}
