//! Test 012: Tempo Changes
//!
//! Tests the ->NNNbpm inline tempo change syntax

#[test]
fn test_inline_tempo_change() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// Dm7/// ->140bpm Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // Initial tempo should be 120
    assert!(chart.tempo.is_some());

    // Should have one tempo change
    assert_eq!(chart.tempo_changes.len(), 1);

    // The tempo change should be to 140
    assert_eq!(chart.tempo_changes[0].to_tempo.bpm as u32, 140);

    // The from_tempo should be 120
    assert!(chart.tempo_changes[0].from_tempo.is_some());
    assert_eq!(chart.tempo_changes[0].from_tempo.unwrap().bpm as u32, 120);
}

#[test]
fn test_tempo_change_syntax_variations() {
    // Test with bpm suffix
    let input1 = r#"
Test Song - Artist
100bpm 4/4 #C

VS 4
Cmaj7/// ->150bpm Dm7///
"#;

    let chart1 = keyflow::parse(input1).unwrap();
    assert_eq!(chart1.tempo_changes.len(), 1);
    assert_eq!(chart1.tempo_changes[0].to_tempo.bpm as u32, 150);

    // Test with BPM suffix (uppercase)
    let input2 = r#"
Test Song - Artist
100bpm 4/4 #C

VS 4
Cmaj7/// ->160BPM Dm7///
"#;

    let chart2 = keyflow::parse(input2).unwrap();
    assert_eq!(chart2.tempo_changes.len(), 1);
    assert_eq!(chart2.tempo_changes[0].to_tempo.bpm as u32, 160);

    // Test without suffix
    let input3 = r#"
Test Song - Artist
100bpm 4/4 #C

VS 4
Cmaj7/// ->170 Dm7///
"#;

    let chart3 = keyflow::parse(input3).unwrap();
    assert_eq!(chart3.tempo_changes.len(), 1);
    assert_eq!(chart3.tempo_changes[0].to_tempo.bpm as u32, 170);
}

#[test]
fn test_multiple_tempo_changes() {
    let input = r#"
Test Song - Artist
100bpm 4/4 #C

VS 8
Cmaj7/// ->120bpm Dm7/// Em7/// ->140bpm Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // Should have two tempo changes
    assert_eq!(chart.tempo_changes.len(), 2);

    // First change: 100 -> 120
    assert_eq!(chart.tempo_changes[0].to_tempo.bpm as u32, 120);
    assert_eq!(chart.tempo_changes[0].from_tempo.unwrap().bpm as u32, 100);

    // Second change: 120 -> 140
    assert_eq!(chart.tempo_changes[1].to_tempo.bpm as u32, 140);
    assert_eq!(chart.tempo_changes[1].from_tempo.unwrap().bpm as u32, 120);
}

#[test]
fn test_tempo_change_after_chords() {
    // Tempo change appearing between chords
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 4
Cmaj7/// Dm7/// ->140bpm Em7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // Should have one tempo change
    assert_eq!(chart.tempo_changes.len(), 1);
    assert_eq!(chart.tempo_changes[0].to_tempo.bpm as u32, 140);
}

#[test]
fn test_tempo_change_in_different_sections() {
    let input = r#"
Test Song - Artist
100bpm 4/4 #C

VS 8
Cmaj7/// Dm7/// Em7/// Fmaj7///

CH 8
->120bpm Gmaj7/// Am7/// Bm7/// Cmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    assert_eq!(chart.sections.len(), 2);

    // Should have one tempo change in the chorus
    assert_eq!(chart.tempo_changes.len(), 1);
    assert_eq!(chart.tempo_changes[0].to_tempo.bpm as u32, 120);
    assert_eq!(chart.tempo_changes[0].section_index, 1); // Chorus is section 1
}

#[test]
fn test_tempo_change_parse_syntax() {
    use keyflow::chart::TempoChange;

    // Valid syntax
    assert!(TempoChange::parse_syntax("->120bpm").is_some());
    assert!(TempoChange::parse_syntax("->120BPM").is_some());
    assert!(TempoChange::parse_syntax("->120").is_some());
    assert!(TempoChange::parse_syntax("->200bpm").is_some());

    // Invalid syntax
    assert!(TempoChange::parse_syntax("120bpm").is_none()); // Missing ->
    assert!(TempoChange::parse_syntax("->abc").is_none()); // Invalid number
    assert!(TempoChange::parse_syntax("-120bpm").is_none()); // Single dash
}

#[test]
fn test_tempo_change_serialization() {
    // Test that tempo changes are detected and stored
    let input = r#"Test Song - Artist
120bpm 4/4 #C

VS 4
Cmaj7/// ->140bpm Dm7///
"#;

    let chart = keyflow::parse(input).unwrap();

    // Should have one tempo change
    assert_eq!(chart.tempo_changes.len(), 1);
    assert_eq!(chart.tempo_changes[0].to_tempo.bpm as u32, 140);

    // Serialize and check it contains the tempo change
    let syntax = chart.to_syntax();
    println!("Serialized:\n{}", syntax);
    assert!(syntax.contains("->140bpm"));
}
