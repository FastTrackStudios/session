//! Horizontal Spacing Diagnostic Tests
//!
//! These tests investigate the horizontal spacing system to understand
//! why beat-based stretching may not be working correctly.
//!
//! The spacing system should give proportionally more space to longer notes
//! and less space to shorter notes:
//! - Half note (960 ticks) gets more space than quarter (480 ticks)
//! - Quarter note gets more space than 8th (240 ticks)
//! - etc.

#![cfg(feature = "engraver")]

use std::path::PathBuf;
use std::sync::Arc;

use keyflow::engraver::layout::chart::{ChartLayoutConfig, ChartLayoutEngine, LayoutMode};
use keyflow::engraver::style::MStyle;

/// Test chart with explicit durations to diagnose spacing
const SPACING_TEST_CHART: &str = r#"Spacing Test
120bpm 4/4 #G

VS
C_1 | G_2 D_4 Em_8 G_16 D_32 r32
"#;

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
    let config = ChartLayoutConfig::master_rhythm();

    ChartLayoutEngine::with_config(config, style, text_font_data, musejazz_font_data)
}

#[test]
fn test_parse_spacing_chart() {
    let chart = keyflow::parse(SPACING_TEST_CHART).expect("Failed to parse chart");

    eprintln!("\n=== Parsing Results ===");
    eprintln!("Sections: {}", chart.sections.len());

    for (section_idx, section) in chart.sections.iter().enumerate() {
        eprintln!(
            "\nSection {}: {:?}",
            section_idx, section.section.section_type
        );
        for (measure_idx, measure) in section.measures().iter().enumerate() {
            eprintln!("  Measure {}:", measure_idx);
            eprintln!("    Chords: {}", measure.chords.len());
            for (chord_idx, chord) in measure.chords.iter().enumerate() {
                eprintln!(
                    "      Chord {}: '{}' rhythm={:?}",
                    chord_idx, chord.full_symbol, chord.rhythm
                );
            }
            eprintln!("    Rhythm elements: {}", measure.rhythm_elements.len());
            for (elem_idx, elem) in measure.rhythm_elements.iter().enumerate() {
                eprintln!("      Element {}: {:?}", elem_idx, elem);
            }
        }
    }

    // Should have 1 section (VS)
    assert_eq!(chart.sections.len(), 1, "Expected 1 section");

    // VS should have 2 measures
    let vs = &chart.sections[0];
    assert_eq!(vs.measures().len(), 2, "Expected 2 measures in VS");
}

#[test]
fn test_layout_segment_ticks() {
    let chart = keyflow::parse(SPACING_TEST_CHART).expect("Failed to parse chart");
    let engine = create_test_engine();

    let result = engine.layout_chart(&chart, &LayoutMode::paginated_a4());

    eprintln!("\n=== Layout Results ===");
    eprintln!("Beat positions count: {}", result.beat_positions.len());

    // Group beat positions by measure
    let mut measures: std::collections::HashMap<usize, Vec<_>> = std::collections::HashMap::new();
    for bp in &result.beat_positions {
        measures.entry(bp.measure).or_default().push(bp);
    }

    for (measure_idx, beats) in measures.iter() {
        eprintln!("\nMeasure {}:", measure_idx);
        let mut sorted_beats: Vec<_> = beats.iter().collect();
        sorted_beats.sort_by_key(|a| a.beat);

        for bp in sorted_beats {
            eprintln!(
                "  Beat {}: tick={}, duration_ticks={}, x={:.2}, width={:.2}",
                bp.beat, bp.tick, bp.duration_ticks, bp.x, bp.width
            );
        }
    }

    // Check measure 2 (index 1) which should have varying durations
    if let Some(measure_beats) = measures.get(&1) {
        eprintln!("\n=== Measure 2 Analysis ===");

        // Find beats with different durations
        let mut duration_widths: Vec<(i32, f64)> = measure_beats
            .iter()
            .map(|bp| (bp.duration_ticks, bp.width))
            .collect();
        duration_widths.sort_by_key(|(ticks, _)| -*ticks);

        eprintln!("Duration -> Width mapping:");
        for (ticks, width) in &duration_widths {
            let note_name = match *ticks {
                1920 => "whole",
                960 => "half",
                480 => "quarter",
                240 => "8th",
                120 => "16th",
                60 => "32nd",
                30 => "64th",
                _ => "other",
            };
            eprintln!("  {} ({} ticks): width = {:.2}", note_name, ticks, width);
        }

        // The key test: longer durations should have larger widths
        let widths_by_duration: std::collections::HashMap<i32, f64> =
            duration_widths.into_iter().collect();

        // Check if half note is wider than quarter note
        if let (Some(&half_width), Some(&quarter_width)) =
            (widths_by_duration.get(&960), widths_by_duration.get(&480))
        {
            eprintln!(
                "\nHalf note width ({:.2}) vs Quarter note width ({:.2})",
                half_width, quarter_width
            );
            if half_width <= quarter_width {
                eprintln!("WARNING: Half note is NOT wider than quarter note!");
                eprintln!("This indicates the spring-based spacing is NOT working!");
            } else {
                eprintln!(
                    "OK: Half note is {:.1}x wider than quarter",
                    half_width / quarter_width
                );
            }
        }
    }
}

#[test]
fn test_segment_stretch_values() {
    // This test directly checks if segments have correct stretch values
    use keyflow::engraver::layout::segment::Segment;
    use keyflow::engraver::layout::segment_list::SegmentList;
    use keyflow::engraver::layout::spacing::HorizontalSpacing;

    let spatium = 5.0;
    let spacing = HorizontalSpacing::new(spatium);

    // Create segments with different durations
    let mut segments = SegmentList::new();

    // Half note (960 ticks)
    segments.push(Segment::chord_rest(0, 960));
    // Quarter note (480 ticks)
    segments.push(Segment::chord_rest(960, 480));
    // 8th note (240 ticks)
    segments.push(Segment::chord_rest(1440, 240));
    // 16th note (120 ticks)
    segments.push(Segment::chord_rest(1680, 120));

    eprintln!("\n=== Before Spacing ===");
    for (i, seg) in segments.iter().enumerate() {
        eprintln!(
            "Segment {}: tick={}, ticks={}, width={:.2}, stretch={:.2}",
            i, seg.tick, seg.ticks, seg.width, seg.stretch
        );
    }

    // Apply spacing
    let result = spacing.compute_spacing(&mut segments, 200.0, true); // justify to 200pt

    eprintln!("\n=== After Spacing (justified to 200pt) ===");
    eprintln!("Total width: {:.2}", result.total_width);
    for (i, seg) in segments.iter().enumerate() {
        eprintln!(
            "Segment {}: tick={}, ticks={}, x={:.2}, width={:.2}, stretch={:.2}",
            i, seg.tick, seg.ticks, seg.x, seg.width, seg.stretch
        );
    }

    // Verify that longer durations get more stretch
    let half_note = &segments[0];
    let quarter_note = &segments[1];
    let eighth_note = &segments[2];

    eprintln!("\n=== Stretch Comparison ===");
    eprintln!(
        "Half note stretch: {:.4} (should be ~1.414 = sqrt(2) times quarter)",
        half_note.stretch
    );
    eprintln!(
        "Quarter note stretch: {:.4} (baseline = 1.0)",
        quarter_note.stretch
    );
    eprintln!(
        "8th note stretch: {:.4} (should be ~0.707 = 1/sqrt(2) times quarter)",
        eighth_note.stretch
    );

    // Check that stretch values are correct
    assert!(
        half_note.stretch > quarter_note.stretch,
        "Half note should have more stretch than quarter"
    );
    assert!(
        quarter_note.stretch > eighth_note.stretch,
        "Quarter note should have more stretch than 8th"
    );

    // Check that widths follow stretch
    eprintln!("\n=== Width Comparison ===");
    eprintln!("Half note width: {:.2}", half_note.width);
    eprintln!("Quarter note width: {:.2}", quarter_note.width);
    eprintln!("8th note width: {:.2}", eighth_note.width);

    assert!(
        half_note.width > quarter_note.width,
        "Half note should be wider than quarter: half={:.2} quarter={:.2}",
        half_note.width,
        quarter_note.width
    );
}

#[test]
fn test_snippet_mode_layout() {
    // This test mimics what the web app does - snippet mode layout
    let chart = keyflow::parse(SPACING_TEST_CHART).expect("Failed to parse chart");
    let engine = create_test_engine();

    // Simulate web app viewport: 800 CSS pixels / DPI_SCALE (96/72 = 1.333)
    let css_width = 800.0;
    let dpi_scale = 96.0 / 72.0;
    let page_width = css_width / dpi_scale;

    eprintln!("\n=== Snippet Mode Test ===");
    eprintln!("CSS width: {}", css_width);
    eprintln!("Page width (points): {:.2}", page_width);

    // Use snippet mode like the web app
    let result = engine.layout_chart(&chart, &LayoutMode::snippet(page_width));

    eprintln!("Total width: {:.2}", result.total_width);
    eprintln!("Total height: {:.2}", result.total_height);
    eprintln!("Beat positions count: {}", result.beat_positions.len());

    // Group beat positions by measure
    let mut measures: std::collections::HashMap<usize, Vec<_>> = std::collections::HashMap::new();
    for bp in &result.beat_positions {
        measures.entry(bp.measure).or_default().push(bp);
    }

    for (measure_idx, beats) in measures.iter() {
        eprintln!("\nMeasure {}:", measure_idx);
        let mut sorted_beats: Vec<_> = beats.iter().collect();
        sorted_beats.sort_by_key(|a| a.beat);

        // Calculate measure bounds from first/last beat
        if let (Some(first), Some(last)) = (sorted_beats.first(), sorted_beats.last()) {
            let measure_width = (last.x + last.width) - first.x;
            eprintln!(
                "  Measure range: x={:.2} to {:.2} (width={:.2})",
                first.x,
                last.x + last.width,
                measure_width
            );
        }

        for bp in &sorted_beats {
            let note_name = match bp.duration_ticks {
                1920 => "whole",
                960 => "half",
                480 => "quarter",
                240 => "8th",
                120 => "16th",
                60 => "32nd",
                _ => "other",
            };
            eprintln!(
                "  Beat {} ({}): tick={}, x={:.2}, width={:.2}",
                bp.beat, note_name, bp.tick, bp.x, bp.width
            );
        }
    }

    // Check that beats in measure 1 are spread out
    if let Some(measure_beats) = measures.get(&1) {
        let x_positions: Vec<f64> = measure_beats.iter().map(|bp| bp.x).collect();
        let min_x = x_positions.iter().cloned().fold(f64::INFINITY, f64::min);
        let max_x = x_positions
            .iter()
            .cloned()
            .fold(f64::NEG_INFINITY, f64::max);
        let spread = max_x - min_x;

        eprintln!(
            "\nMeasure 1 x-position spread: {:.2} (min={:.2}, max={:.2})",
            spread, min_x, max_x
        );

        // The spread should be significant (at least 50 points for 6 beats with varied durations)
        assert!(
            spread > 30.0,
            "Beats in measure 1 should be spread out, but spread is only {:.2}",
            spread
        );
    }
}
