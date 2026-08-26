#![cfg(feature = "midi-import")]
//! Test 024: MIDI Import - Movin' Out (Sammy Rae & The Friends)
//!
//! Tests new features:
//! - Staccato push notation (>'.Chord) for eighth-note-only pushed chords
//! - Line breaks with | for complex sections
//! - Repeat syntax (x2, x4) for repeated patterns
//! - Solo sections (Guitar Solo, Synth Solo, Drum Solo)
//! - Sixteenth note rhythms

use keyflow::engraver::import::{generate_chart_text, MidiChartConfig, MidiFile};

fn load_midi() -> MidiFile {
    let bytes = include_bytes!("fixtures/movin_out_sammy_rae.mid");
    MidiFile::parse(bytes).expect("Failed to parse MIDI file")
}

fn generate_movin_out_chart() -> String {
    let midi = load_midi();
    let config = MidiChartConfig {
        key_root: Some("A".to_string()),
        title: Some(
            "Movin' Out - Sammy Rae & The Friends\nTranscribed By: Cody Wright".to_string(),
        ),
        swing: midi.swing(),
        ..Default::default()
    };
    generate_chart_text(&midi, &config)
}

#[test]
fn test_chart_generates_without_panic() {
    let chart_text = generate_movin_out_chart();
    println!("{}", chart_text);
    assert!(!chart_text.is_empty(), "Chart text should not be empty");
}

// NOTE: This test documents the EXPECTED output after implementing new features:
// - Staccato push notation (>'.Chord)
// - Line breaks with |
// - Repeat syntax (x2, x4)
// - Solo sections
// - Sixteenth note rhythms
//
// For now, this test is SKIPPED until those features are implemented.
// Run test_chart_generates_without_panic to see current output.
#[test]
#[ignore = "lineage divergence: MIDI-import/chart-Display canonicality undecided"]
fn test_exact_output() {
    let chart_text = generate_movin_out_chart();

    let expected = r#"Movin' Out - Sammy Rae & The Friends
Transcribed By: Cody Wright
150bpm 4/4 #A

COUNT 2

IN
'F#m7 'B7 'E 'Amaj7 x2

VS 8
'F#m7 'B7 'E 'Amaj7 x2

VS
F#m7 B7 D E
'F#m7 'B7 'E 'Amaj7

CH 7
D E C#/F 'F#m // 'Ebm // 
D 'Abm7b5 'C#7

INST 4
'F#m7 'B7 'E 'Amaj7

VS
>'.F#m7 r1. B7' //
Abm7b5/D // 'Ab //  'Amaj7

'F#m7 'B7 'E 'Amaj7

VS
F#m7 B7 D E
'F#m7 'B7 'E 'Amaj7

CH
D // D/F# //  E // E7/D // C# // C#/F // F#m // F#m/E // 
D // D/F# // Abm7b5 Db7

INST 8
>'.F#m7 S1 x4
 

VS
>'.F#m7 r1 r1 D E
'F#m7 'B7 'E 'Amaj7

CH
D E C#/F 'F#m // 'Ebm // 
D 'Abm7b5 'C#7

INST
'F#m7 'B7 'E 'Amaj7 x2
'F#m7 // 'F#7/C# // 'Bm7 // 'Bm7/D // 
'.E r4 .E_8 r8 r8 .E/G#_8 r4 | >A_8 r16 >A_8 r16 >A_8 r8 B_4.

Outro 8
F#7/A# Bm7 E/G# C#7 // Bdim7 // x2

Guitar Solo 16
F#7/A# Bm7 E/G# C#7 // Bdim7 // x4

Synth Solo 16
F#7/A# Bm7 E/G# C#7 // Bdim7 // x4

Drum Solo 8
F#7/A# Bm7 E/G# C#7 // Bdim7 // x2
"#;

    assert_eq!(
        chart_text.trim(),
        expected.trim(),
        "Chart text must match EXACTLY.\n\nActual:\n{}\n\nExpected:\n{}",
        chart_text.trim(),
        expected.trim(),
    );
}
