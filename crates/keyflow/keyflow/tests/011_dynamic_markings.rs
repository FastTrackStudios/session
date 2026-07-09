//! Test 011: Dynamic Markings
//!
//! Tests the <Build>, <Down>, <Go Crazy> dynamic marking syntax
//! Note: We use angle brackets <> to avoid conflict with custom section syntax []

#[test]
fn test_standalone_dynamic_marking() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
<Build>
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    assert_eq!(chart.sections.len(), 1);

    let section = &chart.sections[0];
    assert!(!section.measures().is_empty());

    // First measure should have the dynamic marking
    let first_measure = &section.measures()[0];
    assert_eq!(first_measure.dynamics.len(), 1);
    assert_eq!(first_measure.dynamics[0].text, "Build");
    assert_eq!(first_measure.dynamics[0].beat, None);
}

#[test]
fn test_multiple_standalone_dynamics() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
<Soft>
<Build>
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();

    let section = &chart.sections[0];
    let first_measure = &section.measures()[0];

    // Should have both dynamics attached to first measure
    assert_eq!(first_measure.dynamics.len(), 2);
    assert_eq!(first_measure.dynamics[0].text, "Soft");
    assert_eq!(first_measure.dynamics[1].text, "Build");
}

#[test]
fn test_inline_dynamic_marking() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// <Build> Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();

    let section = &chart.sections[0];

    // The <Build> should be attached to a measure
    // Let's just check that at least one measure has the dynamic
    let measure_with_dynamic = section.measures().iter().find(|m| !m.dynamics.is_empty());

    assert!(
        measure_with_dynamic.is_some(),
        "Should have a measure with dynamic marking"
    );
    assert_eq!(measure_with_dynamic.unwrap().dynamics[0].text, "Build");
}

#[test]
fn test_dynamic_with_spaces() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
<Go Crazy>
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();

    let section = &chart.sections[0];
    let first_measure = &section.measures()[0];

    assert_eq!(first_measure.dynamics.len(), 1);
    assert_eq!(first_measure.dynamics[0].text, "Go Crazy");
}

#[test]
fn test_dynamic_with_beat_position() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
<Hit>:3
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();

    let section = &chart.sections[0];
    let first_measure = &section.measures()[0];

    assert_eq!(first_measure.dynamics.len(), 1);
    assert_eq!(first_measure.dynamics[0].text, "Hit");
    assert_eq!(first_measure.dynamics[0].beat, Some(3));
}

#[test]
fn test_dynamics_in_different_sections() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
<Soft>
Cmaj7/// Dm7/// Em7/// Fmaj7///

CH 8
<Full Band>
Gmaj7/// Am7/// Bm7/// Cmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();

    assert_eq!(chart.sections.len(), 2);

    // Verse
    let verse = &chart.sections[0];
    let verse_measure = verse
        .measures()
        .iter()
        .find(|m| !m.dynamics.is_empty())
        .expect("Verse should have dynamic");
    assert_eq!(verse_measure.dynamics[0].text, "Soft");

    // Chorus
    let chorus = &chart.sections[1];
    let chorus_measure = chorus
        .measures()
        .iter()
        .find(|m| !m.dynamics.is_empty())
        .expect("Chorus should have dynamic");
    assert_eq!(chorus_measure.dynamics[0].text, "Full Band");
}

#[test]
fn test_inline_dynamic_between_chords() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7//// <Down> Dm7////
"#;

    let chart = keyflow::parse(input).unwrap();

    let section = &chart.sections[0];

    // Find measure with dynamic
    let measure_with_dynamic = section.measures().iter().find(|m| !m.dynamics.is_empty());

    assert!(measure_with_dynamic.is_some());
    assert_eq!(measure_with_dynamic.unwrap().dynamics[0].text, "Down");
}

#[test]
fn test_mixed_cues_and_dynamics() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
@keys "pad"
<Build>
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();

    let section = &chart.sections[0];
    let first_measure = &section.measures()[0];

    // Should have both text cue and dynamic
    assert_eq!(first_measure.text_cues.len(), 1);
    assert_eq!(first_measure.text_cues[0].text, "pad");

    assert_eq!(first_measure.dynamics.len(), 1);
    assert_eq!(first_measure.dynamics[0].text, "Build");
}

#[test]
fn test_round_trip_dynamics() {
    let input = r#"Test Song - Artist
120bpm 4/4 #C

VS 8
<Build>
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart1 = keyflow::parse(input).unwrap();
    let syntax = chart1.to_syntax();
    println!("Serialized:\n{}", syntax);

    // Parse it again
    let chart2 = keyflow::parse(&syntax).unwrap();

    // Find measures with dynamics in both charts
    let m1_with_dynamic = chart1.sections[0]
        .measures()
        .iter()
        .find(|m| !m.dynamics.is_empty());
    let m2_with_dynamic = chart2.sections[0]
        .measures()
        .iter()
        .find(|m| !m.dynamics.is_empty());

    // Both should have a measure with dynamics
    assert!(
        m1_with_dynamic.is_some(),
        "Original chart should have dynamics"
    );
    assert!(
        m2_with_dynamic.is_some(),
        "Re-parsed chart should have dynamics"
    );

    // The text should match
    assert_eq!(
        m1_with_dynamic.unwrap().dynamics[0].text,
        m2_with_dynamic.unwrap().dynamics[0].text
    );
}
