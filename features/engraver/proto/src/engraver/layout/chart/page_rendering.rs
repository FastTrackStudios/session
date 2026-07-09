//! Page rendering utilities for chart layout.
//!
//! This module provides functions for drawing page elements:
//! staff lines, barlines, backgrounds, footers, and headers.

use super::count_in_renderer::{self, CountInBeatGeometry, CountInSnippetConfig};
use crate::engraver::layout::orchestrator::PageMargins;
use crate::engraver::layout::tlayout::BarlineType;
use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::{FontStyle, FontWeight, PaintCommand, TextAnchor};
use kurbo::Rect;
use peniko::Color;

/// Staff line thickness as a fraction of spatium.
///
/// FastTrack charts use thin 0.05-spatium staff lines to keep dense rhythm
/// charts readable at page scale.
pub const STAFF_LINE_THICKNESS_RATIO: f64 = 0.05;

/// Thin barline thickness as a fraction of spatium.
/// Engraving convention: ~0.16 (MuseScore default).
pub const BARLINE_THIN_RATIO: f64 = 0.16;

/// Thick (end / final / repeat) barline thickness as a fraction of spatium.
/// Engraving convention: ~0.5 spatium.
pub const BARLINE_THICK_RATIO: f64 = 0.5;

/// Draw staff lines at the given position with default thickness.
///
/// Draws 5 horizontal lines at standard staff spacing.
/// Thickness is calculated as `spatium * STAFF_LINE_THICKNESS_RATIO`.
///
/// # Arguments
/// * `x` - Left edge X position
/// * `y` - Top line Y position
/// * `width` - Staff width
/// * `spatium` - Staff space (distance between lines)
#[must_use]
pub fn draw_staff_lines(x: f64, y: f64, width: f64, spatium: f64) -> Vec<PaintCommand> {
    let thickness = spatium * STAFF_LINE_THICKNESS_RATIO;
    draw_staff_lines_with_thickness(x, y, width, spatium, thickness)
}

/// Draw staff lines at the given position with custom thickness.
///
/// Draws 5 horizontal lines at standard staff spacing.
///
/// # Arguments
/// * `x` - Left edge X position
/// * `y` - Top line Y position
/// * `width` - Staff width
/// * `spatium` - Staff space (distance between lines)
/// * `thickness` - Line thickness in points
#[must_use]
pub fn draw_staff_lines_with_thickness(
    x: f64,
    y: f64,
    width: f64,
    spatium: f64,
    thickness: f64,
) -> Vec<PaintCommand> {
    let mut commands = Vec::new();

    for i in 0..5 {
        let line_y = y + i as f64 * spatium;
        commands.push(PaintCommand::line(
            kurbo::Point::new(x, line_y),
            kurbo::Point::new(x + width, line_y),
            Color::BLACK,
            thickness,
        ));
    }

    commands
}

/// Draw a barline at the given position.
///
/// Supports single, double, and end barlines. `spatium` is the active staff
/// space (points) — barline thickness scales with it so lines stay visible
/// at small staff sizes and proportional at large ones.
#[must_use]
pub fn draw_barline(
    x: f64,
    y: f64,
    height: f64,
    barline_type: BarlineType,
    spatium: f64,
) -> SceneNode {
    let mut commands = Vec::new();
    let thin = spatium * BARLINE_THIN_RATIO;
    let thick = spatium * BARLINE_THICK_RATIO;
    // Double-barline spacing also scales with spatium (about half a space).
    let double_gap = spatium * 0.6;
    // End-barline gap between thin and thick lines.
    let end_gap = spatium * 1.0;

    match barline_type {
        BarlineType::Single => {
            commands.push(PaintCommand::line(
                kurbo::Point::new(x, y),
                kurbo::Point::new(x, y + height),
                Color::BLACK,
                thin,
            ));
        }
        BarlineType::Double => {
            commands.push(PaintCommand::line(
                kurbo::Point::new(x - double_gap, y),
                kurbo::Point::new(x - double_gap, y + height),
                Color::BLACK,
                thin,
            ));
            commands.push(PaintCommand::line(
                kurbo::Point::new(x, y),
                kurbo::Point::new(x, y + height),
                Color::BLACK,
                thin,
            ));
        }
        BarlineType::End => {
            commands.push(PaintCommand::line(
                kurbo::Point::new(x - end_gap, y),
                kurbo::Point::new(x - end_gap, y + height),
                Color::BLACK,
                thin,
            ));
            commands.push(PaintCommand::line(
                kurbo::Point::new(x, y),
                kurbo::Point::new(x, y + height),
                Color::BLACK,
                thick,
            ));
        }
        BarlineType::EndRepeat => {
            draw_bracket_repeat(&mut commands, x, y, height, spatium, false);
        }
        BarlineType::StartRepeat => {
            draw_bracket_repeat(&mut commands, x, y, height, spatium, true);
        }
        BarlineType::Dashed => {
            let dash = spatium * 0.45;
            let gap = spatium * 0.3;
            let mut cy = y;
            while cy < y + height {
                let end = (cy + dash).min(y + height);
                commands.push(PaintCommand::line(
                    kurbo::Point::new(x, cy),
                    kurbo::Point::new(x, end),
                    Color::BLACK,
                    thin,
                ));
                cy = end + gap;
            }
        }
        _ => {
            commands.push(PaintCommand::line(
                kurbo::Point::new(x, y),
                kurbo::Point::new(x, y + height),
                Color::BLACK,
                thin,
            ));
        }
    }

    SceneNode::anonymous_leaf(commands)
}

fn draw_bracket_repeat(
    commands: &mut Vec<PaintCommand>,
    x: f64,
    staff_y: f64,
    staff_height: f64,
    spatium: f64,
    opens_right: bool,
) {
    let red = Color::from_rgb8(220, 38, 38);
    let stroke = spatium * 0.34;
    let top = staff_y - spatium * 0.5;
    let bottom = staff_y + staff_height + spatium * 0.5;
    let arm = spatium * 1.0;
    let dir = if opens_right { 1.0 } else { -1.0 };
    let dot_x = x + dir * spatium * 0.85;

    commands.push(PaintCommand::line(
        kurbo::Point::new(x, top),
        kurbo::Point::new(x, bottom),
        red,
        stroke,
    ));
    commands.push(PaintCommand::line(
        kurbo::Point::new(x, top),
        kurbo::Point::new(x + dir * arm, top - arm),
        red,
        stroke,
    ));
    commands.push(PaintCommand::line(
        kurbo::Point::new(x, bottom),
        kurbo::Point::new(x + dir * arm, bottom + arm),
        red,
        stroke,
    ));
    draw_repeat_dots_with_color(commands, dot_x, staff_y, spatium, red);
}

fn draw_repeat_dots_with_color(
    commands: &mut Vec<PaintCommand>,
    dot_x: f64,
    staff_y: f64,
    spatium: f64,
    color: Color,
) {
    let dot_radius = spatium * 0.18;
    // Caller passes `y = staff top`; centre the dot pair on staff lines 2 & 4
    // of a 5-line staff (i.e. ±0.5 spatium from the middle line).
    let staff_height_estimate = spatium * 4.0;
    let middle_y = staff_y + staff_height_estimate / 2.0;
    let upper = middle_y - spatium * 0.5;
    let lower = middle_y + spatium * 0.5;
    commands.push(PaintCommand::filled_circle(
        kurbo::Point::new(dot_x, upper),
        dot_radius,
        color,
    ));
    commands.push(PaintCommand::filled_circle(
        kurbo::Point::new(dot_x, lower),
        dot_radius,
        color,
    ));
}

/// Add page background (paper with shadow).
pub fn add_page_background(
    root: &mut SceneNode,
    page_x: f64,
    page_y: f64,
    page_width: f64,
    page_height: f64,
) {
    let shadow_offset = 4.0;

    // Shadow
    root.add_child(SceneNode::anonymous_leaf(vec![PaintCommand::filled_rect(
        Rect::new(
            page_x + shadow_offset,
            page_y + shadow_offset,
            page_x + shadow_offset + page_width,
            page_y + shadow_offset + page_height,
        ),
        Color::from_rgb8(180, 180, 180),
    )]));

    // White paper
    root.add_child(SceneNode::anonymous_leaf(vec![PaintCommand::filled_rect(
        Rect::new(page_x, page_y, page_x + page_width, page_y + page_height),
        Color::WHITE,
    )]));
}

/// Add simple snippet background (white box only, no shadow).
pub fn add_snippet_background(
    root: &mut SceneNode,
    page_x: f64,
    page_y: f64,
    page_width: f64,
    page_height: f64,
) {
    // Just white paper, no shadow
    root.add_child(SceneNode::anonymous_leaf(vec![PaintCommand::filled_rect(
        Rect::new(page_x, page_y, page_x + page_width, page_y + page_height),
        Color::WHITE,
    )]));
}

/// Add page footer with "Created with FastTrackStudio" text.
pub fn add_page_footer(
    root: &mut SceneNode,
    page_x: f64,
    page_y: f64,
    page_width: f64,
    page_height: f64,
) {
    let footer_text = "Created with FastTrackStudio";
    let version_text = concat!("v.alpha.", env!("CARGO_PKG_VERSION"));
    let font_size = 8.0;
    let version_font_size = 6.0;
    let footer_y = page_y + page_height - 15.0; // 15 points from bottom
    let center_x = page_x + page_width / 2.0;

    // Rough monospace-ish width estimate so the version subscript sits just to
    // the right of the centered main text.
    let footer_width = footer_text.chars().count() as f64 * font_size * 0.5;
    let version_x = center_x + footer_width / 2.0 + 3.0;
    let version_y = footer_y + 2.0; // dropped baseline -> subscript

    let color = Color::from_rgb8(160, 160, 160); // Light gray
    root.add_child(SceneNode::anonymous_leaf(vec![
        PaintCommand::Text {
            text: footer_text.to_string(),
            font_family: "sans-serif".to_string(),
            font_size,
            position: kurbo::Point::new(center_x, footer_y),
            color,
            anchor: TextAnchor::Middle,
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        },
        PaintCommand::Text {
            text: version_text.to_string(),
            font_family: "sans-serif".to_string(),
            font_size: version_font_size,
            position: kurbo::Point::new(version_x, version_y),
            color,
            anchor: TextAnchor::Start,
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        },
    ]));
}

/// Configuration for count-in snippet in header.
#[derive(Debug, Clone, Default)]
pub struct CountInHeaderConfig {
    /// Number of count-in measures (0 means no count-in).
    pub num_measures: usize,
    /// Beats per measure (numerator of time signature).
    pub beats_per_measure: u8,
    /// Beat unit (denominator of time signature).
    pub beat_unit: u8,
    /// Whether there's a pushed chord on beat 1 that spills into the count-in.
    /// When true, extra vertical space is added above the count-in for chord rendering.
    pub has_pushed_chord: bool,
    /// Per-measure labels rendered above each measure in the snippet (e.g.
    /// `["1","2"]` for an LotF-style count-in). Empty means no labels.
    pub measure_numbers: Vec<String>,
}

/// Parameters for [`add_title_header_with_count_in`].
pub struct TitleHeaderParams<'a> {
    pub root: &'a mut SceneNode,
    pub page_x: f64,
    pub page_y: f64,
    pub page_width: f64,
    pub margins: &'a PageMargins,
    pub spatium: f64,
    pub metadata: &'a crate::SongMetadata,
    pub tempo: Option<&'a crate::time::Tempo>,
    pub count_in: Option<&'a CountInHeaderConfig>,
}

/// Add title header section with optional count-in snippet.
///
/// Renders title, part name, composer/artist, tempo, and (when configured)
/// a compact count-in snippet next to the tempo indicator.
///
/// Returns `(header_height, count_in_beat_geometries)`.
pub fn add_title_header_with_count_in(
    params: TitleHeaderParams<'_>,
) -> (f64, Vec<CountInBeatGeometry>) {
    let TitleHeaderParams {
        root,
        page_x,
        page_y,
        page_width,
        margins,
        spatium,
        metadata,
        tempo,
        count_in,
    } = params;
    // If there's no title, skip the entire header (no part name, version, subtitle, etc.)
    if metadata.title.is_none() {
        return (0.0, Vec::new());
    }

    let mut commands = Vec::new();
    let center_x = page_x + page_width / 2.0;
    let right_x = page_x + page_width - margins.right;
    let left_x = page_x + margins.left;

    let frame_top_y = page_y + margins.top;

    // Header padding for better visual breathing room
    let header_top_padding = 8.0; // Extra space before title

    // Title (large, bold, centered) - at the very top of the header
    let title_font_size = 34.0;
    let title_y = frame_top_y + header_top_padding + title_font_size;

    // Determine part name - default to "Master Rhythm" if not specified
    let part_name = metadata.part_name.as_deref().unwrap_or("Master Rhythm");
    let is_master_rhythm = part_name.eq_ignore_ascii_case("master rhythm");

    // For Master Rhythm, split into two lines and render in bold caps
    let part_name_lines: Vec<String> = if is_master_rhythm {
        vec!["MASTER".to_string(), "RHYTHM".to_string()]
    } else {
        vec![part_name.to_uppercase()]
    };

    let small_font_size = 11.0;
    let composer_text = metadata.composer.as_ref().or(metadata.artist.as_ref());

    // Detect overlap between centered title and the left part-name / right
    // composer text. Without measured metrics we estimate widths from char
    // counts. When either side encroaches on the title's bounding box we
    // push the title onto its own row to avoid visual collision.
    //
    // Avg widths picked conservatively (slightly wide) so we err on the
    // side of stacking rather than overlapping.
    fn approx_text_width(s: &str, font_size: f64) -> f64 {
        s.chars().count() as f64 * font_size * 0.6
    }

    let title_text_for_size = metadata
        .title
        .as_ref()
        .map(|t| format!("\"{t}\""))
        .unwrap_or_default();
    let title_width_est = approx_text_width(&title_text_for_size, title_font_size);
    let title_half = title_width_est / 2.0;
    let left_text_width_est = part_name_lines
        .iter()
        .map(|l| approx_text_width(l, small_font_size))
        .fold(0.0_f64, f64::max);
    let right_text_width_est = composer_text
        .map(|c| approx_text_width(c, small_font_size))
        .unwrap_or(0.0);

    let gap = 10.0_f64; // minimum gap between side text and title
    let left_intrudes = left_x + left_text_width_est + gap > center_x - title_half;
    let right_intrudes = center_x + title_half + gap > right_x - right_text_width_est;
    let title_needs_own_row = left_intrudes || right_intrudes;

    if let Some(ref title) = metadata.title {
        let quoted_title = format!("\"{}\"", title);
        commands.push(PaintCommand::Text {
            text: quoted_title,
            font_family: "FreeSans".to_string(),
            font_size: title_font_size,
            position: kurbo::Point::new(center_x, title_y),
            color: Color::BLACK,
            anchor: TextAnchor::Middle,
            weight: FontWeight::Bold,
            style: FontStyle::Normal,
        });
    }

    // Part name (left) and Composer/Artist (right). If the title doesn't fit
    // on the same row, drop the side text below the title's baseline.
    // Subtitle (if any) renders centered below the title, so when stacking
    // we also push past it to keep part-name from colliding.
    let title_vertical_middle = title_y - (title_font_size * 0.35);
    let subtitle_font_size_ref = 12.0;
    let has_subtitle_meta = metadata
        .subtitle
        .as_ref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let part_start_y = if title_needs_own_row {
        let mut y = title_y + small_font_size + 4.0;
        if has_subtitle_meta {
            y += subtitle_font_size_ref + 2.0;
        }
        y
    } else {
        title_vertical_middle + small_font_size * 0.35
    };

    // Part name - left aligned, bold font
    for (i, line) in part_name_lines.iter().enumerate() {
        let line_y = part_start_y + (i as f64 * (small_font_size + 2.0));
        commands.push(PaintCommand::Text {
            text: line.clone(),
            font_family: "FreeSans".to_string(),
            font_size: small_font_size,
            position: kurbo::Point::new(left_x, line_y),
            color: Color::BLACK,
            anchor: TextAnchor::Start,
            weight: FontWeight::Bold,
            style: FontStyle::Normal,
        });
    }

    // Version - below part name, dark gray, not bold
    let version = metadata.version.unwrap_or(1);
    let version_y = part_start_y + (part_name_lines.len() as f64 * (small_font_size + 2.0));
    commands.push(PaintCommand::Text {
        text: format!("V{}", version),
        font_family: "sans-serif".to_string(),
        font_size: small_font_size,
        position: kurbo::Point::new(left_x, version_y),
        color: Color::from_rgb8(100, 100, 100),
        anchor: TextAnchor::Start,
        weight: FontWeight::Normal,
        style: FontStyle::Normal,
    });

    // Tempo indicator - below version with padding
    // Extra space is added when there's a pushed chord that spills into the count-in
    let has_pushed_chord = count_in.is_some_and(|c| c.has_pushed_chord);
    let tempo_padding = if has_pushed_chord { 20.0 } else { 6.0 };
    let tempo_y = version_y + small_font_size + tempo_padding;
    let tempo_font_size = 12.0;
    let tempo_symbol_ratio = 5.0 / 3.0;
    let tempo_symbol_pt = tempo_font_size * tempo_symbol_ratio;
    let tempo_glyph_size = tempo_symbol_pt / spatium;

    // Track tempo text width for count-in positioning. When there's no
    // tempo, the count-in still needs somewhere to land that isn't on top
    // of the part-name column — push it past the widest part-name line +
    // "V{version}" so the snippet always starts to the right of that block.
    let part_name_width = part_name_lines
        .iter()
        .map(|l| approx_text_width(l, small_font_size))
        .fold(0.0_f64, f64::max);
    let part_name_block_width =
        part_name_width.max(approx_text_width(&format!("V{version}"), small_font_size));
    let mut tempo_end_x = left_x + part_name_block_width + 16.0;

    if let Some(tempo_val) = tempo {
        let tempo_note = if count_in.is_some_and(|c| c.beat_unit == 8) {
            '\u{eca7}' // metNote8thUp
        } else {
            '\u{eca5}' // metNoteQuarterUp
        };
        let glyph_width = tempo_glyph_size * spatium * 0.5;
        let glyph_y = tempo_y + tempo_symbol_pt * 0.15;

        commands.push(PaintCommand::Glyph {
            codepoint: tempo_note,
            position: kurbo::Point::new(left_x, glyph_y),
            size: tempo_glyph_size,
            color: Color::BLACK,
        });

        // Estimate tempo text width (roughly 7 points per character)
        let tempo_text = format!("= {}", tempo_val.bpm as u32);
        let tempo_text_width = tempo_text.len() as f64 * 7.0;

        commands.push(PaintCommand::Text {
            text: tempo_text,
            font_family: "sans-serif".to_string(),
            font_size: tempo_font_size,
            position: kurbo::Point::new(left_x + glyph_width + 4.0, tempo_y),
            color: Color::BLACK,
            anchor: TextAnchor::Start,
            weight: FontWeight::Bold,
            style: FontStyle::Normal,
        });

        tempo_end_x = left_x + glyph_width + 4.0 + tempo_text_width;
    }

    // Add commands accumulated so far
    if !commands.is_empty() {
        root.add_child(SceneNode::anonymous_leaf(commands));
    }

    // Render count-in snippet next to tempo indicator
    let mut count_in_beat_geos = Vec::new();
    if let Some(config) = count_in
        && config.num_measures > 0
    {
        let snippet_config = CountInSnippetConfig {
            beats_per_measure: config.beats_per_measure,
            beat_unit: config.beat_unit,
            num_measures: config.num_measures,
            spatium,
            scale: 0.6, // 60% of normal size for header snippet
            measure_numbers: config.measure_numbers.clone(),
        };

        // Position snippet to the right of tempo indicator
        let snippet_x = tempo_end_x + 8.0; // Closer horizontal spacing

        // Vertically center the snippet's staff with the tempo text
        // Tempo text visual center is ~40% above its baseline
        // Snippet staff center is at snippet_y + staff_height/2
        let scaled_spatium = spatium * 0.6; // Match the snippet's scale
        let staff_height = scaled_spatium * 4.0;
        let tempo_text_center = tempo_y - tempo_font_size * 0.4;
        let snippet_y = tempo_text_center - staff_height / 2.0;

        let snippet_result = count_in_renderer::render_count_in_snippet(
            &snippet_config,
            kurbo::Point::new(snippet_x, snippet_y),
        );

        root.add_child(snippet_result.node);
        count_in_beat_geos = snippet_result.beat_geometries;
    }

    // Composer/Artist - right aligned
    let mut commands = Vec::new();
    if let Some(composer) = composer_text {
        let artists: Vec<&str> = composer.split(',').map(|s| s.trim()).collect();
        for (i, artist) in artists.iter().enumerate() {
            let line_y = part_start_y + (i as f64 * (small_font_size + 2.0));
            commands.push(PaintCommand::Text {
                text: artist.to_string(),
                font_family: "sans-serif".to_string(),
                font_size: small_font_size,
                position: kurbo::Point::new(right_x, line_y),
                color: Color::BLACK,
                anchor: TextAnchor::End,
                weight: FontWeight::Normal,
                style: FontStyle::Normal,
            });
        }
    }

    // Subtitle (medium, centered) - below the title with minimal spacing
    let subtitle_text = metadata
        .subtitle
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let has_subtitle = subtitle_text.is_some();
    let subtitle_font_size = 12.0;
    let subtitle_y = title_y + subtitle_font_size + 2.0;

    if let Some(subtitle_text) = subtitle_text.as_ref() {
        commands.push(PaintCommand::Text {
            text: subtitle_text.clone(),
            font_family: "sans-serif".to_string(),
            font_size: subtitle_font_size,
            position: kurbo::Point::new(center_x, subtitle_y),
            color: Color::from_rgb8(100, 100, 100),
            anchor: TextAnchor::Middle,
            weight: FontWeight::Normal,
            style: FontStyle::Italic,
        });
    }

    if !commands.is_empty() {
        root.add_child(SceneNode::anonymous_leaf(commands));
    }

    // Calculate header height from the title/part/tempo block only. The
    // count-in snippet is an overlay anchored to the tempo row; its internal
    // label/staff/beat-number height must not push the first system down.
    let left_column_bottom = if tempo.is_some() {
        tempo_y + tempo_font_size
    } else {
        version_y + small_font_size
    };
    let current_y = if has_subtitle {
        subtitle_y.max(left_column_bottom)
    } else {
        left_column_bottom
    };

    // Return total header height (from margin top to current_y, plus bottom padding)
    // This padding creates space between header content and the first system
    let header_bottom_padding = 35.0;
    let header_height = current_y - (page_y + margins.top) + header_bottom_padding;
    (header_height.max(0.0), count_in_beat_geos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_draw_staff_lines_count() {
        let commands = draw_staff_lines(0.0, 0.0, 100.0, 10.0);
        assert_eq!(commands.len(), 5);
    }

    #[test]
    fn test_draw_staff_lines_use_thin_fasttrack_width() {
        assert_eq!(STAFF_LINE_THICKNESS_RATIO, 0.05);
        let commands = draw_staff_lines(0.0, 0.0, 100.0, 10.0);
        let PaintCommand::Line { width, .. } = commands[0] else {
            panic!("expected staff line command");
        };
        assert!((width - 0.5).abs() < 1e-6);
    }

    #[test]
    fn test_draw_barline_single() {
        let node = draw_barline(100.0, 0.0, 40.0, BarlineType::Single, 5.0);
        // Single barline should have one command
        assert!(!node.children.is_empty() || !node.commands.is_empty());
    }

    #[test]
    fn test_draw_barline_double() {
        let node = draw_barline(100.0, 0.0, 40.0, BarlineType::Double, 5.0);
        // Double barline should have two commands
        assert!(!node.children.is_empty() || !node.commands.is_empty());
    }

    #[test]
    fn test_draw_barline_end() {
        let node = draw_barline(100.0, 0.0, 40.0, BarlineType::End, 5.0);
        // End barline should have two commands (thin + thick)
        assert!(!node.children.is_empty() || !node.commands.is_empty());
    }

    #[test]
    fn repeat_barlines_are_red_bracket_markers() {
        let node = draw_barline(100.0, 20.0, 40.0, BarlineType::StartRepeat, 10.0);
        assert_eq!(
            node.commands.len(),
            5,
            "bracket repeat should be vertical, two diagonal arms, and two repeat dots"
        );
        let mut colors = Vec::new();
        let mut min_y = f64::INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        let mut dot_centers = Vec::new();
        for command in &node.commands {
            match command {
                PaintCommand::Line {
                    start, end, color, ..
                } => {
                    colors.push(*color);
                    min_y = min_y.min(start.y).min(end.y);
                    max_y = max_y.max(start.y).max(end.y);
                }
                PaintCommand::Circle { center, fill, .. } => {
                    colors.push(fill.expect("repeat marker dots should be filled"));
                    dot_centers.push(*center);
                }
                _ => panic!("repeat marker should only use lines and dots"),
            }
        }
        assert!(
            colors
                .iter()
                .all(|color| *color == Color::from_rgb8(220, 38, 38))
        );
        assert!(
            min_y < 20.0 && max_y > 60.0,
            "repeat marker should extend beyond the staff"
        );
        assert_eq!(
            dot_centers.len(),
            2,
            "repeat marker should include two dots"
        );
        assert!(
            dot_centers.iter().all(|center| center.x > 100.0),
            "start repeat dots should sit inside the repeat, to the right of the bracket"
        );
        let PaintCommand::Line { start, end, .. } = node.commands[1] else {
            panic!("expected top diagonal");
        };
        assert!(end.x > start.x, "start repeat should open to the right");
        assert!(end.y < start.y, "top arm should slope away from the staff");
        let PaintCommand::Line { start, end, .. } = node.commands[2] else {
            panic!("expected bottom diagonal");
        };
        assert!(end.x > start.x, "start repeat should open to the right");
        assert!(
            end.y > start.y,
            "bottom arm should slope away from the staff"
        );
        assert!(
            (end.x - start.x).abs() <= 10.0,
            "repeat tip arms should be short, MuseScore-style bracket tips"
        );
    }

    #[test]
    fn six_eight_header_tempo_uses_eighth_note_glyph() {
        let mut root = SceneNode::anonymous_group();
        let metadata = crate::metadata::SongMetadata {
            title: Some("Tempo Test".to_string()),
            ..Default::default()
        };
        let tempo = crate::time::Tempo::from_bpm(168.0);
        let count_in = CountInHeaderConfig {
            num_measures: 2,
            beats_per_measure: 6,
            beat_unit: 8,
            has_pushed_chord: false,
            measure_numbers: vec!["1".to_string(), "2".to_string()],
        };
        let margins = PageMargins {
            top: 40.0,
            right: 40.0,
            bottom: 40.0,
            left: 80.0,
        };

        add_title_header_with_count_in(TitleHeaderParams {
            root: &mut root,
            page_x: 0.0,
            page_y: 0.0,
            page_width: 800.0,
            margins: &margins,
            spatium: 10.0,
            metadata: &metadata,
            tempo: Some(&tempo),
            count_in: Some(&count_in),
        });

        fn has_eighth_tempo_glyph(node: &SceneNode) -> bool {
            node.commands.iter().any(|command| {
                matches!(
                    command,
                    PaintCommand::Glyph {
                        codepoint: '\u{eca7}',
                        ..
                    }
                )
            }) || node.children.iter().any(has_eighth_tempo_glyph)
        }
        assert!(
            has_eighth_tempo_glyph(&root),
            "6/8 header tempo should render an eighth-note metronome glyph"
        );
    }

    #[test]
    fn count_in_snippet_height_does_not_push_first_system_down() {
        let metadata = crate::metadata::SongMetadata {
            title: Some("Tempo Test".to_string()),
            ..Default::default()
        };
        let tempo = crate::time::Tempo::from_bpm(168.0);
        let margins = PageMargins {
            top: 40.0,
            right: 40.0,
            bottom: 40.0,
            left: 80.0,
        };

        let mut root_without_count_in = SceneNode::anonymous_group();
        let (base_height, _) = add_title_header_with_count_in(TitleHeaderParams {
            root: &mut root_without_count_in,
            page_x: 0.0,
            page_y: 0.0,
            page_width: 800.0,
            margins: &margins,
            spatium: 10.0,
            metadata: &metadata,
            tempo: Some(&tempo),
            count_in: None,
        });

        let mut root_with_count_in = SceneNode::anonymous_group();
        let count_in = CountInHeaderConfig {
            num_measures: 2,
            beats_per_measure: 6,
            beat_unit: 8,
            has_pushed_chord: false,
            measure_numbers: vec!["1".to_string(), "2".to_string()],
        };
        let (count_in_height, geos) = add_title_header_with_count_in(TitleHeaderParams {
            root: &mut root_with_count_in,
            page_x: 0.0,
            page_y: 0.0,
            page_width: 800.0,
            margins: &margins,
            spatium: 10.0,
            metadata: &metadata,
            tempo: Some(&tempo),
            count_in: Some(&count_in),
        });

        assert!(
            !geos.is_empty(),
            "count-in snippet should still render beat geometries"
        );
        assert!(
            (count_in_height - base_height).abs() <= 0.01,
            "count-in overlay should not change reserved header height: base={base_height:.1}, with_count_in={count_in_height:.1}"
        );
    }
}
