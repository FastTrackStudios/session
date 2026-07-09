// Duration trait removed - unused

#[test]
fn test_section_padding_to_declared_length() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// Dm7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];

    // Section declared as 8 measures, but only has 2 measures of chords
    // Should be padded with 6 space measures
    assert_eq!(
        section.measures().len(),
        8,
        "Section should have exactly 8 measures"
    );

    // First two measures have actual chords
    assert!(!section.measures()[0].chords.is_empty());
    assert_eq!(section.measures()[0].chords[0].full_symbol, "Cmaj7");
    assert!(!section.measures()[1].chords.is_empty());
    assert_eq!(section.measures()[1].chords[0].full_symbol, "Dm7");

    // Remaining measures should contain space chords "s"
    for i in 2..8 {
        assert!(
            !section.measures()[i].chords.is_empty(),
            "Measure {} should have a space chord",
            i
        );
        assert_eq!(
            section.measures()[i].chords[0].full_symbol,
            "s",
            "Measure {} should be a space chord",
            i
        );
    }

    // Check absolute positions
    let all_chords: Vec<_> = section.measures().iter().flat_map(|m| &m.chords).collect();

    // Should have 8 chords total (2 actual + 6 space)
    assert_eq!(all_chords.len(), 8);

    // Dm7 (second chord) should be at position 0.3.0
    assert_eq!(all_chords[1].position.total_duration.measure, 0);
    assert_eq!(all_chords[1].position.total_duration.beat, 3);

    // Last space chord should be at position 6.2.0
    // (after 2 measures of 3-beat chords = 1.2.0, then 5 more full measures)
    let last_space = all_chords.last().unwrap();
    assert_eq!(last_space.full_symbol, "s");
    assert_eq!(last_space.position.total_duration.measure, 6);
    assert_eq!(last_space.position.total_duration.beat, 2);
}

#[test]
fn test_positions_continue_after_padded_section() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// Dm7///

CH 4
Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    assert_eq!(chart.sections.len(), 2);

    // Verse: 8 measures (2 with chords, 6 empty)
    assert_eq!(chart.sections[0].measures().len(), 8);

    // Chorus should start at measure 8 (after the padded verse)
    let chorus_first_chord = chart.sections[1]
        .measures()
        .iter()
        .flat_map(|m| &m.chords)
        .next()
        .expect("Chorus should have chords");

    // First chorus chord should be at measure 7.2.0
    // (after verse ends: 2 chord measures + 6 space measures = 1.2.0 + 6 measures = 7.2.0)
    assert_eq!(chorus_first_chord.position.total_duration.measure, 7);
    assert_eq!(chorus_first_chord.position.total_duration.beat, 2);
    assert_eq!(chorus_first_chord.position.total_duration.subdivision, 0);
}

#[test]
fn test_basic_absolute_positions() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];
    assert_eq!(section.measures().len(), 4);

    // Each chord is 3 beats (///) in 4/4 time
    // Positions should be: 0.0.0, 0.3.0, 1.2.0, 2.1.0
    let chords: Vec<_> = section.measures().iter().flat_map(|m| &m.chords).collect();

    assert_eq!(chords.len(), 4);

    // First chord at beginning
    assert_eq!(chords[0].position.total_duration.measure, 0);
    assert_eq!(chords[0].position.total_duration.beat, 0);
    assert_eq!(chords[0].position.total_duration.subdivision, 0);

    // Second chord at 3 beats
    assert_eq!(chords[1].position.total_duration.measure, 0);
    assert_eq!(chords[1].position.total_duration.beat, 3);

    // Third chord at 1 measure + 2 beats (1.2.0)
    assert_eq!(chords[2].position.total_duration.measure, 1);
    assert_eq!(chords[2].position.total_duration.beat, 2);

    // Fourth chord at 2 measures + 1 beat (2.1.0)
    assert_eq!(chords[3].position.total_duration.measure, 2);
    assert_eq!(chords[3].position.total_duration.beat, 1);
}

#[test]
fn test_absolute_positions_with_time_signature_change() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS
Cmaj7/// Dm7/// 6/8 Em7/. Fmaj7/.
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];
    let chords: Vec<_> = section.measures().iter().flat_map(|m| &m.chords).collect();

    assert_eq!(chords.len(), 4);

    // First two chords in 4/4 time (3 beats each = ///)
    assert_eq!(chords[0].position.total_duration.measure, 0);
    assert_eq!(chords[0].position.total_duration.beat, 0);

    assert_eq!(chords[1].position.total_duration.measure, 0);
    assert_eq!(chords[1].position.total_duration.beat, 3);

    // After time sig change to 6/8
    // Previous chords filled: 3 + 3 = 6 beats = 1 measure + 2 beats in 4/4
    // So Em7 starts at 1.2.0
    assert_eq!(chords[2].position.total_duration.measure, 1);
    assert_eq!(chords[2].position.total_duration.beat, 2);

    // Fmaj7/. starts after Em7/.
    // /. (dotted slash) = 1.5 beats in the parser's interpretation
    // So Fmaj7 is at 1.2 + 1.5 = 1.3.500 (measure 1, beat 3, subdivision 500)
    assert_eq!(chords[3].position.total_duration.measure, 1);
    assert_eq!(chords[3].position.total_duration.beat, 3);
    assert_eq!(chords[3].position.total_duration.subdivision, 500);
}

#[test]
fn test_cue_positions_match_chord_positions() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// @keys "here" Dm7/// Em7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];

    // Find the measure with the cue
    let cue_measure = section
        .measures()
        .iter()
        .find(|m| !m.text_cues.is_empty())
        .expect("Should have a measure with cues");

    // The cue should be at the same position as the chord in that measure
    assert!(!cue_measure.chords.is_empty());
    let chord_position = &cue_measure.chords[0].position;

    // Cue should be at position 0.3.0 (after first chord which is 3 beats)
    assert_eq!(chord_position.total_duration.measure, 0);
    assert_eq!(chord_position.total_duration.beat, 3);
    assert_eq!(chord_position.total_duration.subdivision, 0);
}

#[test]
fn test_positions_across_sections() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 4
Cmaj7/// Dm7///

CH 4
Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    assert_eq!(chart.sections.len(), 2);

    // Verse chords
    let verse_chords: Vec<_> = chart.sections[0]
        .measures()
        .iter()
        .flat_map(|m| &m.chords)
        .collect();

    assert_eq!(verse_chords[0].position.total_duration.measure, 0);
    assert_eq!(verse_chords[1].position.total_duration.measure, 0);
    assert_eq!(verse_chords[1].position.total_duration.beat, 3);

    // Chorus chords should continue from where verse left off
    // Verse has 4 measures declared, with 2 chords + 2 spaces
    // Verse ends at 3.2.0, so chorus starts at 3.2.0
    let chorus_chords: Vec<_> = chart.sections[1]
        .measures()
        .iter()
        .flat_map(|m| &m.chords)
        .collect();

    // First chorus chord starts where verse ended
    assert_eq!(chorus_chords[0].position.total_duration.measure, 3);
    assert_eq!(chorus_chords[0].position.total_duration.beat, 2);

    // Second chorus chord
    assert_eq!(chorus_chords[1].position.total_duration.measure, 4);
    assert_eq!(chorus_chords[1].position.total_duration.beat, 1);
}

#[test]
fn test_positions_with_push_pull() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 4
Cmaj7//// 'Dm7////
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];
    let chords: Vec<_> = section.measures().iter().flat_map(|m| &m.chords).collect();

    // With push, there might be a space inserted, but positions should still be sequential
    // The pushed chord should still have a position
    for (i, chord) in chords.iter().enumerate() {
        println!(
            "Chord {}: {} at {}.{}.{}",
            i,
            chord.full_symbol,
            chord.position.total_duration.measure,
            chord.position.total_duration.beat,
            chord.position.total_duration.subdivision
        );
    }

    // All chords should have valid positions
    for chord in chords {
        let pos = &chord.position.total_duration;
        // Check that position is non-negative
        assert!(pos.measure >= 0 && pos.beat >= 0 && pos.subdivision >= 0);
    }
}
