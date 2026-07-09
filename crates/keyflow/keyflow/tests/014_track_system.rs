//! Test 014: Track/Voice System
//!
//! Tests the track system for parallel content including:
//! - [chords] track parsing
//! - [melody] track parsing
//! - [rhythm] track parsing
//! - Multiple tracks per section
//! - Track serialization
//! - Round-trip parsing

#[test]
fn test_single_chord_track_default() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // Should have one section with one track
    assert_eq!(chart.sections.len(), 1);
    assert_eq!(chart.sections[0].tracks.len(), 1);

    // Track should be a chord track
    let track = &chart.sections[0].tracks[0];
    assert_eq!(track.track_type, keyflow::chart::TrackType::Chords);
    assert!(track.name.is_none());

    // Should have 8 measures (padded)
    assert_eq!(track.measures.len(), 8);
}

#[test]
fn test_explicit_chords_track() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 4
[chords] Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    assert_eq!(chart.sections.len(), 1);
    assert_eq!(chart.sections[0].tracks.len(), 1);

    let track = &chart.sections[0].tracks[0];
    assert_eq!(track.track_type, keyflow::chart::TrackType::Chords);
    assert_eq!(track.measures.len(), 4);
}

#[test]
fn test_melody_track() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// Dm7/// Em7/// Fmaj7///
[melody] m{ C_8 D_8 E_4 F_4 G_4 A_4 B_4 C'_4 }
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // Should have one section with two tracks
    assert_eq!(chart.sections.len(), 1);
    assert_eq!(chart.sections[0].tracks.len(), 2);

    // First track should be chords
    let chord_track = &chart.sections[0].tracks[0];
    assert_eq!(chord_track.track_type, keyflow::chart::TrackType::Chords);
    assert_eq!(chord_track.measures.len(), 8);

    // Second track should be melody
    let melody_track = &chart.sections[0].tracks[1];
    assert_eq!(melody_track.track_type, keyflow::chart::TrackType::Melody);
    assert!(melody_track.melody.is_some());

    let melody = melody_track.melody.as_ref().unwrap();
    assert_eq!(melody.notes.len(), 8);
}

#[test]
fn test_named_melody_track() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

CH 8
[chords] Gmaj7/// Am7/// Bm7/// Cmaj7///
[melody lead] m{ G_4 A_4 B_4 C'_4 D'_4 E'_4 F#'_4 G'_4 }
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // Second track should have name "lead"
    let melody_track = &chart.sections[0].tracks[1];
    assert_eq!(melody_track.track_type, keyflow::chart::TrackType::Melody);
    assert_eq!(melody_track.name.as_deref(), Some("lead"));
}

#[test]
fn test_multiple_melody_tracks() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

CH 8
[chords] Gmaj7/// Am7/// Bm7/// Cmaj7///
[melody lead] m{ G_4 A_4 B_4 C'_4 }
[melody harmony] m{ D_4 E_4 F#_4 G_4 }
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("{}", chart);

    // Should have 3 tracks
    assert_eq!(chart.sections[0].tracks.len(), 3);

    // First: chords
    assert_eq!(
        chart.sections[0].tracks[0].track_type,
        keyflow::chart::TrackType::Chords
    );

    // Second: melody lead
    let lead = &chart.sections[0].tracks[1];
    assert_eq!(lead.track_type, keyflow::chart::TrackType::Melody);
    assert_eq!(lead.name.as_deref(), Some("lead"));
    assert!(lead.melody.is_some());

    // Third: melody harmony
    let harmony = &chart.sections[0].tracks[2];
    assert_eq!(harmony.track_type, keyflow::chart::TrackType::Melody);
    assert_eq!(harmony.name.as_deref(), Some("harmony"));
    assert!(harmony.melody.is_some());
}

#[test]
fn test_track_backward_compat_measures_method() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 4
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();

    // The measures() method should work for backward compatibility
    let section = &chart.sections[0];
    assert_eq!(section.measures().len(), 4);
    assert_eq!(section.measures()[0].chords[0].full_symbol, "Cmaj7");
}

#[test]
fn test_track_serialization_single_chord() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 4
Cmaj7/// Dm7/// Em7/// Fmaj7///
"#;

    let chart = keyflow::parse(input).unwrap();
    let syntax = chart.to_syntax();
    println!("Serialized:\n{}", syntax);

    // Should NOT have track markers (single default chord track)
    assert!(!syntax.contains("[chords]"));
    assert!(!syntax.contains("[melody]"));
}

#[test]
fn test_track_serialization_with_melody() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// Dm7/// Em7/// Fmaj7///
[melody] m{ C_4 D_4 E_4 F_4 }
"#;

    let chart = keyflow::parse(input).unwrap();
    let syntax = chart.to_syntax();
    println!("Serialized:\n{}", syntax);

    // Should have the [melody] marker since there are multiple tracks
    assert!(syntax.contains("[chords]") || syntax.contains("Cmaj7"));
    assert!(syntax.contains("[melody]"));
    assert!(syntax.contains("m{"));
}

#[test]
fn test_track_round_trip() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 8
Cmaj7/// Dm7/// Em7/// Fmaj7///
[melody] m{ C_4 D_4 E_4 F_4 }
"#;

    let chart = keyflow::parse(input).unwrap();
    println!("Original chart:");
    println!("{}", chart);

    let syntax = chart.to_syntax();
    println!("Serialized:\n{}", syntax);

    let reparsed = keyflow::parse(&syntax).unwrap();
    println!("Re-parsed chart:");
    println!("{}", reparsed);

    // Both should have same structure
    assert_eq!(chart.sections.len(), reparsed.sections.len());
    assert_eq!(
        chart.sections[0].tracks.len(),
        reparsed.sections[0].tracks.len()
    );

    // Chord track should match
    assert_eq!(
        chart.sections[0].tracks[0].measures.len(),
        reparsed.sections[0].tracks[0].measures.len()
    );

    // Melody track should match
    let orig_melody = chart.sections[0].tracks[1].melody.as_ref().unwrap();
    let new_melody = reparsed.sections[0].tracks[1].melody.as_ref().unwrap();
    assert_eq!(orig_melody.notes.len(), new_melody.notes.len());
}

#[test]
fn test_chord_track_helper_method() {
    let input = r#"
Test Song - Artist
120bpm 4/4 #C

VS 4
Cmaj7/// Dm7///
[melody] m{ C_4 D_4 }
"#;

    let chart = keyflow::parse(input).unwrap();
    let section = &chart.sections[0];

    // chord_track() should return the chord track
    let chord_track = section.chord_track();
    assert!(chord_track.is_some());
    assert_eq!(
        chord_track.unwrap().track_type,
        keyflow::chart::TrackType::Chords
    );

    // melody_tracks() should return melody tracks
    let melody_tracks: Vec<_> = section.melody_tracks().collect();
    assert_eq!(melody_tracks.len(), 1);
    assert_eq!(
        melody_tracks[0].track_type,
        keyflow::chart::TrackType::Melody
    );
}
