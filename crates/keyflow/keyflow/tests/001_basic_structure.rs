use keyflow::sections::SectionType;

#[test]
fn test_basic_structure_parsing() {
    let input = r#"Simple Test - Demo
120bpm 4/4 #G

intro 2
vs 4
vs
pre 2
ch 8
post 4
inst 4
vs
pre
ch
post
inst 6
br 8
ch
post
outro 4"#;

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
    assert_eq!(chart.current_key.as_ref().unwrap().root.to_string(), "G");

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

    for (i, expected_section) in expected_sections.iter().enumerate() {
        assert_eq!(
            chart.sections[i].section.section_type, *expected_section,
            "Section {} mismatch",
            i
        );
    }

    // Test auto-numbering by creating a new parser and processing sections
    // Note: When using process_section individually, we need to manually handle retroactive updates
    // Let's test with the batch method instead
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

    // Print the pretty display for visual verification
    println!("\n{}", chart);
}
