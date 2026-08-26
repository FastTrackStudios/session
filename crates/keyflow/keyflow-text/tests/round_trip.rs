//! Chart round-trip: parse → `to_syntax()` → parse again.
//!
//! Lived in `keyflow-proto`'s unit tests until the repo split. It needs
//! the parser, and `keyflow-proto` sits a layer below `keyflow-text` —
//! the dev-dependency that bought was the one edge pointing the wrong
//! way across the daw/session repo boundary, so the test moved to the
//! crate that owns the parser instead.

#[test]
fn a_chart_survives_a_serialize_parse_round_trip() {
    let input = r#"
My Song - Test Artist
120bpm 4/4 #C

Intro 4
C G Am F

VS 8
C G Am F x2

CH 8
F C G Am
"#;

    let chart1 = keyflow_text::chart::parse_chart(input).expect("Should parse successfully");
    let output = chart1.to_syntax();
    let chart2 =
        keyflow_text::chart::parse_chart(&output).expect("Should parse serialized output");

    assert_eq!(chart1.metadata.title, chart2.metadata.title);
    assert_eq!(chart1.metadata.artist, chart2.metadata.artist);
    assert_eq!(chart1.tempo, chart2.tempo);
    assert_eq!(chart1.initial_key, chart2.initial_key);
    assert_eq!(chart1.sections.len(), chart2.sections.len());

    for (s1, s2) in chart1.sections.iter().zip(chart2.sections.iter()) {
        assert_eq!(s1.measures().len(), s2.measures().len());
    }
}
