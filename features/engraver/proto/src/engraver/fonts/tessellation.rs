//! Font glyph tessellation utilities.
//!
//! This module provides utilities for tessellating font glyphs into
//! GPU-renderable triangle meshes using Lyon.

use lyon::lyon_tessellation::{
    BuffersBuilder, FillOptions, FillTessellator, FillVertex, FillVertexConstructor, VertexBuffers,
};
use lyon::path::Path;
use skrifa::{
    FontRef, MetadataProvider,
    instance::Size,
    outline::{DrawSettings, OutlinePen},
    prelude::LocationRef,
};
use smufl::Glyph;

/// A pen that builds a Lyon Path from font glyph outlines.
///
/// This implements skrifa's `OutlinePen` trait to convert font outlines
/// into Lyon paths that can be tessellated into triangles.
pub struct LyonPen {
    builder: lyon::path::Builder,
}

impl LyonPen {
    /// Create a new LyonPen.
    #[must_use]
    pub fn new() -> Self {
        Self {
            builder: Path::builder(),
        }
    }

    /// Consume the pen and build the resulting Path.
    #[must_use]
    pub fn build(self) -> Path {
        self.builder.build()
    }
}

impl Default for LyonPen {
    fn default() -> Self {
        Self::new()
    }
}

impl OutlinePen for LyonPen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.builder.begin(lyon::math::point(x, y));
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.builder.line_to(lyon::math::point(x, y));
    }

    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.builder
            .quadratic_bezier_to(lyon::math::point(cx0, cy0), lyon::math::point(x, y));
    }

    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.builder.cubic_bezier_to(
            lyon::math::point(cx0, cy0),
            lyon::math::point(cx1, cy1),
            lyon::math::point(x, y),
        );
    }

    fn close(&mut self) {
        self.builder.end(true);
    }
}

/// A simple vertex with position and color, suitable for GPU rendering.
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GlyphVertex {
    /// Position in whatever coordinate space the caller desires
    pub position: [f32; 2],
    /// RGBA color
    pub color: [f32; 4],
}

/// Vertex constructor for Lyon tessellation that creates `GlyphVertex` instances.
pub struct GlyphVertexConstructor {
    /// The color to assign to all vertices
    pub color: [f32; 4],
}

impl FillVertexConstructor<GlyphVertex> for GlyphVertexConstructor {
    fn new_vertex(&mut self, vertex: FillVertex) -> GlyphVertex {
        GlyphVertex {
            position: [vertex.position().x, vertex.position().y],
            color: self.color,
        }
    }
}

/// Get the glyph ID for a SMuFL glyph from a font.
///
/// SMuFL glyphs use Unicode codepoints in the Private Use Area (U+E000+).
/// This function looks up the font's character map to find the glyph ID
/// for a given SMuFL glyph.
#[must_use]
pub fn get_glyph_id(font: &FontRef<'_>, smufl_glyph: Glyph) -> Option<skrifa::GlyphId> {
    let cmap = font.charmap();
    let codepoint = smufl_glyph.codepoint();
    cmap.map(codepoint)
}

/// Result of tessellating a glyph.
pub struct TessellatedGlyph {
    /// The tessellated vertices
    pub vertices: Vec<GlyphVertex>,
    /// Indices into the vertex buffer
    pub indices: Vec<u32>,
}

/// Tessellate a font glyph into triangles.
///
/// This function draws the glyph outline at the specified size and position,
/// then tessellates it into triangles suitable for GPU rendering.
///
/// # Arguments
/// * `font` - The font containing the glyph
/// * `glyph_id` - The glyph ID to tessellate
/// * `font_size` - The size to render the glyph at (in pixels/points)
/// * `color` - The color to assign to vertices
///
/// # Returns
/// A `TessellatedGlyph` containing vertices and indices, or `None` if tessellation fails.
#[must_use]
pub fn tessellate_glyph(
    font: &FontRef<'_>,
    glyph_id: skrifa::GlyphId,
    font_size: f32,
    color: [f32; 4],
) -> Option<TessellatedGlyph> {
    let outline_glyphs = font.outline_glyphs();
    let outline = outline_glyphs.get(glyph_id)?;

    // DrawSettings takes the font size in pixels (ppem)
    let settings = DrawSettings::unhinted(Size::new(font_size), LocationRef::default());

    let mut pen = LyonPen::new();
    outline.draw(settings, &mut pen).ok()?;

    let path = pen.build();

    // Tessellate the path
    let mut geometry: VertexBuffers<GlyphVertex, u32> = VertexBuffers::new();
    let mut tessellator = FillTessellator::new();

    tessellator
        .tessellate_path(
            &path,
            &FillOptions::default(),
            &mut BuffersBuilder::new(&mut geometry, GlyphVertexConstructor { color }),
        )
        .ok()?;

    Some(TessellatedGlyph {
        vertices: geometry.vertices,
        indices: geometry.indices,
    })
}

/// Tessellate a glyph and transform vertices to NDC (Normalized Device Coordinates).
///
/// This is a convenience function that tessellates a glyph and transforms
/// all vertices to normalized device coordinates (-1 to 1 range), suitable
/// for direct rendering without further transformation.
///
/// # Arguments
/// * `font` - The font containing the glyph
/// * `glyph_id` - The glyph ID to tessellate
/// * `font_size` - The size to render the glyph at
/// * `x_offset` - X position in pixel coordinates
/// * `y_offset` - Y position in pixel coordinates
/// * `color` - The color to assign to vertices
/// * `canvas_width` - Width of the canvas in pixels
/// * `canvas_height` - Height of the canvas in pixels
///
/// # Returns
/// A vector of vertices in NDC, ready for rendering.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn tessellate_glyph_to_ndc(
    font: &FontRef<'_>,
    glyph_id: skrifa::GlyphId,
    font_size: f32,
    x_offset: f32,
    y_offset: f32,
    color: [f32; 4],
    canvas_width: f32,
    canvas_height: f32,
) -> Vec<GlyphVertex> {
    let Some(tessellated) = tessellate_glyph(font, glyph_id, font_size, color) else {
        return Vec::new();
    };

    // Transform vertices to NDC using the indices to create a triangle list
    tessellated
        .indices
        .iter()
        .map(|&index| {
            let v = &tessellated.vertices[index as usize];
            // Apply position offset
            let px = x_offset + v.position[0];
            // Flip Y because font coordinates are Y-up, screen is Y-down
            let py = y_offset - v.position[1];

            // Convert to NDC
            let ndc_x = (px / canvas_width) * 2.0 - 1.0;
            let ndc_y = 1.0 - (py / canvas_height) * 2.0;

            GlyphVertex {
                position: [ndc_x, ndc_y],
                color: v.color,
            }
        })
        .collect()
}
