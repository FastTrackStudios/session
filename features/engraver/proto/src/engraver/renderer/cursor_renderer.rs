//! Vello renderer for cursor highlight commands.
//!
//! Translates renderer-agnostic [`HighlightCommand`] values produced by
//! [`ChartCursor`] into Vello draw calls. This keeps the cursor logic
//! independent of the rendering backend while providing GPU-accelerated
//! cursor rendering for the desktop app.
//!
//! [`ChartCursor`]: crate::engraver::layout::chart::cursor::ChartCursor
//! [`HighlightCommand`]: crate::engraver::layout::chart::cursor::HighlightCommand

use anyrender::PaintScene;
use kurbo::{Affine, Line, Point, Rect, RoundedRect};
use kurbo::{BezPath, Stroke};
use peniko::{Color, Fill};
use skrifa::{
    MetadataProvider,
    instance::Size,
    outline::{DrawSettings, OutlinePen},
    prelude::LocationRef,
};

use crate::engraver::fonts::SMuFLFont;
use crate::engraver::layout::chart::cursor::{HighlightCommand, Rgba};

/// Convert an `[u8; 4]` RGBA color to a Vello `Color`.
fn rgba_to_color(rgba: Rgba) -> Color {
    Color::from_rgba8(rgba[0], rgba[1], rgba[2], rgba[3])
}

/// Render a list of highlight commands into a Vello scene.
///
/// The `transform` should be the same transform applied to the chart scene
/// (base_scale * zoom, translated by padding and scroll offset) so the
/// cursor aligns with the underlying notation.
///
/// `font` is needed for `StrokeGlyph` and `FillGlyph` commands. If `None`,
/// glyph commands are silently skipped.
pub fn render_cursor_commands(
    scene: &mut impl PaintScene,
    commands: &[HighlightCommand],
    transform: Affine,
    font: Option<&SMuFLFont<'_>>,
) {
    for cmd in commands {
        match cmd {
            HighlightCommand::StrokeLine {
                x,
                y_top,
                y_bottom,
                color,
                width,
            } => {
                let line = Line::new(Point::new(*x, *y_top), Point::new(*x, *y_bottom));
                let stroke = Stroke::new(*width).with_caps(kurbo::Cap::Round);
                scene.stroke(&stroke, transform, rgba_to_color(*color), None, &line);
            }

            HighlightCommand::FillRect {
                x,
                y,
                width,
                height,
                color,
            } => {
                let rect = Rect::new(*x, *y, x + width, y + height);
                scene.fill(Fill::NonZero, transform, rgba_to_color(*color), None, &rect);
            }

            HighlightCommand::FillRoundedRect {
                x,
                y,
                width,
                height,
                radius,
                color,
            } => {
                let rect = Rect::new(*x, *y, x + width, y + height);
                let rounded = RoundedRect::from_rect(rect, *radius);
                scene.fill(
                    Fill::NonZero,
                    transform,
                    rgba_to_color(*color),
                    None,
                    &rounded,
                );
            }

            HighlightCommand::StrokeGlyph {
                codepoint,
                font_size,
                x,
                y,
                stroke_width,
                color,
            } => {
                if let Some(font) = font
                    && let Some(path) = get_glyph_path(font, *codepoint, *font_size)
                {
                    // Font outlines are Y-up; screen is Y-down → flip Y
                    let glyph_transform = transform
                        * Affine::translate((*x, *y))
                        * Affine::scale_non_uniform(1.0, -1.0);
                    let stroke = Stroke::new(*stroke_width);
                    scene.stroke(&stroke, glyph_transform, rgba_to_color(*color), None, &path);
                }
            }

            HighlightCommand::FillGlyph {
                codepoint,
                font_size,
                x,
                y,
                color,
            } => {
                if let Some(font) = font
                    && let Some(path) = get_glyph_path(font, *codepoint, *font_size)
                {
                    let glyph_transform = transform
                        * Affine::translate((*x, *y))
                        * Affine::scale_non_uniform(1.0, -1.0);
                    scene.fill(
                        Fill::NonZero,
                        glyph_transform,
                        rgba_to_color(*color),
                        None,
                        &path,
                    );
                }
            }
        }
    }
}

/// A pen that builds a kurbo `BezPath` from font glyph outlines.
struct CursorPen {
    path: BezPath,
}

impl CursorPen {
    fn new() -> Self {
        Self {
            path: BezPath::new(),
        }
    }
    fn build(self) -> BezPath {
        self.path
    }
}

impl OutlinePen for CursorPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.path.move_to((x as f64, y as f64));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.path.line_to((x as f64, y as f64));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.path
            .quad_to((cx0 as f64, cy0 as f64), (x as f64, y as f64));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.path.curve_to(
            (cx0 as f64, cy0 as f64),
            (cx1 as f64, cy1 as f64),
            (x as f64, y as f64),
        );
    }
    fn close(&mut self) {
        self.path.close_path();
    }
}

/// Extract a glyph outline as a BezPath from the SMuFL font.
fn get_glyph_path(font: &SMuFLFont<'_>, codepoint: char, font_size: f64) -> Option<BezPath> {
    let font_ref = font.font();
    let cmap = font_ref.charmap();
    let glyph_id = cmap.map(codepoint)?;
    let outline_glyphs = font_ref.outline_glyphs();
    let outline = outline_glyphs.get(glyph_id)?;
    let settings = DrawSettings::unhinted(Size::new(font_size as f32), LocationRef::default());
    let mut pen = CursorPen::new();
    outline.draw(settings, &mut pen).ok()?;
    Some(pen.build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_stroke_line() {
        let mut scene = anyrender::NullScenePainter::new();
        let commands = vec![HighlightCommand::StrokeLine {
            x: 100.0,
            y_top: 50.0,
            y_bottom: 150.0,
            color: [255, 60, 60, 255],
            width: 6.0,
        }];
        render_cursor_commands(&mut scene, &commands, Affine::IDENTITY, None);
        // Should complete without panic
    }

    #[test]
    fn render_fill_rect() {
        let mut scene = anyrender::NullScenePainter::new();
        let commands = vec![HighlightCommand::FillRect {
            x: 10.0,
            y: 20.0,
            width: 100.0,
            height: 30.0,
            color: [255, 60, 60, 89],
        }];
        render_cursor_commands(&mut scene, &commands, Affine::IDENTITY, None);
    }

    #[test]
    fn render_fill_rounded_rect() {
        let mut scene = anyrender::NullScenePainter::new();
        let commands = vec![HighlightCommand::FillRoundedRect {
            x: 10.0,
            y: 20.0,
            width: 50.0,
            height: 20.0,
            radius: 3.0,
            color: [255, 60, 60, 89],
        }];
        render_cursor_commands(&mut scene, &commands, Affine::IDENTITY, None);
    }

    #[test]
    fn glyph_commands_skipped_without_font() {
        let mut scene = anyrender::NullScenePainter::new();
        let commands = vec![
            HighlightCommand::StrokeGlyph {
                codepoint: '\u{E101}',
                font_size: 20.0,
                x: 100.0,
                y: 200.0,
                stroke_width: 6.0,
                color: [255, 60, 60, 102],
            },
            HighlightCommand::FillGlyph {
                codepoint: '\u{E101}',
                font_size: 20.0,
                x: 100.0,
                y: 200.0,
                color: [255, 60, 60, 255],
            },
        ];
        // Without a font, glyph commands should be silently skipped
        render_cursor_commands(&mut scene, &commands, Affine::IDENTITY, None);
    }
}
