//! Integration tests for lyrics parsing, serialization, and alignment

use keyflow::text::chart::parse_chart;

#[test]
fn test_plain_lyrics_track() {
    let input = r#"
Test Song
120bpm 4/4 #C

VS
C G Am F
[lyrics] Twinkle twinkle little star
"#;

    let chart = parse_chart(input).expect("Failed to parse chart");
    assert_eq!(chart.sections.len(), 1);

    let section = &chart.sections[0];

    // Should have a chord track and a lyrics track
    assert!(section.chord_track().is_some());
    assert!(section.lyrics_track().is_some());

    let lyrics_track = section.lyrics_track().unwrap();
    let lyric_line = lyrics_track.lyrics.as_ref().expect("Should have lyrics");
    assert_eq!(lyric_line.syllables.len(), 4);
    assert_eq!(lyric_line.syllables[0].text, "Twinkle");
    assert_eq!(lyric_line.syllables[3].text, "star");
}

#[test]
fn test_lyrics_with_chord_markers() {
    let input = r#"
Test Song
120bpm 4/4 #Gm

VS
Gm //// | A# //// | F //// | Gm ////
[lyrics] {Gm}Slow down you {A#}crazy child, {F}you're so {Gm}am-bi-tious
"#;

    let chart = parse_chart(input).expect("Failed to parse chart");
    let section = &chart.sections[0];

    let lyrics_track = section.lyrics_track().unwrap();
    let lyric_line = lyrics_track.lyrics.as_ref().expect("Should have lyrics");

    // Check that chord attachments exist
    let chords_attached: Vec<_> = lyric_line
        .syllables
        .iter()
        .filter_map(|s| s.chord.as_ref())
        .collect();
    assert_eq!(chords_attached.len(), 4, "Should have 4 chord attachments");
    assert_eq!(chords_attached[0], "Gm");
    assert_eq!(chords_attached[1], "A#");
    assert_eq!(chords_attached[2], "F");
    assert_eq!(chords_attached[3], "Gm");
}

#[test]
fn test_lyrics_with_hyphenated_syllables() {
    let input = r#"
120bpm 4/4 #C

VS
C G Am F
[lyrics] A-ma-zing grace how sweet
"#;

    let chart = parse_chart(input).expect("Failed to parse chart");
    let section = &chart.sections[0];
    let lyric_line = section.lyrics_track().unwrap().lyrics.as_ref().unwrap();

    // "A-ma-zing" splits into 3 syllables, then "grace", "how", "sweet" = 6 total
    assert_eq!(lyric_line.syllables.len(), 6);
    assert_eq!(lyric_line.syllables[0].text, "A");
    assert!(lyric_line.syllables[0].hyphen_after);
    assert_eq!(lyric_line.syllables[1].text, "ma");
    assert!(lyric_line.syllables[1].hyphen_after);
    assert_eq!(lyric_line.syllables[2].text, "zing");
    assert!(!lyric_line.syllables[2].hyphen_after);
}

#[test]
fn test_chord_syllable_alignment_computed() {
    let input = r#"
Test Song
120bpm 4/4 #C

VS
C G Am F
[lyrics] {C}Hello {G}world {Am}how {F}are you
"#;

    let chart = parse_chart(input).expect("Failed to parse chart");
    let section = &chart.sections[0];

    // Alignment should be computed since we have both chords and lyrics
    assert!(
        section.alignment.is_some(),
        "Alignment should be computed for section with chords + lyrics"
    );

    let alignment = section.alignment.as_ref().unwrap();
    assert!(
        !alignment.mappings.is_empty(),
        "Should have at least one mapping"
    );
}

#[test]
fn test_lyrics_round_trip_with_chords() {
    let input = r#"
Test Song
120bpm 4/4 #C

VS
C G Am F
[lyrics] {C}Hello {G}world {Am}foo {F}bar
"#;

    let chart = parse_chart(input).expect("Failed to parse chart");
    let output = chart.to_syntax();

    // Verify the output contains the lyrics line with chord markers
    assert!(
        output.contains("{C}Hello"),
        "Output should contain {{C}}Hello, got:\n{}",
        output
    );
    assert!(
        output.contains("{G}world"),
        "Output should contain {{G}}world, got:\n{}",
        output
    );
}

#[test]
fn test_lyrics_round_trip_plain_text() {
    let input = r#"
Test Song
120bpm 4/4 #C

VS
C G Am F
[lyrics] just plain words here
"#;

    let chart = parse_chart(input).expect("Failed to parse chart");
    let output = chart.to_syntax();

    // Plain lyrics should be serialized without chord markers
    assert!(
        output.contains("just plain words here"),
        "Output should contain plain lyrics, got:\n{}",
        output
    );
}

#[test]
fn test_document_parsing() {
    let content = r#"--- keyflow ---
Test Song
120bpm 4/4 #C

VS
C G Am F

--- chordpro ---
{title: Test Song}
[vs]
[C]Hello [G]World [Am]How [F]Are
"#;

    let (chart, doc) = keyflow::parse_document(content).expect("Failed to parse document");

    // Document should have 2 blocks
    assert_eq!(doc.blocks.len(), 2);
    assert!(doc.find_block("keyflow").is_some());
    assert!(doc.find_block("chordpro").is_some());

    // Chart should be parsed from the keyflow block
    assert_eq!(chart.sections.len(), 1);
    assert_eq!(chart.sections[0].measures().len(), 4);
}

#[test]
fn test_document_parsing_no_delimiters() {
    // Without delimiters, entire content should be treated as keyflow
    let content = r#"
Test Song
120bpm 4/4 #C

VS
C G Am F
"#;

    let (chart, doc) = keyflow::parse_document(content).expect("Failed to parse document");
    assert!(doc.is_plain_keyflow());
    assert_eq!(chart.sections.len(), 1);
}

#[test]
fn test_section_without_lyrics_has_no_alignment() {
    let input = r#"
120bpm 4/4 #C

VS
C G Am F
"#;

    let chart = parse_chart(input).expect("Failed to parse chart");
    assert!(
        chart.sections[0].alignment.is_none(),
        "Section without lyrics should have no alignment"
    );
}
