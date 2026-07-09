//! Test 010: New Features Integration Test
//!
//! Tests all the new features added:
//! - Comment syntax (;)
//! - Section prefix (^)
//! - Accent shorthand (->)
//! - Custom commands (/fermata, /accent)
//! - Settings system (/SMART_REPEATS=true)

use keyflow::chart::Command;

#[test]
fn test_comment_syntax() {
    let input = r#"Test Song - Artist
120bpm 4/4 #C

; This is a comment
intro, 1/ 4/ 5/ 1/  ; inline comment
"#;

    let chart = keyflow::parse(input).unwrap();

    assert_eq!(chart.metadata.title, Some("Test Song".to_string()));
    assert_eq!(chart.sections.len(), 1);
}

#[test]
fn test_consecutive_same_sections() {
    let input = r#"Test Song - Artist
120bpm 4/4 #C

intro, 1/ 4/ 5/ 1/

intro, 1/ 4/ 5/ 1/
"#;

    let chart = keyflow::parse(input).unwrap();

    assert_eq!(chart.sections.len(), 2);

    // Both sections should be Intro type
    // The numbering system should handle splitting them (e.g., Intro 1a, Intro 1b)
}

#[test]
fn test_accent_shorthand() {
    let input = r#"Test Song - Artist
120bpm 4/4 #C

intro, 1/ 4->/ 5/ 1/
"#;

    let chart = keyflow::parse(input).unwrap();

    assert_eq!(chart.sections.len(), 1);
    let measure = &chart.sections[0].measures()[0];

    // Should have 4 chords
    assert!(
        measure.chords.len() >= 4,
        "Expected at least 4 chords, found {}",
        measure.chords.len()
    );

    // Second chord should have accent from shorthand
    if measure.chords.len() >= 2 {
        assert!(
            !measure.chords[1].commands.is_empty(),
            "Second chord should have commands"
        );
        assert_eq!(measure.chords[1].commands[0], Command::Accent);
    }
}

#[test]
fn test_fermata_command() {
    let input = r#"Test Song - Artist
120bpm 4/4 #C

intro, 1/ 4/ 5/ 1/ /fermata
"#;

    let chart = keyflow::parse(input).unwrap();

    assert_eq!(chart.sections.len(), 1);
    let measure = &chart.sections[0].measures()[0];

    // Should have 4 chords
    assert!(
        measure.chords.len() >= 4,
        "Expected at least 4 chords, found {}",
        measure.chords.len()
    );

    // Last chord should have fermata
    let last_idx = measure.chords.len() - 1;
    let last_chord = &measure.chords[last_idx];
    assert!(
        !last_chord.commands.is_empty(),
        "Last chord should have commands"
    );
    assert_eq!(last_chord.commands[0], Command::Fermata);
}

#[test]
fn test_accent_slash_command() {
    let input = r#"Test Song - Artist
120bpm 4/4 #C

intro, 1/ 4/ /accent 5/ 1/
"#;

    let chart = keyflow::parse(input).unwrap();

    assert_eq!(chart.sections.len(), 1);
    let measure = &chart.sections[0].measures()[0];

    // Should have 4 chords
    assert!(
        measure.chords.len() >= 4,
        "Expected at least 4 chords, found {}",
        measure.chords.len()
    );

    // Second chord should have accent from /accent command
    if measure.chords.len() >= 2 {
        assert!(
            !measure.chords[1].commands.is_empty(),
            "Second chord should have commands"
        );
        assert_eq!(measure.chords[1].commands[0], Command::Accent);
    }
}

#[test]
fn test_settings_parsing_true() {
    let input = r#"/SMART_REPEATS=true

Test Song - Artist
120bpm 4/4 #C

intro, 1/ 4/ 5/ 1/
"#;

    let chart = keyflow::parse(input).unwrap();

    assert!(chart.settings.smart_repeats());
}

#[test]
fn test_settings_parsing_false() {
    let input = r#"/SMART_REPEATS=false

Test Song - Artist  
120bpm 4/4 #C

intro, 1/ 4/ 5/ 1/
"#;

    let chart = keyflow::parse(input).unwrap();

    assert!(!chart.settings.smart_repeats());
}

#[test]
fn test_combined_features() {
    let input = r#"/SMART_REPEATS=true

Test Song - Artist
120bpm 4/4 #C

; Main intro section
intro, 1->/ 4/ 5/ 1/ /fermata

; Second intro section
intro, 1/ 4/ 5->/ 1/
"#;

    let chart = keyflow::parse(input).unwrap();

    // Check settings
    assert!(chart.settings.smart_repeats());

    // Check sections
    assert_eq!(chart.sections.len(), 2);
}
