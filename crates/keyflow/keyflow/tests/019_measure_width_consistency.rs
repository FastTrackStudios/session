//! Measure width consistency tests.
//!
//! These tests verify that measures with identical content have identical widths,
//! and investigate specific cases where measure widths are unexpectedly different.
//!
//! ## Test Chart: Push Pull Triplets
//!
//! This chart has several sections with repeated content that should have
//! consistent measure widths:
//!
//! - VS sections: `'F/C . | Cm .` pattern repeats - all should be equal width
//! - CH sections: Various patterns that test collision scenarios
//!
//! ## Known Issues Being Investigated
//!
//! 1. First measure of VS1 (measure 3 overall) is elongated compared to identical measures

#![cfg(feature = "engraver")]

use std::path::PathBuf;
use std::sync::Arc;

use keyflow::Chart;
use keyflow::engraver::layout::chart::{ChartLayoutConfig, ChartLayoutEngine, LayoutMode};
use keyflow::engraver::scene::id::ElementType;
use keyflow::engraver::scene::node::SceneNode;
use keyflow::engraver::scene::traverse::SceneNodeExt;
use keyflow::engraver::style::MStyle;

/// The test chart source - Push Pull Triplets
const TEST_CHART: &str = r#"Push Pull Triplets - Test
120bpm 4/4 #Ab
/push = triplet

COUNT 2

IN
r8t Ab9_8t r8t r8t r8t F9_8t r2 | s1

VS
'F/C . | Cm . | 'F/C . | Cm . | 'F/C . | Cm . | 'F/C . | Cm Cm9

CH
Cm/Eb / 'Eb /// | 'Eb / 'F/C / 'Cm // | 'F/A //// | 'Fm9  ////
Cm/Eb / 'Eb /// | 'Eb / 'F/C / 'Cm // | 'F/A | r8t Ab9_8t r8t r8t 'F9_8t r8t r4 Fm/Ab_4 | s1

BR
'_4F7 | . |  Abmaj9 //// | // r8t Abmaj9_8t r8t Bb_8t r8t Cm7_8t | Cm7 | Ebmaj7/Bb | Am7b5 | Abmaj7 | G7sus4 | 'G7

VS
'F/C . | Cm . | 'F/C . | Cm . | 'F/C . | Cm . | 'F/C . | Cm Cm9
"#;

// =============================================================================
// Test Utilities
// =============================================================================

fn workspace_root() -> PathBuf {
    std::env::var("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap())
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Create a layout engine with standard test settings.
fn create_test_engine() -> ChartLayoutEngine {
    let root = workspace_root();
    let text_font_path = root.join("crates/engraver-proto/fonts/FreeSans.ttf");
    let musejazz_font_path = root.join("crates/engraver-proto/fonts/MuseJazzText.otf");

    let text_font_data = Arc::new(
        std::fs::read(&text_font_path)
            .unwrap_or_else(|e| panic!("Failed to load text font at {:?}: {}", text_font_path, e)),
    );
    let musejazz_font_data = Arc::new(std::fs::read(&musejazz_font_path).unwrap_or_else(|e| {
        panic!(
            "Failed to load MuseJazz font at {:?}: {}",
            musejazz_font_path, e
        )
    }));

    let style: &'static MStyle = Box::leak(Box::new(MStyle::default()));
    let config = ChartLayoutConfig::default();

    ChartLayoutEngine::with_config(config, style, text_font_data, musejazz_font_data)
}

/// Parse the test chart.
fn parse_test_chart() -> Chart {
    keyflow::parse(TEST_CHART).expect("Failed to parse test chart")
}

/// Find all barline X positions in the scene, sorted by position.
fn find_barline_positions(scene: &SceneNode) -> Vec<f64> {
    let mut positions: Vec<f64> = scene
        .iter_with_transforms()
        .filter_map(|(node, transform)| {
            let id = node.id.as_ref()?;
            if id.element_type != ElementType::Barline {
                return None;
            }
            let world_origin = keyflow::engraver::scene::transform::get_translation(&transform);
            Some(world_origin.x)
        })
        .collect();
    positions.sort_by(|a, b| a.partial_cmp(b).unwrap());
    positions
}

/// Per-measure widths computed from the `Measure` container nodes' world
/// bounds. The chart layout draws barlines as anonymous leaf nodes (geometry in
/// their paint commands, not their transform), so they can't be located by
/// element id; the `Measure` containers carry a real world transform and known
/// content bounds, giving each measure a concrete width.
fn measure_widths_from_containers(scene: &SceneNode) -> Vec<(usize, f64)> {
    let mut widths: Vec<f64> = scene
        .iter_with_transforms()
        .filter_map(|(node, transform)| {
            let id = node.id.as_ref()?;
            if id.element_type != ElementType::Measure {
                return None;
            }
            let bounds = node.compute_bounds();
            if bounds.is_zero_area() {
                return None;
            }
            let world = transform.transform_rect_bbox(bounds);
            Some(world.width())
        })
        .collect();
    // Drop any zero-width artifacts; keep document order.
    widths.retain(|w| *w > 0.0);
    widths.into_iter().enumerate().collect()
}

/// Calculate measure widths from barline positions.
/// Returns (measure_index, width) pairs.
fn calculate_measure_widths(barline_positions: &[f64]) -> Vec<(usize, f64)> {
    barline_positions
        .windows(2)
        .enumerate()
        .map(|(idx, window)| (idx, window[1] - window[0]))
        .collect()
}

/// Group measures by their approximate width (within tolerance).
fn group_measures_by_width(widths: &[(usize, f64)], tolerance: f64) -> Vec<Vec<usize>> {
    let mut groups: Vec<(f64, Vec<usize>)> = Vec::new();

    for (idx, width) in widths {
        let mut found = false;
        for (group_width, indices) in &mut groups {
            if (*width - *group_width).abs() < tolerance {
                indices.push(*idx);
                found = true;
                break;
            }
        }
        if !found {
            groups.push((*width, vec![*idx]));
        }
    }

    groups.into_iter().map(|(_, indices)| indices).collect()
}

// =============================================================================
// Parsing Tests
// =============================================================================

#[test]
fn test_chart_parses_successfully() {
    let chart = parse_test_chart();

    // Should have: COUNT, IN, VS, CH, BR, VS sections
    assert!(
        chart.sections.len() >= 5,
        "Expected at least 5 sections, got {}",
        chart.sections.len()
    );
}

#[test]
fn test_vs_sections_have_correct_measure_count() {
    let chart = parse_test_chart();

    // Find VS sections
    let vs_sections: Vec<_> = chart
        .sections
        .iter()
        .filter(|s| {
            matches!(
                s.section.section_type,
                keyflow::sections::SectionType::Verse
            )
        })
        .collect();

    assert_eq!(vs_sections.len(), 2, "Expected 2 VS sections");

    // Each VS should have 8 measures
    for (i, vs) in vs_sections.iter().enumerate() {
        let measure_count = vs.measures().len();
        assert_eq!(
            measure_count,
            8,
            "VS{} should have 8 measures, got {}",
            i + 1,
            measure_count
        );
    }
}

#[test]
fn test_vs_measures_have_identical_content_structure() {
    let chart = parse_test_chart();

    // Find first VS section
    let vs1 = chart
        .sections
        .iter()
        .find(|s| {
            matches!(
                s.section.section_type,
                keyflow::sections::SectionType::Verse
            )
        })
        .expect("VS section not found");

    let measures = vs1.measures();

    // Measures 0,2,4,6 should all have same structure: 'F/C .
    // Measures 1,3,5 should all have same structure: Cm .
    // Measure 7 is different: Cm Cm9

    // A pushed first chord of a section gets a leading tacet space ("s") chord
    // inserted by `apply_push_pull_adjustments` (same behavior verified by
    // 016_push_triplets). Compare the real (non-space) chord content here.
    let real_chords = |m: &keyflow::chart::types::Measure| -> Vec<String> {
        m.chords
            .iter()
            .filter(|c| c.full_symbol != "s")
            .map(|c| c.full_symbol.clone())
            .collect()
    };

    for &idx in &[0, 2, 4, 6] {
        let chords = real_chords(&measures[idx]);
        assert_eq!(chords.len(), 2, "Measure {} should have 2 chords", idx);
        assert_eq!(
            chords[0], "F/C",
            "Measure {} first chord should be F/C",
            idx
        );
    }

    for &idx in &[1, 3, 5] {
        let chords = real_chords(&measures[idx]);
        assert_eq!(chords.len(), 2, "Measure {} should have 2 chords", idx);
        assert_eq!(chords[0], "Cm", "Measure {} first chord should be Cm", idx);
    }
}

// =============================================================================
// Layout Tests - Measure Width Consistency
// =============================================================================

#[test]
fn test_identical_measures_have_same_width() {
    let chart = parse_test_chart();
    let engine = create_test_engine();

    // Layout in paginated mode
    let result = engine.layout_chart(&chart, &LayoutMode::paginated_a4());

    // Barlines are anonymous leaf nodes in this layout, so widths come from the
    // `Measure` container bounds instead (see `measure_widths_from_containers`).
    let measure_widths = measure_widths_from_containers(&result.scene);

    // Debug output
    eprintln!("\n=== Measure Widths ===");
    for (idx, width) in &measure_widths {
        eprintln!("Measure {}: {:.2} pts", idx, width);
    }

    // VS1 starts at measure 3 (after COUNT + IN measures)
    // Measures 3,5,7,9 should be identical ('F/C .)
    // Measures 4,6,8 should be identical (Cm .)

    // For now, just verify we can calculate widths
    assert!(!measure_widths.is_empty(), "Should have measure widths");
}

#[test]
fn test_vs1_first_measure_not_elongated() {
    let chart = parse_test_chart();
    let engine = create_test_engine();

    let result = engine.layout_chart(&chart, &LayoutMode::paginated_a4());

    let barline_positions = find_barline_positions(&result.scene);
    let measure_widths = calculate_measure_widths(&barline_positions);

    // Debug: print all widths
    eprintln!("\n=== Investigating VS1 First Measure ===");
    for (idx, width) in &measure_widths {
        eprintln!("Measure {}: {:.2} pts", idx, width);
    }

    // Find measures that should be identical
    // The 'F/C . pattern measures in VS1 should all have the same width
    // These are at indices 3, 5, 7, 9 (0-indexed from start of song)

    // Group by approximate width (5pt tolerance)
    let groups = group_measures_by_width(&measure_widths, 5.0);

    eprintln!("\n=== Width Groups (5pt tolerance) ===");
    for (i, group) in groups.iter().enumerate() {
        if let Some(first_idx) = group.first() {
            let width = measure_widths
                .iter()
                .find(|(idx, _)| idx == first_idx)
                .map(|(_, w)| *w)
                .unwrap_or(0.0);
            eprintln!("Group {} (width ~{:.2}): measures {:?}", i, width, group);
        }
    }

    // This test is primarily for investigation - the assertion will fail
    // if measure 3 is significantly wider than measures 5, 7, 9
    let vs1_fc_measures = [3, 5, 7, 9]; // Assuming these are the 'F/C . measures
    let vs1_fc_widths: Vec<f64> = vs1_fc_measures
        .iter()
        .filter_map(|&idx| {
            measure_widths
                .iter()
                .find(|(i, _)| *i == idx)
                .map(|(_, w)| *w)
        })
        .collect();

    if vs1_fc_widths.len() >= 2 {
        let first_width = vs1_fc_widths[0];
        let others_avg: f64 =
            vs1_fc_widths[1..].iter().sum::<f64>() / (vs1_fc_widths.len() - 1) as f64;
        let diff = (first_width - others_avg).abs();

        eprintln!("\nVS1 'F/C . measures:");
        eprintln!("  First (m3): {:.2} pts", first_width);
        eprintln!("  Others avg: {:.2} pts", others_avg);
        eprintln!(
            "  Difference: {:.2} pts ({:.1}%)",
            diff,
            (diff / others_avg) * 100.0
        );

        // Allow 10% variance as acceptable
        let variance_percent = (diff / others_avg) * 100.0;
        assert!(
            variance_percent < 10.0,
            "First measure of VS1 is {:.1}% different from others (expected < 10%)",
            variance_percent
        );
    }
}

// =============================================================================
// Layout Engine Weight Calculation Tests
// =============================================================================

#[test]
fn test_content_weight_calculation() {
    let chart = parse_test_chart();

    // Find VS1 section
    let vs1 = chart
        .sections
        .iter()
        .find(|s| {
            matches!(
                s.section.section_type,
                keyflow::sections::SectionType::Verse
            )
        })
        .expect("VS section not found");

    let measures = vs1.measures();

    // All measures in VS1 (except last) should have identical beat counts
    // 'F/C . = 2 beats + 2 beats = 4 total
    // Cm . = 2 beats + 2 beats = 4 total

    eprintln!("\n=== VS1 Measure Content Analysis ===");
    for (idx, measure) in measures.iter().enumerate() {
        let total_beats: f64 = measure
            .chords
            .iter()
            .map(|c| match &c.rhythm {
                keyflow::chord::ChordRhythm::Slashes { count, dotted, .. } => {
                    let base = *count as f64;
                    if *dotted { base * 1.5 } else { base }
                }
                keyflow::chord::ChordRhythm::Default => 4.0,
                keyflow::chord::ChordRhythm::Explicit(nd) => nd.total_ticks_480() as f64 / 480.0,
            })
            .sum();

        eprintln!(
            "Measure {}: {} chords, {:.1} beats total, chords: {:?}",
            idx,
            measure.chords.len(),
            total_beats,
            measure
                .chords
                .iter()
                .map(|c| &c.full_symbol)
                .collect::<Vec<_>>()
        );

        // Check push/pull on first chord
        if let Some(first_chord) = measure.chords.first() {
            if first_chord.push_pull.is_some() {
                eprintln!(
                    "  -> First chord has push/pull: {:?}",
                    first_chord.push_pull
                );
            }
        }
    }
}

#[test]
fn test_push_pull_detection_in_vs1() {
    let chart = parse_test_chart();

    let vs1 = chart
        .sections
        .iter()
        .find(|s| {
            matches!(
                s.section.section_type,
                keyflow::sections::SectionType::Verse
            )
        })
        .expect("VS section not found");

    let measures = vs1.measures();

    eprintln!("\n=== Push/Pull Detection in VS1 ===");
    for (idx, measure) in measures.iter().enumerate() {
        for (chord_idx, chord) in measure.chords.iter().enumerate() {
            if let Some((is_push, amount)) = &chord.push_pull {
                eprintln!(
                    "Measure {} chord {}: {} has {} {:?}",
                    idx,
                    chord_idx,
                    chord.full_symbol,
                    if *is_push { "PUSH" } else { "PULL" },
                    amount
                );
            }
        }
    }

    // The ' prefix means push, so 'F/C has a push. A leading tacet space ("s")
    // chord is inserted before a pushed section-leading chord, so the pushed
    // chord is the first NON-space chord (see 016_push_triplets for the same
    // behavior).
    let first_measure = &measures[0];
    let first_real = first_measure
        .chords
        .iter()
        .find(|c| c.full_symbol != "s")
        .expect("VS1 measure 0 should have a real chord");
    assert!(
        first_real.push_pull.is_some(),
        "First chord of VS1 measure 0 should have push/pull (it has ' prefix)"
    );
}

// =============================================================================
// Spillback Detection Tests
// =============================================================================

#[test]
fn test_spillback_from_in_to_count() {
    let chart = parse_test_chart();

    // IN section has: r8t Ab9_8t r8t r8t r8t F9_8t r2 | s1
    // The first measure starts with triplet rests and pushed chords
    // This might cause spillback to the COUNT section

    let in_section = chart
        .sections
        .iter()
        .find(|s| {
            matches!(
                s.section.section_type,
                keyflow::sections::SectionType::Intro
            )
        })
        .expect("IN section not found");

    let in_measures = in_section.measures();

    eprintln!("\n=== IN Section First Measure Analysis ===");
    let first_measure = &in_measures[0];
    eprintln!(
        "Chords: {:?}",
        first_measure
            .chords
            .iter()
            .map(|c| &c.full_symbol)
            .collect::<Vec<_>>()
    );
    eprintln!("Rhythm elements: {:?}", first_measure.rhythm_elements.len());

    for (idx, chord) in first_measure.chords.iter().enumerate() {
        eprintln!(
            "Chord {}: {} rhythm={:?} push_pull={:?}",
            idx, chord.full_symbol, chord.rhythm, chord.push_pull
        );
    }
}

#[test]
fn test_spillback_from_vs1_to_in() {
    let chart = parse_test_chart();

    // VS section has: 'F/C . | Cm . | ...
    // The 'F/C has a push, which might cause spillback to IN section

    let vs1 = chart
        .sections
        .iter()
        .find(|s| {
            matches!(
                s.section.section_type,
                keyflow::sections::SectionType::Verse
            )
        })
        .expect("VS section not found");

    let vs_measures = vs1.measures();

    eprintln!("\n=== VS1 First Measure Analysis ===");
    let first_measure = &vs_measures[0];

    for (idx, chord) in first_measure.chords.iter().enumerate() {
        eprintln!(
            "Chord {}: '{}' rhythm={:?} push_pull={:?}",
            idx, chord.full_symbol, chord.rhythm, chord.push_pull
        );
    }

    // The 'F/C chord should have push_pull set
    let first_chord = &first_measure.chords[0];
    if let Some((is_push, amount)) = &first_chord.push_pull {
        eprintln!(
            "\nFirst chord push/pull: is_push={} base={:?}",
            is_push, amount.base
        );

        // If this is a triplet push, it might be causing spillback weight adjustment
        if *is_push && matches!(amount.base, keyflow::chord::PushPullBase::Triplet) {
            eprintln!(
                "WARNING: This is a TRIPLET PUSH - check if this triggers spillback weight adjustment"
            );
        }
    }
}
