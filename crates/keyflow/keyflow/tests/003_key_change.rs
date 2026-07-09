use keyflow::sections::SectionType;

#[test]
fn test_key_change_mid_song() {
    let input = r#"Key Change Test - Demo
120bpm 4/4 #G

intro 2
gmaj7 em7
vs 4
gmaj7 cmaj7 em7 d7
ch 8
em7 d cmaj7 am7
gmaj7 cmaj7 bm7 d
post 4
gmaj7 cmaj7 #C# c#maj7 g#m7
"#;

    let chart = keyflow::parse(input).unwrap();

    // Test metadata
    assert_eq!(chart.metadata.title, Some("Key Change Test".to_string()));
    assert_eq!(chart.metadata.artist, Some("Demo".to_string()));

    // Test tempo
    assert!(chart.tempo.is_some());
    assert_eq!(chart.tempo.as_ref().unwrap().bpm, 120.0);

    // Test time signature
    assert!(chart.time_signature.is_some());
    assert_eq!(chart.time_signature.as_ref().unwrap().numerator, 4);
    assert_eq!(chart.time_signature.as_ref().unwrap().denominator, 4);

    // Test initial key (G major)
    assert!(chart.initial_key.is_some());
    let initial_key = chart.initial_key.as_ref().unwrap();
    assert_eq!(format!("{}", initial_key.root), "G");

    // Test ending key (should be C# major after key change)
    assert!(chart.ending_key.is_some());
    let ending_key = chart.ending_key.as_ref().unwrap();
    assert_eq!(format!("{}", ending_key.root), "C#");

    // Test key changes were recorded
    assert_eq!(chart.key_changes.len(), 1);
    let key_change = &chart.key_changes[0];

    assert!(key_change.from_key.is_some());
    assert_eq!(
        format!("{}", key_change.from_key.as_ref().unwrap().root),
        "G"
    );
    assert_eq!(format!("{}", key_change.to_key.root), "C#");

    // Test sections
    assert_eq!(chart.sections.len(), 4);

    // Verify section types
    assert_eq!(chart.sections[0].section.section_type, SectionType::Intro);
    assert_eq!(chart.sections[1].section.section_type, SectionType::Verse);
    assert_eq!(chart.sections[2].section.section_type, SectionType::Chorus);
    assert_eq!(
        chart.sections[3].section.section_type,
        SectionType::Post(Box::new(SectionType::Chorus))
    );

    // Verify chords before key change (in G major)
    let intro_chords = &chart.sections[0].measures();
    assert_eq!(intro_chords[0].chords[0].full_symbol, "Gmaj7");
    assert_eq!(intro_chords[1].chords[0].full_symbol, "Em7");

    // Verify chords after key change (in C# major)
    let post_chords = &chart.sections[3].measures();
    assert_eq!(post_chords[2].chords[0].full_symbol, "C#maj7"); // First chord in C# major
    assert_eq!(post_chords[3].chords[0].full_symbol, "G#m7"); // vi in C# major

    // Display the chart
    println!("\n{}", chart);
}

#[test]
fn test_key_change_with_inferred_qualities() {
    let input = r#"Key Change with Inference - Demo
120bpm 4/4 #G

intro 4
G C Em D
vs 4
1 4 6 5
pre 4
I IV vi V
ch 4
#C# C# G# A#m F#
post 4
1 4 6 5
br 4
I IV vi V
"#;

    let chart = keyflow::parse(input).unwrap();

    // Test metadata
    assert_eq!(
        chart.metadata.title,
        Some("Key Change with Inference".to_string())
    );
    assert_eq!(chart.metadata.artist, Some("Demo".to_string()));

    // Test initial key (G major)
    assert!(chart.initial_key.is_some());
    let initial_key = chart.initial_key.as_ref().unwrap();
    assert_eq!(format!("{}", initial_key.root), "G");

    // Test ending key (C# major)
    assert!(chart.ending_key.is_some());
    let ending_key = chart.ending_key.as_ref().unwrap();
    assert_eq!(format!("{}", ending_key.root), "C#");

    // === G Major Section Tests ===

    // Intro: Explicit chord symbols in G major
    let intro_measures = &chart.sections[0].measures();
    assert_eq!(intro_measures[0].chords[0].full_symbol, "G");
    assert_eq!(intro_measures[1].chords[0].full_symbol, "C");
    assert_eq!(intro_measures[2].chords[0].full_symbol, "Em");
    assert_eq!(intro_measures[3].chords[0].full_symbol, "D");

    // Verse: Scale degrees (1-7) in G major
    let verse_measures = &chart.sections[1].measures();
    assert_eq!(verse_measures[0].chords[0].full_symbol, "1"); // 1 -> 1 (quality implied by key)
    assert_eq!(verse_measures[1].chords[0].full_symbol, "4"); // 4 -> 4 (quality implied by key)
    assert_eq!(verse_measures[2].chords[0].full_symbol, "6"); // 6 -> 6 (quality implied by key)
    assert_eq!(verse_measures[3].chords[0].full_symbol, "5"); // 5 -> 5 (quality implied by key)

    // Pre: Roman numerals in G major
    let pre_measures = &chart.sections[2].measures();
    assert_eq!(pre_measures[0].chords[0].full_symbol, "I"); // I -> I (quality implied by key)
    assert_eq!(pre_measures[1].chords[0].full_symbol, "IV"); // IV -> IV (quality implied by key)
    assert_eq!(pre_measures[2].chords[0].full_symbol, "vim"); // vi -> vim (lowercase = minor)
    assert_eq!(pre_measures[3].chords[0].full_symbol, "V"); // V -> V (quality implied by key)

    // === C# Major Section Tests ===

    // Chorus: Explicit chord symbols in C# major (key change with #C#)
    let chorus_measures = &chart.sections[3].measures();
    assert_eq!(chorus_measures[0].chords[0].full_symbol, "C#");
    assert_eq!(chorus_measures[1].chords[0].full_symbol, "G#");
    assert_eq!(chorus_measures[2].chords[0].full_symbol, "A#m");
    assert_eq!(chorus_measures[3].chords[0].full_symbol, "F#");

    // Post: Scale degrees (1-7) in C# major
    let post_measures = &chart.sections[4].measures();
    assert_eq!(post_measures[0].chords[0].full_symbol, "1"); // 1 -> 1 (quality implied by key)
    assert_eq!(post_measures[1].chords[0].full_symbol, "4"); // 4 -> 4 (quality implied by key)
    assert_eq!(post_measures[2].chords[0].full_symbol, "6"); // 6 -> 6 (quality implied by key)
    assert_eq!(post_measures[3].chords[0].full_symbol, "5"); // 5 -> 5 (quality implied by key)

    // Bridge: Roman numerals in C# major
    let bridge_measures = &chart.sections[5].measures();
    assert_eq!(bridge_measures[0].chords[0].full_symbol, "I"); // I -> I (quality implied by key)
    assert_eq!(bridge_measures[1].chords[0].full_symbol, "IV"); // IV -> IV (quality implied by key)
    assert_eq!(bridge_measures[2].chords[0].full_symbol, "vim"); // vi -> vim (lowercase = minor)
    assert_eq!(bridge_measures[3].chords[0].full_symbol, "V"); // V -> V (quality implied by key)

    // Display the chart
    println!("\n{}", chart);
}
