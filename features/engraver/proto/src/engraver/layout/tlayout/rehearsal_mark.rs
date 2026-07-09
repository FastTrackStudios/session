//! Rehearsal Mark / Section Label Layout
//!
//! Provides layout functions for rendering section markers like "VS 1", "CH 2", "BR 1",
//! "INTRO", "OUTRO", etc. These are drawn as capsule-shaped labels (rounded rectangles
//! with text inside).
//!
//! # Rendering via Vello
//!
//! The layout functions generate `PaintCommand`s that the VelloSceneRenderer handles:
//! - `PaintCommand::Rect` with `corner_radius` for the capsule background
//! - `PaintCommand::Text` for the label text
//!
//! Vello efficiently renders these using GPU-accelerated vector graphics.

use kurbo::{Point, Rect};
use peniko::Color;

use crate::engraver::layout::context::LayoutContext;
use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::PaintCommand;
use crate::engraver::ui::format_rehearsal_label_with_letter;

/// Parameters for rehearsal mark layout.
#[derive(Debug, Clone)]
pub struct RehearsalMarkParams {
    /// Unique identifier
    pub id: u64,
    /// The label text to display (e.g., "VS 1", "INTRO")
    pub text: String,
    /// Position X (left edge of capsule or center, depending on alignment)
    pub x: f64,
    /// Position Y (baseline or top, depending on config)
    pub y: f64,
    /// Style configuration
    pub style: RehearsalMarkStyle,
}

impl Default for RehearsalMarkParams {
    fn default() -> Self {
        Self {
            id: 0,
            text: String::new(),
            x: 0.0,
            y: 0.0,
            style: RehearsalMarkStyle::default(),
        }
    }
}

/// Style configuration for rehearsal marks.
#[derive(Debug, Clone)]
pub struct RehearsalMarkStyle {
    /// Background color of the capsule
    pub background_color: Color,
    /// Text color
    pub text_color: Color,
    /// Border color (None = no border)
    pub border_color: Option<Color>,
    /// Border width
    pub border_width: f64,
    /// Font size for the text
    pub font_size: f64,
    /// Horizontal padding inside the capsule
    pub padding_h: f64,
    /// Vertical padding inside the capsule
    pub padding_v: f64,
    /// Estimated character width ratio (actual width / font_size)
    pub char_width_ratio: f64,
}

impl Default for RehearsalMarkStyle {
    fn default() -> Self {
        Self {
            background_color: Color::from_rgb8(66, 66, 66),
            text_color: Color::WHITE,
            border_color: None,
            border_width: 1.0,
            font_size: 12.0,
            padding_h: 8.0,
            padding_v: 4.0,
            char_width_ratio: 0.6,
        }
    }
}

/// Layout data returned from rehearsal mark layout.
#[derive(Debug, Clone)]
pub struct RehearsalMarkLayoutData {
    /// Bounding box of the entire element
    pub bounds: Rect,
    /// Width of the capsule
    pub width: f64,
    /// Height of the capsule
    pub height: f64,
}

/// Layout a rehearsal mark / section label.
///
/// Returns layout data and a scene node containing the capsule and text.
///
/// # Arguments
///
/// * `params` - Rehearsal mark parameters
/// * `_ctx` - Layout context (unused currently, but available for font metrics)
///
/// # Returns
///
/// A tuple of (layout data, scene node).
#[must_use]
pub fn layout_rehearsal_mark(
    params: &RehearsalMarkParams,
    _ctx: &LayoutContext<'_>,
) -> (RehearsalMarkLayoutData, SceneNode) {
    let style = &params.style;

    // Estimate text width based on character count and font size
    // In a full implementation, we'd use actual font metrics from glyphon/skrifa
    let estimated_text_width = params.text.len() as f64 * style.font_size * style.char_width_ratio;

    // Compute capsule dimensions
    let capsule_width = estimated_text_width + style.padding_h * 2.0;
    let capsule_height = style.font_size + style.padding_v * 2.0;
    let corner_radius = capsule_height / 4.0;

    // Position the capsule (params.x, params.y is top-left)
    let capsule_x = params.x;
    let capsule_y = params.y;

    // Create the capsule rectangle
    let capsule_rect = Rect::new(
        capsule_x,
        capsule_y,
        capsule_x + capsule_width,
        capsule_y + capsule_height,
    );

    // Build paint commands
    let mut commands = Vec::new();

    // 1. Background capsule
    commands.push(PaintCommand::Rect {
        rect: capsule_rect,
        fill: Some(style.background_color),
        stroke: style.border_color,
        stroke_width: style.border_width,
        corner_radius: Some(corner_radius),
    });

    // 2. Text (centered in capsule)
    let text_x = capsule_x + style.padding_h;
    let text_y = capsule_y + style.padding_v + style.font_size * 0.85; // Approximate baseline
    commands.push(PaintCommand::text(
        params.text.clone(),
        "sans-serif",
        style.font_size,
        Point::new(text_x, text_y),
        style.text_color,
    ));

    // Create layout data
    let layout_data = RehearsalMarkLayoutData {
        bounds: capsule_rect,
        width: capsule_width,
        height: capsule_height,
    };

    // Create scene node
    let node = SceneNode::anonymous_leaf(commands);

    (layout_data, node)
}

/// Convenience function to create a section label with standard formatting.
///
/// Uses the format_rehearsal_label function to create consistent labels:
/// - Intro/Outro: Full name uppercase ("INTRO", "OUTRO")
/// - Others: Abbreviation + number ("VS 1", "CH 2")
///
/// # Arguments
///
/// * `section_type` - Type name (e.g., "Verse", "Chorus", "Bridge")
/// * `abbreviation` - Short form (e.g., "VS", "CH", "BR")
/// * `number` - Optional section number
/// * `x` - X position
/// * `y` - Y position
/// * `style` - Style configuration
/// * `ctx` - Layout context
///
/// # Returns
///
/// A tuple of (layout data, scene node).
#[must_use]
pub fn layout_section_label(
    section_type: &str,
    abbreviation: &str,
    number: Option<u32>,
    x: f64,
    y: f64,
    style: Option<RehearsalMarkStyle>,
    ctx: &LayoutContext<'_>,
) -> (RehearsalMarkLayoutData, SceneNode) {
    let text = format_rehearsal_label_with_letter(section_type, abbreviation, number, None);
    let params = RehearsalMarkParams {
        id: 0,
        text,
        x,
        y,
        style: style.unwrap_or_default(),
    };
    layout_rehearsal_mark(&params, ctx)
}

/// Parameters for margin-positioned section labels.
#[derive(Debug, Clone)]
pub struct MarginLabelParams {
    /// Section type name (e.g., "Verse", "Chorus")
    pub section_type: String,
    /// Abbreviation (e.g., "VS", "CH")
    pub abbreviation: String,
    /// Optional section number
    pub number: Option<u32>,
    /// Optional section letter for consecutive repeats (A, B, C, etc.)
    pub letter: Option<char>,
    /// Optional comment/annotation to display below the capsule (e.g., "Down", "Build", "Horns", "Half-time")
    pub comment: Option<String>,
    /// Optional already-formatted label text. Used when the source chart has
    /// stacked pass labels such as `VS 1b\nVS 2`.
    pub label_override: Option<String>,
    /// Left edge of the page
    pub page_x: f64,
    /// Available margin width (distance from page edge to content)
    pub margin_width: f64,
    /// Top of the staff (Y coordinate)
    pub staff_y: f64,
    /// Height of the staff (4 * spatium)
    pub staff_height: f64,
    /// Horizontal padding from page edge
    pub padding_h: f64,
    /// Vertical padding from staff top/bottom
    pub padding_v: f64,
    /// Style configuration
    pub style: RehearsalMarkStyle,
}

impl Default for MarginLabelParams {
    fn default() -> Self {
        Self {
            section_type: String::new(),
            abbreviation: String::new(),
            number: None,
            letter: None,
            comment: None,
            label_override: None,
            page_x: 0.0,
            margin_width: 50.0,
            staff_y: 0.0,
            staff_height: 20.0,
            padding_h: 4.0,
            padding_v: 3.0,
            style: RehearsalMarkStyle::default(),
        }
    }
}

/// Layout a section label that fits within the left margin of a staff.
///
/// The label will be sized to fit within the available margin space,
/// with multiline support for long text. The font is maximized to fill
/// the available space. Section letters (A, B, C, D) are placed on a new line.
///
/// # Arguments
///
/// * `params` - Margin label parameters
/// * `_ctx` - Layout context
///
/// # Returns
///
/// A tuple of (layout data, scene node).
#[must_use]
pub fn layout_margin_label(
    params: &MarginLabelParams,
    _ctx: &LayoutContext<'_>,
) -> (RehearsalMarkLayoutData, SceneNode) {
    let raw_text = params.label_override.clone().unwrap_or_else(|| {
        format_rehearsal_label_with_letter(
            &params.section_type,
            &params.abbreviation,
            params.number,
            params.letter,
        )
    });
    let style = &params.style;

    // Calculate available space in the margin
    let capsule_x = params.page_x + params.padding_h;
    let available_width = params.margin_width - (params.padding_h * 2.0);
    // Align top of capsule with top of staff (capsule grows downward)
    let capsule_y = params.staff_y;

    let internal_padding = 4.0;
    let text_available_width = available_width - internal_padding * 2.0;

    // Split text into lines for multiline layout
    let lines = split_into_lines(&raw_text, text_available_width, style.char_width_ratio);
    let num_lines = lines.len().max(1);

    // Use a FIXED font size - don't shrink for multiline
    // This ensures consistent text size regardless of number of lines
    let base_font_size = params.staff_height * 0.7;

    // Only scale down if a line is too wide to fit (width constraint only)
    let mut font_size = base_font_size;
    for line in &lines {
        if !line.is_empty() {
            let line_width = line.len() as f64 * font_size * style.char_width_ratio;
            if line_width > text_available_width {
                let scale = text_available_width / (line.len() as f64 * style.char_width_ratio);
                font_size = font_size.min(scale);
            }
        }
    }

    // Calculate capsule height based on actual text needs
    // Capsule grows DOWNWARD from the top edge to accommodate multiple lines
    let line_spacing = font_size * 0.2;
    let total_text_height = if num_lines == 1 {
        font_size
    } else {
        (num_lines as f64 * font_size) + ((num_lines - 1) as f64 * line_spacing)
    };
    let capsule_height = total_text_height + params.padding_v * 2.0;
    let capsule_width = available_width;
    let corner_radius = (capsule_height / (num_lines as f64 + 2.0)).min(capsule_height / 4.0);

    // Create the capsule rectangle
    let capsule_rect = Rect::new(
        capsule_x,
        capsule_y,
        capsule_x + capsule_width,
        capsule_y + capsule_height,
    );

    // Build paint commands
    let mut commands = Vec::new();

    // 1. Background capsule
    commands.push(PaintCommand::Rect {
        rect: capsule_rect,
        fill: Some(style.background_color),
        stroke: style.border_color,
        stroke_width: style.border_width,
        corner_radius: Some(corner_radius),
    });

    // 2. Text lines (centered in capsule)
    // Calculate the starting Y position to vertically center all lines
    // The text baseline is at the bottom of the text, so we offset by 0.75 * font_size
    let block_top_y = capsule_y + (capsule_height - total_text_height) / 2.0;

    // Capsule center X for horizontal text centering
    let capsule_center_x = capsule_x + capsule_width / 2.0;

    for (i, line) in lines.iter().enumerate() {
        // Position vertically - baseline is at 0.75 of font size from top
        let line_top = block_top_y + (i as f64 * (font_size + line_spacing));
        let text_y = line_top + font_size * 0.75;

        // Use centered text - renderer will handle actual font metrics for centering
        commands.push(PaintCommand::text_centered(
            line.clone(),
            "sans-serif",
            font_size,
            Point::new(capsule_center_x, text_y),
            style.text_color,
        ));
    }

    // 3. Render comment text below the capsule (if present)
    // Comment is rendered in a smaller font below the capsule, ALL CAPS, black text
    let mut total_height = capsule_height;
    if let Some(ref comment) = params.comment {
        let comment_font_size = base_font_size * 0.75; // Slightly smaller than main text
        let comment_spacing = 4.0; // Gap between capsule and comment
        let comment_y = capsule_y + capsule_height + comment_spacing + comment_font_size * 0.75;

        commands.push(PaintCommand::section_comment(
            comment.to_uppercase(),
            comment_font_size,
            Point::new(capsule_center_x, comment_y),
            Color::BLACK,
        ));

        total_height += comment_spacing + comment_font_size;
    }

    // Create layout data - include note height in bounds if present
    let total_bounds = Rect::new(
        capsule_x,
        capsule_y,
        capsule_x + capsule_width,
        capsule_y + total_height,
    );
    let layout_data = RehearsalMarkLayoutData {
        bounds: total_bounds,
        width: capsule_width,
        height: total_height,
    };

    // Create scene node
    let node = SceneNode::anonymous_leaf(commands);

    (layout_data, node)
}

/// Split text into lines for multiline layout.
///
/// Rules:
/// - Section letters (single A-D at end) go on their own line
/// - Words are broken at spaces
/// - Long words that don't fit are kept together (will be scaled down)
fn split_into_lines(text: &str, available_width: f64, char_width_ratio: f64) -> Vec<String> {
    if text.contains('\n') {
        return text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect();
    }

    let mut lines = Vec::new();

    // Check for section letter at end (e.g., "CH 1 B" -> ["CH 1", "B"])
    let parts: Vec<&str> = text.split_whitespace().collect();
    if let (Some(last), true) = (parts.last(), parts.len() >= 2) {
        // Check if last part is a single letter A-Z (section identifier)
        if last.len() == 1 && last.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
            // Put section letter on its own line
            let main_text: String = parts[..parts.len() - 1].join(" ");
            lines.push(main_text);
            lines.push((*last).to_string());
            return lines;
        }
    }

    // Check if text fits on one line
    let estimated_width = text.len() as f64 * 12.0 * char_width_ratio; // Use base font size for estimation
    if estimated_width <= available_width {
        lines.push(text.to_string());
        return lines;
    }

    // Split at spaces for multi-word text
    if parts.len() >= 2 {
        // Try splitting into two lines at the middle
        let mid = parts.len() / 2;
        let line1: String = parts[..mid].join(" ");
        let line2: String = parts[mid..].join(" ");
        lines.push(line1);
        lines.push(line2);
    } else {
        // Single word - keep as is (will be scaled down)
        lines.push(text.to_string());
    }

    lines
}

/// Pre-defined color themes for rehearsal marks.
pub mod themes {
    use super::RehearsalMarkStyle;
    use peniko::Color;

    // ═══════════════════════════════════════════════════════════════════════════
    // Tailwind Color Palette
    // These follow Tailwind CSS color naming (50-950 shades)
    // https://tailwindcss.com/docs/customizing-colors
    // ═══════════════════════════════════════════════════════════════════════════

    pub mod tailwind {
        use peniko::Color;

        // Slate (neutral cool gray)
        pub const SLATE_200: Color = Color::from_rgb8(0xe2, 0xe8, 0xf0);
        pub const SLATE_400: Color = Color::from_rgb8(0x94, 0xa3, 0xb8);
        pub const SLATE_600: Color = Color::from_rgb8(0x47, 0x55, 0x69);
        pub const SLATE_800: Color = Color::from_rgb8(0x1e, 0x29, 0x3b);

        // Orange
        pub const ORANGE_200: Color = Color::from_rgb8(0xfe, 0xd7, 0xaa);
        pub const ORANGE_400: Color = Color::from_rgb8(0xfb, 0x92, 0x3c);
        pub const ORANGE_600: Color = Color::from_rgb8(0xea, 0x58, 0x0c);
        pub const ORANGE_800: Color = Color::from_rgb8(0x9a, 0x34, 0x12);

        // Amber
        pub const AMBER_200: Color = Color::from_rgb8(0xfd, 0xe6, 0x8a);
        pub const AMBER_400: Color = Color::from_rgb8(0xfb, 0xbf, 0x24);
        pub const AMBER_600: Color = Color::from_rgb8(0xd9, 0x77, 0x06);
        pub const AMBER_800: Color = Color::from_rgb8(0x92, 0x40, 0x0e);

        // Yellow
        pub const YELLOW_200: Color = Color::from_rgb8(0xfe, 0xf0, 0x8a);
        pub const YELLOW_400: Color = Color::from_rgb8(0xfa, 0xcc, 0x15);
        pub const YELLOW_600: Color = Color::from_rgb8(0xca, 0x8a, 0x04);
        pub const YELLOW_800: Color = Color::from_rgb8(0x85, 0x4d, 0x0e);

        // Emerald
        pub const EMERALD_200: Color = Color::from_rgb8(0xa7, 0xf3, 0xd0);
        pub const EMERALD_400: Color = Color::from_rgb8(0x34, 0xd3, 0x99);
        pub const EMERALD_600: Color = Color::from_rgb8(0x05, 0x96, 0x69);
        pub const EMERALD_800: Color = Color::from_rgb8(0x06, 0x5f, 0x46);

        // Blue
        pub const BLUE_200: Color = Color::from_rgb8(0xbf, 0xdb, 0xfe);
        pub const BLUE_400: Color = Color::from_rgb8(0x60, 0xa5, 0xfa);
        pub const BLUE_500: Color = Color::from_rgb8(0x3b, 0x82, 0xf6);
        pub const BLUE_600: Color = Color::from_rgb8(0x25, 0x63, 0xeb);
        pub const BLUE_800: Color = Color::from_rgb8(0x1e, 0x40, 0xaf);

        // Violet
        pub const VIOLET_200: Color = Color::from_rgb8(0xdd, 0xd6, 0xfe);
        pub const VIOLET_400: Color = Color::from_rgb8(0xa7, 0x8b, 0xfa);
        pub const VIOLET_600: Color = Color::from_rgb8(0x7c, 0x3a, 0xed);
        pub const VIOLET_800: Color = Color::from_rgb8(0x5b, 0x21, 0xb6);

        // Rose
        pub const ROSE_200: Color = Color::from_rgb8(0xfe, 0xcd, 0xd3);
        pub const ROSE_400: Color = Color::from_rgb8(0xfb, 0x71, 0x85);
        pub const ROSE_600: Color = Color::from_rgb8(0xe1, 0x1d, 0x48);
        pub const ROSE_800: Color = Color::from_rgb8(0x9f, 0x12, 0x39);
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // Section Color Palette (using Tailwind colors)
    // Designed for clear visual hierarchy and accessibility
    // ═══════════════════════════════════════════════════════════════════════════

    /// Intro section - Orange 400 (warm, welcoming start)
    #[must_use]
    pub fn intro() -> RehearsalMarkStyle {
        RehearsalMarkStyle {
            background_color: tailwind::ORANGE_400,
            text_color: Color::WHITE,
            border_color: None,
            ..Default::default()
        }
    }

    /// Verse section - Emerald 400 (fresh, natural progression)
    #[must_use]
    pub fn verse() -> RehearsalMarkStyle {
        RehearsalMarkStyle {
            background_color: tailwind::EMERALD_400,
            text_color: Color::WHITE,
            border_color: None,
            ..Default::default()
        }
    }

    /// Pre-Verse section - Emerald 200 (lighter verse)
    #[must_use]
    pub fn pre_verse() -> RehearsalMarkStyle {
        RehearsalMarkStyle {
            background_color: tailwind::EMERALD_200,
            text_color: tailwind::EMERALD_800,
            border_color: None,
            ..Default::default()
        }
    }

    /// Chorus section - Blue 500 (strong, memorable hook)
    #[must_use]
    pub fn chorus() -> RehearsalMarkStyle {
        RehearsalMarkStyle {
            background_color: tailwind::BLUE_500,
            text_color: Color::WHITE,
            border_color: None,
            ..Default::default()
        }
    }

    /// Pre-Chorus / Post-Chorus - Blue 200 (lighter chorus)
    #[must_use]
    pub fn pre_chorus() -> RehearsalMarkStyle {
        RehearsalMarkStyle {
            background_color: tailwind::BLUE_200,
            text_color: tailwind::BLUE_800,
            border_color: None,
            ..Default::default()
        }
    }

    /// Bridge section - Violet 400 (contrast, transitional)
    #[must_use]
    pub fn bridge() -> RehearsalMarkStyle {
        RehearsalMarkStyle {
            background_color: tailwind::VIOLET_400,
            text_color: Color::WHITE,
            border_color: None,
            ..Default::default()
        }
    }

    /// Pre-Bridge / Post-Bridge - Violet 200 (lighter bridge)
    #[must_use]
    pub fn pre_bridge() -> RehearsalMarkStyle {
        RehearsalMarkStyle {
            background_color: tailwind::VIOLET_200,
            text_color: tailwind::VIOLET_800,
            border_color: None,
            ..Default::default()
        }
    }

    /// Outro section - Amber 400 (warm conclusion)
    #[must_use]
    pub fn outro() -> RehearsalMarkStyle {
        RehearsalMarkStyle {
            background_color: tailwind::AMBER_400,
            text_color: tailwind::AMBER_800,
            border_color: None,
            ..Default::default()
        }
    }

    /// Instrumental section - Orange 200 (related to intro, lighter)
    #[must_use]
    pub fn instrumental() -> RehearsalMarkStyle {
        RehearsalMarkStyle {
            background_color: tailwind::ORANGE_200,
            text_color: tailwind::ORANGE_800,
            border_color: None,
            ..Default::default()
        }
    }

    /// Solo section - Rose 400 (vibrant, standout)
    #[must_use]
    pub fn solo() -> RehearsalMarkStyle {
        RehearsalMarkStyle {
            background_color: tailwind::ROSE_400,
            text_color: Color::WHITE,
            border_color: None,
            ..Default::default()
        }
    }

    /// Interlude section - Yellow 400 (bright pause/break)
    #[must_use]
    pub fn interlude() -> RehearsalMarkStyle {
        RehearsalMarkStyle {
            background_color: tailwind::YELLOW_400,
            text_color: tailwind::YELLOW_800,
            border_color: None,
            ..Default::default()
        }
    }

    /// Breakdown / Hits - Slate 400 (neutral, rhythmic sections)
    #[must_use]
    pub fn breakdown() -> RehearsalMarkStyle {
        RehearsalMarkStyle {
            background_color: tailwind::SLATE_400,
            text_color: Color::WHITE,
            border_color: None,
            ..Default::default()
        }
    }

    /// Custom sections (Solo, etc.) - Slate 200 with border
    #[must_use]
    pub fn custom() -> RehearsalMarkStyle {
        RehearsalMarkStyle {
            background_color: tailwind::SLATE_200,
            text_color: tailwind::SLATE_800,
            border_color: Some(tailwind::SLATE_400),
            border_width: 1.0,
            ..Default::default()
        }
    }

    /// Outline-only style (transparent background with border).
    #[must_use]
    pub fn outline() -> RehearsalMarkStyle {
        RehearsalMarkStyle {
            background_color: Color::TRANSPARENT,
            text_color: Color::BLACK,
            border_color: Some(Color::BLACK),
            border_width: 1.5,
            ..Default::default()
        }
    }

    // ═══════════════════════════════════════════════════════════════════════════
    // Legacy themes (for backwards compatibility)
    // ═══════════════════════════════════════════════════════════════════════════

    /// Dark theme with white text on dark background.
    #[must_use]
    pub fn dark() -> RehearsalMarkStyle {
        RehearsalMarkStyle {
            background_color: tailwind::SLATE_800,
            text_color: Color::WHITE,
            border_color: None,
            ..Default::default()
        }
    }

    /// Light theme with dark text on light background.
    #[must_use]
    pub fn light() -> RehearsalMarkStyle {
        RehearsalMarkStyle {
            background_color: tailwind::SLATE_200,
            text_color: tailwind::SLATE_800,
            border_color: Some(tailwind::SLATE_400),
            ..Default::default()
        }
    }

    /// Blue accent theme (legacy).
    #[must_use]
    pub fn blue() -> RehearsalMarkStyle {
        chorus()
    }

    /// Green accent theme (legacy).
    #[must_use]
    pub fn green() -> RehearsalMarkStyle {
        verse()
    }

    /// Purple accent theme (legacy).
    #[must_use]
    pub fn purple() -> RehearsalMarkStyle {
        bridge()
    }

    /// Gray theme (legacy alias for breakdown).
    #[must_use]
    pub fn gray() -> RehearsalMarkStyle {
        breakdown()
    }

    // Legacy pastel aliases (deprecated, use semantic names instead)
    #[must_use]
    pub fn pastel_orange() -> RehearsalMarkStyle {
        intro()
    }
    #[must_use]
    pub fn pastel_orange_light() -> RehearsalMarkStyle {
        outro()
    }
    #[must_use]
    pub fn pastel_green() -> RehearsalMarkStyle {
        verse()
    }
    #[must_use]
    pub fn pastel_green_light() -> RehearsalMarkStyle {
        pre_verse()
    }
    #[must_use]
    pub fn pastel_blue() -> RehearsalMarkStyle {
        chorus()
    }
    #[must_use]
    pub fn pastel_blue_light() -> RehearsalMarkStyle {
        pre_chorus()
    }
    #[must_use]
    pub fn pastel_purple() -> RehearsalMarkStyle {
        bridge()
    }
    #[must_use]
    pub fn pastel_purple_light() -> RehearsalMarkStyle {
        pre_bridge()
    }
    #[must_use]
    pub fn pastel_yellow() -> RehearsalMarkStyle {
        interlude()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engraver::layout::context::LayoutConfiguration;

    fn make_ctx() -> LayoutContext<'static> {
        use crate::engraver::style::MStyle;
        let style = Box::leak(Box::new(MStyle::default()));
        LayoutContext::new_for_test(LayoutConfiguration::default(), style)
    }

    #[test]
    fn test_basic_rehearsal_mark() {
        let ctx = make_ctx();
        let params = RehearsalMarkParams {
            id: 1,
            text: "VS 1".to_string(),
            x: 100.0,
            y: 50.0,
            ..Default::default()
        };

        let (layout, node) = layout_rehearsal_mark(&params, &ctx);

        assert!(layout.width > 0.0);
        assert!(layout.height > 0.0);
        assert!(!node.commands.is_empty());
    }

    #[test]
    fn test_section_label_verse() {
        let ctx = make_ctx();
        let (layout, _node) = layout_section_label("Verse", "VS", Some(1), 0.0, 0.0, None, &ctx);
        assert!(layout.width > 0.0);
    }

    #[test]
    fn test_section_label_intro() {
        let ctx = make_ctx();
        let (layout, _node) = layout_section_label("Intro", "IN", None, 0.0, 0.0, None, &ctx);
        // Intro should produce "INTRO" which is wider than "VS 1"
        assert!(layout.width > 0.0);
    }

    #[test]
    fn margin_label_preserves_stacked_repeat_pass_labels() {
        let ctx = make_ctx();
        let (_layout, node) = layout_margin_label(
            &MarginLabelParams {
                section_type: "Verse".to_string(),
                abbreviation: "VS".to_string(),
                number: Some(1),
                label_override: Some("VS 1b\nVS 2".to_string()),
                ..Default::default()
            },
            &ctx,
        );

        let rendered_text = node
            .commands
            .iter()
            .filter_map(|cmd| match cmd {
                PaintCommand::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(rendered_text, vec!["VS 1b", "VS 2"]);
    }

    #[test]
    fn test_themes() {
        let dark = themes::dark();
        let light = themes::light();
        let outline = themes::outline();

        assert!(dark.border_color.is_none());
        assert!(light.border_color.is_some());
        assert!(outline.background_color == Color::TRANSPARENT);
    }
}
