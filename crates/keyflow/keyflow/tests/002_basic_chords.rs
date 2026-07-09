use keyflow::sections::SectionType;

#[test]
fn test_basic_structure_parsing() {
    let input = r#"Simple Test - Demo
120bpm 4/4 #G

intro 2
g e
vs 4
1 4 6 5
vs
pre 3
I IV V
ch 8
e d c a
g c b d
post 4
1 4 1 4
inst 4
1*4
vs
pre
ch
post
inst 6
6 5 4 3 2 1
br 8
I ii vi V
I ii vi V
ch
post
outro 4
1*4
"#;

    let chart = keyflow::parse(input).unwrap();

    // Test metadata
    assert_eq!(chart.metadata.title, Some("Simple Test".to_string()));
    assert_eq!(chart.metadata.artist, Some("Demo".to_string()));

    // Test tempo
    assert!(chart.tempo.is_some());
    assert_eq!(chart.tempo.as_ref().unwrap().bpm, 120.0);

    // Test time signature
    assert!(chart.time_signature.is_some());
    assert_eq!(chart.time_signature.as_ref().unwrap().numerator, 4);
    assert_eq!(chart.time_signature.as_ref().unwrap().denominator, 4);

    // Test key
    assert!(chart.current_key.is_some());
    let key = chart.current_key.as_ref().unwrap();
    assert_eq!(key.root.to_string(), "G");
    assert_eq!(key.scale_type(), keyflow::key::keys::ScaleType::Diatonic);
    assert_eq!(key.mode, keyflow::key::keys::ScaleMode::ionian());

    // Test sections
    assert_eq!(chart.sections.len(), 16);

    // Verify section order and types
    let expected_sections = vec![
        SectionType::Intro,
        SectionType::Verse,
        SectionType::Verse,
        SectionType::Pre(Box::new(SectionType::Chorus)),
        SectionType::Chorus,
        SectionType::Post(Box::new(SectionType::Chorus)),
        SectionType::Instrumental,
        SectionType::Verse,
        SectionType::Pre(Box::new(SectionType::Chorus)),
        SectionType::Chorus,
        SectionType::Post(Box::new(SectionType::Chorus)),
        SectionType::Instrumental,
        SectionType::Bridge,
        SectionType::Chorus,
        SectionType::Post(Box::new(SectionType::Chorus)),
        SectionType::Outro,
    ];

    // Debug: Print actual sections
    for (i, section) in chart.sections.iter().enumerate() {
        println!("Section {}: {:?}", i, section.section.section_type);
    }

    for (i, expected_section) in expected_sections.iter().enumerate() {
        assert_eq!(
            chart.sections[i].section.section_type, *expected_section,
            "Section {} mismatch",
            i
        );
    }

    // Test auto-numbering using batch method
    let mut numberer = keyflow::sections::SectionNumberer::new();

    let mut test_sections = vec![
        keyflow::sections::Section::new(SectionType::Intro),
        keyflow::sections::Section::new(SectionType::Verse),
        keyflow::sections::Section::new(SectionType::Verse), // Consecutive - both should get split letters
        keyflow::sections::Section::new(SectionType::Pre(Box::new(SectionType::Chorus))),
        keyflow::sections::Section::new(SectionType::Chorus),
        keyflow::sections::Section::new(SectionType::Post(Box::new(SectionType::Chorus))),
        keyflow::sections::Section::new(SectionType::Instrumental),
        keyflow::sections::Section::new(SectionType::Verse), // Non-consecutive - Verse 2
        keyflow::sections::Section::new(SectionType::Pre(Box::new(SectionType::Chorus))),
        keyflow::sections::Section::new(SectionType::Chorus), // Chorus 2
        keyflow::sections::Section::new(SectionType::Post(Box::new(SectionType::Chorus))),
        keyflow::sections::Section::new(SectionType::Instrumental),
        keyflow::sections::Section::new(SectionType::Bridge),
        keyflow::sections::Section::new(SectionType::Chorus), // Chorus 3
        keyflow::sections::Section::new(SectionType::Post(Box::new(SectionType::Chorus))),
        keyflow::sections::Section::new(SectionType::Outro),
    ];

    numberer.number_sections(&mut test_sections);

    // Verify auto-numbering
    assert_eq!(test_sections[0].number, None); // Intro doesn't number
    assert_eq!(test_sections[0].split_letter, None);

    assert_eq!(test_sections[1].number, Some(1)); // Verse 1a (retroactively assigned)
    assert_eq!(test_sections[1].split_letter, Some('a'));

    assert_eq!(test_sections[2].number, Some(1)); // Verse 1b
    assert_eq!(test_sections[2].split_letter, Some('b'));

    assert_eq!(test_sections[3].number, None); // Pre-Chorus doesn't number
    assert_eq!(test_sections[3].split_letter, None);

    assert_eq!(test_sections[4].number, Some(1)); // Chorus 1
    assert_eq!(test_sections[4].split_letter, None);

    assert_eq!(test_sections[5].number, None); // Post-Chorus doesn't number
    assert_eq!(test_sections[5].split_letter, None);

    assert_eq!(test_sections[6].number, None); // Instrumental doesn't number
    assert_eq!(test_sections[6].split_letter, None);

    assert_eq!(test_sections[7].number, Some(2)); // Verse 2
    assert_eq!(test_sections[7].split_letter, None);

    assert_eq!(test_sections[9].number, Some(2)); // Chorus 2
    assert_eq!(test_sections[9].split_letter, None);

    assert_eq!(test_sections[13].number, Some(3)); // Chorus 3
    assert_eq!(test_sections[13].split_letter, None);

    assert_eq!(test_sections[15].number, None); // Outro doesn't number
    assert_eq!(test_sections[15].split_letter, None);

    // Display the chart to see the parsed chords
    println!("{}", chart);
}
