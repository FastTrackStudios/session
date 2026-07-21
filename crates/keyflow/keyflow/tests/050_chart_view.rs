//! End-to-end tests for the chart-view transposition / notation service.
//!
//! These exercise the real path a UI takes: `keyflow::parse` (text → `Chart`)
//! → `keyflow::apply_view` → inspect the rewritten chord symbols. They live in
//! the facade crate so there is a single `keyflow-proto` in the graph (the
//! proto crate's own unit tests can't use the text parser because of the
//! keyflow-text ↔ keyflow-proto dev-dep cycle).

use keyflow::transpose::{apply_view, ChartView, NotationSystem};
use keyflow::{parse, Chart, Key};

/// Visible chord symbols across a parsed chart, in order.
fn symbols(chart: &Chart) -> Vec<String> {
    let mut out = Vec::new();
    for section in &chart.sections {
        for track in &section.tracks {
            for measure in &track.measures {
                for chord in &measure.chords {
                    let s = &chord.full_symbol;
                    if !s.is_empty() && s != "s" && s != "r" {
                        out.push(s.clone());
                    }
                }
            }
        }
    }
    out
}

fn key(s: &str) -> Key {
    Key::parse(s).unwrap()
}

fn chart(src: &str) -> Chart {
    parse(src).expect("chart should parse")
}

#[test]
fn letters_a_to_g_end_to_end() {
    let c = chart("120bpm 4/4 #A\n\nVS 4\nA D E F#m\n");
    let out = apply_view(
        &c,
        &ChartView {
            target_key: Some(key("G")),
            notation: NotationSystem::Letters,
            capo: 0,
        },
    );
    assert_eq!(symbols(&out), vec!["G", "C", "D", "Em"]);
    assert_eq!(out.initial_key, Some(key("G")));
    // Source chart is untouched.
    assert_eq!(symbols(&c), vec!["A", "D", "E", "F#m"]);
}

#[test]
fn nashville_and_roman_end_to_end() {
    let c = chart("120bpm 4/4 #G\n\nVS 4\nG C D Em\n");

    let nash = apply_view(
        &c,
        &ChartView {
            notation: NotationSystem::Nashville,
            ..Default::default()
        },
    );
    assert_eq!(symbols(&nash), vec!["1", "4", "5", "6m"]);

    let roman = apply_view(
        &c,
        &ChartView {
            notation: NotationSystem::Roman,
            ..Default::default()
        },
    );
    assert_eq!(symbols(&roman), vec!["I", "IV", "V", "vi"]);
}

#[test]
fn capo_renders_shape_key_end_to_end() {
    // Chart in B; play it sounding in B with a capo 4 → finger G shapes.
    let c = chart("120bpm 4/4 #B\n\nVS 4\nB E F# B\n");
    let view = ChartView {
        target_key: Some(key("B")),
        notation: NotationSystem::Letters,
        capo: 4,
    };
    let out = apply_view(&c, &view);
    assert_eq!(symbols(&out), vec!["G", "C", "D", "G"]);
    assert_eq!(view.capo_caption().as_deref(), Some("Capo 4 (sounds B)"));
}
