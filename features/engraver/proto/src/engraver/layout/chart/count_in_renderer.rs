//! Count-in mini-snippet renderer.
//!
//! Renders a scaled-down count-in display for the header area, positioned next to
//! the tempo indicator. This replaces inline count-in measures on the first system.
//!
//! The snippet uses the same proportions as normal staffs, scaled down uniformly:
//! - 5 staff lines at scaled spatium spacing (half thickness for readability)
//! - Barlines at start, between measures, and at end
//! - Properly sized slash noteheads using SMuFL glyphs
//! - Beat numbers below slashes (1, 2, 3, 4)

use super::page_rendering::draw_staff_lines_with_thickness;
use super::types::slash_glyph_for_ticks;
use crate::engraver::layout::context::LayoutContextOwned;
use crate::engraver::layout::tlayout::{TimeSigParams, TimeSigType, layout_timesig};
use crate::engraver::notation::{Duration, MeasureBuilder, RhythmEntry};
use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::{FontStyle, FontWeight, PaintCommand, TextAnchor};
use crate::engraver::style::{MStyle, Sid, StyleValue};
use kurbo::{Affine, Point};
use peniko::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CountInPulse {
    beat_index: usize,
    duration_ticks: i32,
    dotted: bool,
    label: Option<&'static str>,
}

/// Configuration for count-in snippet rendering.
#[derive(Debug, Clone)]
pub struct CountInSnippetConfig {
    /// Number of beats per measure (e.g., 4 for 4/4 time).
    pub beats_per_measure: u8,
    /// Beat unit (denominator, e.g., 4 for quarter note).
    pub beat_unit: u8,
    /// Number of count-in measures (typically 1 or 2).
    pub num_measures: usize,
    /// Base spatium from the main chart (staff space in points).
    pub spatium: f64,
    /// Scale factor for the snippet (e.g., 0.6 for 60% size).
    /// Applied uniformly to all dimensions to maintain proportions.
    pub scale: f64,
    /// Per-measure labels to render above each measure (e.g. `["1","2"]`).
    /// Empty means no labels are drawn (preserves old behavior).
    pub measure_numbers: Vec<String>,
}

impl Default for CountInSnippetConfig {
    fn default() -> Self {
        Self {
            beats_per_measure: 4,
            beat_unit: 4,
            num_measures: 1,
            spatium: 5.0,
            scale: 0.6, // 60% of normal size
            measure_numbers: Vec::new(),
        }
    }
}

/// Geometry of a single count-in beat, for creating `BeatPosition` entries.
#[derive(Debug, Clone)]
pub struct CountInBeatGeometry {
    /// X position of this beat (in page coordinates).
    pub x: f64,
    /// Width of this beat slot.
    pub width: f64,
    /// Y position of the staff top.
    pub staff_y: f64,
    /// Staff height.
    pub staff_height: f64,
    /// Measure index within the count-in (0-indexed).
    pub measure_index: usize,
    /// Beat index within the measure (0-indexed).
    pub beat_index: usize,
    /// SMuFL glyph codepoint for the slash notehead.
    pub glyph_codepoint: char,
    /// Glyph size in spatiums (scaled).
    pub glyph_size: f64,
    /// Glyph Y center position.
    pub glyph_y: f64,
}

/// Result of rendering a count-in snippet.
#[derive(Debug)]
pub struct CountInSnippetResult {
    /// The rendered scene node.
    pub node: SceneNode,
    /// Total width consumed by the snippet.
    pub width: f64,
    /// Total height of the snippet.
    pub height: f64,
    /// Beat geometry for each beat in the count-in, for cursor highlighting.
    pub beat_geometries: Vec<CountInBeatGeometry>,
}

/// Render a count-in snippet at the given position.
///
/// Uses the same proportions as normal staff rendering, uniformly scaled:
/// - All dimensions derived from `spatium * scale`
/// - Maintains correct glyph-to-staff relationships
/// - Slash codepoints from `slash_glyph_for_ticks()`
/// - Barlines at start, between measures, and at end
/// - Staff lines at half thickness for better readability at small sizes
///
/// # Arguments
/// * `config` - Configuration for the count-in snippet
/// * `position` - Top-left position for the snippet
///
/// # Returns
/// The rendered scene node with width/height information.
#[must_use]
pub fn render_count_in_snippet(
    config: &CountInSnippetConfig,
    position: Point,
) -> CountInSnippetResult {
    let mut commands = Vec::new();

    // Scale the spatium - this ensures all proportions remain correct
    let spatium = config.spatium * config.scale;

    // Staff line thickness: use standard ratio with base spatium
    let staff_line_thickness = config.spatium * 0.05;

    // Barline thickness: same base value as main sheet (0.5), scaled proportionally
    // This keeps the same visual weight relative to the scaled staff
    let barline_thickness = 0.5 * config.scale;

    // Layout constants - same proportions as main chart, using scaled spatium
    let beat_spacing = spatium * 4.0; // Space between beat centers
    let staff_height = spatium * 4.0; // 5 lines, 4 spaces
    let beat_number_offset = spatium * 4.4; // Below staff, clear of beamed count notes

    // Measure layout: beats positioned with padding at measure boundaries
    // Slash glyphs are wide diagonal shapes that extend significantly to the right,
    // so we need substantial end padding to prevent touching the barline
    let notation_start_inset = spatium * 3.2; // Time signature column + air before beat 1.
    let start_padding = beat_spacing * 1.2; // Padding before first beat and time signature
    let end_padding = beat_spacing * 1.0; // Padding after last beat
    let beats_content_width = beat_spacing * (config.beats_per_measure - 1) as f64;
    let measure_width = start_padding + beats_content_width + end_padding;

    // Total width: all measures side by side
    let staff_width = measure_width * config.num_measures as f64;
    // Label height + padding + staff + beat numbers below
    let label_height = spatium * 1.8 + spatium * 0.5; // label_font_size + label_padding
    let total_height = label_height + staff_height + beat_number_offset + spatium * 1.5;

    // Font size for beat numbers - proportional to scaled spatium
    let beat_number_font_size = spatium * 2.0;

    // Font size for "Count-In" label - small text above the staff
    let label_font_size = spatium * 1.8;
    let label_padding = spatium * 0.5; // Space between label and staff

    // Position calculations
    // Leave room above for the "Count-In" label
    let label_y = position.y + label_font_size;
    let staff_top_y = label_y + label_padding;
    let staff_center_y = staff_top_y + staff_height / 2.0; // Center line of staff
    let close_beat_number_y = staff_top_y + staff_height + beat_number_font_size;
    let low_beat_number_y = staff_top_y + staff_height + beat_number_font_size + spatium * 2.4;
    let staff_start_x = position.x;

    // Draw 5 staff lines
    commands.extend(draw_staff_lines_with_thickness(
        staff_start_x,
        staff_top_y,
        staff_width,
        spatium,
        staff_line_thickness,
    ));

    // Draw barlines at measure boundaries: start, between measures, and end
    // Uses the same rendering approach as draw_barline() in page_rendering.rs,
    // but with thickness scaled to match the snippet scale
    // Barline positions: 0, measure_width, 2*measure_width, ..., num_measures*measure_width
    for i in 0..=config.num_measures {
        let barline_x = staff_start_x + measure_width * i as f64;
        commands.push(PaintCommand::line(
            Point::new(barline_x, staff_top_y),
            Point::new(barline_x, staff_top_y + staff_height),
            Color::BLACK,
            barline_thickness,
        ));
    }

    // Collect beat geometry for cursor highlighting
    let mut beat_geometries = Vec::new();
    let mut notation_style = MStyle::default();
    notation_style.set(Sid::Spatium, StyleValue::Real(spatium as f32));
    let notation_context = LayoutContextOwned::new_minimal(notation_style);
    let notation_ctx = notation_context.as_context();
    let mut notation_children = Vec::new();

    // Mini time signature at the start of the count-in staff. Use the same
    // SMuFL path as regular system prefixes so it matches main notation.
    let time_sig_params = TimeSigParams {
        id: 9_900,
        sig_type: TimeSigType::Numeric {
            numerator: config.beats_per_measure,
            denominator: config.beat_unit,
        },
        ..Default::default()
    };
    let (_, mut time_sig_node) = layout_timesig(&time_sig_params, &notation_ctx);
    time_sig_node.transform = Affine::translate((
        staff_start_x + spatium * 0.85,
        staff_top_y + staff_height / 2.0,
    ));
    notation_children.push(time_sig_node);

    // Per-measure labels above the staff. Align to the top-left of each
    // measure; the first label carries the Count-In text after the number.
    for i in 0..config.num_measures {
        let measure_label = config
            .measure_numbers
            .get(i)
            .filter(|label| !label.is_empty())
            .cloned()
            .unwrap_or_else(|| (i + 1).to_string());
        let text = if i == 0 {
            format!("{measure_label} Count-In")
        } else {
            measure_label
        };
        let label_x = staff_start_x + measure_width * i as f64 + spatium * 0.35;
        commands.push(PaintCommand::Text {
            text,
            font_family: "FreeSans".to_string(),
            font_size: label_font_size,
            position: Point::new(label_x, label_y),
            color: Color::BLACK,
            anchor: TextAnchor::Start,
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        });
    }

    // Render count-in notation using the same measure/rhythm path as normal
    // measures. The header still owns staff-line/barline thickness and labels.
    for measure_idx in 0..config.num_measures {
        let measure_start_x = staff_start_x + measure_width * measure_idx as f64;
        let notation_x = measure_start_x + notation_start_inset;
        let notation_width = (measure_width - notation_start_inset - spatium).max(spatium * 6.0);
        let measure_scene =
            count_in_measure_scene(config, measure_idx, notation_width, &notation_ctx);
        let chord_positions = measure_scene
            .segments
            .iter()
            .filter(|segment| segment.seg_type.is_chord_rest())
            .map(|segment| notation_x + segment.x)
            .collect::<Vec<_>>();

        let mut notation_node = measure_scene.scene;
        notation_node.transform = Affine::translate((notation_x, staff_center_y));
        notation_children.push(notation_node);

        for (pulse_idx, pulse) in count_in_pulses_for_measure(config, measure_idx)
            .into_iter()
            .enumerate()
        {
            let beat_number_y = if count_in_measure_needs_low_labels(config, measure_idx) {
                low_beat_number_y
            } else {
                close_beat_number_y
            };
            let beat_x = chord_positions
                .get(pulse_idx)
                .copied()
                .unwrap_or(notation_x + start_padding + beat_spacing * pulse.beat_index as f64);
            let slash_codepoint = slash_glyph_for_ticks(pulse.duration_ticks);

            // Record beat geometry for cursor positioning
            beat_geometries.push(CountInBeatGeometry {
                x: beat_x,
                width: beat_spacing,
                staff_y: staff_top_y,
                staff_height,
                measure_index: measure_idx,
                beat_index: pulse.beat_index,
                glyph_codepoint: slash_codepoint,
                glyph_size: spatium,
                glyph_y: staff_center_y,
            });

            if let Some(text) = pulse.label {
                commands.push(PaintCommand::Text {
                    text: text.to_string(),
                    font_family: "FreeSans".to_string(),
                    font_size: beat_number_font_size,
                    position: Point::new(beat_x, beat_number_y),
                    color: Color::BLACK,
                    anchor: TextAnchor::Middle,
                    weight: FontWeight::Normal,
                    style: FontStyle::Normal,
                });
            }
        }
    }

    let mut node = SceneNode::anonymous_leaf(commands);
    for child in notation_children {
        node.add_child(child);
    }

    CountInSnippetResult {
        node,
        width: staff_width,
        height: total_height,
        beat_geometries,
    }
}

fn count_in_pulses_for_measure(
    config: &CountInSnippetConfig,
    measure_idx: usize,
) -> Vec<CountInPulse> {
    if config.beat_unit == 8 && config.beats_per_measure == 6 {
        if config.num_measures == 2 && measure_idx == 0 {
            return vec![
                CountInPulse {
                    beat_index: 0,
                    duration_ticks: 720,
                    dotted: true,
                    label: Some("1"),
                },
                CountInPulse {
                    beat_index: 3,
                    duration_ticks: 720,
                    dotted: true,
                    label: Some("2"),
                },
            ];
        }

        return (0..6)
            .map(|beat_index| CountInPulse {
                beat_index,
                duration_ticks: 240,
                dotted: false,
                label: Some(match beat_index {
                    0 => "1",
                    1 => "2",
                    2 => "3",
                    3 => "4",
                    4 => "5",
                    _ => "6",
                }),
            })
            .collect();
    }

    let is_half_time = config.num_measures == 2 && measure_idx == 0;
    (0..config.beats_per_measure as usize)
        .filter_map(|beat_index| {
            let label = if is_half_time {
                match beat_index {
                    0 => Some("1"),
                    2 => Some("2"),
                    _ => None,
                }
            } else {
                None
            };

            if is_half_time && label.is_none() {
                return None;
            }

            Some(CountInPulse {
                beat_index,
                duration_ticks: 480,
                dotted: false,
                label: label.or(match beat_index {
                    0 => Some("1"),
                    1 => Some("2"),
                    2 => Some("3"),
                    3 => Some("4"),
                    4 => Some("5"),
                    5 => Some("6"),
                    _ => None,
                }),
            })
        })
        .collect()
}

fn count_in_measure_scene(
    config: &CountInSnippetConfig,
    measure_idx: usize,
    measure_width: f64,
    ctx: &crate::engraver::layout::context::LayoutContext<'_>,
) -> crate::engraver::notation::MeasureScene {
    let entries = count_in_pulses_for_measure(config, measure_idx)
        .into_iter()
        .map(|pulse| {
            let duration = match (pulse.duration_ticks, pulse.dotted) {
                (720, true) => Duration::DottedQuarter,
                (240, false) => Duration::Eighth,
                (480, false) => Duration::Quarter,
                _ => Duration::Quarter,
            };
            RhythmEntry::Note(duration)
        })
        .collect::<Vec<_>>();

    MeasureBuilder::new()
        .id_base(10_000 + measure_idx as u64 * 100)
        .justify_to(measure_width / ctx.spatium())
        .no_barlines()
        .entries(entries)
        .rhythmic()
        .time_signature_meta(crate::engraver::notation::TimeSignature::new(
            config.beats_per_measure,
            config.beat_unit,
        ))
        .build(ctx)
}

fn count_in_measure_needs_low_labels(config: &CountInSnippetConfig, measure_idx: usize) -> bool {
    count_in_pulses_for_measure(config, measure_idx)
        .iter()
        .any(|pulse| pulse.duration_ticks <= 240)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engraver::scene::traverse::SceneNodeExt;

    #[test]
    fn test_default_config() {
        let config = CountInSnippetConfig::default();
        assert_eq!(config.beats_per_measure, 4);
        assert_eq!(config.beat_unit, 4);
        assert_eq!(config.num_measures, 1);
        assert!((config.spatium - 5.0).abs() < f64::EPSILON);
        assert!((config.scale - 0.6).abs() < f64::EPSILON);
    }

    #[test]
    fn test_render_single_measure() {
        let config = CountInSnippetConfig {
            beats_per_measure: 4,
            beat_unit: 4,
            num_measures: 1,
            spatium: 5.0,
            scale: 0.6,
            measure_numbers: Vec::new(),
        };
        let result = render_count_in_snippet(&config, Point::new(0.0, 0.0));
        assert!(result.width > 0.0);
        assert!(result.height > 0.0);
    }

    #[test]
    fn test_render_two_measures() {
        let config = CountInSnippetConfig {
            beats_per_measure: 4,
            beat_unit: 4,
            num_measures: 2,
            spatium: 5.0,
            scale: 0.6,
            measure_numbers: Vec::new(),
        };
        let result = render_count_in_snippet(&config, Point::new(0.0, 0.0));
        // Two measures should be wider than one
        let single_config = CountInSnippetConfig {
            num_measures: 1,
            ..config.clone()
        };
        let single_result = render_count_in_snippet(&single_config, Point::new(0.0, 0.0));
        assert!(result.width > single_result.width);
    }

    #[test]
    fn measure_labels_are_top_left_with_count_in_after_first_number() {
        let config = CountInSnippetConfig {
            beats_per_measure: 4,
            beat_unit: 4,
            num_measures: 2,
            spatium: 5.0,
            scale: 1.0,
            measure_numbers: vec!["1".to_string(), "2".to_string()],
        };

        let result = render_count_in_snippet(&config, Point::new(10.0, 20.0));
        let labels = result
            .node
            .commands
            .iter()
            .filter_map(|cmd| {
                if let PaintCommand::Text {
                    text,
                    position,
                    anchor,
                    ..
                } = cmd
                {
                    Some((text.as_str(), *position, *anchor))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        let first = labels
            .iter()
            .find(|(text, _, _)| *text == "1 Count-In")
            .expect("first count-in measure label should include Count-In text");
        let second = labels
            .iter()
            .find(|(text, _, _)| *text == "2")
            .expect("second count-in measure label should render its measure number");

        assert_eq!(first.2, TextAnchor::Start);
        assert_eq!(second.2, TextAnchor::Start);
        assert!(
            first.1.x < second.1.x,
            "measure labels should advance left-to-right by measure"
        );
        assert!(
            (first.1.y - second.1.y).abs() < f64::EPSILON,
            "measure labels should share the same header row"
        );
    }

    #[test]
    fn renders_mini_time_signature() {
        let config = CountInSnippetConfig {
            beats_per_measure: 6,
            beat_unit: 8,
            num_measures: 1,
            spatium: 5.0,
            scale: 1.0,
            measure_numbers: Vec::new(),
        };

        let result = render_count_in_snippet(&config, Point::new(0.0, 0.0));
        let time_sig_glyphs = result
            .node
            .iter_with_transforms()
            .flat_map(|(node, transform)| {
                node.commands.iter().filter_map(move |cmd| {
                    if let PaintCommand::Glyph {
                        codepoint,
                        position,
                        ..
                    } = cmd
                    {
                        Some((*codepoint, transform * *position))
                    } else {
                        None
                    }
                })
            })
            .filter(|(codepoint, _)| {
                *codepoint == crate::engraver::layout::tlayout::timesig::glyphs::TIMESIG_6
                    || *codepoint == crate::engraver::layout::tlayout::timesig::glyphs::TIMESIG_8
            })
            .collect::<Vec<_>>();

        assert!(time_sig_glyphs.iter().any(|(codepoint, _)| *codepoint
            == crate::engraver::layout::tlayout::timesig::glyphs::TIMESIG_6));
        assert!(time_sig_glyphs.iter().any(|(codepoint, _)| *codepoint
            == crate::engraver::layout::tlayout::timesig::glyphs::TIMESIG_8));
    }

    #[test]
    fn six_eight_count_in_keeps_slashes_clear_of_time_signature() {
        let config = CountInSnippetConfig {
            beats_per_measure: 6,
            beat_unit: 8,
            num_measures: 2,
            spatium: 5.0,
            scale: 1.0,
            measure_numbers: Vec::new(),
        };

        let result = render_count_in_snippet(&config, Point::new(0.0, 0.0));
        let first_beat_x = result
            .beat_geometries
            .iter()
            .find(|beat| beat.measure_index == 0 && beat.beat_index == 0)
            .expect("first count-in beat should have geometry")
            .x;
        let time_sig_right = 5.0 * 0.85 + 5.0 * 1.0;

        assert!(
            first_beat_x > time_sig_right + 5.0,
            "beat 1 slash should start after the mini time signature: beat_x={first_beat_x:.1}, time_sig_right={time_sig_right:.1}"
        );
    }

    #[test]
    fn six_eight_count_in_labels_sit_under_beamed_notes() {
        let config = CountInSnippetConfig {
            beats_per_measure: 6,
            beat_unit: 8,
            num_measures: 2,
            spatium: 5.0,
            scale: 1.0,
            measure_numbers: Vec::new(),
        };

        let result = render_count_in_snippet(&config, Point::new(0.0, 0.0));
        let labels = result
            .node
            .commands
            .iter()
            .filter_map(|cmd| {
                if let PaintCommand::Text { text, position, .. } = cmd {
                    text.parse::<usize>()
                        .ok()
                        .map(|_| (text.as_str(), *position))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        for beat in result
            .beat_geometries
            .iter()
            .filter(|beat| beat.measure_index == 1)
        {
            let label = labels
                .iter()
                .find(|(text, position)| {
                    *text == (beat.beat_index + 1).to_string()
                        && (position.x - beat.x).abs() <= 0.01
                })
                .expect("each second-bar eighth beat should have a count label");
            assert!(
                label.1.y >= beat.staff_y + beat.staff_height + 21.5,
                "count label should sit below the staff/beamed notes with clearance: label={:?}, staff_bottom={:.1}",
                label,
                beat.staff_y + beat.staff_height
            );
        }
    }

    #[test]
    fn six_eight_count_in_labels_only_drop_for_beamed_measure() {
        let config = CountInSnippetConfig {
            beats_per_measure: 6,
            beat_unit: 8,
            num_measures: 2,
            spatium: 5.0,
            scale: 1.0,
            measure_numbers: Vec::new(),
        };

        let result = render_count_in_snippet(&config, Point::new(0.0, 0.0));
        let labels = result
            .node
            .commands
            .iter()
            .filter_map(|cmd| {
                if let PaintCommand::Text { text, position, .. } = cmd {
                    text.parse::<usize>()
                        .ok()
                        .map(|_| (text.as_str(), *position))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        let first_bar_one = result
            .beat_geometries
            .iter()
            .find(|beat| beat.measure_index == 0 && beat.beat_index == 0)
            .expect("first count-in bar should have beat 1");
        let second_bar_one = result
            .beat_geometries
            .iter()
            .find(|beat| beat.measure_index == 1 && beat.beat_index == 0)
            .expect("second count-in bar should have beat 1");

        let first_label_y = labels
            .iter()
            .find(|(text, position)| *text == "1" && (position.x - first_bar_one.x).abs() <= 0.01)
            .expect("first bar beat 1 label should exist")
            .1
            .y;
        let second_label_y = labels
            .iter()
            .find(|(text, position)| *text == "1" && (position.x - second_bar_one.x).abs() <= 0.01)
            .expect("second bar beat 1 label should exist")
            .1
            .y;

        assert!(
            second_label_y > first_label_y + 10.0,
            "beamed second bar labels should drop, but stemless first bar labels should stay close: first={first_label_y:.1}, second={second_label_y:.1}"
        );
    }

    #[test]
    fn test_scale_affects_dimensions() {
        let base_config = CountInSnippetConfig {
            beats_per_measure: 4,
            beat_unit: 4,
            num_measures: 1,
            spatium: 5.0,
            scale: 1.0, // Full size
            measure_numbers: Vec::new(),
        };
        let scaled_config = CountInSnippetConfig {
            scale: 0.5, // Half size
            ..base_config.clone()
        };

        let full_result = render_count_in_snippet(&base_config, Point::new(0.0, 0.0));
        let scaled_result = render_count_in_snippet(&scaled_config, Point::new(0.0, 0.0));

        // Scaled result should be approximately half the width and height
        // (not exact due to barline padding adjustments)
        assert!(scaled_result.width < full_result.width);
        assert!(scaled_result.height < full_result.height);
    }

    #[test]
    fn test_uses_correct_slash_glyph() {
        // Quarter note = 480 ticks should return a filled slash
        let codepoint = slash_glyph_for_ticks(480);
        // The codepoint should be a valid SMuFL slash notehead
        assert!(codepoint as u32 >= 0xE100); // SMuFL noteheads range
    }

    #[test]
    fn six_eight_two_bar_count_in_uses_compound_then_full_count() {
        let config = CountInSnippetConfig {
            beats_per_measure: 6,
            beat_unit: 8,
            num_measures: 2,
            spatium: 5.0,
            scale: 1.0,
            measure_numbers: Vec::new(),
        };

        assert_eq!(
            count_in_pulses_for_measure(&config, 0),
            vec![
                CountInPulse {
                    beat_index: 0,
                    duration_ticks: 720,
                    dotted: true,
                    label: Some("1")
                },
                CountInPulse {
                    beat_index: 3,
                    duration_ticks: 720,
                    dotted: true,
                    label: Some("2")
                }
            ],
            "first 6/8 count-in bar should render /. /. with 2 on beat 4"
        );
        assert_eq!(
            count_in_pulses_for_measure(&config, 1)
                .iter()
                .map(|pulse| (
                    pulse.beat_index,
                    pulse.duration_ticks,
                    pulse.dotted,
                    pulse.label
                ))
                .collect::<Vec<_>>(),
            vec![
                (0, 240, false, Some("1")),
                (1, 240, false, Some("2")),
                (2, 240, false, Some("3")),
                (3, 240, false, Some("4")),
                (4, 240, false, Some("5")),
                (5, 240, false, Some("6")),
            ],
            "second 6/8 count-in bar should render /// /// with labels 1-6"
        );

        let result = render_count_in_snippet(&config, Point::new(0.0, 0.0));
        let glyphs = result
            .node
            .iter_with_transforms()
            .flat_map(|(_node, transform)| {
                _node.commands.iter().filter_map(move |cmd| {
                    if let PaintCommand::Glyph {
                        codepoint,
                        position,
                        ..
                    } = cmd
                    {
                        Some((*codepoint, transform * *position))
                    } else {
                        None
                    }
                })
            })
            .collect::<Vec<_>>();
        let slash_positions = glyphs
            .iter()
            .filter_map(|(codepoint, position)| {
                (*codepoint
                    == crate::engraver::layout::tlayout::note::glyphs::NOTEHEAD_SLASH_HORIZONTAL)
                    .then_some(*position)
            })
            .collect::<Vec<_>>();
        let dot_positions = glyphs
            .iter()
            .filter_map(|(codepoint, position)| {
                (*codepoint == crate::engraver::layout::tlayout::note::glyphs::AUGMENTATION_DOT)
                    .then_some(*position)
            })
            .collect::<Vec<_>>();
        assert_eq!(
            slash_positions.len(),
            8,
            "want 2 compound slashes + 6 eighth slashes"
        );
        assert_eq!(dot_positions.len(), 2);
        for (slash, dot) in slash_positions.iter().take(2).zip(dot_positions.iter()) {
            assert!(
                dot.x > slash.x,
                "dotted-slash dot should sit clearly to the right of the slash: slash={slash:?}, dot={dot:?}"
            );
            assert!(
                dot.y < slash.y,
                "dotted-slash dot should sit in the upper-right dot position: slash={slash:?}, dot={dot:?}"
            );
        }
    }

    #[test]
    fn test_barlines_rendered() {
        let config = CountInSnippetConfig {
            beats_per_measure: 4,
            beat_unit: 4,
            num_measures: 2,
            spatium: 5.0,
            scale: 1.0,
            measure_numbers: Vec::new(),
        };
        let result = render_count_in_snippet(&config, Point::new(0.0, 0.0));

        // Count line commands (barlines + staff lines)
        // Should have: 5 staff lines + 3 barlines (start, middle, end) = 8 lines
        let line_count = result
            .node
            .commands
            .iter()
            .filter(|cmd| matches!(cmd, PaintCommand::Line { .. }))
            .count();
        assert_eq!(line_count, 8, "Expected 5 staff lines + 3 barlines");
    }
}
