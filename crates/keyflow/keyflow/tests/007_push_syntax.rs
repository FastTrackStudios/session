use keyflow::chart::types::ChordInstance;
use keyflow::chord::ChordRhythm;
use keyflow::sections::SectionType;

// Helper function to find a chord by symbol in a section
fn find_chord_by_symbol<'a>(
    section: &'a keyflow::chart::ChartSection,
    symbol: &str,
) -> Option<&'a ChordInstance> {
    for measure in section.measures() {
        for chord in &measure.chords {
            if chord.full_symbol == symbol && chord.full_symbol != "s" {
                return Some(chord);
            }
        }
    }
    None
}

#[test]
fn test_push_syntax_anticipation() {
    let input = r#"Push Syntax Test - Demo
120bpm 4/4 #C

vs
'C Dm 'Em F
"#;

    let chart = keyflow::parse(input).unwrap();

    // Display the chart to show push notation
    println!("\n{}", chart);

    // Test metadata
    assert_eq!(chart.metadata.title, Some("Push Syntax Test".to_string()));
    assert_eq!(chart.metadata.artist, Some("Demo".to_string()));

    // Test sections
    assert_eq!(chart.sections.len(), 1);

    // Test Verse section
    let verse_section = &chart.sections[0];
    assert_eq!(verse_section.section.section_type, SectionType::Verse);

    // Find chords by symbol instead of index
    let chord_c = find_chord_by_symbol(verse_section, "C").expect("Should find C chord");
    assert_eq!(chord_c.full_symbol, "C");
    match chord_c.push_pull {
        Some((true, amount)) => {
            use keyflow::chord::PushPullAmount;
            assert_eq!(amount, PushPullAmount::eighth());
        }
        _ => panic!("Expected Push for 'C, got {:?}", chord_c.push_pull),
    }

    // Check normal Dm (no push)
    let chord_dm = find_chord_by_symbol(verse_section, "Dm").expect("Should find Dm chord");
    assert_eq!(chord_dm.full_symbol, "Dm");
    assert_eq!(chord_dm.rhythm, ChordRhythm::Default);

    // Check push notation on 'Em
    let chord_em = find_chord_by_symbol(verse_section, "Em").expect("Should find Em chord");
    assert_eq!(chord_em.full_symbol, "Em");
    match chord_em.push_pull {
        Some((true, amount)) => {
            use keyflow::chord::PushPullAmount;
            assert_eq!(amount, PushPullAmount::eighth());
        }
        _ => panic!("Expected Push for 'Em, got {:?}", chord_em.push_pull),
    }

    // Check normal F (no push)
    let chord_f = find_chord_by_symbol(verse_section, "F").expect("Should find F chord");
    assert_eq!(chord_f.full_symbol, "F");
    assert_eq!(chord_f.rhythm, ChordRhythm::Default);
}

#[test]
fn test_pull_syntax_delay() {
    let input = r#"Pull Syntax Test - Demo
120bpm 4/4 #C

vs
C' Dm Em' F
"#;

    let chart = keyflow::parse(input).unwrap();

    // Display the chart to show pull notation
    println!("\n{}", chart);

    // Test Verse section
    let verse_section = &chart.sections[0];
    assert_eq!(verse_section.measures().len(), 4);

    // Check pull notation on C'
    let chord1 = &verse_section.measures()[0].chords[0];
    assert_eq!(chord1.full_symbol, "C");
    match chord1.push_pull {
        Some((false, amount)) => {
            use keyflow::chord::PushPullAmount;
            assert_eq!(amount, PushPullAmount::eighth());
        }
        _ => panic!("Expected Pull for C', got {:?}", chord1.push_pull),
    }

    // Check normal Dm (no pull)
    let chord2 = &verse_section.measures()[1].chords[0];
    assert_eq!(chord2.full_symbol, "Dm");
    assert_eq!(chord2.rhythm, ChordRhythm::Default);

    // Check pull notation on Em'
    let chord3 = &verse_section.measures()[2].chords[0];
    assert_eq!(chord3.full_symbol, "Em");
    match chord3.push_pull {
        Some((false, amount)) => {
            use keyflow::chord::PushPullAmount;
            assert_eq!(amount, PushPullAmount::eighth());
        }
        _ => panic!("Expected Pull for Em', got {:?}", chord3.push_pull),
    }

    // Check normal F (no pull)
    let chord4 = &verse_section.measures()[3].chords[0];
    assert_eq!(chord4.full_symbol, "F");
    assert_eq!(chord4.rhythm, ChordRhythm::Default);
}

#[test]
fn test_double_apostrophe_push() {
    let input = r#"Double Push Test - Demo
120bpm 4/4 #C

vs
''C D ''Em F
"#;

    let chart = keyflow::parse(input).unwrap();

    // Display the chart
    println!("\n{}", chart);

    // Test Verse section
    let verse_section = &chart.sections[0];

    // Check double push on ''C (sixteenth note anticipation)
    let chord_c = find_chord_by_symbol(verse_section, "C").expect("Should find C chord");
    assert_eq!(chord_c.full_symbol, "C");
    match chord_c.push_pull {
        Some((true, amount)) => {
            use keyflow::chord::PushPullAmount;
            assert_eq!(amount, PushPullAmount::sixteenth());
        }
        _ => panic!(
            "Expected Push(Sixteenth) for ''C, got {:?}",
            chord_c.push_pull
        ),
    }

    // Check double push on ''Em
    let chord_em = find_chord_by_symbol(verse_section, "Em").expect("Should find Em chord");
    assert_eq!(chord_em.full_symbol, "Em");
    match chord_em.push_pull {
        Some((true, amount)) => {
            use keyflow::chord::PushPullAmount;
            assert_eq!(amount, PushPullAmount::sixteenth());
        }
        _ => panic!(
            "Expected Push(Sixteenth) for ''Em, got {:?}",
            chord_em.push_pull
        ),
    }
}

#[test]
fn test_triple_apostrophe_pull() {
    let input = r#"Triple Pull Test - Demo
120bpm 4/4 #C

vs
C''' D Em''' F
"#;

    let chart = keyflow::parse(input).unwrap();

    // Display the chart
    println!("\n{}", chart);

    // Test Verse section
    let verse_section = &chart.sections[0];

    // Check triple pull on C''' (32nd note delay)
    let chord1 = &verse_section.measures()[0].chords[0];
    assert_eq!(chord1.full_symbol, "C");
    match chord1.push_pull {
        Some((false, amount)) => {
            use keyflow::chord::PushPullAmount;
            assert_eq!(amount, PushPullAmount::thirty_second());
        }
        _ => panic!(
            "Expected Pull(ThirtySecond) for C''', got {:?}",
            chord1.push_pull
        ),
    }

    // Check triple pull on Em'''
    let chord3 = &verse_section.measures()[2].chords[0];
    assert_eq!(chord3.full_symbol, "Em");
    match chord3.push_pull {
        Some((false, amount)) => {
            use keyflow::chord::PushPullAmount;
            assert_eq!(amount, PushPullAmount::thirty_second());
        }
        _ => panic!(
            "Expected Pull(ThirtySecond) for Em''', got {:?}",
            chord3.push_pull
        ),
    }
}

#[test]
fn test_mixed_push_pull() {
    let input = r#"Mixed Push/Pull Test - Demo
120bpm 4/4 #C

vs
'C Dm' ''Em F'''
"#;

    let chart = keyflow::parse(input).unwrap();

    // Display the chart
    println!("\n{}", chart);

    // Test Verse section
    let verse_section = &chart.sections[0];

    // Check push on 'C (eighth note early)
    let chord_c = find_chord_by_symbol(verse_section, "C").expect("Should find C chord");
    assert_eq!(chord_c.full_symbol, "C");
    match chord_c.push_pull {
        Some((true, amount)) => {
            use keyflow::chord::PushPullAmount;
            assert_eq!(amount, PushPullAmount::eighth());
        }
        _ => panic!("Expected Push(Eighth) for 'C"),
    }

    // Check pull on Dm' (eighth note late)
    let chord_dm = find_chord_by_symbol(verse_section, "Dm").expect("Should find Dm chord");
    assert_eq!(chord_dm.full_symbol, "Dm");
    match chord_dm.push_pull {
        Some((false, amount)) => {
            use keyflow::chord::PushPullAmount;
            assert_eq!(amount, PushPullAmount::eighth());
        }
        _ => panic!("Expected Pull(Eighth) for D'"),
    }

    // Check push on ''Em (sixteenth note early)
    let chord_em = find_chord_by_symbol(verse_section, "Em").expect("Should find Em chord");
    assert_eq!(chord_em.full_symbol, "Em");
    match chord_em.push_pull {
        Some((true, amount)) => {
            use keyflow::chord::PushPullAmount;
            assert_eq!(amount, PushPullAmount::sixteenth());
        }
        _ => panic!("Expected Push(Sixteenth) for ''Em"),
    }

    // Check pull on F''' (32nd note late)
    let chord_f = find_chord_by_symbol(verse_section, "F").expect("Should find F chord");
    assert_eq!(chord_f.full_symbol, "F");
    match chord_f.push_pull {
        Some((false, amount)) => {
            use keyflow::chord::PushPullAmount;
            assert_eq!(amount, PushPullAmount::thirty_second());
        }
        _ => panic!("Expected Pull(ThirtySecond) for F'''"),
    }
}

#[test]
fn test_push_pull_with_slash_notation() {
    let input = r#"Push/Pull + Slashes Test - Demo
120bpm 4/4 #C

vs
'C//// D'// 'Em// F////
"#;

    let chart = keyflow::parse(input).unwrap();

    // Display the chart
    println!("\n{}", chart);

    // Test Verse section - measures may be grouped differently with push/pull
    let verse_section = &chart.sections[0];

    // Check 'C//// - push with slash notation
    // Note: push/pull is separate from rhythm notation
    // Find the first chord
    let mut found_c = false;
    for measure in verse_section.measures() {
        for chord in &measure.chords {
            if chord.full_symbol == "C" {
                // Should have push notation
                assert!(chord.push_pull.is_some());
                if let Some((is_push, _)) = chord.push_pull {
                    assert!(is_push, "Expected 'C to have push notation");
                }
                // Should also have slash rhythm
                assert!(matches!(chord.rhythm, ChordRhythm::Slashes { .. }));
                found_c = true;
                break;
            }
        }
        if found_c {
            break;
        }
    }
    assert!(found_c, "Should have found C chord");
}

#[test]
fn test_push_pull_with_scale_degrees() {
    let input = r#"Push/Pull Scale Degrees - Demo
120bpm 4/4 #C

vs
'1 '4 5' 6'
"#;

    let chart = keyflow::parse(input).unwrap();

    // Display the chart
    println!("\n{}", chart);

    // Test Verse section
    let verse_section = &chart.sections[0];

    // All chords should parse correctly with push/pull
    let chord1 = find_chord_by_symbol(verse_section, "1").expect("Should find 1 chord");
    assert_eq!(chord1.full_symbol, "1"); // I chord (C major)

    let chord4 = find_chord_by_symbol(verse_section, "4").expect("Should find 4 chord");
    assert_eq!(chord4.full_symbol, "4"); // IV chord (F major)

    let chord5 = find_chord_by_symbol(verse_section, "5").expect("Should find 5 chord");
    assert_eq!(chord5.full_symbol, "5"); // V chord (G major)

    let chord6 = find_chord_by_symbol(verse_section, "6").expect("Should find 6 chord");
    assert_eq!(chord6.full_symbol, "6"); // vi chord (quality implied by key)
}
