use keyflow::chart::InstrumentGroup;

#[test]
fn test_basic_text_cue() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
@keys "synth here"
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // Should have one section
    assert_eq!(chart.sections.len(), 1);

    // First section should have measures with a text cue
    let section = &chart.sections[0];
    assert!(!section.measures().is_empty());

    // Find the measure with the cue
    let cue_measure = section
        .measures()
        .iter()
        .find(|m| !m.text_cues.is_empty())
        .expect("Should have a measure with text cues");

    assert_eq!(cue_measure.text_cues.len(), 1);
    assert_eq!(cue_measure.text_cues[0].group, InstrumentGroup::Keys);
    assert_eq!(cue_measure.text_cues[0].text, "synth here");
}

#[test]
fn test_multiple_text_cues() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
@keys "synth here"
@drums "kick on 1 and 3"
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];
    let cue_measure = section
        .measures()
        .iter()
        .find(|m| !m.text_cues.is_empty())
        .expect("Should have a measure with text cues");

    assert_eq!(cue_measure.text_cues.len(), 2);
    assert_eq!(cue_measure.text_cues[0].group, InstrumentGroup::Keys);
    assert_eq!(cue_measure.text_cues[0].text, "synth here");
    assert_eq!(cue_measure.text_cues[1].group, InstrumentGroup::Drums);
    assert_eq!(cue_measure.text_cues[1].text, "kick on 1 and 3");
}

#[test]
fn test_text_cue_between_chords() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// Dm7///
@guitar "let ring"
Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];

    // Cue should be attached to the first measure after it (Em7)
    let cue_measure = section
        .measures()
        .iter()
        .find(|m| !m.text_cues.is_empty())
        .expect("Should have a measure with text cues");

    assert_eq!(cue_measure.text_cues.len(), 1);
    assert_eq!(cue_measure.text_cues[0].group, InstrumentGroup::Guitar);
    assert_eq!(cue_measure.text_cues[0].text, "let ring");

    // Verify that the cue is on the correct measure (should have Em7)
    assert!(!cue_measure.chords.is_empty());
    assert_eq!(cue_measure.chords[0].full_symbol, "Em7");
}

#[test]
fn test_all_instrument_groups() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
@all "intro hit"
@keys "pad"
@drums "fill"
@bass "root notes"
@guitar "palm mute"
@vocals "ooh ahh"
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];
    let cue_measure = section
        .measures()
        .iter()
        .find(|m| !m.text_cues.is_empty())
        .expect("Should have a measure with text cues");

    assert_eq!(cue_measure.text_cues.len(), 6);

    // Verify each instrument group
    assert_eq!(cue_measure.text_cues[0].group, InstrumentGroup::All);
    assert_eq!(cue_measure.text_cues[1].group, InstrumentGroup::Keys);
    assert_eq!(cue_measure.text_cues[2].group, InstrumentGroup::Drums);
    assert_eq!(cue_measure.text_cues[3].group, InstrumentGroup::Bass);
    assert_eq!(cue_measure.text_cues[4].group, InstrumentGroup::Guitar);
    assert_eq!(cue_measure.text_cues[5].group, InstrumentGroup::Vocals);
}

#[test]
fn test_custom_instrument_group() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
@synth "lead melody"
@strings "long notes"
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];
    let cue_measure = section
        .measures()
        .iter()
        .find(|m| !m.text_cues.is_empty())
        .expect("Should have a measure with text cues");

    assert_eq!(cue_measure.text_cues.len(), 2);

    // Custom groups
    match &cue_measure.text_cues[0].group {
        InstrumentGroup::Custom(name) => assert_eq!(name, "synth"),
        _ => panic!("Expected Custom instrument group"),
    }

    match &cue_measure.text_cues[1].group {
        InstrumentGroup::Custom(name) => assert_eq!(name, "strings"),
        _ => panic!("Expected Custom instrument group"),
    }
}

#[test]
fn test_text_cue_in_multiple_sections() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
@keys "sparse"
Cmaj7/// Dm7/// Em7/// Fmaj7///

CH 8
@keys "full chords"
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    assert_eq!(chart.sections.len(), 2);

    // Verse cue
    let verse = &chart.sections[0];
    let verse_cue_measure = verse
        .measures()
        .iter()
        .find(|m| !m.text_cues.is_empty())
        .expect("Verse should have text cues");
    assert_eq!(verse_cue_measure.text_cues[0].text, "sparse");

    // Chorus cue
    let chorus = &chart.sections[1];
    let chorus_cue_measure = chorus
        .measures()
        .iter()
        .find(|m| !m.text_cues.is_empty())
        .expect("Chorus should have text cues");
    assert_eq!(chorus_cue_measure.text_cues[0].text, "full chords");
}

#[test]
fn test_pending_cue_attachment() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
@drums "fill into section"
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];

    // Cue should be attached to the first measure (pending attachment)
    assert!(!section.measures().is_empty());
    let first_measure = &section.measures()[0];
    assert!(!first_measure.text_cues.is_empty());
    assert_eq!(first_measure.text_cues[0].group, InstrumentGroup::Drums);
    assert_eq!(first_measure.text_cues[0].text, "fill into section");
}

#[test]
fn test_inline_text_cue() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// Dm7/// @keys "synth here" Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];
    assert_eq!(section.measures().len(), 8); // VS 8 = 8 measures total

    // The cue should be attached to the measure with Em7 (the chord after the cue)
    let em_measure = &section.measures()[2]; // Third measure
    assert!(!em_measure.text_cues.is_empty());
    assert_eq!(em_measure.text_cues[0].group, InstrumentGroup::Keys);
    assert_eq!(em_measure.text_cues[0].text, "synth here");

    // Verify the chord is Em7
    assert_eq!(em_measure.chords[0].full_symbol, "Em7");
}

#[test]
fn test_multiple_inline_cues() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// @drums "kick" Dm7/// @keys "pad" Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];

    // Drums cue on Dm7 (measure 1)
    let dm_measure = &section.measures()[1];
    assert!(!dm_measure.text_cues.is_empty());
    assert_eq!(dm_measure.text_cues[0].group, InstrumentGroup::Drums);
    assert_eq!(dm_measure.text_cues[0].text, "kick");

    // Keys cue on Em7 (measure 2)
    let em_measure = &section.measures()[2];
    assert!(!em_measure.text_cues.is_empty());
    assert_eq!(em_measure.text_cues[0].group, InstrumentGroup::Keys);
    assert_eq!(em_measure.text_cues[0].text, "pad");
}

#[test]
fn test_beat_specific_text_cue() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
@keys:2 "synth stab"
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];
    let cue_measure = section
        .measures()
        .iter()
        .find(|m| !m.text_cues.is_empty())
        .expect("Should have a measure with text cues");

    assert_eq!(cue_measure.text_cues.len(), 1);
    assert_eq!(cue_measure.text_cues[0].group, InstrumentGroup::Keys);
    assert_eq!(cue_measure.text_cues[0].text, "synth stab");
    assert_eq!(cue_measure.text_cues[0].beat, Some(2));
}

#[test]
fn test_multiple_beat_specific_cues() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
@drums:1 "kick"
@drums:3 "snare"
@keys:4 "hit"
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];
    let cue_measure = section
        .measures()
        .iter()
        .find(|m| !m.text_cues.is_empty())
        .expect("Should have a measure with text cues");

    assert_eq!(cue_measure.text_cues.len(), 3);

    assert_eq!(cue_measure.text_cues[0].group, InstrumentGroup::Drums);
    assert_eq!(cue_measure.text_cues[0].text, "kick");
    assert_eq!(cue_measure.text_cues[0].beat, Some(1));

    assert_eq!(cue_measure.text_cues[1].group, InstrumentGroup::Drums);
    assert_eq!(cue_measure.text_cues[1].text, "snare");
    assert_eq!(cue_measure.text_cues[1].beat, Some(3));

    assert_eq!(cue_measure.text_cues[2].group, InstrumentGroup::Keys);
    assert_eq!(cue_measure.text_cues[2].text, "hit");
    assert_eq!(cue_measure.text_cues[2].beat, Some(4));
}

#[test]
fn test_mixed_beat_and_non_beat_cues() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
@keys "throughout"
@drums:3 "crash"
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    let section = &chart.sections[0];
    let cue_measure = section
        .measures()
        .iter()
        .find(|m| !m.text_cues.is_empty())
        .expect("Should have a measure with text cues");

    assert_eq!(cue_measure.text_cues.len(), 2);

    // First cue has no beat
    assert_eq!(cue_measure.text_cues[0].text, "throughout");
    assert_eq!(cue_measure.text_cues[0].beat, None);

    // Second cue has beat 3
    assert_eq!(cue_measure.text_cues[1].text, "crash");
    assert_eq!(cue_measure.text_cues[1].beat, Some(3));
}

#[test]
fn test_beat_cue_round_trip() {
    let input = r#"Test Song - Artist
120bpm 4/4 #C

VS 8
@keys:2 "stab"
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart1 = keyflow::parse(input).unwrap();
    let syntax = chart1.to_syntax();
    println!("Serialized:\n{}", syntax);

    let chart2 = keyflow::parse(&syntax).unwrap();

    // Find measure with cue in both charts
    let m1 = chart1.sections[0]
        .measures()
        .iter()
        .find(|m| !m.text_cues.is_empty())
        .unwrap();
    let m2 = chart2.sections[0]
        .measures()
        .iter()
        .find(|m| !m.text_cues.is_empty())
        .unwrap();

    assert_eq!(m1.text_cues[0].text, m2.text_cues[0].text);
    assert_eq!(m1.text_cues[0].beat, m2.text_cues[0].beat);
}
