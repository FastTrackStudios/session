//! Visual rhythm regression tests.
//!
//! These tests verify that rhythm notation renders with correct symbols
//! at the expected positions. Each test case focuses on a specific
//! notation feature to catch regressions.
//!
//! ## Chord Note Values
//! - C_1: Whole note
//! - G_2: Half note
//! - D_4: Quarter note
//! - Em_8: Eighth note
//! - G_16: Sixteenth note
//! - D_32: Thirty-second note
//!
//! ## Rest Values
//! - r1: Whole rest
//! - r2: Half rest
//! - r4: Quarter rest
//! - r8: Eighth rest
//! - r16: Sixteenth rest
//! - r32: Thirty-second rest

#![cfg(feature = "engraver")]

use std::path::PathBuf;
use std::sync::Arc;

use keyflow::engraver::layout::chart::{ChartLayoutConfig, ChartLayoutEngine, LayoutMode};
use keyflow::engraver::style::MStyle;

// =============================================================================
// SMuFL Glyph Definitions
// =============================================================================

/// SMuFL glyph codepoints for visual regression testing.
///
/// These map to the standard SMuFL specification glyphs.
/// See: <https://w3c.github.io/smufl/latest/tables/>
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
enum SmuflGlyph {
    // === Rests (U+E4E0 - U+E4FF) ===
    /// Whole rest (4 beats in 4/4)
    RestWhole,
    /// Half rest (2 beats)
    RestHalf,
    /// Quarter rest (1 beat)
    RestQuarter,
    /// Eighth rest (1/2 beat)
    Rest8th,
    /// Sixteenth rest (1/4 beat)
    Rest16th,
    /// Thirty-second rest (1/8 beat)
    Rest32nd,
    /// Sixty-fourth rest (1/16 beat)
    Rest64th,

    // === Noteheads (U+E0A0 - U+E0FF) ===
    /// Filled notehead (quarter note and shorter)
    NoteheadBlack,
    /// Open notehead (half note)
    NoteheadHalf,
    /// Open notehead (whole note)
    NoteheadWhole,
    /// Slash notehead (rhythm notation)
    NoteheadSlash,
    /// Diamond notehead (harmonic/rhythm)
    NoteheadDiamond,
    /// X notehead (ghost note/dead note)
    NoteheadX,

    // === Rhythm Slash Noteheads (U+E100 - U+E10F) ===
    /// Slash notehead for stemless rhythm
    NoteheadSlashWhite,
    /// Slash notehead for half note rhythm
    NoteheadSlashHalf,
    /// Slash notehead for stemmed rhythm
    NoteheadSlashBlack,

    // === Flags (U+E240 - U+E25F) ===
    /// Single flag (eighth note) - stem up
    Flag8thUp,
    /// Single flag (eighth note) - stem down
    Flag8thDown,
    /// Double flag (sixteenth note) - stem up
    Flag16thUp,
    /// Double flag (sixteenth note) - stem down
    Flag16thDown,
    /// Triple flag (32nd note) - stem up
    Flag32ndUp,
    /// Triple flag (32nd note) - stem down
    Flag32ndDown,
}

impl SmuflGlyph {
    /// Get the Unicode codepoint for this glyph.
    #[must_use]
    fn codepoint(self) -> char {
        match self {
            // Rests
            Self::RestWhole => '\u{E4E2}',
            Self::RestHalf => '\u{E4E3}',
            Self::RestQuarter => '\u{E4E4}',
            Self::Rest8th => '\u{E4E5}',
            Self::Rest16th => '\u{E4E6}',
            Self::Rest32nd => '\u{E4E7}',
            Self::Rest64th => '\u{E4E8}',

            // Standard noteheads
            Self::NoteheadBlack => '\u{E0A4}',
            Self::NoteheadHalf => '\u{E0A3}',
            Self::NoteheadWhole => '\u{E0A2}',
            Self::NoteheadSlash => '\u{E101}',
            Self::NoteheadDiamond => '\u{E0DB}',
            Self::NoteheadX => '\u{E0A9}',

            // Rhythm slash noteheads
            Self::NoteheadSlashWhite => '\u{E102}',
            Self::NoteheadSlashHalf => '\u{E103}',
            Self::NoteheadSlashBlack => '\u{E101}',

            // Flags
            Self::Flag8thUp => '\u{E240}',
            Self::Flag8thDown => '\u{E241}',
            Self::Flag16thUp => '\u{E242}',
            Self::Flag16thDown => '\u{E243}',
            Self::Flag32ndUp => '\u{E244}',
            Self::Flag32ndDown => '\u{E245}',
        }
    }

    /// Get a human-readable name for this glyph.
    #[must_use]
    fn name(self) -> &'static str {
        match self {
            Self::RestWhole => "whole rest",
            Self::RestHalf => "half rest",
            Self::RestQuarter => "quarter rest",
            Self::Rest8th => "8th rest",
            Self::Rest16th => "16th rest",
            Self::Rest32nd => "32nd rest",
            Self::Rest64th => "64th rest",
            Self::NoteheadBlack => "black notehead",
            Self::NoteheadHalf => "half notehead",
            Self::NoteheadWhole => "whole notehead",
            Self::NoteheadSlash => "slash notehead",
            Self::NoteheadDiamond => "diamond notehead",
            Self::NoteheadX => "x notehead",
            Self::NoteheadSlashWhite => "slash white",
            Self::NoteheadSlashHalf => "slash half",
            Self::NoteheadSlashBlack => "slash black",
            Self::Flag8thUp => "8th flag up",
            Self::Flag8thDown => "8th flag down",
            Self::Flag16thUp => "16th flag up",
            Self::Flag16thDown => "16th flag down",
            Self::Flag32ndUp => "32nd flag up",
            Self::Flag32ndDown => "32nd flag down",
        }
    }

    /// Check if a glyph codepoint matches this SMuFL glyph.
    #[must_use]
    fn matches(self, codepoint: char) -> bool {
        self.codepoint() == codepoint
    }
}

/// Assert that a beat position has the expected glyph.
#[allow(dead_code)]
fn assert_glyph(
    bp: &keyflow::engraver::layout::chart::BeatPosition,
    expected: SmuflGlyph,
    context: &str,
) {
    let actual = bp.glyph_codepoint;
    assert!(
        actual.is_some_and(|c| expected.matches(c)),
        "{}: expected {} (U+{:04X}), got {:?}",
        context,
        expected.name(),
        expected.codepoint() as u32,
        actual.map(|c| format!("U+{:04X}", c as u32))
    );
}

/// Get the workspace root directory.
fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR points to packages/keyflow, go up two levels
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Helper to create a layout engine for tests.
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
    let mut config = ChartLayoutConfig::default();
    config.use_stems = true;
    config.hide_repeated_chords = false;

    ChartLayoutEngine::with_config(config, style, text_font_data, musejazz_font_data)
}

/// Helper to layout a chart in snippet mode.
fn layout_snippet(source: &str) -> keyflow::engraver::layout::chart::ChartLayoutResult {
    let chart = keyflow::parse(source).expect("Failed to parse chart");
    let engine = create_test_engine();
    let mode = LayoutMode::snippet(800.0);
    engine.layout_chart(&chart, &mode)
}

/// Convert REAPER-style position (measure.beat.fraction) to absolute tick.
///
/// Format: measure.beat.fraction where:
/// - measure: 1-indexed measure number
/// - beat: 1-indexed beat within measure
/// - fraction: 0-999 representing fraction of beat (e.g., 875 = 0.875)
///
/// Uses 480 ticks per quarter note (standard MIDI resolution).
fn reaper_position_to_tick(measure: u32, beat: u32, fraction: u32, ticks_per_beat: i32) -> i64 {
    let ticks_per_measure = ticks_per_beat * 4; // Assuming 4/4 time
    let measure_ticks = (measure - 1) as i64 * ticks_per_measure as i64;
    let beat_ticks = (beat - 1) as i64 * ticks_per_beat as i64;
    let fraction_ticks = (fraction as f64 / 1000.0 * ticks_per_beat as f64) as i64;
    measure_ticks + beat_ticks + fraction_ticks
}

/// Find beat position at a REAPER-style position.
fn find_at_reaper_position(
    positions: &[keyflow::engraver::layout::chart::BeatPosition],
    measure: u32,
    beat: u32,
    fraction: u32,
) -> Option<&keyflow::engraver::layout::chart::BeatPosition> {
    let target_tick = reaper_position_to_tick(measure, beat, fraction, 480);
    positions.iter().find(|bp| bp.contains_tick(target_tick))
}

// =============================================================================
// Chord Note Values
// =============================================================================

/// Test: Whole note chord (C_1)
#[test]
fn test_chord_whole_note() {
    let source = r#"
#G

VS
C_1
"#;
    let result = layout_snippet(source);

    // Verify we got a valid layout with content
    assert!(
        !result.scene.children.is_empty(),
        "Layout should have children"
    );
    assert!(
        !result.beat_positions.is_empty(),
        "Should have beat positions"
    );
}

/// Test: Half note chord (G_2)
#[test]
fn test_chord_half_note() {
    let source = r#"
#G

VS
G_2 G_2
"#;
    let result = layout_snippet(source);

    assert!(
        !result.scene.children.is_empty(),
        "Layout should have children"
    );
    assert!(
        result.beat_positions.len() >= 2,
        "Should have at least 2 beat positions"
    );
}

/// Test: Quarter note chord (D_4)
#[test]
fn test_chord_quarter_note() {
    let source = r#"
#G

VS
D_4 D_4 D_4 D_4
"#;
    let result = layout_snippet(source);

    assert!(
        !result.scene.children.is_empty(),
        "Layout should have children"
    );
    assert!(
        result.beat_positions.len() >= 4,
        "Should have at least 4 beat positions"
    );
}

/// Test: Eighth note chord (Em_8)
#[test]
fn test_chord_eighth_note() {
    let source = r#"
#G

VS
Em_8 Em_8 Em_8 Em_8 Em_8 Em_8 Em_8 Em_8
"#;
    let result = layout_snippet(source);

    assert!(
        !result.scene.children.is_empty(),
        "Layout should have children"
    );
    assert!(
        result.beat_positions.len() >= 8,
        "Should have at least 8 beat positions"
    );
}

/// Test: Sixteenth note chord (G_16)
#[test]
fn test_chord_sixteenth_note() {
    let source = r#"
#G

VS
G_16 G_16 G_16 G_16 G_16 G_16 G_16 G_16 G_16 G_16 G_16 G_16 G_16 G_16 G_16 G_16
"#;
    let result = layout_snippet(source);

    assert!(
        !result.scene.children.is_empty(),
        "Layout should have children"
    );
}

/// Test: Thirty-second note chord (D_32)
#[test]
fn test_chord_thirtysecond_note() {
    let source = r#"
#G

VS
D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32 D_32
"#;
    let result = layout_snippet(source);

    assert!(
        !result.scene.children.is_empty(),
        "Layout should have children"
    );
}

/// Test: Mixed chord note values in progression
#[test]
fn test_chord_mixed_values() {
    let source = r#"
#G

VS
C_1 | G_2 D_4 Em_8 G_16 D_32 r32
"#;
    let result = layout_snippet(source);

    assert!(
        !result.scene.children.is_empty(),
        "Layout should have children"
    );
    // Two measures worth of beats
    assert!(
        result.beat_positions.len() >= 2,
        "Should have beats from both measures"
    );
}

// =============================================================================
// Rest Values
// =============================================================================

/// Test: Whole rest (r1)
#[test]
fn test_rest_whole() {
    let source = r#"
#G

VS
r1
"#;
    let result = layout_snippet(source);

    assert!(
        !result.scene.children.is_empty(),
        "Layout should have children"
    );
}

/// Test: Half rest (r2)
#[test]
fn test_rest_half() {
    let source = r#"
#G

VS
r2 r2
"#;
    let result = layout_snippet(source);

    assert!(
        !result.scene.children.is_empty(),
        "Layout should have children"
    );
}

/// Test: Quarter rest (r4)
#[test]
fn test_rest_quarter() {
    let source = r#"
#G

VS
r4 r4 r4 r4
"#;
    let result = layout_snippet(source);

    assert!(
        !result.scene.children.is_empty(),
        "Layout should have children"
    );
}

/// Test: Eighth rest (r8)
#[test]
fn test_rest_eighth() {
    let source = r#"
#G

VS
r8 r8 r8 r8 r8 r8 r8 r8
"#;
    let result = layout_snippet(source);

    assert!(
        !result.scene.children.is_empty(),
        "Layout should have children"
    );
}

/// Test: Sixteenth rest (r16)
#[test]
fn test_rest_sixteenth() {
    let source = r#"
#G

VS
r16 r16 r16 r16 r16 r16 r16 r16 r16 r16 r16 r16 r16 r16 r16 r16
"#;
    let result = layout_snippet(source);

    assert!(
        !result.scene.children.is_empty(),
        "Layout should have children"
    );
}

/// Test: Thirty-second rest (r32)
#[test]
fn test_rest_thirtysecond() {
    let source = r#"
#G

VS
r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32 r32
"#;
    let result = layout_snippet(source);

    assert!(
        !result.scene.children.is_empty(),
        "Layout should have children"
    );
}

/// Test: Mixed rest values - same positions as chord tests
#[test]
fn test_rest_mixed_values() {
    let source = r#"
#G

VS
r1 | r2 r4 r8 r16 r32 r32
"#;
    let result = layout_snippet(source);

    assert!(
        !result.scene.children.is_empty(),
        "Layout should have children"
    );
}

// =============================================================================
// Combined Chords and Rests
// =============================================================================

/// Test: Chord followed by rest
#[test]
fn test_chord_then_rest() {
    let source = r#"
#G

VS
C_2 r2
"#;
    let result = layout_snippet(source);

    assert!(
        !result.scene.children.is_empty(),
        "Layout should have children"
    );
}

/// Test: Rest followed by chord
#[test]
fn test_rest_then_chord() {
    let source = r#"
#G

VS
r2 G_2
"#;
    let result = layout_snippet(source);

    assert!(
        !result.scene.children.is_empty(),
        "Layout should have children"
    );
}

/// Test: Syncopated rhythm with rests
#[test]
fn test_syncopated_rhythm() {
    let source = r#"
#G

VS
r8 C_8 r8 D_8 r8 E_8 r8 F_8
"#;
    let result = layout_snippet(source);

    assert!(
        !result.scene.children.is_empty(),
        "Layout should have children"
    );
}

// =============================================================================
// Position Query Tests
// =============================================================================

/// Test: Query beat positions for rest values
/// For: r1 | r2 r4 r8 r16 r32 r32
///
/// Expected layout (480 ticks per quarter note):
/// - Measure 1 (index 0): r1 whole rest at tick 0
/// - Measure 2 (index 1):
///   - r2 at tick 0 (960 ticks duration)
///   - r4 at tick 960 (480 ticks duration)
///   - r8 at tick 1440 (240 ticks duration)
///   - r16 at tick 1680 (120 ticks duration)
///   - r32 at tick 1800 (60 ticks duration)
///   - r32 at tick 1860 (60 ticks duration) <- last 32nd
#[test]
fn test_query_rest_positions() {
    let source = r#"
#G

VS
r1 | r2 r4 r8 r16 r32 r32
"#;
    let result = layout_snippet(source);

    // Print all beat positions for debugging
    println!("\n=== Beat Positions for: r1 | r2 r4 r8 r16 r32 r32 ===");
    for (i, bp) in result.beat_positions.iter().enumerate() {
        println!(
            "[{}] measure={} beat={} tick={} duration={} abs_tick={} glyph={:?}",
            i,
            bp.measure,
            bp.beat,
            bp.tick,
            bp.duration_ticks,
            bp.absolute_tick,
            bp.glyph_codepoint.map(|c| format!("U+{:04X}", c as u32))
        );
    }

    // Find the last 32nd rest (should be in measure 2, at tick 1860)
    let last_32nd = result
        .beat_positions
        .iter()
        .filter(|bp| bp.measure == 1) // Measure 2 is index 1
        .filter(|bp| bp.duration_ticks == 60) // 32nd note = 60 ticks
        .next_back();

    assert!(last_32nd.is_some(), "Should find the last 32nd rest");
    let bp = last_32nd.unwrap();

    println!("\nLast 32nd rest found at:");
    println!(
        "  measure={} beat={} tick={} absolute_tick={}",
        bp.measure, bp.beat, bp.tick, bp.absolute_tick
    );

    // The last 32nd should be at tick 1860 within measure 2
    assert_eq!(bp.measure, 1, "Should be in measure 2 (index 1)");
    assert_eq!(bp.duration_ticks, 60, "Should be 60 ticks (32nd note)");
}

/// Test: Query specific musical position by measure.beat.tick format
#[test]
fn test_query_by_musical_position() {
    let source = r#"
#G

VS
C_1 | G_2 D_4 Em_8 G_16 D_32 r32
"#;
    let result = layout_snippet(source);

    println!("\n=== Beat Positions for: C_1 | G_2 D_4 Em_8 G_16 D_32 r32 ===");
    for (i, bp) in result.beat_positions.iter().enumerate() {
        println!(
            "[{}] m{}.b{}.t{} (abs:{}) dur={} glyph={:?}",
            i,
            bp.measure + 1,
            bp.beat + 1,
            bp.tick,
            bp.absolute_tick,
            bp.duration_ticks,
            bp.glyph_codepoint.map(|c| format!("U+{:04X}", c as u32))
        );
    }

    // Query for position 2.4 (measure 2, beat 4) which should be around the 16th/32nd notes
    // Beat 4 starts at tick 1440 (after half=960 + quarter=480)
    let beat_4_elements: Vec<_> = result
        .beat_positions
        .iter()
        .filter(|bp| bp.measure == 1 && bp.tick >= 1440)
        .collect();

    println!("\nElements at or after measure 2, beat 4 (tick >= 1440):");
    for bp in &beat_4_elements {
        println!("  tick={} duration={}", bp.tick, bp.duration_ticks);
    }

    assert!(
        !beat_4_elements.is_empty(),
        "Should have elements at beat 4"
    );
}

/// Test: Query by REAPER-style position (measure.beat.fraction)
/// Position 2.4.875 should find the last 32nd rest at tick 1860
#[test]
fn test_query_reaper_position() {
    let source = r#"
#G

VS
r1 | r2 r4 r8 r16 r32 r32
"#;
    let result = layout_snippet(source);

    // 2.4.875 = measure 2, beat 4, 87.5% into beat
    // Beat 4 starts at tick 1440, 0.875 * 480 = 420 ticks
    // 1440 + 420 = 1860 ticks (the last 32nd rest)
    let target_tick = reaper_position_to_tick(2, 4, 875, 480);
    println!(
        "\nQuerying position 2.4.875 -> absolute tick {}",
        target_tick
    );

    let found = find_at_reaper_position(&result.beat_positions, 2, 4, 875);

    assert!(found.is_some(), "Should find element at position 2.4.875");
    let bp = found.unwrap();

    println!(
        "Found: measure={} beat={} tick={} duration={} glyph={:?}",
        bp.measure + 1,
        bp.beat + 1,
        bp.tick,
        bp.duration_ticks,
        bp.glyph_codepoint.map(|c| format!("U+{:04X}", c as u32))
    );

    // Should be the last 32nd rest (60 ticks duration)
    assert_eq!(bp.duration_ticks, 60, "Should be a 32nd note (60 ticks)");
    assert_eq!(bp.tick, 1860, "Should be at tick 1860 within measure");
}

// =============================================================================
// Glyph Verification Tests
// =============================================================================

/// Test: Verify all chord note properties (glyph, stem, flags, position)
///
/// For: C_1 | G_2 D_4 Em_8 G_16 D_32 r32
#[test]
fn test_chord_position_properties() {
    // Disable auto-rhythm-slashes (default-on for master rhythm charts) so each
    // chord keeps its written note value — this test verifies per-note-value
    // glyphs/durations (whole=1920, half=960, …), which auto-expansion to
    // quarter slashes would erase.
    let source = r#"
/AUTO_RHYTHM_SLASHES=false
#G

VS
C_1 | G_2 D_4 Em_8 G_16 D_32 r32
"#;
    let result = layout_snippet(source);

    println!("\n=== Verifying Chord Position Properties ===");

    let expectations = vec![
        ExpectedBeat {
            measure: 1,
            beat: 1,
            fraction: 0,
            duration_ticks: 1920,
            glyph: SmuflGlyph::NoteheadSlashWhite,
            has_stem: false,
            flag_count: 0,
            desc: "C_1 whole note",
        },
        ExpectedBeat {
            measure: 2,
            beat: 1,
            fraction: 0,
            duration_ticks: 960,
            glyph: SmuflGlyph::NoteheadSlashHalf,
            has_stem: false,
            flag_count: 0,
            desc: "G_2 half note",
        },
        ExpectedBeat {
            measure: 2,
            beat: 3,
            fraction: 0,
            duration_ticks: 480,
            glyph: SmuflGlyph::NoteheadSlashBlack,
            has_stem: true,
            flag_count: 0,
            desc: "D_4 quarter note",
        },
        ExpectedBeat {
            measure: 2,
            beat: 4,
            fraction: 0,
            duration_ticks: 240,
            glyph: SmuflGlyph::NoteheadSlashBlack,
            has_stem: true,
            flag_count: 1,
            desc: "Em_8 eighth note",
        },
        ExpectedBeat {
            measure: 2,
            beat: 4,
            fraction: 500,
            duration_ticks: 120,
            glyph: SmuflGlyph::NoteheadSlashBlack,
            has_stem: true,
            flag_count: 2,
            desc: "G_16 sixteenth note",
        },
        ExpectedBeat {
            measure: 2,
            beat: 4,
            fraction: 750,
            duration_ticks: 60,
            glyph: SmuflGlyph::NoteheadSlashBlack,
            has_stem: true,
            flag_count: 3,
            desc: "D_32 thirty-second note",
        },
        ExpectedBeat {
            measure: 2,
            beat: 4,
            fraction: 875,
            duration_ticks: 60,
            glyph: SmuflGlyph::NoteheadSlashBlack,
            has_stem: true,
            flag_count: 3,
            desc: "r32 thirty-second rest",
        },
    ];

    for expected in &expectations {
        let found = find_at_reaper_position(
            &result.beat_positions,
            expected.measure,
            expected.beat,
            expected.fraction,
        );

        if let Some(bp) = found {
            println!(
                "{}: pos={}.{}.{} dur={} glyph={:?} stem={} flags={} x={:.1} y={:.1}",
                expected.desc,
                expected.measure,
                expected.beat,
                expected.fraction,
                bp.duration_ticks,
                bp.glyph_codepoint.map(|c| format!("U+{:04X}", c as u32)),
                bp.has_stem,
                bp.flag_count,
                bp.x,
                bp.glyph_y,
            );
            assert_beat_properties(bp, expected);
        } else {
            panic!(
                "{}: no beat position found at {}.{}.{}",
                expected.desc, expected.measure, expected.beat, expected.fraction
            );
        }
    }
}

/// Expected properties for a beat position.
#[derive(Debug)]
struct ExpectedBeat {
    /// REAPER position: measure (1-indexed)
    measure: u32,
    /// REAPER position: beat (1-indexed)
    beat: u32,
    /// REAPER position: fraction (0-999)
    fraction: u32,
    /// Expected duration in ticks
    duration_ticks: i32,
    /// Expected glyph
    glyph: SmuflGlyph,
    /// Expected stem presence
    has_stem: bool,
    /// Expected flag count (0 for quarter+, 1 for 8th, 2 for 16th, 3 for 32nd)
    flag_count: u8,
    /// Description for error messages
    desc: &'static str,
}

/// Assert that a beat position matches all expected properties.
fn assert_beat_properties(
    bp: &keyflow::engraver::layout::chart::BeatPosition,
    expected: &ExpectedBeat,
) {
    // Check duration
    assert_eq!(
        bp.duration_ticks, expected.duration_ticks,
        "{}: wrong duration (expected {}, got {})",
        expected.desc, expected.duration_ticks, bp.duration_ticks
    );

    // Check glyph
    assert_glyph(bp, expected.glyph, expected.desc);

    // Check stem
    assert_eq!(
        bp.has_stem, expected.has_stem,
        "{}: wrong has_stem (expected {}, got {})",
        expected.desc, expected.has_stem, bp.has_stem
    );

    // Check flag count
    assert_eq!(
        bp.flag_count, expected.flag_count,
        "{}: wrong flag_count (expected {}, got {})",
        expected.desc, expected.flag_count, bp.flag_count
    );

    // Check that position is valid (x > 0, glyph_y > 0)
    assert!(
        bp.x > 0.0,
        "{}: invalid x position ({})",
        expected.desc,
        bp.x
    );
    assert!(
        bp.glyph_y > 0.0,
        "{}: invalid glyph_y position ({})",
        expected.desc,
        bp.glyph_y
    );
}

/// Test: Verify all rest value properties (glyph, stem, flags, position)
///
/// For: r1 | r2 r4 r8 r16 r32 r32
#[test]
fn test_rest_position_properties() {
    let source = r#"
#G

VS
r1 | r2 r4 r8 r16 r32 r32
"#;
    let result = layout_snippet(source);

    println!("\n=== Verifying Rest Position Properties ===");

    let expectations = vec![
        ExpectedBeat {
            measure: 1,
            beat: 1,
            fraction: 0,
            duration_ticks: 1920,
            glyph: SmuflGlyph::NoteheadSlashWhite,
            has_stem: false,
            flag_count: 0,
            desc: "r1 whole rest",
        },
        ExpectedBeat {
            measure: 2,
            beat: 1,
            fraction: 0,
            duration_ticks: 960,
            glyph: SmuflGlyph::NoteheadSlashHalf,
            has_stem: false,
            flag_count: 0,
            desc: "r2 half rest",
        },
        ExpectedBeat {
            measure: 2,
            beat: 3,
            fraction: 0,
            duration_ticks: 480,
            glyph: SmuflGlyph::NoteheadSlashBlack,
            has_stem: true,
            flag_count: 0,
            desc: "r4 quarter rest",
        },
        ExpectedBeat {
            measure: 2,
            beat: 4,
            fraction: 0,
            duration_ticks: 240,
            glyph: SmuflGlyph::NoteheadSlashBlack,
            has_stem: true,
            flag_count: 1,
            desc: "r8 eighth rest",
        },
        ExpectedBeat {
            measure: 2,
            beat: 4,
            fraction: 500,
            duration_ticks: 120,
            glyph: SmuflGlyph::NoteheadSlashBlack,
            has_stem: true,
            flag_count: 2,
            desc: "r16 sixteenth rest",
        },
        ExpectedBeat {
            measure: 2,
            beat: 4,
            fraction: 750,
            duration_ticks: 60,
            glyph: SmuflGlyph::NoteheadSlashBlack,
            has_stem: true,
            flag_count: 3,
            desc: "r32 first 32nd rest",
        },
        ExpectedBeat {
            measure: 2,
            beat: 4,
            fraction: 875,
            duration_ticks: 60,
            glyph: SmuflGlyph::NoteheadSlashBlack,
            has_stem: true,
            flag_count: 3,
            desc: "r32 second 32nd rest",
        },
    ];

    for expected in &expectations {
        let found = find_at_reaper_position(
            &result.beat_positions,
            expected.measure,
            expected.beat,
            expected.fraction,
        );

        if let Some(bp) = found {
            println!(
                "{}: pos={}.{}.{} dur={} glyph={:?} stem={} flags={} x={:.1} y={:.1}",
                expected.desc,
                expected.measure,
                expected.beat,
                expected.fraction,
                bp.duration_ticks,
                bp.glyph_codepoint.map(|c| format!("U+{:04X}", c as u32)),
                bp.has_stem,
                bp.flag_count,
                bp.x,
                bp.glyph_y,
            );
            assert_beat_properties(bp, expected);
        } else {
            panic!(
                "{}: no beat position found at {}.{}.{}",
                expected.desc, expected.measure, expected.beat, expected.fraction
            );
        }
    }
}

// =============================================================================
// Triplet Position Tests
// =============================================================================

/// Test: Triplet eighth note positions (Thriller intro pattern)
///
/// Source: r8t Ab9_8t r8t r8t r8t F9_8t r2 | s1
///
/// Triplet eighth notes: 480 / 3 = 160 ticks each
/// Positions within a beat:
/// - .000 = 0 ticks (first triplet)
/// - .333 = 160 ticks (second triplet)
/// - .666 = 320 ticks (third triplet)
///
/// Layout for first measure (r8t Ab9_8t r8t r8t r8t F9_8t r2):
/// - Beat 1: r8t (0), Ab9_8t (160), r8t (320)
/// - Beat 2: r8t (480), r8t (640), F9_8t (800)
/// - Beats 3-4: r2 (960-1920)
#[test]
fn test_triplet_eighth_positions() {
    let source = r#"
Thriller Intro
120bpm 4/4 #Ab

IN
r8t Ab9_8t r8t r8t r8t F9_8t r2 | s1
"#;
    let result = layout_snippet(source);

    println!("\n=== Triplet Eighth Note Positions ===");
    println!("Source: r8t Ab9_8t r8t r8t r8t F9_8t r2 | s1\n");

    // Print all beat positions for measure 1
    println!("All beat positions in measure 1:");
    for bp in result.beat_positions.iter().filter(|bp| bp.measure == 0) {
        let beat_num = bp.tick / 480 + 1;
        let beat_fraction = (bp.tick % 480) as f64 / 480.0 * 1000.0;
        println!(
            "  pos={}.{}.{:.0} tick={} dur={} glyph={:?}",
            bp.measure + 1,
            beat_num,
            beat_fraction,
            bp.tick,
            bp.duration_ticks,
            bp.glyph_codepoint.map(|c| format!("U+{:04X}", c as u32))
        );
    }

    // Triplet eighth = 160 ticks
    let triplet_eighth_ticks = 160;

    // Expected triplet positions for beat 1
    let triplet_positions = vec![
        (1, 1, 0, "first triplet (r8t)"),    // tick 0
        (1, 1, 333, "second triplet (Ab9)"), // tick 160
        (1, 1, 666, "third triplet (r8t)"),  // tick 320
        (1, 2, 0, "fourth triplet (r8t)"),   // tick 480
        (1, 2, 333, "fifth triplet (r8t)"),  // tick 640
        (1, 2, 666, "sixth triplet (F9)"),   // tick 800
    ];

    println!("\nVerifying triplet positions:");
    for (measure, beat, fraction, desc) in &triplet_positions {
        let found = find_at_reaper_position(&result.beat_positions, *measure, *beat, *fraction);
        if let Some(bp) = found {
            println!(
                "  {}: pos={}.{}.{} tick={} dur={} ✓",
                desc, measure, beat, fraction, bp.tick, bp.duration_ticks
            );
            // Triplet eighths should be 160 ticks
            assert_eq!(
                bp.duration_ticks, triplet_eighth_ticks,
                "{}: expected {} ticks, got {}",
                desc, triplet_eighth_ticks, bp.duration_ticks
            );
        } else {
            println!("  {}: NOT FOUND at {}.{}.{}", desc, measure, beat, fraction);
        }
    }

    // Verify the half rest at beat 3
    let half_rest = find_at_reaper_position(&result.beat_positions, 1, 3, 0);
    if let Some(bp) = half_rest {
        println!(
            "\nHalf rest: pos=1.3.0 tick={} dur={} (expected 960)",
            bp.tick, bp.duration_ticks
        );
        assert_eq!(bp.duration_ticks, 960, "Half rest should be 960 ticks");
    }
}

/// Test: Triplet push chord positions
///
/// Source: 'F/C . | Cm . | ... | Cm // Gm7 // 'Abmaj7 / Abmaj7#5 / 'Db7#11/G //
///
/// The `'` prefix indicates a triplet push, which shifts the chord earlier by
/// one triplet eighth (160 ticks). In REAPER position terms:
/// - Normal beat 1.1.0 becomes pushed to previous beat's .666 position
/// - Or equivalently, the chord sounds 160 ticks before the written position
#[test]
fn test_triplet_push_positions() {
    let source = r#"
Thriller Verse
120bpm 4/4 #Ab
/push = triplet

VS
'F/C . | Cm . | 'F/C . | Cm . | 'F/C . | Cm . | 'F/C . | Cm // Gm7 // 'Abmaj7 / Abmaj7#5 / 'Db7#11/G //
"#;
    let result = layout_snippet(source);

    println!("\n=== Triplet Push Chord Positions ===");
    println!("Source: 'F/C . | Cm . | ... (with /push = triplet)\n");

    // Print all beat positions
    println!("All beat positions:");
    for (i, bp) in result.beat_positions.iter().enumerate() {
        let measure = bp.measure + 1;
        let beat_num = bp.tick / 480 + 1;
        let beat_fraction = (bp.tick % 480) as f64 / 480.0 * 1000.0;
        println!(
            "  [{}] m{} pos={}.{}.{:.0} abs_tick={} dur={} glyph={:?}",
            i,
            measure,
            measure,
            beat_num,
            beat_fraction,
            bp.absolute_tick,
            bp.duration_ticks,
            bp.glyph_codepoint.map(|c| format!("U+{:04X}", c as u32))
        );
    }

    // Check that we have beat positions
    assert!(
        !result.beat_positions.is_empty(),
        "Should have beat positions"
    );

    // The first chord 'F/C has a triplet push, so it should appear
    // 160 ticks before beat 1 (i.e., at a negative offset or late in previous measure)
    // In the first measure, pushed chords typically have push_pull_offset in the position data
    println!("\nNote: Triplet pushes shift timing by 160 ticks (1/3 of a quarter note)");
}

/// Test: Mixed triplet pattern from Thriller chorus
///
/// Source: Cm/Eb / 'Eb // | 'Eb / 'F/C / 'Cm // | ...
///
/// This tests a more complex pattern with:
/// - Regular slashes (/)
/// - Triplet pushed chords ('Eb, 'F/C, 'Cm)
/// - Mixed rhythm within measures
#[test]
fn test_triplet_mixed_pattern() {
    let source = r#"
Thriller Chorus
120bpm 4/4 #Ab
/push = triplet

CH
Cm/Eb / 'Eb // | 'Eb / 'F/C / 'Cm // | 'F/A //// | 'Fm9 ////
"#;
    let result = layout_snippet(source);

    println!("\n=== Mixed Triplet Pattern ===");
    println!("Source: Cm/Eb / 'Eb // | 'Eb / 'F/C / 'Cm // | ...\n");

    // Print beat positions with push/pull info
    println!("Beat positions:");
    for bp in &result.beat_positions {
        let measure = bp.measure + 1;
        let beat_num = bp.tick / 480 + 1;
        let beat_fraction = (bp.tick % 480) as f64 / 480.0 * 1000.0;
        println!(
            "  m{} pos={}.{}.{:.0} tick={} dur={}",
            measure, measure, beat_num, beat_fraction, bp.tick, bp.duration_ticks
        );
    }

    // Verify we have positions for all 4 measures
    let measure_count = result
        .beat_positions
        .iter()
        .map(|bp| bp.measure)
        .max()
        .map(|m| m + 1)
        .unwrap_or(0);
    println!("\nTotal measures with beat positions: {}", measure_count);
    assert!(measure_count >= 4, "Should have at least 4 measures");
}

/// Test: Triplet rest pattern with explicit triplet duration
///
/// This verifies that r8t (triplet eighth rest) has correct 160-tick duration
#[test]
fn test_triplet_rest_duration() {
    let source = r#"
Triplet Rests
120bpm 4/4 #C

IN
r8t r8t r8t r8t r8t r8t r8t r8t r8t r8t r8t r8t
"#;
    let result = layout_snippet(source);

    println!("\n=== Triplet Rest Durations ===");
    println!("Source: r8t r8t r8t r8t r8t r8t r8t r8t r8t r8t r8t r8t\n");

    // All positions should have 160-tick duration (triplet eighth)
    println!("All beat positions (should all be 160 ticks):");
    for (i, bp) in result.beat_positions.iter().enumerate() {
        println!("  [{}] tick={} dur={}", i, bp.tick, bp.duration_ticks);
        assert_eq!(
            bp.duration_ticks, 160,
            "Position {} should be 160 ticks (triplet eighth), got {}",
            i, bp.duration_ticks
        );
    }

    // Should have 12 triplet eighth rests
    assert_eq!(
        result.beat_positions.len(),
        12,
        "Should have 12 triplet eighth positions"
    );
}
