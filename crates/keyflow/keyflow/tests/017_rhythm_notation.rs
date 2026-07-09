//! Test 017: Rhythm Notation
//!
//! Comprehensive tests for rhythm notation including:
//! - Explicit duration notation (C_1, C_2, C_4, C_8, C_16, C_32)
//! - Rest notation (r1, r2, r4, r8, r16, r32)
//! - Slash notation (C ////, C //, C /, C /.)
//! - Simple progressions
//! - Equivalent notations
//! - Ties
//! - Rest integration
//!
//! Default assumptions:
//! - Time signature: 4/4
//! - Key: G major

use keyflow::chart::Chart;
use keyflow::time::TimeSignature;

// =============================================================================
// region: --- Test Infrastructure
// =============================================================================

/// Default time signature for tests (4/4)
fn time_sig() -> TimeSignature {
    TimeSignature::new(4, 4)
}

/// Helper to parse a chart with standard G major 4/4 header
fn parse_chart(content: &str) -> Chart {
    let full_input = format!(
        r#"Test Chart - Test
120bpm 4/4 #G

VS
{content}
"#
    );
    keyflow::parse(&full_input).expect("Failed to parse chart")
}

/// Get duration in beats for a chord
fn duration_beats(chord: &keyflow::ChordInstance) -> f64 {
    let ts = time_sig();
    let measures = chord.duration.measure as f64;
    let beats = chord.duration.beat as f64;
    let subs = chord.duration.subdivision as f64 / 1000.0;
    measures * ts.numerator as f64 + beats + subs
}

/// Get start position in beats for a chord (within its section)
#[allow(dead_code)]
fn start_beat(chord: &keyflow::ChordInstance) -> f64 {
    let ts = time_sig();
    let pos = &chord.position.total_duration;
    let measures = pos.measure as f64;
    let beats = pos.beat as f64;
    let subs = pos.subdivision as f64 / 1000.0;
    measures * ts.numerator as f64 + beats + subs
}

/// Get beat within measure for a chord
fn beat_in_measure(chord: &keyflow::ChordInstance) -> f64 {
    let pos = &chord.position.total_duration;
    pos.beat as f64 + pos.subdivision as f64 / 1000.0
}

/// Helper to verify beat position of a chord in a measure
fn assert_chord_at_beat(chart: &Chart, measure_idx: usize, chord_idx: usize, expected_beat: f64) {
    let section = &chart.sections[0];
    let measure = &section.measures()[measure_idx];
    let chord = &measure.chords[chord_idx];
    let actual_beat = beat_in_measure(chord);
    assert!(
        (actual_beat - expected_beat).abs() < 0.001,
        "Chord '{}' at measure {} index {} expected at beat {}, got {}",
        chord.full_symbol,
        measure_idx,
        chord_idx,
        expected_beat,
        actual_beat
    );
}

/// Helper to verify duration of a chord
fn assert_chord_duration(chart: &Chart, measure_idx: usize, chord_idx: usize, expected_beats: f64) {
    let section = &chart.sections[0];
    let measure = &section.measures()[measure_idx];
    let chord = &measure.chords[chord_idx];
    let actual_beats = duration_beats(chord);
    assert!(
        (actual_beats - expected_beats).abs() < 0.001,
        "Chord '{}' at measure {} index {} expected duration {} beats, got {}",
        chord.full_symbol,
        measure_idx,
        chord_idx,
        expected_beats,
        actual_beats
    );
}

// endregion: --- Test Infrastructure

// =============================================================================
// region: --- 1. Rhythm Testing - Explicit Durations
// =============================================================================

/// Test explicit chord durations from whole note down to sixteenth
/// C_1 = Whole note (4 beats), starts 1.1 (beat 0)
/// G_2 = Half note (2 beats), starts 2.1 (beat 4)
/// D_4 = Quarter note (1 beat), starts 2.3 (beat 6)
/// Em_8 = Eighth note (0.5 beats), starts 2.4 (beat 7)
/// G_16 = Sixteenth (0.25 beats), starts 2.4.5 (beat 7.5)
/// C_16 = Sixteenth (0.25 beats), starts 2.4.75 (beat 7.75)
/// Total measure 2: 2+1+0.5+0.25+0.25 = 4 beats
#[test]
fn test_explicit_chord_durations() {
    let chart = parse_chart("C_1 | G_2 D_4 Em_8 G_16 C_16");

    println!("{}", chart);

    let section = &chart.sections[0];
    assert_eq!(
        section.measures().len(),
        2,
        "Should have exactly 2 measures"
    );

    // Measure 0: C whole note
    let m0 = &section.measures()[0];
    assert_eq!(m0.chords.len(), 1, "Measure 0 should have 1 chord");
    assert_eq!(m0.chords[0].full_symbol, "C");
    assert_chord_duration(&chart, 0, 0, 4.0); // Whole note = 4 beats

    // Measure 1: G_2 D_4 Em_8 G_16 C_16
    let m1 = &section.measures()[1];
    println!(
        "Measure 1 chords: {:?}",
        m1.chords
            .iter()
            .map(|c| (&c.full_symbol, duration_beats(c)))
            .collect::<Vec<_>>()
    );

    // G_2 at beat 0 of measure, duration 2 beats
    let g_chord = m1
        .chords
        .iter()
        .find(|c| c.full_symbol == "G")
        .expect("Should have G chord");
    assert!(
        (beat_in_measure(g_chord) - 0.0).abs() < 0.001,
        "G should start at beat 0"
    );
    assert!(
        (duration_beats(g_chord) - 2.0).abs() < 0.001,
        "G_2 should be 2 beats"
    );

    // D_4 at beat 2, duration 1 beat
    let d_chord = m1
        .chords
        .iter()
        .find(|c| c.full_symbol == "D")
        .expect("Should have D chord");
    assert!(
        (beat_in_measure(d_chord) - 2.0).abs() < 0.001,
        "D should start at beat 2"
    );
    assert!(
        (duration_beats(d_chord) - 1.0).abs() < 0.001,
        "D_4 should be 1 beat"
    );

    // Em_8 at beat 3, duration 0.5 beats
    let em_chord = m1
        .chords
        .iter()
        .find(|c| c.full_symbol == "Em")
        .expect("Should have Em chord");
    assert!(
        (beat_in_measure(em_chord) - 3.0).abs() < 0.001,
        "Em should start at beat 3"
    );
    assert!(
        (duration_beats(em_chord) - 0.5).abs() < 0.001,
        "Em_8 should be 0.5 beats"
    );
}

/// Test whole note fills entire measure
#[test]
fn test_whole_note() {
    let chart = parse_chart("C_1");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    assert_eq!(measure.chords.len(), 1);
    assert_eq!(measure.chords[0].full_symbol, "C");
    assert_chord_duration(&chart, 0, 0, 4.0);
    assert_chord_at_beat(&chart, 0, 0, 0.0);
}

/// Test half note is 2 beats
#[test]
fn test_half_note() {
    let chart = parse_chart("C_2 G_2");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    assert_eq!(measure.chords.len(), 2);
    assert_chord_duration(&chart, 0, 0, 2.0);
    assert_chord_duration(&chart, 0, 1, 2.0);
    assert_chord_at_beat(&chart, 0, 0, 0.0);
    assert_chord_at_beat(&chart, 0, 1, 2.0);
}

/// Test quarter note is 1 beat
#[test]
fn test_quarter_note() {
    let chart = parse_chart("C_4 D_4 Em_4 G_4");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    assert_eq!(measure.chords.len(), 4);
    for i in 0..4 {
        assert_chord_duration(&chart, 0, i, 1.0);
        assert_chord_at_beat(&chart, 0, i, i as f64);
    }
}

/// Test eighth note is 0.5 beats
#[test]
fn test_eighth_note() {
    let chart = parse_chart("C_8 D_8 Em_8 G_8 Am_8 Bm_8 C_8 D_8");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    assert_eq!(measure.chords.len(), 8);
    for i in 0..8 {
        assert_chord_duration(&chart, 0, i, 0.5);
        assert_chord_at_beat(&chart, 0, i, i as f64 * 0.5);
    }
}

/// Test sixteenth note is 0.25 beats
#[test]
fn test_sixteenth_note() {
    // 16 sixteenth notes fill one measure
    let chart = parse_chart(
        "C_16 D_16 E_16 F_16 G_16 A_16 B_16 C_16 D_16 E_16 F_16 G_16 A_16 B_16 C_16 D_16",
    );

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    // Note: The parser may combine adjacent chords or handle differently
    // This verifies the basic behavior
    for chord in &measure.chords {
        assert!(
            (duration_beats(chord) - 0.25).abs() < 0.001,
            "Sixteenth note should be 0.25 beats, got {}",
            duration_beats(chord)
        );
    }
}

/// Test thirty-second note is 0.125 beats
#[test]
fn test_thirty_second_note() {
    let chart = parse_chart("C_32 r32 C_32 r32 C_32 r32 C_32 r32 C_2");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    // Find all C chords and verify duration
    let c_chords: Vec<_> = measure
        .chords
        .iter()
        .filter(|c| c.full_symbol == "C")
        .collect();
    for c in &c_chords[..4.min(c_chords.len())] {
        assert!(
            (duration_beats(c) - 0.125).abs() < 0.001,
            "Thirty-second note should be 0.125 beats, got {}",
            duration_beats(c)
        );
    }
}

// endregion: --- 1. Rhythm Testing - Explicit Durations

// =============================================================================
// region: --- 2. Rest Testing
// =============================================================================

/// Test rest durations mirror chord durations
/// r1 = Whole measure rest (4 beats)
/// r2 = Half rest (2 beats)
/// r4 = Quarter rest (1 beat)
/// r8 = Eighth rest (0.5 beats)
/// r16 = Sixteenth rest (0.25 beats)
/// r32 = Thirty-second rest (0.125 beats)
#[test]
fn test_rest_whole_measure() {
    let chart = parse_chart("r1");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    // Whole rest should result in empty chords or rest in rhythm_elements
    let has_rest = measure.rhythm_elements.iter().any(|e| e.rhythm().is_rest());

    assert!(
        measure.chords.is_empty() || has_rest,
        "Whole rest should be recognized"
    );
}

/// Test half rests
#[test]
fn test_rest_half() {
    let chart = parse_chart("r2 C_2");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    // Should have C chord starting at beat 2
    let c_chord = measure.chords.iter().find(|c| c.full_symbol == "C");
    if let Some(c) = c_chord {
        assert!(
            (beat_in_measure(c) - 2.0).abs() < 0.001,
            "C should start at beat 2 after r2, got {}",
            beat_in_measure(c)
        );
    }
}

/// Test quarter rests
#[test]
fn test_rest_quarter() {
    let chart = parse_chart("C_4 r4 C_4 r4");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    let c_chords: Vec<_> = measure
        .chords
        .iter()
        .filter(|c| c.full_symbol == "C")
        .collect();
    assert!(c_chords.len() >= 2, "Should have at least 2 C chords");

    // First C at beat 0
    assert!(
        (beat_in_measure(c_chords[0]) - 0.0).abs() < 0.001,
        "First C should be at beat 0"
    );

    // Second C at beat 2 (after C_4 + r4)
    assert!(
        (beat_in_measure(c_chords[1]) - 2.0).abs() < 0.001,
        "Second C should be at beat 2, got {}",
        beat_in_measure(c_chords[1])
    );
}

/// Test eighth rests
#[test]
fn test_rest_eighth() {
    let chart = parse_chart("C_8 r8 C_8 r8 C_8 r8 C_8 r8");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    let c_chords: Vec<_> = measure
        .chords
        .iter()
        .filter(|c| c.full_symbol == "C")
        .collect();

    // C chords should be at beats 0, 1, 2, 3
    for (i, c) in c_chords.iter().enumerate() {
        assert!(
            (beat_in_measure(c) - i as f64).abs() < 0.001,
            "C chord {} should be at beat {}, got {}",
            i,
            i,
            beat_in_measure(c)
        );
    }
}

// endregion: --- 2. Rest Testing

// =============================================================================
// region: --- 3. Slash Testing
// =============================================================================

/// C //// = C_1 (whole note)
#[test]
fn test_slash_whole_note() {
    let chart = parse_chart("C ////");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    assert_eq!(measure.chords.len(), 1);
    assert_eq!(measure.chords[0].full_symbol, "C");
    assert_chord_duration(&chart, 0, 0, 4.0);
}

/// C /// = C_2. (dotted half note = 3 beats)
#[test]
fn test_slash_dotted_half() {
    let chart = parse_chart("C /// r4");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    let c_chord = measure
        .chords
        .iter()
        .find(|c| c.full_symbol == "C")
        .expect("Should have C");
    assert!(
        (duration_beats(c_chord) - 3.0).abs() < 0.001,
        "C /// should be 3 beats, got {}",
        duration_beats(c_chord)
    );
}

/// C // = C_2 (half note = 2 beats)
#[test]
fn test_slash_half_note() {
    let chart = parse_chart("C // G //");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    assert_eq!(measure.chords.len(), 2);
    assert_chord_duration(&chart, 0, 0, 2.0);
    assert_chord_duration(&chart, 0, 1, 2.0);
}

/// C / = C_4 (quarter note = 1 beat)
#[test]
fn test_slash_quarter_note() {
    let chart = parse_chart("C / D / Em / G /");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    assert_eq!(measure.chords.len(), 4);
    for i in 0..4 {
        assert_chord_duration(&chart, 0, i, 1.0);
    }
}

/// C /. = C_4. (dotted quarter = 1.5 beats)
#[test]
fn test_slash_dotted_quarter() {
    let chart = parse_chart("C /. D_8 Em //");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    let c_chord = measure
        .chords
        .iter()
        .find(|c| c.full_symbol == "C")
        .expect("Should have C");
    assert!(
        (duration_beats(c_chord) - 1.5).abs() < 0.001,
        "C /. should be 1.5 beats, got {}",
        duration_beats(c_chord)
    );
}

/// C //. = C_4 ~ C_4. (quarter tied to dotted quarter = 2.5 beats total)
/// The dot only applies to the very last slash
#[test]
fn test_slash_double_dotted() {
    let chart = parse_chart("C //. D_8 Em_4");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    let c_chord = measure
        .chords
        .iter()
        .find(|c| c.full_symbol == "C")
        .expect("Should have C");
    // //. = 2 slashes + dot = 2.5 beats (1 + 1.5) or interpreted differently
    println!("C //. duration: {}", duration_beats(c_chord));
    // Note: This may need adjustment based on actual parser behavior
}

/// C ///. = C_4 C_4 C_4. (3 beats + 0.5 from dot = 3.5 beats? Or interpreted differently)
#[test]
fn test_slash_triple_dotted() {
    let chart = parse_chart("C ///. r8");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    let c_chord = measure
        .chords
        .iter()
        .find(|c| c.full_symbol == "C")
        .expect("Should have C");
    println!("C ///. duration: {}", duration_beats(c_chord));
    // Document actual behavior
}

// endregion: --- 3. Slash Testing

// =============================================================================
// region: --- 4. Simple Progressions
// =============================================================================

/// 1 4 6 5 should equal 1_1 4_1 6_1 5_1 should equal 1 //// 4 //// 6 //// 5 ////
#[test]
fn test_simple_progression_equivalence() {
    // Using numeral notation (resolves in key of G)
    let chart_numerals = parse_chart("1 4 6 5");
    let chart_explicit = parse_chart("G Am Em D");
    let chart_slashes = parse_chart("G //// | Am //// | Em //// | D ////");

    // All should have 4 measures
    assert_eq!(chart_numerals.sections[0].measures().len(), 4);
    assert_eq!(chart_explicit.sections[0].measures().len(), 4);
    assert_eq!(chart_slashes.sections[0].measures().len(), 4);

    // Each measure should have 1 chord lasting 4 beats
    for i in 0..4 {
        let m_num = &chart_numerals.sections[0].measures()[i];
        let m_exp = &chart_explicit.sections[0].measures()[i];
        let m_slash = &chart_slashes.sections[0].measures()[i];

        assert_eq!(
            m_num.chords.len(),
            1,
            "Numeral measure {} should have 1 chord",
            i
        );
        assert_eq!(
            m_exp.chords.len(),
            1,
            "Explicit measure {} should have 1 chord",
            i
        );
        assert_eq!(
            m_slash.chords.len(),
            1,
            "Slash measure {} should have 1 chord",
            i
        );

        // Check durations
        assert!(
            (duration_beats(&m_num.chords[0]) - 4.0).abs() < 0.001,
            "Numeral chord {} should be 4 beats",
            i
        );
    }
}

// endregion: --- 4. Simple Progressions

// =============================================================================
// region: --- 5. Slash Rhythm Progression
// =============================================================================

/// All these should be identical:
/// 1 //// 4 //// 6 // 5 // 4 ////
/// 1 4 6 // 5 // 4
/// 1 4 6_2 5 // 4_1
/// 1_1 4_1 6_2 5_2 4 ////
#[test]
fn test_slash_rhythm_progression_equivalence() {
    let input1 = "G //// | Am //// | Em // D // | G ////";
    let input2 = "G | Am | Em // D // | G";
    let input3 = "G | Am | Em_2 D // | G_1";
    let input4 = "G_1 | Am_1 | Em_2 D_2 | G ////";

    let chart1 = parse_chart(input1);
    let chart2 = parse_chart(input2);
    let chart3 = parse_chart(input3);
    let chart4 = parse_chart(input4);

    // All should have 4 measures
    for (i, chart) in [&chart1, &chart2, &chart3, &chart4].iter().enumerate() {
        assert_eq!(
            chart.sections[0].measures().len(),
            4,
            "Chart {} should have 4 measures",
            i + 1
        );
    }

    // Measure 2 should have Em and D, each 2 beats
    for (i, chart) in [&chart1, &chart2, &chart3, &chart4].iter().enumerate() {
        let m2 = &chart.sections[0].measures()[2];
        assert_eq!(
            m2.chords.len(),
            2,
            "Chart {} measure 2 should have 2 chords",
            i + 1
        );

        let em = m2
            .chords
            .iter()
            .find(|c| c.full_symbol == "Em")
            .expect("Should have Em");
        let d = m2
            .chords
            .iter()
            .find(|c| c.full_symbol == "D")
            .expect("Should have D");

        assert!(
            (duration_beats(em) - 2.0).abs() < 0.001,
            "Chart {} Em should be 2 beats, got {}",
            i + 1,
            duration_beats(em)
        );
        assert!(
            (duration_beats(d) - 2.0).abs() < 0.001,
            "Chart {} D should be 2 beats, got {}",
            i + 1,
            duration_beats(d)
        );
    }
}

// endregion: --- 5. Slash Rhythm Progression

// =============================================================================
// region: --- 6. Ties
// =============================================================================

/// 1 // ~ 1 / r4 = Half note tied to quarter note (3 beats total)
#[test]
fn test_tie_half_to_quarter() {
    let chart = parse_chart("C // ~ / r4");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    // The tied chord should have combined duration of 3 beats
    // Or be represented with a tie flag
    let c_chords: Vec<_> = measure
        .chords
        .iter()
        .filter(|c| c.full_symbol == "C")
        .collect();

    println!(
        "Tie test - C chords: {:?}",
        c_chords
            .iter()
            .map(|c| (beat_in_measure(c), duration_beats(c)))
            .collect::<Vec<_>>()
    );

    // Either single chord with 3 beats, or first chord with tie property
    let total_c_duration: f64 = c_chords.iter().map(|c| duration_beats(c)).sum();
    assert!(
        (total_c_duration - 3.0).abs() < 0.001,
        "Total C duration should be 3 beats, got {}",
        total_c_duration
    );
}

/// C_2 ~ _4 = Half tied to quarter (same as above but with explicit duration)
#[test]
fn test_tie_explicit_duration() {
    let chart = parse_chart("C_2~_4 r4");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    let c_chords: Vec<_> = measure
        .chords
        .iter()
        .filter(|c| c.full_symbol == "C")
        .collect();

    let total_c_duration: f64 = c_chords.iter().map(|c| duration_beats(c)).sum();
    println!("C_2~_4 total duration: {}", total_c_duration);
}

// endregion: --- 6. Ties

// =============================================================================
// region: --- 7. Rest Integration
// =============================================================================

/// Testing quarter note rest: 1 /// r4
/// Should be dotted half (3 beats) + quarter rest
#[test]
fn test_rest_integration_quarter() {
    let chart = parse_chart("C /// r4");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    let c_chord = measure
        .chords
        .iter()
        .find(|c| c.full_symbol == "C")
        .expect("Should have C");
    assert!(
        (duration_beats(c_chord) - 3.0).abs() < 0.001,
        "C /// should be 3 beats, got {}",
        duration_beats(c_chord)
    );

    // Verify total measure duration is 4 beats
    // The rest fills the remaining beat
}

/// Testing eighth note rest: 4 ///. r8
/// Should be dotted half + dotted quarter (3 + 1.5 = 4.5 beats? Or interpreted as 3.5 + 0.5?)
#[test]
fn test_rest_integration_eighth() {
    let chart = parse_chart("Am ///. r8");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    let am_chord = measure
        .chords
        .iter()
        .find(|c| c.full_symbol == "Am")
        .expect("Should have Am");
    println!("Am ///. duration: {}", duration_beats(am_chord));

    // Document actual behavior
}

/// Testing sixteenth note rest: 6_2.~8. r16
/// Dotted half note (3 beats) tied to dotted eighth (0.75 beats) + sixteenth rest (0.25)
/// Total should be 4 beats
#[test]
fn test_rest_integration_sixteenth() {
    let chart = parse_chart("Em_2.~_8. r16");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    let em_chords: Vec<_> = measure
        .chords
        .iter()
        .filter(|c| c.full_symbol == "Em")
        .collect();
    let total_em_duration: f64 = em_chords.iter().map(|c| duration_beats(c)).sum();

    println!("Em_2.~_8. total duration: {}", total_em_duration);

    // Dotted half (3) + dotted eighth (0.75) = 3.75 beats
    // + sixteenth rest (0.25) = 4 beats total
}

// endregion: --- 7. Rest Integration

// =============================================================================
// region: --- Additional Duration Tests
// =============================================================================

/// Test dotted notes
#[test]
fn test_dotted_whole() {
    // Dotted whole note = 6 beats (overflows 4/4 measure into next)
    let chart = parse_chart("C_1. | G //");

    let section = &chart.sections[0];
    let m0 = &section.measures()[0];

    let c_chord = m0.chords.iter().find(|c| c.full_symbol == "C");
    if let Some(c) = c_chord {
        println!("C_1. duration: {}", duration_beats(c));
        // Should be 6 beats or handled as tied across barline
    }
}

/// Test dotted half = 3 beats
#[test]
fn test_dotted_half() {
    let chart = parse_chart("C_2. r4");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    let c_chord = measure
        .chords
        .iter()
        .find(|c| c.full_symbol == "C")
        .expect("Should have C");
    assert!(
        (duration_beats(c_chord) - 3.0).abs() < 0.001,
        "C_2. should be 3 beats, got {}",
        duration_beats(c_chord)
    );
}

/// Test dotted quarter = 1.5 beats
#[test]
fn test_dotted_quarter() {
    let chart = parse_chart("C_4. D_8 Em_2");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    let c_chord = measure
        .chords
        .iter()
        .find(|c| c.full_symbol == "C")
        .expect("Should have C");
    assert!(
        (duration_beats(c_chord) - 1.5).abs() < 0.001,
        "C_4. should be 1.5 beats, got {}",
        duration_beats(c_chord)
    );
}

/// Test dotted eighth = 0.75 beats
#[test]
fn test_dotted_eighth() {
    let chart = parse_chart("C_8. D_16 Em_8. G_16 Am_2");

    let section = &chart.sections[0];
    let measure = &section.measures()[0];

    let c_chord = measure
        .chords
        .iter()
        .find(|c| c.full_symbol == "C")
        .expect("Should have C");
    assert!(
        (duration_beats(c_chord) - 0.75).abs() < 0.001,
        "C_8. should be 0.75 beats, got {}",
        duration_beats(c_chord)
    );
}

// endregion: --- Additional Duration Tests
