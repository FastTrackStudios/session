//! Test 015: Measure Count Expressions
//!
//! Tests the expression system for section measure counts including:
//! - Basic expressions: 8+1, 4x4, 8-1
//! - Section memory: VS 8 then VS remembers 8
//! - Relative expressions: VS 8 then VS +1 = 9
//! - Subtraction: VS 8 then VS -1 = 7

#[test]
fn test_basic_absolute() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    assert_eq!(chart.sections.len(), 1);
    assert_eq!(chart.sections[0].measures().len(), 8);
}

#[test]
fn test_addition_expression() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8+1
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // 8+1 = 9 measures
    assert_eq!(chart.sections.len(), 1);
    assert_eq!(chart.sections[0].measures().len(), 9);
}

#[test]
fn test_subtraction_expression() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8-1
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // 8-1 = 7 measures
    assert_eq!(chart.sections.len(), 1);
    assert_eq!(chart.sections[0].measures().len(), 7);
}

#[test]
fn test_multiplication_expression() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 4x4
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // 4x4 = 16 measures
    assert_eq!(chart.sections.len(), 1);
    assert_eq!(chart.sections[0].measures().len(), 16);
}

#[test]
fn test_multiplication_with_asterisk() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 4*4
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // 4*4 = 16 measures
    assert_eq!(chart.sections.len(), 1);
    assert_eq!(chart.sections[0].measures().len(), 16);
}

#[test]
fn test_section_memory_basic() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// Dm7/// Em7/// Fmaj7///

VS
Gmaj7/// Am7/// Bm7/// Cmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // Both verses should have 8 measures
    assert_eq!(chart.sections.len(), 2);
    assert_eq!(chart.sections[0].measures().len(), 8);
    assert_eq!(chart.sections[1].measures().len(), 8);
}

#[test]
fn test_relative_add() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// Dm7/// Em7/// Fmaj7///

VS +1
Gmaj7/// Am7/// Bm7/// Cmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // First verse: 8 measures
    // Second verse: 8+1 = 9 measures
    assert_eq!(chart.sections.len(), 2);
    assert_eq!(chart.sections[0].measures().len(), 8);
    assert_eq!(chart.sections[1].measures().len(), 9);
}

#[test]
fn test_relative_subtract() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// Dm7/// Em7/// Fmaj7///

VS -1
Gmaj7/// Am7/// Bm7/// Cmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // First verse: 8 measures
    // Second verse: 8-1 = 7 measures
    assert_eq!(chart.sections.len(), 2);
    assert_eq!(chart.sections[0].measures().len(), 8);
    assert_eq!(chart.sections[1].measures().len(), 7);
}

#[test]
fn test_memory_per_section_type() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// Dm7/// Em7/// Fmaj7///

CH 4
Gmaj7/// Am7///

VS
Bm7/// Cmaj7/// Dmaj7/// Em7///

CH
Fmaj7/// Gmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // Each section type has its own memory
    assert_eq!(chart.sections.len(), 4);
    assert_eq!(chart.sections[0].measures().len(), 8); // VS 8
    assert_eq!(chart.sections[1].measures().len(), 4); // CH 4
    assert_eq!(chart.sections[2].measures().len(), 8); // VS (remembers 8)
    assert_eq!(chart.sections[3].measures().len(), 4); // CH (remembers 4)
}

#[test]
fn test_complex_expressions_with_memory() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 4x2
Cmaj7/// Dm7/// Em7/// Fmaj7///

VS +2
Gmaj7/// Am7/// Bm7/// Cmaj7///

VS -2
Dmaj7/// Em7/// Fmaj7/// Gmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // Relative expressions don't update memory, only absolute values do
    // VS 4x2 = 8 measures (memory = 8)
    // VS +2 = 8+2 = 10 measures (memory stays 8)
    // VS -2 = 8-2 = 6 measures (memory stays 8, not 10-2=8)
    assert_eq!(chart.sections.len(), 3);
    assert_eq!(chart.sections[0].measures().len(), 8); // 4x2 = 8
    assert_eq!(chart.sections[1].measures().len(), 10); // 8+2 = 10
    assert_eq!(chart.sections[2].measures().len(), 6); // 8-2 = 6
}

#[test]
fn test_custom_section_with_expression() {
    // Test custom section with measure count - should pad to declared length
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

[SOLO] 8
Cmaj7/// Dm7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // Custom section should be recognized
    assert_eq!(chart.sections.len(), 1);
    assert_eq!(
        chart.sections[0].section.section_type,
        keyflow::sections::SectionType::Custom("SOLO".to_string())
    );

    // Section should be padded to declared length of 8 measures
    assert_eq!(chart.sections[0].measures().len(), 8);
}

#[test]
fn test_chorus_memory_independent() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7///

CH 4
Dm7///

VS +4
Em7///

CH +2
Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // Memory is independent per section type
    assert_eq!(chart.sections.len(), 4);
    assert_eq!(chart.sections[0].measures().len(), 8); // VS 8
    assert_eq!(chart.sections[1].measures().len(), 4); // CH 4
    assert_eq!(chart.sections[2].measures().len(), 12); // VS +4 (8+4=12)
    assert_eq!(chart.sections[3].measures().len(), 6); // CH +2 (4+2=6)
}

#[test]
fn test_incomplete_expression_trailing_operator() {
    // Incomplete expressions like "16+" should extract the number (16)
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 16+
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // "16+" should parse as 16 measures
    assert_eq!(chart.sections.len(), 1);
    assert_eq!(chart.sections[0].measures().len(), 16);
}

#[test]
fn test_incomplete_expression_various() {
    // Test various incomplete expression forms
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8-
Cmaj7///

CH 4x
Dm7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // "8-" should parse as 8, "4x" should parse as 4
    assert_eq!(chart.sections.len(), 2);
    assert_eq!(chart.sections[0].measures().len(), 8);
    assert_eq!(chart.sections[1].measures().len(), 4);
}

#[test]
fn test_expression_with_melody_track() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 4x2
Cmaj7/// Dm7/// Em7/// Fmaj7///
[melody] m{ C_4 D_4 E_4 F_4 G_4 A_4 B_4 C'_4 }
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // 4x2 = 8 measures
    assert_eq!(chart.sections.len(), 1);
    assert_eq!(chart.sections[0].measures().len(), 8);

    // Should have 2 tracks: chords and melody
    assert_eq!(chart.sections[0].tracks.len(), 2);

    // Melody track should have notes
    let melody_tracks: Vec<_> = chart.sections[0].melody_tracks().collect();
    assert_eq!(melody_tracks.len(), 1);
    assert!(melody_tracks[0].melody.is_some());
    assert_eq!(melody_tracks[0].melody.as_ref().unwrap().notes.len(), 8);
}

#[test]
fn test_memory_with_melody_tracks() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// Dm7/// Em7/// Fmaj7///
[melody lead] m{ C_4 D_4 E_4 F_4 }

CH 4
Gmaj7/// Am7///
[melody harmony] m{ G_4 A_4 B_4 C'_4 }

VS +2
Bm7/// Cmaj7///
[melody lead] m{ D_4 E_4 F_4 G_4 A_4 B_4 }
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // VS 8, CH 4, VS +2 (8+2=10)
    assert_eq!(chart.sections.len(), 3);
    assert_eq!(chart.sections[0].measures().len(), 8);
    assert_eq!(chart.sections[1].measures().len(), 4);
    assert_eq!(chart.sections[2].measures().len(), 10);

    // Each section should have 2 tracks
    assert_eq!(chart.sections[0].tracks.len(), 2);
    assert_eq!(chart.sections[1].tracks.len(), 2);
    assert_eq!(chart.sections[2].tracks.len(), 2);

    // Check melody track names
    let vs1_melody: Vec<_> = chart.sections[0].melody_tracks().collect();
    assert_eq!(vs1_melody[0].name.as_deref(), Some("lead"));

    let ch_melody: Vec<_> = chart.sections[1].melody_tracks().collect();
    assert_eq!(ch_melody[0].name.as_deref(), Some("harmony"));

    let vs2_melody: Vec<_> = chart.sections[2].melody_tracks().collect();
    assert_eq!(vs2_melody[0].name.as_deref(), Some("lead"));
}

#[test]
fn test_multiple_melodies_with_expression() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

CH 2x4
Cmaj7/// Dm7/// Em7/// Fmaj7///
[melody lead] m{ C_4 E_4 G_4 C'_4 }
[melody bass] m{ C,_2 G,_2 }
[melody pad] m{ E_1 }
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // 2x4 = 8 measures
    assert_eq!(chart.sections.len(), 1);
    assert_eq!(chart.sections[0].measures().len(), 8);

    // Should have 4 tracks: chords + 3 melody tracks
    assert_eq!(chart.sections[0].tracks.len(), 4);

    let melody_tracks: Vec<_> = chart.sections[0].melody_tracks().collect();
    assert_eq!(melody_tracks.len(), 3);
    assert_eq!(melody_tracks[0].name.as_deref(), Some("lead"));
    assert_eq!(melody_tracks[1].name.as_deref(), Some("bass"));
    assert_eq!(melody_tracks[2].name.as_deref(), Some("pad"));
}

#[test]
fn test_section_recall_preserves_structure() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// Dm7/// Em7/// Fmaj7///

CH 4
Gmaj7/// Am7///
[melody] m{ G_4 A_4 B_4 C'_4 }

VS
Bm7/// Cmaj7/// Dmaj7/// Em7///

CH
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // VS 8, CH 4, VS (8 from memory), CH (4 from memory)
    assert_eq!(chart.sections.len(), 4);
    assert_eq!(chart.sections[0].measures().len(), 8);
    assert_eq!(chart.sections[1].measures().len(), 4);
    assert_eq!(chart.sections[2].measures().len(), 8); // From memory
    assert_eq!(chart.sections[3].measures().len(), 4); // From memory

    // First chorus has melody, second should recall template with melody
    assert_eq!(chart.sections[1].tracks.len(), 2); // chords + melody
}
