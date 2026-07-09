//! Paint commands for rendering scene graph nodes.
//!
//! Provides backend-agnostic drawing commands that can be rendered to:
//! - WGPU (via tessellation to vertex buffers)
//! - SVG (via serialization to XML elements)
//!
//! Based on patterns from anyrender's `PaintScene` trait.

use kurbo::{BezPath, Point, Rect, Shape};
use peniko::Color;
use serde::{Deserialize, Serialize};

/// A paint command representing a single drawing operation.
///
/// Commands are backend-agnostic and can be rendered to WGPU or SVG.
#[derive(Debug, Clone)]
pub enum PaintCommand {
    /// Fill a path with a solid color.
    Fill {
        /// The path to fill
        path: BezPath,
        /// Fill color
        color: Color,
        /// Fill rule (non-zero or even-odd)
        fill_rule: FillRule,
    },

    /// Stroke a path.
    Stroke {
        /// The path to stroke
        path: BezPath,
        /// Stroke color
        color: Color,
        /// Stroke width
        width: f64,
        /// Line cap style
        line_cap: LineCap,
        /// Line join style
        line_join: LineJoin,
        /// Dash pattern (empty for solid)
        dash_pattern: Vec<f64>,
        /// Dash offset
        dash_offset: f64,
    },

    /// Draw a SMuFL music glyph.
    Glyph {
        /// SMuFL glyph identifier (Unicode codepoint)
        codepoint: char,
        /// Position (baseline left)
        position: Point,
        /// Size in staff spaces (spatium)
        size: f64,
        /// Color
        color: Color,
    },

    /// Draw text (non-music).
    Text {
        /// Text content
        text: String,
        /// Font family name
        font_family: String,
        /// Font size in points
        font_size: f64,
        /// Position
        position: Point,
        /// Color
        color: Color,
        /// Text anchor point
        anchor: TextAnchor,
        /// Font weight
        weight: FontWeight,
        /// Font style (normal, italic)
        style: FontStyle,
    },

    /// Draw a line (optimized for common case).
    Line {
        /// Start point
        start: Point,
        /// End point
        end: Point,
        /// Line width
        width: f64,
        /// Color
        color: Color,
        /// Line cap style
        line_cap: LineCap,
    },

    /// Draw a rectangle (optimized for common case).
    Rect {
        /// Rectangle bounds
        rect: Rect,
        /// Fill color (None for no fill)
        fill: Option<Color>,
        /// Stroke color (None for no stroke)
        stroke: Option<Color>,
        /// Stroke width (if stroking)
        stroke_width: f64,
        /// Corner radius for rounded rectangles
        corner_radius: Option<f64>,
    },

    /// Draw a circle.
    Circle {
        /// Center point
        center: Point,
        /// Radius
        radius: f64,
        /// Fill color (None for no fill)
        fill: Option<Color>,
        /// Stroke color (None for no stroke)
        stroke: Option<Color>,
        /// Stroke width (if stroking)
        stroke_width: f64,
    },

    /// Draw an ellipse.
    Ellipse {
        /// Center point
        center: Point,
        /// Horizontal radius
        radius_x: f64,
        /// Vertical radius
        radius_y: f64,
        /// Fill color (None for no fill)
        fill: Option<Color>,
        /// Stroke color (None for no stroke)
        stroke: Option<Color>,
        /// Stroke width (if stroking)
        stroke_width: f64,
    },
}

/// Fill rule for path filling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FillRule {
    /// Non-zero winding rule (default)
    #[default]
    NonZero,
    /// Even-odd rule
    EvenOdd,
}

impl FillRule {
    /// Get the SVG fill-rule attribute value.
    #[must_use]
    pub const fn svg_value(&self) -> &'static str {
        match self {
            Self::NonZero => "nonzero",
            Self::EvenOdd => "evenodd",
        }
    }
}

/// Line cap style for strokes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LineCap {
    /// Flat cap at line end
    #[default]
    Butt,
    /// Rounded cap
    Round,
    /// Square cap extending past line end
    Square,
}

impl LineCap {
    /// Get the SVG stroke-linecap attribute value.
    #[must_use]
    pub const fn svg_value(&self) -> &'static str {
        match self {
            Self::Butt => "butt",
            Self::Round => "round",
            Self::Square => "square",
        }
    }
}

/// Line join style for strokes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LineJoin {
    /// Mitered join
    #[default]
    Miter,
    /// Rounded join
    Round,
    /// Beveled join
    Bevel,
}

impl LineJoin {
    /// Get the SVG stroke-linejoin attribute value.
    #[must_use]
    pub const fn svg_value(&self) -> &'static str {
        match self {
            Self::Miter => "miter",
            Self::Round => "round",
            Self::Bevel => "bevel",
        }
    }
}

/// Text anchor point for text positioning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum TextAnchor {
    /// Anchor at start (left for LTR text)
    #[default]
    Start,
    /// Anchor at middle
    Middle,
    /// Anchor at end (right for LTR text)
    End,
}

impl TextAnchor {
    /// Get the SVG text-anchor attribute value.
    #[must_use]
    pub const fn svg_value(&self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Middle => "middle",
            Self::End => "end",
        }
    }
}

/// Font weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FontWeight {
    /// Normal weight (400)
    #[default]
    Normal,
    /// Bold weight (700)
    Bold,
    /// Light weight (300)
    Light,
    /// Custom weight
    Custom(u16),
}

impl FontWeight {
    /// Get the numeric weight value.
    #[must_use]
    pub const fn value(&self) -> u16 {
        match self {
            Self::Light => 300,
            Self::Normal => 400,
            Self::Bold => 700,
            Self::Custom(w) => *w,
        }
    }

    /// Get the SVG font-weight attribute value.
    #[must_use]
    pub fn svg_value(&self) -> String {
        self.value().to_string()
    }
}

/// Font style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FontStyle {
    /// Normal (upright)
    #[default]
    Normal,
    /// Italic
    Italic,
    /// Oblique
    Oblique,
}

impl FontStyle {
    /// Get the SVG font-style attribute value.
    #[must_use]
    pub const fn svg_value(&self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Italic => "italic",
            Self::Oblique => "oblique",
        }
    }
}

// Builder methods for common paint commands
impl PaintCommand {
    /// Create a filled rectangle.
    #[must_use]
    pub fn filled_rect(rect: Rect, color: Color) -> Self {
        Self::Rect {
            rect,
            fill: Some(color),
            stroke: None,
            stroke_width: 0.0,
            corner_radius: None,
        }
    }

    /// Create a stroked rectangle.
    #[must_use]
    pub fn stroked_rect(rect: Rect, color: Color, width: f64) -> Self {
        Self::Rect {
            rect,
            fill: None,
            stroke: Some(color),
            stroke_width: width,
            corner_radius: None,
        }
    }

    /// Create a filled and stroked rectangle.
    #[must_use]
    pub fn rect_with_stroke(rect: Rect, fill: Color, stroke: Color, stroke_width: f64) -> Self {
        Self::Rect {
            rect,
            fill: Some(fill),
            stroke: Some(stroke),
            stroke_width,
            corner_radius: None,
        }
    }

    /// Create a rounded rectangle.
    #[must_use]
    pub fn rounded_rect(rect: Rect, color: Color, radius: f64) -> Self {
        Self::Rect {
            rect,
            fill: Some(color),
            stroke: None,
            stroke_width: 0.0,
            corner_radius: Some(radius),
        }
    }

    /// Create a simple line.
    #[must_use]
    pub fn line(start: Point, end: Point, color: Color, width: f64) -> Self {
        Self::Line {
            start,
            end,
            width,
            color,
            line_cap: LineCap::Butt,
        }
    }

    /// Create a line with rounded ends.
    #[must_use]
    pub fn rounded_line(start: Point, end: Point, color: Color, width: f64) -> Self {
        Self::Line {
            start,
            end,
            width,
            color,
            line_cap: LineCap::Round,
        }
    }

    /// Create a filled circle.
    #[must_use]
    pub fn filled_circle(center: Point, radius: f64, color: Color) -> Self {
        Self::Circle {
            center,
            radius,
            fill: Some(color),
            stroke: None,
            stroke_width: 0.0,
        }
    }

    /// Create a music glyph (SMuFL).
    #[must_use]
    pub fn glyph(codepoint: char, position: Point, size: f64, color: Color) -> Self {
        Self::Glyph {
            codepoint,
            position,
            size,
            color,
        }
    }

    /// Create a filled path.
    #[must_use]
    pub fn filled_path(path: BezPath, color: Color) -> Self {
        Self::Fill {
            path,
            color,
            fill_rule: FillRule::NonZero,
        }
    }

    /// Create a stroked path.
    #[must_use]
    pub fn stroked_path(path: BezPath, color: Color, width: f64) -> Self {
        Self::Stroke {
            path,
            color,
            width,
            line_cap: LineCap::Butt,
            line_join: LineJoin::Miter,
            dash_pattern: Vec::new(),
            dash_offset: 0.0,
        }
    }

    /// Create a text command.
    #[must_use]
    pub fn text(
        text: impl Into<String>,
        font_family: impl Into<String>,
        font_size: f64,
        position: Point,
        color: Color,
    ) -> Self {
        Self::Text {
            text: text.into(),
            font_family: font_family.into(),
            font_size,
            position,
            color,
            anchor: TextAnchor::Start,
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        }
    }

    /// Create an italic, left-anchored text command.
    #[must_use]
    pub fn text_italic(
        text: impl Into<String>,
        font_family: impl Into<String>,
        font_size: f64,
        position: Point,
        color: Color,
    ) -> Self {
        Self::Text {
            text: text.into(),
            font_family: font_family.into(),
            font_size,
            position,
            color,
            anchor: TextAnchor::Start,
            weight: FontWeight::Normal,
            style: FontStyle::Italic,
        }
    }

    /// Create a centered text command.
    ///
    /// The position is the center point of the text. The renderer will
    /// use actual font metrics to calculate the correct offset.
    #[must_use]
    pub fn text_centered(
        text: impl Into<String>,
        font_family: impl Into<String>,
        font_size: f64,
        position: Point,
        color: Color,
    ) -> Self {
        Self::Text {
            text: text.into(),
            font_family: font_family.into(),
            font_size,
            position,
            color,
            anchor: TextAnchor::Middle,
            weight: FontWeight::Normal,
            style: FontStyle::Normal,
        }
    }

    /// Create a section comment text command (italic, centered).
    ///
    /// Used for section annotations like "Down", "Build", "Horns", "Half-time".
    /// Rendered in italic style below the section capsule.
    #[must_use]
    pub fn section_comment(
        text: impl Into<String>,
        font_size: f64,
        position: Point,
        color: Color,
    ) -> Self {
        Self::Text {
            text: text.into(),
            font_family: "section-comment".to_string(),
            font_size,
            position,
            color,
            anchor: TextAnchor::Middle,
            weight: FontWeight::Normal,
            style: FontStyle::Italic,
        }
    }

    /// Get the bounding box of this paint command.
    ///
    /// Returns `None` for commands where the bounding box cannot be easily calculated
    /// (e.g., text, glyphs without font metrics).
    #[must_use]
    pub fn bounding_box(&self) -> Option<Rect> {
        match self {
            Self::Fill { path, .. } => Some(path.bounding_box()),
            Self::Stroke { path, width, .. } => {
                let bbox = path.bounding_box();
                let half_width = width / 2.0;
                Some(Rect::new(
                    bbox.x0 - half_width,
                    bbox.y0 - half_width,
                    bbox.x1 + half_width,
                    bbox.y1 + half_width,
                ))
            }
            Self::Line {
                start, end, width, ..
            } => {
                let half_width = width / 2.0;
                Some(Rect::new(
                    start.x.min(end.x) - half_width,
                    start.y.min(end.y) - half_width,
                    start.x.max(end.x) + half_width,
                    start.y.max(end.y) + half_width,
                ))
            }
            Self::Rect {
                rect, stroke_width, ..
            } => {
                let half_width = stroke_width / 2.0;
                Some(Rect::new(
                    rect.x0 - half_width,
                    rect.y0 - half_width,
                    rect.x1 + half_width,
                    rect.y1 + half_width,
                ))
            }
            Self::Circle {
                center,
                radius,
                stroke_width,
                ..
            } => {
                let total_radius = radius + stroke_width / 2.0;
                Some(Rect::new(
                    center.x - total_radius,
                    center.y - total_radius,
                    center.x + total_radius,
                    center.y + total_radius,
                ))
            }
            Self::Ellipse {
                center,
                radius_x,
                radius_y,
                stroke_width,
                ..
            } => {
                let half_stroke = stroke_width / 2.0;
                Some(Rect::new(
                    center.x - radius_x - half_stroke,
                    center.y - radius_y - half_stroke,
                    center.x + radius_x + half_stroke,
                    center.y + radius_y + half_stroke,
                ))
            }
            // Text and glyph bounds require font metrics, return None
            Self::Text { .. } | Self::Glyph { .. } => None,
        }
    }
}

/// Convert a color to SVG hex format.
#[must_use]
pub fn color_to_svg(color: &Color) -> String {
    let rgba = color.to_rgba8();
    let a = color.components[3];

    if (a - 1.0).abs() < f32::EPSILON {
        format!("#{:02x}{:02x}{:02x}", rgba.r, rgba.g, rgba.b)
    } else {
        format!("rgba({},{},{},{a:.3})", rgba.r, rgba.g, rgba.b)
    }
}

/// Convert a BezPath to SVG path data.
#[must_use]
pub fn path_to_svg_d(path: &BezPath, precision: u8) -> String {
    use kurbo::PathEl;

    let mut d = String::new();
    let p = precision as usize;

    for el in path.elements() {
        match el {
            PathEl::MoveTo(pt) => {
                d.push_str(&format!("M{:.p$} {:.p$}", pt.x, pt.y));
            }
            PathEl::LineTo(pt) => {
                d.push_str(&format!("L{:.p$} {:.p$}", pt.x, pt.y));
            }
            PathEl::QuadTo(p1, p2) => {
                d.push_str(&format!(
                    "Q{:.p$} {:.p$} {:.p$} {:.p$}",
                    p1.x, p1.y, p2.x, p2.y
                ));
            }
            PathEl::CurveTo(p1, p2, p3) => {
                d.push_str(&format!(
                    "C{:.p$} {:.p$} {:.p$} {:.p$} {:.p$} {:.p$}",
                    p1.x, p1.y, p2.x, p2.y, p3.x, p3.y
                ));
            }
            PathEl::ClosePath => {
                d.push('Z');
            }
        }
    }

    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_color_to_svg_opaque() {
        let color = Color::from_rgb8(255, 128, 0);
        assert_eq!(color_to_svg(&color), "#ff8000");
    }

    #[test]
    fn test_color_to_svg_transparent() {
        let color = Color::from_rgba8(255, 128, 0, 128);
        let svg = color_to_svg(&color);
        assert!(svg.starts_with("rgba(255,128,0,"));
    }

    #[test]
    fn test_path_to_svg_d() {
        let mut path = BezPath::new();
        path.move_to(Point::new(0.0, 0.0));
        path.line_to(Point::new(100.0, 0.0));
        path.line_to(Point::new(100.0, 100.0));
        path.close_path();

        let d = path_to_svg_d(&path, 2);
        assert!(d.contains("M0.00 0.00"));
        assert!(d.contains("L100.00 0.00"));
        assert!(d.contains('Z'));
    }

    #[test]
    fn test_filled_rect() {
        let cmd = PaintCommand::filled_rect(Rect::new(0.0, 0.0, 100.0, 50.0), Color::BLACK);
        if let PaintCommand::Rect { fill, stroke, .. } = cmd {
            assert!(fill.is_some());
            assert!(stroke.is_none());
        } else {
            panic!("Expected Rect command");
        }
    }

    #[test]
    fn test_line_command() {
        let cmd = PaintCommand::line(
            Point::new(0.0, 0.0),
            Point::new(100.0, 100.0),
            Color::BLACK,
            2.0,
        );
        if let PaintCommand::Line {
            width, line_cap, ..
        } = cmd
        {
            assert_eq!(width, 2.0);
            assert_eq!(line_cap, LineCap::Butt);
        } else {
            panic!("Expected Line command");
        }
    }
}
