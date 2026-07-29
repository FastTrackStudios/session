//! Regression for issue #48: `A //// // D //` must parse as TWO measures,
//! with the bare `//` group occupying beats 1–2 of the second measure as
//! continuation Spaces so D lands on beat 3.

use keyflow_text::chart::parse_chart;
use keyflow_text::chart::types::RhythmElement;

#[test]
fn bare_slash_group_continues_previous_chord() {
    let src = "\
Slash Test
#A 120bpm 4/4

VS 2
A //// // D //";

    let chart = parse_chart(src).expect("chart should parse");
    let section = &chart.sections[0];
    let measures = section.measures();
    assert_eq!(measures.len(), 2, "A //// // D // should be two measures");

    // Measure 1: A for the full bar.
    assert_eq!(measures[0].chords.len(), 1);
    assert_eq!(measures[0].chords[0].full_symbol, "A");

    // Measure 2: two beats of continuation (Spaces), then D on beat 3.
    let m2 = &measures[1];
    assert_eq!(m2.chords.len(), 1);
    let d = &m2.chords[0];
    assert_eq!(d.full_symbol, "D");
    assert_eq!(
        d.position.beats(),
        2,
        "D should sit on beat 3 (0-indexed beat 2) of measure 2"
    );
    let space_beats: usize = m2
        .rhythm_elements
        .iter()
        .filter(|e| matches!(e, RhythmElement::Space(_)))
        .count();
    assert_eq!(space_beats, 2, "the bare // should become two beats of continuation");
}
