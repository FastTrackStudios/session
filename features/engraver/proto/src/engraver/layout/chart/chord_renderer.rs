//! Chord symbol rendering for chart layout.
//!
//! This module extracts the duplicated chord rendering logic from
//! `layout_paginated` and `layout_continuous` into reusable functions.
//!
//! # Multi-Pass Layout Architecture
//!
//! With the multi-pass layout system (see [`super::measure_pass`]), chord symbol
//! widths are pre-measured during Pass 1 (Measure). This provides accurate minimum
//! measure widths to the layout pass, which should prevent most collisions.
//!
//! # Collision Detection (Safety Net)
//!
//! Despite accurate pre-measurement, collision detection is retained as a safety net.
//! When adjacent chord symbols would overlap or be closer than the minimum gap:
//! - The first chord shifts slightly left
//! - The second chord stays at its notehead-aligned position
//!
//! With proper pre-measurement, collisions should be rare. The debug logging
//! (via `tracing::debug!`) reports when collisions occur, which
//! can indicate issues with the measure pass or layout distribution.
//!
//! To disable collision detection, set `min_chord_symbol_gap = 0.0` in config.

use kurbo::{Rect, Vec2};
use peniko::Color;

use crate::chart::commands::Command;
use crate::chart::types::{ChordInstance, Measure, RhythmElement};
use crate::chord::ChordRhythm;
use crate::engraver::layout::context::LayoutContext;
use crate::engraver::layout::tlayout::{HarmonyStyle, layout_harmony, parse_chord};
use crate::engraver::scene::id::{ElementType, SemanticId};
use crate::engraver::scene::node::{SceneNode, metadata_keys};
use crate::engraver::scene::paint::PaintCommand;
use crate::time::{MusicalPositionExt, TimeSignature};
use crate::{ChartPosition, SourceLink};

use super::PushSpillback;
use super::measure_pass::MeasureMeasurements;

/// Create push marker color (red for visibility)
fn push_marker_color() -> Color {
    Color::from_rgba8(0xCC, 0x00, 0x00, 0xFF)
}

/// Create pull marker color (blue to distinguish from push)
fn pull_marker_color() -> Color {
    Color::from_rgba8(0x00, 0x00, 0xCC, 0xFF)
}

/// Create accent marker color (red for visibility)
fn accent_marker_color() -> Color {
    Color::from_rgba8(0xCC, 0x00, 0x00, 0xFF)
}

/// Context for rendering chord symbols in a measure.
#[derive(Debug, Clone)]
pub struct ChordRenderContext<'a> {
    /// Measure x position (start of measure content).
    pub measure_x: f64,
    /// Measure width (for collision boundary clamping).
    pub measure_width: f64,
    /// Chord y position (above staff).
    pub chord_y: f64,
    /// Page number (1-indexed, for paginated mode).
    pub page_number: Option<u32>,
    /// Global system index (0-indexed).
    pub global_system_index: usize,
    /// Section measure index (within this section).
    pub measure_idx: usize,
    /// Local measure index (within the current system).
    pub local_measure_idx: usize,
    /// Section type name for metadata.
    pub section_name: &'a str,
    /// Segment positions from measure layout.
    pub segment_positions: &'a [f64],
    /// Internal push positions from rhythm builder.
    pub internal_push_positions: &'a [(usize, usize)],
    /// Harmony style for chord symbols.
    pub harmony_style: &'a HarmonyStyle,
    /// Time signature (numerator, denominator).
    pub time_signature: (u8, u8),
    /// Whether to hide repeated consecutive chords.
    pub hide_repeated_chords: bool,
    /// Minimum horizontal gap between adjacent chord symbols (in points).
    /// Set to 0.0 to disable collision detection.
    pub min_chord_symbol_gap: f64,
    /// Whether push/pull notation alters the rhythm display.
    /// When false, shows apostrophe markers on chord symbols instead.
    pub push_alters_rhythm: bool,
    /// Spatium (staff space) for sizing apostrophe markers.
    pub spatium: f64,
    /// Pre-computed measurements for this measure (from measure pass).
    /// When provided, collision detection uses cached chord layouts instead
    /// of re-measuring during render.
    pub measure_measurements: Option<&'a MeasureMeasurements>,
    /// Spillback positions computed by rhythm builder.
    /// Maps (rhythm_index, chord_symbol) for chords from next measure pushing back.
    /// Used to place spillback chords at correct triplet positions.
    pub spillback_positions: &'a [(usize, String)],
    /// Per-rhythm-entry notehead stack bounds `(min_line, max_line)`.
    /// Used to lift chord symbols just enough to clear the music at the
    /// same beat position.
    pub note_line_stacks: &'a [Option<(i32, i32)>],
}

/// Result of chord symbol rendering.
#[derive(Debug)]
pub struct ChordRenderResult {
    /// Rendered chord nodes.
    pub nodes: Vec<SceneNode>,
    /// Final world-space chord symbol bounds after collision adjustments.
    ///
    /// Staff-attached notation uses these as skyline obstacles so text,
    /// dynamics, figured bass, and hairpins avoid the actual chord symbols
    /// instead of a coarse measure-wide band.
    pub chord_bounds: Vec<Rect>,
    /// Updated previous chord symbol (for duplicate detection).
    pub last_chord_symbol: Option<String>,
    /// Next ID counter value.
    pub next_id: u64,
}

// ============================================================================
// Collision Detection
// ============================================================================

/// Information about a rendered chord for collision detection.
#[derive(Debug)]
struct ChordBoundsInfo {
    /// Index in the nodes vector.
    node_idx: usize,
    /// Original X position (in world coordinates).
    original_x: f64,
    /// Bounding box in world coordinates.
    world_bounds: Rect,
}

/// Result of chord collision resolution.
#[derive(Debug)]
pub struct ChordCollisionResult {
    /// X-position adjustments for each chord (indexed same as input).
    /// Positive = shift right, negative = shift left.
    pub adjustments: Vec<f64>,
    /// Whether any collisions were detected and resolved.
    pub had_collisions: bool,
}

/// Detect and resolve overlapping chord symbols.
///
/// This function takes rendered chord bounds and computes position adjustments
/// needed to achieve the minimum gap between adjacent chords.
///
/// # Algorithm
///
/// For each adjacent pair of chords:
/// 1. Compute the gap (left edge of B - right edge of A)
/// 2. If gap < min_gap, compute the overlap amount
/// 3. **Only shift the first chord left** - the second chord's notehead position
///    is already correct because `compute_chord_min_widths()` set segment minimums
///
/// The segment minimum widths ensure noteheads are positioned correctly.
/// This function is a safety net that only moves the FIRST chord symbol left
/// when there's still a collision (e.g., spring system didn't allocate full space).
///
/// # Arguments
///
/// * `chord_bounds` - List of chord bounds info, sorted by X position
/// * `min_gap` - Minimum required horizontal gap between chords (in points)
/// * `_measure_start_x` - Left boundary (unused - we allow moving past clef)
/// * `_measure_end_x` - Right boundary (unused - we only shift first chord left)
///
/// # Returns
///
/// `ChordCollisionResult` with adjustments for each chord.
fn resolve_chord_collisions(
    chord_bounds: &[ChordBoundsInfo],
    min_gap: f64,
    _measure_start_x: f64,
    _measure_end_x: f64,
) -> ChordCollisionResult {
    if chord_bounds.len() < 2 {
        return ChordCollisionResult {
            adjustments: vec![0.0; chord_bounds.len()],
            had_collisions: false,
        };
    }

    let mut adjustments = vec![0.0; chord_bounds.len()];
    let mut had_collisions = false;

    // Process adjacent pairs left-to-right
    for i in 0..chord_bounds.len() - 1 {
        let bounds_a = &chord_bounds[i];
        let bounds_b = &chord_bounds[i + 1];

        // Apply accumulated adjustments to get current positions
        let adjusted_right_a = bounds_a.world_bounds.x1 + adjustments[i];
        let adjusted_left_b = bounds_b.world_bounds.x0 + adjustments[i + 1];

        // Compute current gap
        let gap = adjusted_left_b - adjusted_right_a;

        if gap < min_gap {
            // Collision detected - compute overlap
            let overlap = min_gap - gap;
            had_collisions = true;

            // Only shift the first chord left - the second chord's notehead
            // is already correctly positioned via segment minimum widths.
            // Moving only the first chord left preserves notehead-chord alignment.
            adjustments[i] -= overlap;

            tracing::debug!(
                "[chord-collision] Detected overlap of {:.1}pt between chords {} and {}. \
                 Moving first chord left by {:.1}pt (preserves notehead alignment)",
                overlap,
                i,
                i + 1,
                overlap
            );
        }
    }

    ChordCollisionResult {
        adjustments,
        had_collisions,
    }
}

/// Create an apostrophe marker node for pushed/pulled chords.
///
/// When `push_alters_rhythm=false`, this creates a visual indicator showing
/// the chord is pushed (') or pulled (') without altering the rhythm notation.
///
/// # Arguments
/// * `is_push` - true for push (before chord), false for pull (after chord)
/// * `chord_bounds` - The bounding box of the chord symbol for positioning
/// * `chord_y` - Y position (same as chord symbol baseline)
/// * `spatium` - Staff space for sizing
/// * `id` - Unique ID for the node
fn create_push_marker(
    is_push: bool,
    chord_bounds: Rect,
    chord_y: f64,
    spatium: f64,
    id: u64,
) -> SceneNode {
    let marker_text = "'";
    let color = if is_push {
        push_marker_color()
    } else {
        pull_marker_color()
    };

    // Size the apostrophe relative to spatium (similar to chord symbol text)
    let font_size = spatium * 2.5;

    // Position: push markers go before (left of) the chord, pull markers after
    // The chord_y is the baseline of the chord symbol
    let marker_x = if is_push {
        chord_bounds.x0 - spatium * 0.6 // Slightly left of chord
    } else {
        chord_bounds.x1 + spatium * 0.1 // Slightly right of chord
    };

    let paint = PaintCommand::text(
        marker_text,
        "MuseJazz Text",
        font_size,
        kurbo::Point::new(marker_x, chord_y),
        color,
    );

    let mut node = SceneNode::leaf(SemanticId::new(ElementType::Articulation, id), vec![paint]);
    node.set_element_type("push_marker");
    node
}

/// Create an accent marker node for accented chords.
///
/// Renders the SMuFL accent articulation glyph (>) above the rhythm slash notehead.
/// Following MuseScore's articulation positioning:
/// - Accent placed just above the top staff line
/// - Minimum distance of 0.4 spatiums from the notehead
/// - Horizontally centered on the rhythm slash (segment position)
///
/// # Arguments
/// * `segment_x` - X position of the rhythm segment (slash position) for horizontal centering
/// * `chord_y` - Y position of chord symbol baseline (used to derive staff position)
/// * `spatium` - Staff space for sizing and positioning
/// * `id` - Unique ID for the node
fn create_accent_marker(segment_x: f64, chord_y: f64, spatium: f64, id: u64) -> SceneNode {
    use super::constants::{ARTICULATION_MAG, ARTICULATION_MIN_DISTANCE_SPATIUMS, CHORD_Y_OFFSET};

    // SMuFL articAccentAbove: U+E4A0
    let accent_glyph = '\u{E4A0}';
    let color = accent_marker_color();

    // Size relative to spatium (with articulation magnification factor)
    // Use 1.2x spatium for a more compact accent
    let font_size = spatium * 1.2 * ARTICULATION_MAG;

    // Calculate staff Y position from chord_y
    // chord_y = staff_y + CHORD_Y_OFFSET.
    let staff_y = chord_y - CHORD_Y_OFFSET;

    // Position accent just above the top staff line
    // Staff top line is at staff_y, we want the accent above that
    // Use minimal distance for a tighter appearance
    let accent_y = staff_y - ARTICULATION_MIN_DISTANCE_SPATIUMS * spatium;

    // Horizontally center on the rhythm slash (segment position)
    // The segment_x is the left edge of the segment, add half a spatium to center on the slash
    let accent_x = segment_x + spatium * 0.5;

    let paint = PaintCommand::glyph(
        accent_glyph,
        kurbo::Point::new(accent_x, accent_y),
        font_size,
        color,
    );

    let mut node = SceneNode::leaf(SemanticId::new(ElementType::Articulation, id), vec![paint]);
    node.set_element_type("accent");
    node
}

/// Create a staccato marker node for staccato chords.
///
/// Renders the SMuFL staccato articulation glyph (·) above the rhythm slash notehead.
/// Staccato indicates a short, detached chord hit — common in intro stabs and rhythmic figures.
/// Positioned similarly to accents but uses a smaller dot glyph closer to the notehead.
///
/// # Arguments
/// * `segment_x` - X position of the rhythm segment (slash position) for horizontal centering
/// * `chord_y` - Y position of chord symbol baseline (used to derive staff position)
/// * `spatium` - Staff space for sizing and positioning
/// * `id` - Unique ID for the node
fn create_staccato_marker(segment_x: f64, chord_y: f64, spatium: f64, id: u64) -> SceneNode {
    use super::constants::{ARTICULATION_MAG, ARTICULATION_MIN_DISTANCE_SPATIUMS, CHORD_Y_OFFSET};

    // SMuFL articStaccatoAbove: U+E4A2
    let staccato_glyph = '\u{E4A2}';
    let color = Color::from_rgba8(0xCC, 0x00, 0x00, 0xFF); // Red — matches accent markers

    // Size relative to spatium (with articulation magnification factor)
    let font_size = spatium * 1.2 * ARTICULATION_MAG;

    // Calculate staff Y position from chord_y
    let staff_y = chord_y - CHORD_Y_OFFSET;

    // Position staccato just above the top staff line (same region as accent)
    // Staccato sits closer to the notehead than accent per convention
    let staccato_y = staff_y - ARTICULATION_MIN_DISTANCE_SPATIUMS * spatium;

    // Horizontally center on the rhythm slash
    let staccato_x = segment_x + spatium * 0.5;

    let paint = PaintCommand::glyph(
        staccato_glyph,
        kurbo::Point::new(staccato_x, staccato_y),
        font_size,
        color,
    );

    let mut node = SceneNode::leaf(SemanticId::new(ElementType::Articulation, id), vec![paint]);
    node.set_element_type("staccato");
    node
}

/// Stop sign color (red, like a real stop sign)
fn stop_sign_color() -> Color {
    Color::from_rgba8(0xCC, 0x00, 0x00, 0xFF)
}

/// Stop sign text color (white text on red background)
fn stop_sign_text_color() -> Color {
    Color::from_rgba8(0xFF, 0xFF, 0xFF, 0xFF)
}

/// Stop sign border color (white border like a real stop sign)
fn stop_sign_border_color() -> Color {
    Color::from_rgba8(0xFF, 0xFF, 0xFF, 0xFF)
}

/// Build an octagon BezPath centered at (cx, cy) with the given radius.
/// Rotated so the top and bottom edges are flat (like a real stop sign).
fn octagon_path(cx: f64, cy: f64, radius: f64) -> kurbo::BezPath {
    use kurbo::{BezPath, Point};
    let mut path = BezPath::new();
    for i in 0..8 {
        let angle = std::f64::consts::PI / 8.0 + (i as f64) * std::f64::consts::PI / 4.0;
        let px = cx + radius * angle.cos();
        let py = cy + radius * angle.sin();
        if i == 0 {
            path.move_to(Point::new(px, py));
        } else {
            path.line_to(Point::new(px, py));
        }
    }
    path.close_path();
    path
}

/// Create a stop sign marker node (octagonal shape with "STOP" text).
///
/// Rendered inline at chord symbol level, positioned before or after the chord.
/// - Before (`!STOP C`): stop sign to the left of the chord — "stop, then hit C"
/// - After (`C !STOP`): stop sign to the right of the chord — "hit C, then stop"
///
/// # Arguments
/// * `chord_bounds` - Bounding box of the chord symbol for positioning
/// * `chord_y` - Y position of chord symbol baseline
/// * `spatium` - Staff space for sizing and positioning
/// * `after` - If true, position after (right of) the chord; if false, before (left)
/// * `id` - Unique ID for the node
fn create_stop_marker(
    chord_bounds: Rect,
    chord_y: f64,
    spatium: f64,
    after: bool,
    id: u64,
) -> SceneNode {
    use kurbo::Point;

    let size = spatium * 2.4; // Outer radius — slightly larger for breathing room
    let red_border = spatium * 0.2; // Thin red outer ring
    let white_band = spatium * 0.15; // White inset band
    let gap = spatium * 0.5;

    let center_y = chord_y - spatium * 1.0;
    let center_x = if after {
        chord_bounds.x1 + gap + size
    } else {
        chord_bounds.x0 - gap - size
    };

    // Layer 1: Red outer octagon (the thin red border)
    let outer = octagon_path(center_x, center_y, size);
    // Layer 2: White inset band
    let white = octagon_path(center_x, center_y, size - red_border);
    // Layer 3: Red fill (main body inside the white band)
    let inner = octagon_path(center_x, center_y, size - red_border - white_band);

    let mut paints = Vec::new();
    paints.push(PaintCommand::filled_path(outer, stop_sign_color()));
    paints.push(PaintCommand::filled_path(white, stop_sign_border_color()));
    paints.push(PaintCommand::filled_path(inner, stop_sign_color()));
    // "STOP" text centered — sized to leave padding inside the white band
    paints.push(PaintCommand::text_centered(
        "STOP",
        "FreeSans",
        spatium * 1.2,
        Point::new(center_x, center_y + spatium * 0.38),
        stop_sign_text_color(),
    ));

    let mut node = SceneNode::leaf(SemanticId::new(ElementType::Articulation, id), paints);
    node.set_element_type("stop_sign");
    node
}

/// Create a stop groove marker node (circular shape with "STOP" text).
///
/// Rendered inline at chord symbol level, positioned before or after the chord.
/// Same positioning logic as `create_stop_marker` but with a circular shape.
///
/// # Arguments
/// * `chord_bounds` - Bounding box of the chord symbol for positioning
/// * `chord_y` - Y position of chord symbol baseline
/// * `spatium` - Staff space for sizing and positioning
/// * `after` - If true, position after (right of) the chord; if false, before (left)
/// * `id` - Unique ID for the node
fn create_stop_groove_marker(
    chord_bounds: Rect,
    chord_y: f64,
    spatium: f64,
    after: bool,
    id: u64,
) -> SceneNode {
    use kurbo::Point;

    let radius = spatium * 2.4;
    let red_border = spatium * 0.2;
    let white_band = spatium * 0.15;
    let gap = spatium * 0.5;

    // Same vertical alignment as stop marker
    let center_y = chord_y - spatium * 1.0;
    let center_x = if after {
        chord_bounds.x1 + gap + radius
    } else {
        chord_bounds.x0 - gap - radius
    };

    let center = Point::new(center_x, center_y);
    let mut paints = Vec::new();

    // Layer 1: Red outer circle (thin red border)
    paints.push(PaintCommand::filled_circle(
        center,
        radius,
        stop_sign_color(),
    ));
    // Layer 2: White inset band
    paints.push(PaintCommand::filled_circle(
        center,
        radius - red_border,
        stop_sign_border_color(),
    ));
    // Layer 3: Red fill (main body)
    paints.push(PaintCommand::filled_circle(
        center,
        radius - red_border - white_band,
        stop_sign_color(),
    ));
    // Two-line text: "STOP" / "GROOVE" — sized to leave padding inside the white band
    let line_spacing = spatium * 0.8;
    paints.push(PaintCommand::text_centered(
        "STOP",
        "FreeSans",
        spatium * 0.9,
        Point::new(center_x, center_y - spatium * 0.05),
        stop_sign_text_color(),
    ));
    paints.push(PaintCommand::text_centered(
        "GROOVE",
        "FreeSans",
        spatium * 0.62,
        Point::new(center_x, center_y + line_spacing),
        stop_sign_text_color(),
    ));

    let mut node = SceneNode::leaf(SemanticId::new(ElementType::Articulation, id), paints);
    node.set_element_type("stop_groove");
    node
}

/// Render text cues for a measure below the staff.
///
/// Cues are instrument-specific directions like `@keys "synth here"` that render
/// in the instrument's color below the staff. Text uses MuseJazzText font with
/// an underline in the same color (overline when rendered above staff).
pub fn render_text_cues(
    cues: &[crate::chart::cues::TextCue],
    measure_x: f64,
    _measure_width: f64,
    staff_y: f64,
    staff_height: f64,
    spatium: f64,
    text_metrics: &crate::engraver::layout::text_metrics::TextFontMetrics,
    id_counter: &mut u64,
) -> Vec<SceneNode> {
    let mut nodes = Vec::new();
    let font_size = spatium * 2.6;
    let line_thickness = spatium * 0.12;
    // Position below the staff with padding
    let base_y = staff_y + staff_height + spatium * 3.0;

    for (i, cue) in cues.iter().enumerate() {
        let (r, g, b, a) = cue.group.color_rgba();
        let color = Color::from_rgba8(r, g, b, a);

        // Just the text — color is enough to identify the instrument
        let display_text = &cue.text;

        // Stack multiple cues vertically
        let cue_y = base_y + (i as f64) * spatium * 2.8;
        let text_x = measure_x + spatium * 0.3;

        let mut paints = Vec::new();

        // Text in MuseJazzText font
        paints.push(PaintCommand::text(
            display_text,
            "MuseJazz Text",
            font_size,
            kurbo::Point::new(text_x, cue_y),
            color,
        ));

        // Underline: use actual font metrics for precise text width
        let text_width = text_metrics.horizontal_advance(display_text, font_size);
        let line_y = cue_y + spatium * 0.4;
        let line_end_x = text_x + text_width;
        paints.push(PaintCommand::line(
            kurbo::Point::new(text_x, line_y),
            kurbo::Point::new(line_end_x, line_y),
            color,
            line_thickness,
        ));

        let mut node = SceneNode::leaf(
            SemanticId::new(ElementType::Articulation, *id_counter),
            paints,
        );
        node.set_element_type("text_cue");
        *id_counter += 1;
        nodes.push(node);
    }

    nodes
}

/// Determine if a chord should be skipped (is a space/rest placeholder).
#[must_use]
pub fn is_placeholder_chord(symbol: &str) -> bool {
    symbol.is_empty() || symbol == "s" || symbol == "r"
}

/// Check if this is the first real (non-placeholder) chord in the measure.
#[must_use]
pub fn is_first_real_chord(chords: &[ChordInstance], chord_idx: usize) -> bool {
    chords
        .iter()
        .take(chord_idx)
        .all(|c| is_placeholder_chord(&c.full_symbol))
}

/// Check if a chord should be rendered at a section/system boundary.
///
/// Pushed chords at boundaries should show even if they would normally spill back.
#[must_use]
pub fn is_at_boundary(measure_idx: usize, local_measure_idx: usize) -> bool {
    let is_first_measure_of_section = measure_idx == 0;
    let is_first_measure_of_system = local_measure_idx == 0;
    is_first_measure_of_section || is_first_measure_of_system
}

/// Calculate the segment index for a chord symbol.
///
/// This complex logic handles multiple cases:
/// - Pushed chords at boundaries (force to segment 0)
/// - Internal pushed chords (use precomputed positions)
/// - Explicit rhythm notation
/// - Slash notation
/// - Simple measures
#[must_use]
pub fn calculate_segment_index(
    measure: &Measure,
    chord_idx: usize,
    chord: &ChordInstance,
    segment_positions: &[f64],
    internal_push_positions: &[(usize, usize)],
    is_first_real: bool,
    is_boundary: bool,
    time_signature: (u8, u8),
) -> usize {
    // Check if this is a pushed chord at a boundary
    let is_pushed_at_boundary = chord
        .push_pull
        .as_ref()
        .is_some_and(|(is_push, _)| *is_push)
        && is_first_real
        && is_boundary;

    if is_pushed_at_boundary {
        // Force pushed chord to segment 0 (beat 1) at section/line start
        return 0;
    }

    // Check precomputed segment positions (used by internal pushes AND staccato measures)
    if !internal_push_positions.is_empty()
        && let Some((_, seg_idx)) = internal_push_positions
            .iter()
            .find(|(c_idx, _)| *c_idx == chord_idx)
    {
        return *seg_idx;
    }

    // Check for explicit rhythm elements
    if !measure.rhythm_elements.is_empty() {
        // Also treat staccato measures as explicit rhythm for segment mapping
        let has_staccato = measure.chords.iter().any(|c| {
            c.commands
                .iter()
                .any(|cmd| matches!(cmd, Command::Staccato))
        });
        let has_explicit_rhythm = measure_has_explicit_chord_rhythm(measure) || has_staccato;

        if has_explicit_rhythm {
            // Explicit rhythm: find chord's index in rhythm_elements
            // Each RhythmElement::Chord maps to a segment in order
            let mut seen_chord_count = 0;
            let mut found_idx = None;
            for (idx, elem) in measure.rhythm_elements.iter().enumerate() {
                if let RhythmElement::Chord(_) = elem {
                    if seen_chord_count == chord_idx {
                        found_idx = Some(idx);
                        break;
                    }
                    seen_chord_count += 1;
                }
            }

            // Debug: log segment count mismatch
            if segment_positions.len() < measure.rhythm_elements.len() {
                tracing::debug!(
                    "[chord-position] WARNING: segment_positions.len()={} < rhythm_elements.len()={} for chord_idx={}",
                    segment_positions.len(),
                    measure.rhythm_elements.len(),
                    chord_idx
                );
            }

            return found_idx
                .unwrap_or(chord_idx)
                .min(segment_positions.len().saturating_sub(1));
        }

        // Slash notation: calculate segment from cumulative beat durations
        let mut cumulative_beats = 0usize;
        let mut found_beat_pos = None;
        let mut seen_chord_count = 0;

        for elem in measure.rhythm_elements.iter() {
            if let RhythmElement::Chord(c) = elem {
                if seen_chord_count == chord_idx {
                    found_beat_pos = Some(cumulative_beats);
                    break;
                }
                let chord_beats = match &c.rhythm {
                    ChordRhythm::Slashes { count, .. } => *count as usize,
                    ChordRhythm::Default => 1,
                    _ => 1,
                };
                cumulative_beats += chord_beats;
                seen_chord_count += 1;
            }
        }
        return found_beat_pos
            .unwrap_or(chord_idx)
            .min(segment_positions.len().saturating_sub(1));
    }

    if !segment_positions.is_empty() {
        let beat = chord.position.beats() as f64 + chord.position.subdivisions() as f64 / 1000.0;
        let beats_per_segment = f64::from(time_signature.0) / segment_positions.len() as f64;
        if beats_per_segment > 0.0 {
            return (beat / beats_per_segment)
                .round()
                .clamp(0.0, (segment_positions.len() - 1) as f64) as usize;
        }
    }

    // Simple measure - calculate segment from cumulative chord beats
    let mut cumulative_beats = 0usize;
    for (idx, c) in measure.chords.iter().enumerate() {
        if idx == chord_idx {
            break;
        }
        let chord_beats = match &c.rhythm {
            ChordRhythm::Slashes { count, .. } => *count as usize,
            ChordRhythm::Default => 1,
            _ => 1,
        };
        cumulative_beats += chord_beats;
    }
    cumulative_beats.min(segment_positions.len().saturating_sub(1))
}

/// Check if a measure has explicit chord rhythms (Lily or Rest notation).
fn measure_has_explicit_chord_rhythm(measure: &Measure) -> bool {
    super::rhythm_builder::measure_has_explicit_chord_rhythm(measure)
}

/// Check if a chord should be hidden due to being a duplicate.
#[must_use]
pub fn should_hide_chord(
    chord: &ChordInstance,
    current_symbol: &str,
    previous_symbol: Option<&str>,
    is_pushed_at_boundary: bool,
    time_signature: (u8, u8),
    hide_repeated_chords: bool,
) -> bool {
    if !hide_repeated_chords {
        return false;
    }

    // Short duration chords should always be shown (hits/stabs)
    let ts = TimeSignature::new(time_signature.0.into(), time_signature.1.into());
    let chord_beats = chord.duration.to_beats(ts);
    let is_short_duration = chord_beats <= 0.5;

    if is_short_duration {
        return false;
    }

    // Pushed chords at boundaries should show
    if is_pushed_at_boundary {
        return false;
    }

    // Check for duplicate
    previous_symbol == Some(current_symbol)
}

fn chord_lift_for_note_stack(note_stack: Option<(i32, i32)>, spatium: f64) -> f64 {
    let Some((_, max_line)) = note_stack else {
        return 0.0;
    };

    // Top staff line is +4. Notes above it need ledger space; notes within
    // the staff still get a graded lift so ascending melodies visibly nudge
    // chord symbols upward without returning to the old oversized offsets.
    let staff_lift = (max_line + 4).max(0) as f64 * spatium * 0.025;
    let ledger_lift = (max_line - 4).max(0) as f64 * spatium * 0.12;
    staff_lift + ledger_lift
}

fn normalize_chord_lifts(lifts: &[f64]) -> f64 {
    if lifts.len() < 2 || lifts.iter().any(|lift| *lift <= 0.0) {
        return 0.0;
    }

    lifts.iter().copied().fold(f64::INFINITY, f64::min)
}

fn chord_lift_bias_for_measure(
    ctx: &ChordRenderContext<'_>,
    measure: &Measure,
    is_boundary: bool,
) -> f64 {
    let lifts = measure
        .chords
        .iter()
        .enumerate()
        .filter_map(|(chord_idx, chord)| {
            if is_placeholder_chord(&chord.full_symbol) {
                return None;
            }

            let is_first_real = is_first_real_chord(&measure.chords, chord_idx);
            if chord
                .push_pull
                .as_ref()
                .is_some_and(|(is_push, _)| *is_push)
                && is_first_real
                && !is_boundary
            {
                return None;
            }

            let segment_idx = calculate_segment_index(
                measure,
                chord_idx,
                chord,
                ctx.segment_positions,
                ctx.internal_push_positions,
                is_first_real,
                is_boundary,
                ctx.time_signature,
            );
            Some(chord_lift_for_note_stack(
                ctx.note_line_stacks.get(segment_idx).copied().flatten(),
                ctx.spatium,
            ))
        })
        .collect::<Vec<_>>();

    normalize_chord_lifts(&lifts)
}

fn chord_symbol_anchor_offset(
    params: &crate::engraver::layout::tlayout::HarmonyParams,
    style: &HarmonyStyle,
    use_compact_anchor: bool,
) -> f64 {
    if params.root_accidental.is_empty() || !use_compact_anchor {
        return 0.0;
    }

    // In crowded chord runs, compact accidental roots like F#m7 and C#m can
    // reclaim a root-letter width by putting the accidental near the beat.
    // Isolated chords stay left-aligned to the beat.
    let root_width = params.root.chars().count() as f64 * style.root_size * 0.56;
    -root_width
}

fn has_nearby_following_visible_chord(
    measure: &Measure,
    chord_idx: usize,
    current_beat: f64,
    threshold_beats: f64,
) -> bool {
    measure
        .chords
        .iter()
        .skip(chord_idx + 1)
        .filter(|chord| !is_placeholder_chord(&chord.full_symbol))
        .map(chord_beat_position)
        .find(|beat| *beat > current_beat)
        .is_some_and(|beat| beat - current_beat <= threshold_beats)
}

fn chord_beat_position(chord: &ChordInstance) -> f64 {
    chord.position.beats() as f64 + chord.position.subdivisions() as f64 / 1000.0
}

/// Onset beat of the chord at `chord_idx` within its OWN measure, from the
/// cumulative duration of the chords before it. Drift-free, unlike
/// `position.beats()` (the absolute-timeline beat), which a short earlier
/// measure can shift across a barline.
fn chord_local_onset_beat(measure: &Measure, chord_idx: usize, time_signature: (u8, u8)) -> f64 {
    let ts = TimeSignature::new(time_signature.0.into(), time_signature.1.into());
    measure
        .chords
        .iter()
        .take(chord_idx)
        .map(|c| c.duration.to_beats(ts))
        .sum()
}

fn chord_x_from_local_beat(ctx: &ChordRenderContext<'_>, local_beat: f64) -> f64 {
    let measure_beats = f64::from(ctx.time_signature.0).max(1.0);
    let beat = local_beat.clamp(0.0, measure_beats);
    ctx.measure_x + ctx.measure_width * (beat / measure_beats)
}

/// Anchor x for a chord symbol.
///
/// Chord symbols sit over the rhythm slash (or notehead) that carries their
/// beat — not at the measure barline. We therefore anchor to the resolved
/// segment's spring-laid-out x whenever that segment faithfully represents the
/// chord's beat. Only when the chord's beat falls outside the available
/// segments — e.g. a chord on beat 4 over a single measure-long melody note —
/// do we fall back to the linear musical position so the symbol still lands at
/// its beat instead of collapsing onto an earlier segment's slash.
fn chord_render_x(
    ctx: &ChordRenderContext<'_>,
    segment_idx: usize,
    segment_x: f64,
    local_beat: f64,
) -> f64 {
    if segment_is_beat_faithful(ctx, segment_idx, local_beat) {
        ctx.measure_x + segment_x
    } else {
        chord_x_from_local_beat(ctx, local_beat)
    }
}

/// Whether the segment at `segment_idx` actually carries `chord`'s beat.
///
/// True when the segment's expected beat lands within half a segment of the
/// chord's beat (the normal slash/notehead case where segments map 1:1 to
/// beats). False when the chord's beat sits beyond the segments the measure
/// rendered (long-note / sparse-melody measures), so the caller uses the
/// linear musical position instead.
fn segment_is_beat_faithful(
    ctx: &ChordRenderContext<'_>,
    segment_idx: usize,
    local_beat: f64,
) -> bool {
    let seg_count = ctx.segment_positions.len();
    if seg_count == 0 {
        return false;
    }
    let measure_beats = f64::from(ctx.time_signature.0).max(1.0);
    let beats_per_segment = measure_beats / seg_count as f64;
    if beats_per_segment <= 0.0 {
        return false;
    }
    let chord_beat = local_beat.clamp(0.0, measure_beats);
    let segment_beat = segment_idx as f64 * beats_per_segment;
    (segment_beat - chord_beat).abs() < beats_per_segment / 2.0
}

/// Render chord symbols for a measure with automatic collision detection.
///
/// This handles all the complex logic for determining which chords to render,
/// where to position them, and what metadata to attach. After rendering,
/// it detects collisions between adjacent chord symbols and adjusts their
/// positions to maintain the minimum gap.
///
/// Note: `internal_push_positions` should already be included in `ctx`.
pub fn render_chord_symbols(
    ctx: &ChordRenderContext<'_>,
    measure: &Measure,
    previous_chord_symbol: Option<&str>,
    mut id_counter: u64,
    layout_ctx: &LayoutContext<'_>,
) -> ChordRenderResult {
    let mut nodes = Vec::new();
    let mut chord_bounds_info: Vec<ChordBoundsInfo> = Vec::new();
    let mut last_chord_symbol = previous_chord_symbol.map(String::from);

    let is_boundary = is_at_boundary(ctx.measure_idx, ctx.local_measure_idx);
    let is_hits = ctx.section_name.eq_ignore_ascii_case("hits");
    let mut hits_chord_shown = false;
    let note_lift_bias = chord_lift_bias_for_measure(ctx, measure, is_boundary);

    if ctx.measure_idx == 0 {
        tracing::debug!(
            "[chord-render-start] section={} measure={} is_boundary={} chord_count={} chords={:?}",
            ctx.section_name,
            ctx.measure_idx,
            is_boundary,
            measure.chords.len(),
            measure
                .chords
                .iter()
                .map(|c| (&c.full_symbol, c.push_pull.as_ref().map(|(p, _)| *p)))
                .collect::<Vec<_>>()
        );
    }

    for (chord_idx, chord) in measure.chords.iter().enumerate() {
        let current_symbol = &chord.full_symbol;

        // Skip placeholders
        if is_placeholder_chord(current_symbol) {
            continue;
        }

        let is_first_real = is_first_real_chord(&measure.chords, chord_idx);

        // Skip pushed chords that spill back (except at boundaries)
        if let Some((is_push, _)) = &chord.push_pull
            && *is_push
            && is_first_real
            && !is_boundary
        {
            continue;
        }

        // Check for pushed chord at boundary
        let is_pushed_at_boundary = chord
            .push_pull
            .as_ref()
            .is_some_and(|(is_push, _)| *is_push)
            && is_first_real
            && is_boundary;

        // Calculate segment index (needed for both chord rendering and accent markers)
        let segment_idx = calculate_segment_index(
            measure,
            chord_idx,
            chord,
            ctx.segment_positions,
            ctx.internal_push_positions,
            is_first_real,
            is_boundary,
            ctx.time_signature,
        );

        // Get segment x position for metadata/collision context. Chord symbols
        // themselves use their musical beat position below so a chord at beat 4
        // still has a beat-4 anchor even when the melody only has one long note
        // segment at beat 1.
        let segment_x = ctx
            .segment_positions
            .get(segment_idx)
            .copied()
            .unwrap_or_else(|| ctx.segment_positions.first().copied().unwrap_or(0.0));

        let local_beat = chord_local_onset_beat(measure, chord_idx, ctx.time_signature);
        let chord_x = chord_render_x(ctx, segment_idx, segment_x, local_beat);

        // Check if chord has regular accent (not AccentOnPush - that renders on spillback)
        let has_regular_accent = chord.commands.iter().any(|c| matches!(c, Command::Accent));
        let has_staccato = chord
            .commands
            .iter()
            .any(|c| matches!(c, Command::Staccato));

        // In Hits sections: show chord name only for the first chord, then just accents/staccatos
        let skip_chord_name = is_hits && hits_chord_shown;

        if skip_chord_name {
            // Hits: skip chord name but still render articulation markers
            last_chord_symbol = Some(current_symbol.clone());

            if has_regular_accent {
                let accent_node =
                    create_accent_marker(chord_x, ctx.chord_y, ctx.spatium, id_counter);
                id_counter += 1;
                nodes.push(accent_node);
            }
            if has_staccato {
                let staccato_node =
                    create_staccato_marker(chord_x, ctx.chord_y, ctx.spatium, id_counter);
                id_counter += 1;
                nodes.push(staccato_node);
            }
            // Stop signs in Hits: use a synthetic bounds around the beat position
            let synth_bounds = Rect::new(
                chord_x,
                ctx.chord_y - ctx.spatium * 1.5,
                chord_x + ctx.spatium * 2.0,
                ctx.chord_y,
            );
            for cmd in &chord.commands {
                if cmd.is_stop() {
                    let after = cmd.is_stop_after();
                    let node = if cmd.is_stop_sign() {
                        create_stop_marker(
                            synth_bounds,
                            ctx.chord_y,
                            ctx.spatium,
                            after,
                            id_counter,
                        )
                    } else {
                        create_stop_groove_marker(
                            synth_bounds,
                            ctx.chord_y,
                            ctx.spatium,
                            after,
                            id_counter,
                        )
                    };
                    id_counter += 1;
                    nodes.push(node);
                }
            }
            continue;
        }

        // Check if duplicate (skip for Hits — first-chord logic handles it)
        if !is_hits
            && should_hide_chord(
                chord,
                current_symbol,
                last_chord_symbol.as_deref(),
                is_pushed_at_boundary,
                ctx.time_signature,
                ctx.hide_repeated_chords,
            )
        {
            last_chord_symbol = Some(current_symbol.clone());
            continue;
        }

        // Update tracker
        last_chord_symbol = Some(current_symbol.clone());
        if is_hits {
            hits_chord_shown = true;
        }

        {
            let is_pushed = chord
                .push_pull
                .as_ref()
                .is_some_and(|(is_push, _)| *is_push);
            tracing::debug!(
                "[chord-render] section={} measure={} chord_idx={} '{}' is_pushed={} is_first_real={} is_boundary={} is_pushed_at_boundary={} segment_idx={} internal_push_positions={:?}",
                ctx.section_name,
                ctx.measure_idx,
                chord_idx,
                current_symbol,
                is_pushed,
                is_first_real,
                is_boundary,
                is_pushed_at_boundary,
                segment_idx,
                ctx.internal_push_positions
            );
        }

        let note_lift = (chord_lift_for_note_stack(
            ctx.note_line_stacks.get(segment_idx).copied().flatten(),
            ctx.spatium,
        ) - note_lift_bias)
            .max(0.0);
        let chord_y_offset = if has_regular_accent || has_staccato {
            // Move chord up by 0.5 spatium to make room for articulation below
            -ctx.spatium * 0.5
        } else {
            0.0
        };

        // Create harmony params
        let mut params = super::chord_layout::chord_to_harmony_params(chord, ctx.harmony_style);
        // Vertical-bass slash chords stack root / rule / bass top-to-bottom,
        // so the rendered glyph extends roughly one root-size below its
        // baseline. Lift the baseline enough for staff clearance, but keep
        // stacked slash chords in the chord lane instead of letting them float
        // into the previous system.
        let vertical_bass_lift = if chord.parsed.bass_vertical {
            ctx.harmony_style.root_size * 0.65
        } else {
            0.0
        };
        let compact_anchor = has_nearby_following_visible_chord(
            measure,
            chord_idx,
            chord_beat_position(chord),
            1.25,
        );
        let chord_x_offset = chord_symbol_anchor_offset(&params, ctx.harmony_style, compact_anchor);
        tracing::debug!(
            target: "engraver_proto::engraver::layout::chart::chord",
            measure = ctx.measure_idx,
            chord_idx,
            symbol = current_symbol,
            beat_x = chord_x,
            chord_x_offset,
            compact_anchor,
            note_lift,
            note_lift_bias,
            vertical_bass_lift,
            "[chord-placement] anchor adjustment"
        );
        params.position = kurbo::Point::new(
            chord_x + chord_x_offset,
            ctx.chord_y + chord_y_offset - vertical_bass_lift - note_lift,
        );
        params.id = id_counter;
        id_counter += 1;

        let (layout_data, mut chord_node) = layout_harmony(&params, layout_ctx);

        // Store bounds info for collision detection
        // layout_harmony returns bounds already in world coordinates (includes params.position)
        // layout_harmony's bounds are layout extents. Pad them slightly for
        // collision resolution so adjacent chord ink never kisses/overlaps
        // when glyph metrics are optimistic for jazz/SMuFL text runs.
        let collision_bounds = Rect::new(
            layout_data.bounds.x0 - ctx.spatium * 0.5,
            layout_data.bounds.y0,
            layout_data.bounds.x1 + ctx.spatium * 0.5,
            layout_data.bounds.y1,
        );
        chord_bounds_info.push(ChordBoundsInfo {
            node_idx: nodes.len(),
            original_x: chord_x,
            world_bounds: collision_bounds,
        });

        // Add metadata
        if let Some(page) = ctx.page_number {
            chord_node.set_page(page);
        }
        chord_node.set_system(ctx.global_system_index as u32);
        chord_node.set_measure(ctx.measure_idx as u32);
        chord_node.set_beat(segment_idx as u32);
        chord_node.set_element_type("chord");
        chord_node.set_section_type(ctx.section_name);

        // Chart position for musical coordinates
        let chart_pos = ChartPosition::new(
            ctx.global_system_index as u32,
            ctx.measure_idx as u32,
            chord_idx as u32,
        );
        chord_node.set_json_metadata(metadata_keys::CHART_POSITION, &chart_pos);

        // Source span for click-to-highlight
        if let Some(ref span) = chord.source_span {
            chord_node.set_json_metadata(metadata_keys::SOURCE_SPAN, span);
            let source_link = SourceLink::new(*span, chart_pos.clone());
            chord_node.set_json_metadata(metadata_keys::SOURCE_LINK, &source_link);
        }

        nodes.push(chord_node);

        // Add apostrophe marker for pushed/pulled chords when push_alters_rhythm=false
        if !ctx.push_alters_rhythm
            && let Some((is_push, _amount)) = &chord.push_pull
        {
            let marker_node = create_push_marker(
                *is_push,
                layout_data.bounds,
                ctx.chord_y,
                ctx.spatium,
                id_counter,
            );
            id_counter += 1;
            nodes.push(marker_node);
        }

        // Add accent marker for regular accents (rendered in red above the chord)
        // AccentOnPush accents are rendered on the spillback chord instead
        if has_regular_accent {
            let accent_node = create_accent_marker(chord_x, ctx.chord_y, ctx.spatium, id_counter);
            id_counter += 1;
            nodes.push(accent_node);
        }

        // Add staccato marker (dot above the rhythm slash notehead)
        if has_staccato {
            let staccato_node =
                create_staccato_marker(chord_x, ctx.chord_y, ctx.spatium, id_counter);
            id_counter += 1;
            nodes.push(staccato_node);
        }

        // Add stop sign / stop groove markers inline with chord symbols
        for cmd in &chord.commands {
            if cmd.is_stop() {
                let after = cmd.is_stop_after();
                let node = if cmd.is_stop_sign() {
                    create_stop_marker(
                        layout_data.bounds,
                        ctx.chord_y,
                        ctx.spatium,
                        after,
                        id_counter,
                    )
                } else {
                    create_stop_groove_marker(
                        layout_data.bounds,
                        ctx.chord_y,
                        ctx.spatium,
                        after,
                        id_counter,
                    )
                };
                id_counter += 1;
                nodes.push(node);
            }
        }
    }

    // Perform collision detection and resolution if enabled
    if !chord_bounds_info.is_empty() {
        tracing::debug!(
            "[chord-collision] measure={} chords={} min_gap={:.1} bounds: {:?}",
            ctx.measure_idx,
            chord_bounds_info.len(),
            ctx.min_chord_symbol_gap,
            chord_bounds_info
                .iter()
                .map(|b| (b.original_x, b.world_bounds.x0, b.world_bounds.x1))
                .collect::<Vec<_>>()
        );
    }

    let mut chord_bound_adjustments = vec![0.0; chord_bounds_info.len()];

    if ctx.min_chord_symbol_gap > 0.0 && chord_bounds_info.len() >= 2 {
        let collision_result = resolve_chord_collisions(
            &chord_bounds_info,
            ctx.min_chord_symbol_gap,
            ctx.measure_x,
            ctx.measure_x + ctx.measure_width,
        );

        tracing::debug!(
            "[chord-collision] had_collisions={} adjustments={:?}",
            collision_result.had_collisions,
            collision_result.adjustments
        );

        if collision_result.had_collisions {
            chord_bound_adjustments = collision_result.adjustments.clone();
            // Apply adjustments to nodes
            for (bounds_info, &adjustment) in chord_bounds_info
                .iter()
                .zip(collision_result.adjustments.iter())
            {
                if adjustment.abs() > 0.001 {
                    let node = &mut nodes[bounds_info.node_idx];
                    node.transform = node.transform.then_translate(Vec2::new(adjustment, 0.0));
                }
            }
        }
    }

    let chord_bounds = chord_bounds_info
        .iter()
        .zip(chord_bound_adjustments.iter())
        .map(|(bounds_info, adjustment)| {
            let bounds = bounds_info.world_bounds;
            Rect::new(
                bounds.x0 + *adjustment,
                bounds.y0,
                bounds.x1 + *adjustment,
                bounds.y1,
            )
        })
        .collect();

    ChordRenderResult {
        nodes,
        chord_bounds,
        last_chord_symbol,
        next_id: id_counter,
    }
}

/// Render spillback chord symbols (from next measure pushing back).
pub fn render_spillback_chords(
    ctx: &ChordRenderContext<'_>,
    spillbacks: &[PushSpillback],
    previous_chord_symbol: Option<&str>,
    mut id_counter: u64,
    layout_ctx: &LayoutContext<'_>,
) -> ChordRenderResult {
    let mut nodes = Vec::new();
    let mut last_chord_symbol = previous_chord_symbol.map(String::from);

    for spillback in spillbacks {
        // Look up the correct segment index from spillback_positions computed by rhythm builder.
        // When push_alters_rhythm is enabled and we have a triplet push, the spillback chord
        // goes on the triplet eighth (e.g., segment 4 in [Q,Q,Q,TQ,TE]) not just the last segment.
        let segment_idx = ctx
            .spillback_positions
            .iter()
            .find(|(_, symbol)| symbol == &spillback.chord_symbol)
            .map(|(idx, _)| *idx)
            .unwrap_or_else(|| ctx.segment_positions.len().saturating_sub(1));

        tracing::debug!(
            "[spillback-render] section={} measure={} '{}' beat_pos={} segment_idx={} positions_len={} segment_x={:.2} spillback_positions={:?}",
            ctx.section_name,
            ctx.measure_idx,
            spillback.chord_symbol,
            spillback.beat_position,
            segment_idx,
            ctx.segment_positions.len(),
            ctx.segment_positions
                .get(segment_idx)
                .copied()
                .unwrap_or(0.0),
            ctx.spillback_positions
        );

        let segment_x = ctx
            .segment_positions
            .get(segment_idx)
            .copied()
            .unwrap_or_else(|| ctx.segment_positions.last().copied().unwrap_or(0.0));

        let chord_x = ctx.measure_x + segment_x;

        // Offset chord Y if this spillback has an accent (AccentOnPush)
        let chord_y_offset = if spillback.has_accent {
            -ctx.spatium * 0.5
        } else {
            0.0
        };

        let mut params = parse_chord(&spillback.chord_symbol);
        params.style = ctx.harmony_style.clone();
        params.position = kurbo::Point::new(chord_x, ctx.chord_y + chord_y_offset);
        params.id = id_counter;
        id_counter += 1;

        let (_, mut spillback_node) = layout_harmony(&params, layout_ctx);

        // Add metadata
        if let Some(page) = ctx.page_number {
            spillback_node.set_page(page);
        }
        spillback_node.set_system(ctx.global_system_index as u32);
        spillback_node.set_measure(ctx.measure_idx as u32);
        spillback_node.set_element_type("chord");
        spillback_node
            .metadata
            .insert("spillback".to_string(), "true".to_string());
        spillback_node.set_section_type(ctx.section_name);

        // Update tracker for duplicate detection
        last_chord_symbol = Some(spillback.chord_symbol.clone());

        nodes.push(spillback_node);

        // Render accent if this spillback chord has AccentOnPush (>' syntax)
        // The accent appears on the pushed beat (previous measure)
        if spillback.has_accent {
            let accent_node = create_accent_marker(chord_x, ctx.chord_y, ctx.spatium, id_counter);
            id_counter += 1;
            nodes.push(accent_node);
        }
    }

    ChordRenderResult {
        nodes,
        chord_bounds: Vec::new(),
        last_chord_symbol,
        next_id: id_counter,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chord::{Chord, ChordQuality, ChordRhythm};
    use crate::primitives::RootNotation;
    use crate::time::{AbsolutePosition, MusicalDuration, MusicalPosition};

    fn chord_at(symbol: &str, beat: u32, subdivisions: u32) -> ChordInstance {
        let root = RootNotation::from_string("F").expect("test root should parse");
        ChordInstance::new(
            root.clone(),
            symbol.to_string(),
            Chord::new(root, ChordQuality::Major),
            ChordRhythm::Default,
            symbol.to_string(),
            MusicalDuration::new(0, 1, 0),
            AbsolutePosition::new(
                MusicalPosition::try_new(0, beat as i32, subdivisions as i32).unwrap(),
                0,
            ),
        )
    }

    fn chord_at_chart_beat(symbol: &str, beat: u32, subdivisions: u32) -> ChordInstance {
        assert!(beat >= 1, "chart beats are one-based");
        chord_at(symbol, beat - 1, subdivisions)
    }

    #[test]
    fn test_is_placeholder_chord() {
        assert!(is_placeholder_chord(""));
        assert!(is_placeholder_chord("s"));
        assert!(is_placeholder_chord("r"));
        assert!(!is_placeholder_chord("C"));
        assert!(!is_placeholder_chord("Am7"));
    }

    #[test]
    fn positive_short_duration_repeated_chord_still_shows_as_hit() {
        let mut chord = chord_at_chart_beat("F#m7", 1, 0);
        chord.duration = MusicalDuration::new(0, 0, 500);

        assert!(
            !should_hide_chord(&chord, "F#m7", Some("F#m7"), false, (6, 8), true,),
            "positive short-duration hits should stay visible even when repeated"
        );
    }

    #[test]
    fn test_is_at_boundary() {
        // First measure of section
        assert!(is_at_boundary(0, 0));
        assert!(is_at_boundary(0, 1));

        // First measure of system (not section)
        assert!(is_at_boundary(5, 0));

        // Neither
        assert!(!is_at_boundary(5, 2));
    }

    #[test]
    fn chord_segment_index_uses_musical_position_for_compound_meter() {
        let segment_positions = vec![0.0, 60.0, 120.0, 180.0];
        let measure = Measure {
            time_signature: (6, 8),
            chords: vec![
                chord_at("F#m7", 0, 0),
                chord_at("G#m7", 1, 500),
                chord_at("Amaj7", 3, 0),
                chord_at("B", 4, 500),
            ],
            ..Default::default()
        };

        let indices: Vec<usize> = measure
            .chords
            .iter()
            .enumerate()
            .map(|(idx, chord)| {
                calculate_segment_index(
                    &measure,
                    idx,
                    chord,
                    &segment_positions,
                    &[],
                    idx == 0,
                    false,
                    measure.time_signature,
                )
            })
            .collect();

        assert_eq!(indices, vec![0, 1, 2, 3]);
    }

    #[test]
    fn chord_lift_increases_with_ascending_note_stack() {
        let spatium = 5.0;
        let first = chord_lift_for_note_stack(Some((-5, 6)), spatium);
        let second = chord_lift_for_note_stack(Some((-4, 8)), spatium);
        let third = chord_lift_for_note_stack(Some((-3, 10)), spatium);
        let fourth = chord_lift_for_note_stack(Some((-2, 12)), spatium);

        assert!(second > first, "second chord should lift above first");
        assert!(third > second, "third chord should lift above second");
        assert!(fourth > third, "fourth chord should lift above third");

        let minimum_visible_step = spatium * 0.25;
        assert!(
            second - first >= minimum_visible_step,
            "second chord lift should be visibly higher than first; got delta {}",
            second - first
        );
        assert!(
            third - second >= minimum_visible_step,
            "third chord lift should be visibly higher than second; got delta {}",
            third - second
        );
        assert!(
            fourth - third >= minimum_visible_step,
            "fourth chord lift should be visibly higher than third; got delta {}",
            fourth - third
        );
        assert!(
            fourth - first >= spatium * 0.8,
            "ascending LOTF chord run should have a visible total staircase; got spread {}",
            fourth - first
        );
        assert!(
            fourth < spatium * 1.4,
            "note-driven chord lift should stay subtle; got {fourth}"
        );
    }

    #[test]
    fn chord_lift_normalization_keeps_first_ascending_chord_low() {
        let spatium = 5.0;
        let raw_lifts = [
            chord_lift_for_note_stack(Some((-5, 6)), spatium),
            chord_lift_for_note_stack(Some((-4, 8)), spatium),
            chord_lift_for_note_stack(Some((-3, 10)), spatium),
            chord_lift_for_note_stack(Some((-2, 12)), spatium),
        ];

        let bias = normalize_chord_lifts(&raw_lifts);
        let normalized = raw_lifts.map(|lift| (lift - bias).max(0.0));

        assert_eq!(
            normalized[0], 0.0,
            "first chord in an ascending run should remain at the default height"
        );
        assert!(
            normalized[1] < raw_lifts[1],
            "second chord should be stair-stepped relative to the run, not globally raised"
        );
        assert!(
            normalized[1] > normalized[0]
                && normalized[2] > normalized[1]
                && normalized[3] > normalized[2],
            "normalized lifts should preserve the ascending staircase"
        );
        assert!(
            normalized[3] - normalized[0] >= spatium * 0.8,
            "normalized staircase should remain visible; got spread {}",
            normalized[3] - normalized[0]
        );
    }

    #[test]
    fn chord_anchor_keeps_single_letter_roots_beat_left_aligned() {
        let style = HarmonyStyle::default().with_root_size(20.0);

        let b = parse_chord("B");
        assert_eq!(chord_symbol_anchor_offset(&b, &style, false), 0.0);

        let a_maj7 = parse_chord("Amaj7");
        assert_eq!(chord_symbol_anchor_offset(&a_maj7, &style, false), 0.0);
    }

    #[test]
    fn chord_anchor_keeps_isolated_accidental_roots_left_aligned() {
        let style = HarmonyStyle::default().with_root_size(20.0);
        let f_sharp_min = parse_chord("F#m7");

        assert_eq!(chord_symbol_anchor_offset(&f_sharp_min, &style, false), 0.0);
    }

    #[test]
    fn chord_anchor_places_accidental_near_beat_for_crowded_compact_accidental_roots() {
        let style = HarmonyStyle::default().with_root_size(20.0);
        let f_sharp_min = parse_chord("F#m7");
        let offset = chord_symbol_anchor_offset(&f_sharp_min, &style, true);

        assert!(
            offset < -10.0,
            "F#m7 should shift left so the # aligns near the beat; got {offset}"
        );
        assert!(
            offset > -13.0,
            "F#m7 anchor shift should be root-letter sized, not root+accidental+quality sized; got {offset}"
        );

        let c_sharp_min = parse_chord("C#m");
        assert_eq!(
            chord_symbol_anchor_offset(&c_sharp_min, &style, true),
            offset
        );
    }

    #[test]
    fn nearby_following_chord_enables_compact_anchor() {
        let measure = Measure {
            chords: vec![
                chord_at_chart_beat("C#m", 1, 0),
                chord_at_chart_beat("B", 2, 0),
            ],
            ..Default::default()
        };

        assert!(has_nearby_following_visible_chord(
            &measure,
            0,
            chord_beat_position(&measure.chords[0]),
            1.25
        ));
    }

    #[test]
    fn distant_following_chord_keeps_left_anchor() {
        let measure = Measure {
            chords: vec![
                chord_at_chart_beat("C#m", 1, 0),
                chord_at_chart_beat("B", 4, 0),
            ],
            ..Default::default()
        };

        assert!(!has_nearby_following_visible_chord(
            &measure,
            0,
            chord_beat_position(&measure.chords[0]),
            1.25
        ));
    }

    #[test]
    fn lotf_measure_7_chords_use_musical_beat_positions() {
        let segment_positions = vec![0.0, 120.0];
        let measure = Measure {
            time_signature: (6, 8),
            chords: vec![
                chord_at_chart_beat("C#m", 1, 0),
                chord_at_chart_beat("B/C#", 4, 0),
            ],
            ..Default::default()
        };

        let c_sharp_minor_idx = calculate_segment_index(
            &measure,
            0,
            &measure.chords[0],
            &segment_positions,
            &[],
            true,
            false,
            measure.time_signature,
        );
        let b_over_c_sharp_idx = calculate_segment_index(
            &measure,
            1,
            &measure.chords[1],
            &segment_positions,
            &[],
            false,
            false,
            measure.time_signature,
        );

        assert_eq!(c_sharp_minor_idx, 0, "C#m at 7.1 should align to beat 1");
        assert_eq!(
            b_over_c_sharp_idx, 1,
            "B/C# at 7.4 should align to the second dotted-half segment"
        );
    }

    #[test]
    fn lotf_measure_7_chord_x_uses_beat_4_even_without_melody_segment() {
        let style = HarmonyStyle::default();
        let segment_positions = vec![0.0];
        let ctx = ChordRenderContext {
            measure_x: 200.0,
            measure_width: 180.0,
            chord_y: 40.0,
            page_number: None,
            global_system_index: 0,
            measure_idx: 7,
            local_measure_idx: 0,
            section_name: "Verse",
            segment_positions: &segment_positions,
            internal_push_positions: &[],
            harmony_style: &style,
            time_signature: (6, 8),
            hide_repeated_chords: false,
            min_chord_symbol_gap: 0.0,
            push_alters_rhythm: false,
            spatium: 5.0,
            measure_measurements: None,
            spillback_positions: &[],
            note_line_stacks: &[],
        };

        // Beat 1 → local onset 0; beat 4 in 6/8 → local onset 3 (half the bar).
        let c_sharp_minor_x = chord_x_from_local_beat(&ctx, 0.0);
        let b_over_c_sharp_x = chord_x_from_local_beat(&ctx, 3.0);

        assert_eq!(c_sharp_minor_x, 200.0);
        assert_eq!(b_over_c_sharp_x, 290.0);
        assert!(
            b_over_c_sharp_x > c_sharp_minor_x + ctx.measure_width * 0.45,
            "B/C# at beat 4 should not collapse onto the C#m beat-1 anchor"
        );
    }

    // ==========================================================================
    // Collision Detection Tests
    // ==========================================================================

    fn make_bounds(x0: f64, width: f64) -> ChordBoundsInfo {
        ChordBoundsInfo {
            node_idx: 0,
            original_x: x0,
            world_bounds: Rect::new(x0, 0.0, x0 + width, 20.0),
        }
    }

    #[test]
    fn test_collision_no_chords() {
        let bounds: Vec<ChordBoundsInfo> = vec![];
        let result = resolve_chord_collisions(&bounds, 4.0, 0.0, 200.0);
        assert!(!result.had_collisions);
        assert!(result.adjustments.is_empty());
    }

    #[test]
    fn test_collision_single_chord() {
        let bounds = vec![make_bounds(10.0, 30.0)];
        let result = resolve_chord_collisions(&bounds, 4.0, 0.0, 200.0);
        assert!(!result.had_collisions);
        assert_eq!(result.adjustments.len(), 1);
        assert!((result.adjustments[0]).abs() < 0.001);
    }

    #[test]
    fn test_collision_no_overlap() {
        // Two chords with enough space between them
        let bounds = vec![
            make_bounds(10.0, 30.0), // ends at 40
            make_bounds(50.0, 30.0), // starts at 50, gap of 10
        ];
        let result = resolve_chord_collisions(&bounds, 4.0, 0.0, 200.0);
        assert!(!result.had_collisions);
        assert!((result.adjustments[0]).abs() < 0.001);
        assert!((result.adjustments[1]).abs() < 0.001);
    }

    #[test]
    fn test_collision_detected_and_resolved() {
        // Two chords that overlap (gap of 2, need 4)
        let bounds = vec![
            make_bounds(10.0, 30.0), // ends at 40
            make_bounds(42.0, 30.0), // starts at 42, gap of only 2
        ];
        let result = resolve_chord_collisions(&bounds, 4.0, 0.0, 200.0);
        assert!(result.had_collisions);
        // First chord should shift left by the full overlap amount
        assert!(result.adjustments[0] < 0.0);
        // Second chord should NOT move (notehead position is correct via segment widths)
        assert!((result.adjustments[1]).abs() < 0.001);
        // First chord's adjustment should be approximately the overlap amount (2)
        assert!((result.adjustments[0] + 2.0).abs() < 0.1);
    }

    #[test]
    fn test_collision_allows_past_clef() {
        // Chord at very start of measure CAN shift left past the clef
        // This is intentional - the first chord can move past the clef position
        let bounds = vec![
            make_bounds(0.0, 30.0),  // ends at 30, at measure start
            make_bounds(32.0, 30.0), // starts at 32, gap of only 2
        ];
        let result = resolve_chord_collisions(&bounds, 4.0, 0.0, 200.0);
        assert!(result.had_collisions);
        // First chord moves left by full overlap (even past measure start)
        assert!(result.adjustments[0] < 0.0);
        // Second chord does NOT move (notehead position is correct)
        assert!((result.adjustments[1]).abs() < 0.001);
        // First chord moves left by full overlap amount (2.0)
        assert!((result.adjustments[0] + 2.0).abs() < 0.1);
    }

    #[test]
    fn test_collision_three_chords_cascade() {
        // Three overlapping chords - each first chord of a pair moves left
        let bounds = vec![
            make_bounds(10.0, 30.0), // ends at 40
            make_bounds(42.0, 30.0), // starts at 42, ends at 72 (gap 2)
            make_bounds(74.0, 30.0), // starts at 74 (gap 2)
        ];
        let result = resolve_chord_collisions(&bounds, 4.0, 0.0, 200.0);
        assert!(result.had_collisions);
        // First chord shifts left for first pair collision
        assert!(result.adjustments[0] < 0.0);
        // Middle chord shifts left for second pair collision
        assert!(result.adjustments[1] < 0.0);
        // Last chord should not move (no chord after it)
        assert!((result.adjustments[2]).abs() < 0.001);
    }
}
