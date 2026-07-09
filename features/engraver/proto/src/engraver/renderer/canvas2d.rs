//! 2D Canvas abstraction for GPU-accelerated drawing
//!
//! Provides a simple API for drawing shapes, lines, and paths using lyon tessellation.
//! Compatible with wgpu 28+.

use lyon::geom::point;
use lyon::path::Path;
use lyon::tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, StrokeOptions, StrokeTessellator, VertexBuffers,
};
use std::f32::consts::PI;

/// A vertex with position and color for GPU rendering
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex2D {
    pub position: [f32; 2],
    pub color: [f32; 4],
}

/// RGBA color with values 0.0-1.0
#[derive(Copy, Clone, Debug)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Color {
    pub const fn rgba(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self { r, g, b, a }
    }

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    /// Create from 0-255 values
    pub fn from_u8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self {
            r: r as f32 / 255.0,
            g: g as f32 / 255.0,
            b: b as f32 / 255.0,
            a: a as f32 / 255.0,
        }
    }

    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    // Common colors
    pub const BLACK: Self = Self::rgba(0.0, 0.0, 0.0, 1.0);
    pub const WHITE: Self = Self::rgba(1.0, 1.0, 1.0, 1.0);
    pub const RED: Self = Self::rgba(1.0, 0.0, 0.0, 1.0);
    pub const REHEARSAL_RED: Self = Self::rgba(0.898, 0.129, 0.0, 1.0);
}

/// Rectangle definition
#[derive(Copy, Clone, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// 2D Canvas for building geometry
pub struct Canvas2D {
    vertices: Vec<Vertex2D>,
    indices: Vec<u32>,
    /// Canvas dimensions for NDC conversion
    width: f32,
    height: f32,
}

impl Canvas2D {
    /// Create a new canvas with the given dimensions
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            width,
            height,
        }
    }

    /// Clear all geometry
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
    }

    /// Get the vertices for rendering
    pub fn vertices(&self) -> &[Vertex2D] {
        &self.vertices
    }

    /// Get the indices for rendering (if using indexed drawing)
    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    /// Convert pixel coordinates to NDC (-1 to 1)
    fn px_to_ndc(&self, x: f32, y: f32) -> [f32; 2] {
        [(x / self.width) * 2.0 - 1.0, 1.0 - (y / self.height) * 2.0]
    }

    /// Draw a filled rectangle
    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        let p1 = self.px_to_ndc(rect.x, rect.y);
        let p2 = self.px_to_ndc(rect.x + rect.width, rect.y);
        let p3 = self.px_to_ndc(rect.x, rect.y + rect.height);
        let p4 = self.px_to_ndc(rect.x + rect.width, rect.y + rect.height);
        let c = color.to_array();

        self.vertices.extend_from_slice(&[
            Vertex2D {
                position: p1,
                color: c,
            },
            Vertex2D {
                position: p3,
                color: c,
            },
            Vertex2D {
                position: p2,
                color: c,
            },
            Vertex2D {
                position: p3,
                color: c,
            },
            Vertex2D {
                position: p4,
                color: c,
            },
            Vertex2D {
                position: p2,
                color: c,
            },
        ]);
    }

    /// Draw a line from (x1, y1) to (x2, y2)
    pub fn stroke_line(
        &mut self,
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        thickness: f32,
        color: Color,
    ) {
        let dx = x2 - x1;
        let dy = y2 - y1;
        let len = (dx * dx + dy * dy).sqrt();

        if len < 0.001 {
            return;
        }

        // Perpendicular unit vector
        let nx = -dy / len * thickness * 0.5;
        let ny = dx / len * thickness * 0.5;

        let p1 = self.px_to_ndc(x1 + nx, y1 + ny);
        let p2 = self.px_to_ndc(x1 - nx, y1 - ny);
        let p3 = self.px_to_ndc(x2 + nx, y2 + ny);
        let p4 = self.px_to_ndc(x2 - nx, y2 - ny);
        let c = color.to_array();

        self.vertices.extend_from_slice(&[
            Vertex2D {
                position: p1,
                color: c,
            },
            Vertex2D {
                position: p2,
                color: c,
            },
            Vertex2D {
                position: p3,
                color: c,
            },
            Vertex2D {
                position: p2,
                color: c,
            },
            Vertex2D {
                position: p4,
                color: c,
            },
            Vertex2D {
                position: p3,
                color: c,
            },
        ]);
    }

    /// Draw a rectangle outline (stroke)
    pub fn stroke_rect(&mut self, rect: Rect, thickness: f32, color: Color) {
        let x = rect.x;
        let y = rect.y;
        let w = rect.width;
        let h = rect.height;

        self.stroke_line(x, y, x + w, y, thickness, color);
        self.stroke_line(x + w, y, x + w, y + h, thickness, color);
        self.stroke_line(x + w, y + h, x, y + h, thickness, color);
        self.stroke_line(x, y + h, x, y, thickness, color);
    }

    /// Draw a rounded rectangle outline using lyon stroke tessellation
    pub fn stroke_rounded_rect(&mut self, rect: Rect, radius: f32, thickness: f32, color: Color) {
        use lyon::math::Vector;
        use lyon::path::builder::SvgPathBuilder;

        let x = rect.x;
        let y = rect.y;
        let w = rect.width;
        let h = rect.height;
        let r = radius.min(w / 2.0).min(h / 2.0);
        let c = color.to_array();

        // Use SvgPathBuilder for proper arc support
        let mut builder = Path::builder().with_svg();

        // Start at top edge, after top-left corner
        builder.move_to(point(x + r, y));

        // Top edge
        builder.line_to(point(x + w - r, y));

        // Top-right corner (arc)
        builder.arc_to(
            Vector::new(r, r),
            lyon::math::Angle::radians(0.0),
            lyon::path::ArcFlags {
                large_arc: false,
                sweep: true,
            },
            point(x + w, y + r),
        );

        // Right edge
        builder.line_to(point(x + w, y + h - r));

        // Bottom-right corner (arc)
        builder.arc_to(
            Vector::new(r, r),
            lyon::math::Angle::radians(0.0),
            lyon::path::ArcFlags {
                large_arc: false,
                sweep: true,
            },
            point(x + w - r, y + h),
        );

        // Bottom edge
        builder.line_to(point(x + r, y + h));

        // Bottom-left corner (arc)
        builder.arc_to(
            Vector::new(r, r),
            lyon::math::Angle::radians(0.0),
            lyon::path::ArcFlags {
                large_arc: false,
                sweep: true,
            },
            point(x, y + h - r),
        );

        // Left edge
        builder.line_to(point(x, y + r));

        // Top-left corner (arc)
        builder.arc_to(
            Vector::new(r, r),
            lyon::math::Angle::radians(0.0),
            lyon::path::ArcFlags {
                large_arc: false,
                sweep: true,
            },
            point(x + r, y),
        );

        builder.close();
        let path = builder.build();

        // Tessellate the stroke
        let mut geometry: VertexBuffers<Vertex2D, u32> = VertexBuffers::new();
        let mut tessellator = StrokeTessellator::new();

        let canvas_width = self.width;
        let canvas_height = self.height;

        let stroke_options = StrokeOptions::default()
            .with_line_width(thickness)
            .with_line_join(lyon::tessellation::LineJoin::Round);

        let _ = tessellator.tessellate_path(
            &path,
            &stroke_options,
            &mut BuffersBuilder::new(&mut geometry, |vertex: lyon::tessellation::StrokeVertex| {
                let pos = vertex.position();
                Vertex2D {
                    position: [
                        (pos.x / canvas_width) * 2.0 - 1.0,
                        1.0 - (pos.y / canvas_height) * 2.0,
                    ],
                    color: c,
                }
            }),
        );

        self.vertices.extend(geometry.vertices);
    }

    /// Draw a filled rounded rectangle
    pub fn fill_rounded_rect(&mut self, rect: Rect, radius: f32, color: Color) {
        use lyon::math::Vector;
        use lyon::path::builder::SvgPathBuilder;

        let x = rect.x;
        let y = rect.y;
        let w = rect.width;
        let h = rect.height;
        let r = radius.min(w / 2.0).min(h / 2.0);
        let c = color.to_array();

        // Use SvgPathBuilder for proper arc support
        let mut builder = Path::builder().with_svg();

        // Start at top edge, after top-left corner
        builder.move_to(point(x + r, y));

        // Top edge
        builder.line_to(point(x + w - r, y));

        // Top-right corner (arc)
        builder.arc_to(
            Vector::new(r, r),
            lyon::math::Angle::radians(0.0),
            lyon::path::ArcFlags {
                large_arc: false,
                sweep: true,
            },
            point(x + w, y + r),
        );

        // Right edge
        builder.line_to(point(x + w, y + h - r));

        // Bottom-right corner (arc)
        builder.arc_to(
            Vector::new(r, r),
            lyon::math::Angle::radians(0.0),
            lyon::path::ArcFlags {
                large_arc: false,
                sweep: true,
            },
            point(x + w - r, y + h),
        );

        // Bottom edge
        builder.line_to(point(x + r, y + h));

        // Bottom-left corner (arc)
        builder.arc_to(
            Vector::new(r, r),
            lyon::math::Angle::radians(0.0),
            lyon::path::ArcFlags {
                large_arc: false,
                sweep: true,
            },
            point(x, y + h - r),
        );

        // Left edge
        builder.line_to(point(x, y + r));

        // Top-left corner (arc)
        builder.arc_to(
            Vector::new(r, r),
            lyon::math::Angle::radians(0.0),
            lyon::path::ArcFlags {
                large_arc: false,
                sweep: true,
            },
            point(x + r, y),
        );

        builder.close();
        let path = builder.build();

        // Tessellate the path
        let mut geometry: VertexBuffers<Vertex2D, u32> = VertexBuffers::new();
        let mut tessellator = FillTessellator::new();

        let canvas_width = self.width;
        let canvas_height = self.height;

        let _ = tessellator.tessellate_path(
            &path,
            &FillOptions::default(),
            &mut BuffersBuilder::new(&mut geometry, |vertex: lyon::tessellation::FillVertex| {
                let pos = vertex.position();
                Vertex2D {
                    position: [
                        (pos.x / canvas_width) * 2.0 - 1.0,
                        1.0 - (pos.y / canvas_height) * 2.0,
                    ],
                    color: c,
                }
            }),
        );

        self.vertices.extend(geometry.vertices);
    }

    /// Draw a filled circle
    pub fn fill_circle(&mut self, cx: f32, cy: f32, radius: f32, color: Color) {
        let c = color.to_array();
        let segments = 32;

        // Triangle fan from center
        let center = self.px_to_ndc(cx, cy);

        for i in 0..segments {
            let a0 = (i as f32 / segments as f32) * 2.0 * PI;
            let a1 = ((i + 1) as f32 / segments as f32) * 2.0 * PI;

            let p0 = self.px_to_ndc(cx + radius * a0.cos(), cy + radius * a0.sin());
            let p1 = self.px_to_ndc(cx + radius * a1.cos(), cy + radius * a1.sin());

            self.vertices.extend_from_slice(&[
                Vertex2D {
                    position: center,
                    color: c,
                },
                Vertex2D {
                    position: p0,
                    color: c,
                },
                Vertex2D {
                    position: p1,
                    color: c,
                },
            ]);
        }
    }

    /// Draw a circle outline
    pub fn stroke_circle(&mut self, cx: f32, cy: f32, radius: f32, thickness: f32, color: Color) {
        let segments = 32;

        for i in 0..segments {
            let a0 = (i as f32 / segments as f32) * 2.0 * PI;
            let a1 = ((i + 1) as f32 / segments as f32) * 2.0 * PI;

            let x0 = cx + radius * a0.cos();
            let y0 = cy + radius * a0.sin();
            let x1 = cx + radius * a1.cos();
            let y1 = cy + radius * a1.sin();

            self.stroke_line(x0, y0, x1, y1, thickness, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canvas_creation() {
        let canvas = Canvas2D::new(800.0, 600.0);
        assert!(canvas.vertices().is_empty());
    }

    #[test]
    fn test_fill_rect() {
        let mut canvas = Canvas2D::new(800.0, 600.0);
        canvas.fill_rect(Rect::new(0.0, 0.0, 100.0, 100.0), Color::RED);
        assert_eq!(canvas.vertices().len(), 6); // 2 triangles
    }

    #[test]
    fn test_color_from_u8() {
        let color = Color::from_u8(255, 128, 0, 255);
        assert!((color.r - 1.0).abs() < 0.01);
        assert!((color.g - 0.5).abs() < 0.01);
        assert!((color.b - 0.0).abs() < 0.01);
    }
}
