//! Scene graph renderer for WGPU via Vello.
//!
//! Converts the SceneNode scene graph to Vello's Scene for GPU rendering.
//! This bridges the layout engine's scene graph output to the actual GPU pipeline.

use std::sync::Arc;

use anyrender::{Glyph, PaintScene};
use kurbo::Stroke;
use kurbo::{Affine, BezPath, Circle, Ellipse, Line, Point, Rect, RoundedRect};
use peniko::{Blob, Color, Fill, FontData};
use skrifa::{
    MetadataProvider,
    instance::Size,
    outline::{DrawSettings, OutlinePen},
    prelude::LocationRef,
    raw::{FileRef, FontRef},
};

use crate::engraver::fonts::SMuFLFont;
use crate::engraver::scene::node::SceneNode;
use crate::engraver::scene::paint::{FillRule, LineCap, LineJoin, PaintCommand, TextAnchor};

/// A pen that builds a kurbo BezPath from font glyph outlines.
///
/// This is used for Vello rendering where we need BezPath instead of
/// Lyon's triangle meshes.
struct VelloPen {
    path: BezPath,
}

impl VelloPen {
    fn new() -> Self {
        Self {
            path: BezPath::new(),
        }
    }

    fn build(self) -> BezPath {
        self.path
    }
}

impl OutlinePen for VelloPen {
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

/// Configuration for scene rendering.
#[derive(Debug, Clone)]
pub struct SceneRenderConfig {
    /// Staff space size in pixels (for scaling glyphs)
    pub spatium: f64,
    /// Default stroke width
    pub default_stroke_width: f64,
    /// Music font name for glyphs
    pub music_font_family: String,
    /// Enable debug bounding boxes
    pub show_bounding_boxes: bool,
    /// Debug bounding box color
    pub debug_color: Color,
    /// Optional viewport for culling (in screen coordinates)
    /// Nodes entirely outside this rect will be skipped
    pub viewport: Option<Rect>,
}

impl Default for SceneRenderConfig {
    fn default() -> Self {
        Self {
            spatium: 10.0,
            default_stroke_width: 1.0,
            music_font_family: "Bravura".to_string(),
            show_bounding_boxes: false,
            debug_color: Color::from_rgba8(255, 0, 0, 128),
            viewport: None,
        }
    }
}

/// Renders a SceneNode tree to a Vello Scene.
///
/// This renderer traverses the scene graph and converts PaintCommands
/// to Vello drawing operations. It handles:
/// - Transform stack management
/// - PaintCommand conversion
/// - SMuFL glyph rendering
/// - Text rendering via Vello's draw_glyphs API
pub struct VelloSceneRenderer<'a> {
    /// Rendering configuration
    config: SceneRenderConfig,
    /// SMuFL font for glyph rendering
    font: Option<&'a SMuFLFont<'a>>,
    /// Text font for regular text (lyrics, dynamics text, etc.)
    text_font: Option<FontData>,
    /// Additional fonts by family name for specialized rendering (e.g., chord symbols)
    font_registry: std::collections::HashMap<String, FontData>,
    /// Transform stack for hierarchical transforms
    transform_stack: Vec<Affine>,
    /// Render statistics
    stats: RenderStats,
}

/// Statistics from a render pass.
#[derive(Debug, Clone, Default)]
pub struct RenderStats {
    /// Total nodes visited
    pub nodes_visited: u32,
    /// Nodes actually rendered (visible and in viewport)
    pub nodes_rendered: u32,
    /// Nodes culled by visibility
    pub nodes_culled_invisible: u32,
    /// Nodes culled by viewport
    pub nodes_culled_viewport: u32,
    /// Paint commands rendered
    pub commands_rendered: u32,
}

impl<'a> VelloSceneRenderer<'a> {
    /// Create a new renderer with default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: SceneRenderConfig::default(),
            font: None,
            text_font: None,
            font_registry: std::collections::HashMap::new(),
            transform_stack: vec![Affine::IDENTITY],
            stats: RenderStats::default(),
        }
    }

    /// Create a renderer with custom configuration.
    #[must_use]
    pub fn with_config(config: SceneRenderConfig) -> Self {
        Self {
            config,
            font: None,
            text_font: None,
            font_registry: std::collections::HashMap::new(),
            transform_stack: vec![Affine::IDENTITY],
            stats: RenderStats::default(),
        }
    }

    /// Get render statistics from the last render pass.
    pub fn stats(&self) -> &RenderStats {
        &self.stats
    }

    /// Reset render statistics for a new pass.
    pub fn reset_stats(&mut self) {
        self.stats = RenderStats::default();
    }

    /// Set the SMuFL font for glyph rendering.
    #[must_use]
    pub fn with_font(mut self, font: &'a SMuFLFont<'a>) -> Self {
        self.font = Some(font);
        self
    }

    /// Set the text font for rendering regular text.
    ///
    /// The font data is expected to be a TrueType or OpenType font file.
    #[must_use]
    pub fn with_text_font(mut self, font_data: &[u8]) -> Self {
        self.text_font = Some(FontData::new(Blob::new(Arc::new(font_data.to_vec())), 0));
        self
    }

    /// Set the text font from an Arc'd slice for zero-copy.
    #[must_use]
    pub fn with_text_font_arc(mut self, font_data: Arc<Vec<u8>>) -> Self {
        self.text_font = Some(FontData::new(Blob::new(font_data), 0));
        self
    }

    /// Register an additional font by family name.
    ///
    /// Fonts registered here can be selected using the `font_family` field in PaintCommands.
    /// This is useful for specialized fonts like chord symbol fonts (e.g., Leland).
    #[must_use]
    pub fn with_named_font(mut self, family: &str, font_data: &[u8]) -> Self {
        let font = FontData::new(Blob::new(Arc::new(font_data.to_vec())), 0);
        self.font_registry.insert(family.to_string(), font);
        self
    }

    /// Register an additional font by family name from an Arc'd slice.
    #[must_use]
    pub fn with_named_font_arc(mut self, family: &str, font_data: Arc<Vec<u8>>) -> Self {
        let font = FontData::new(Blob::new(font_data), 0);
        self.font_registry.insert(family.to_string(), font);
        self
    }

    /// Get the current accumulated transform.
    fn current_transform(&self) -> Affine {
        self.transform_stack
            .iter()
            .copied()
            .fold(Affine::IDENTITY, |acc, t| acc * t)
    }

    /// Push a transform onto the stack.
    fn push_transform(&mut self, transform: Affine) {
        self.transform_stack.push(transform);
    }

    /// Pop a transform from the stack.
    fn pop_transform(&mut self) {
        if self.transform_stack.len() > 1 {
            self.transform_stack.pop();
        }
    }

    /// Render a SceneNode tree to a Vello Scene.
    pub fn render(&mut self, scene: &mut impl PaintScene, root: &SceneNode) {
        self.stats = RenderStats::default();
        self.render_node(scene, root);
    }

    /// Render a SceneNode tree with an additional base transform.
    ///
    /// This is more efficient than cloning the scene and modifying its transform,
    /// as it avoids the overhead of cloning the entire scene graph.
    ///
    /// # Arguments
    /// * `scene` - The Vello scene to render into
    /// * `root` - The root SceneNode to render
    /// * `base_transform` - Additional transform to apply (e.g., pan/zoom)
    pub fn render_with_transform(
        &mut self,
        scene: &mut impl PaintScene,
        root: &SceneNode,
        base_transform: Affine,
    ) {
        // Reset stats for this render pass
        self.stats = RenderStats::default();
        // Push base transform first
        self.push_transform(base_transform);
        // Render the scene tree (which will compose with base transform)
        self.render_node(scene, root);
        // Pop the base transform
        self.pop_transform();
    }

    /// Set the viewport for culling optimization.
    ///
    /// When set, nodes entirely outside this rect (in screen coordinates)
    /// will be skipped during rendering, improving performance.
    pub fn set_viewport(&mut self, viewport: Rect) {
        self.config.viewport = Some(viewport);
    }

    /// Clear the viewport, disabling culling.
    pub fn clear_viewport(&mut self) {
        self.config.viewport = None;
    }

    /// Reset the renderer state for reuse.
    ///
    /// Clears the transform stack while preserving fonts and config.
    pub fn reset(&mut self) {
        self.transform_stack.clear();
        self.transform_stack.push(Affine::IDENTITY);
    }

    /// Check if a bounding box (in local coordinates) is visible in the viewport.
    ///
    /// Returns true if there's no viewport set or if the transformed bounds
    /// intersect the viewport.
    fn is_visible_in_viewport(&self, local_bounds: Rect, transform: Affine) -> bool {
        // If no viewport culling, everything is visible
        let Some(viewport) = self.config.viewport else {
            return true;
        };

        // Skip culling for zero-area bounds (they're likely points or lines)
        if local_bounds.is_zero_area() {
            return true;
        }

        // Transform bounds to screen coordinates
        let screen_bounds = transform.transform_rect_bbox(local_bounds);

        // Check if screen bounds intersect viewport
        !screen_bounds.intersect(viewport).is_zero_area()
    }

    /// Render a single node and its children recursively.
    fn render_node(&mut self, scene: &mut impl PaintScene, node: &SceneNode) {
        self.stats.nodes_visited += 1;

        // Skip invisible nodes
        if !node.visible {
            self.stats.nodes_culled_invisible += 1;
            return;
        }

        // Push node transform
        self.push_transform(node.transform);

        let transform = self.current_transform();

        // Viewport culling: skip if entirely outside viewport
        if !self.is_visible_in_viewport(node.bounds, transform) {
            self.stats.nodes_culled_viewport += 1;
            self.pop_transform();
            return;
        }

        self.stats.nodes_rendered += 1;

        // Render paint commands
        for cmd in &node.commands {
            self.render_command(scene, cmd, transform);
            self.stats.commands_rendered += 1;
        }

        // Debug: render bounding box
        if self.config.show_bounding_boxes && !node.bounds.is_zero_area() {
            let stroke = Stroke::new(0.5);
            scene.stroke(
                &stroke,
                transform,
                self.config.debug_color,
                None,
                &node.bounds,
            );
        }

        // Render children
        for child in &node.children {
            self.render_node(scene, child);
        }

        // Pop transform
        self.pop_transform();
    }

    /// Render a single paint command.
    fn render_command(&self, scene: &mut impl PaintScene, cmd: &PaintCommand, transform: Affine) {
        match cmd {
            PaintCommand::Fill {
                path,
                color,
                fill_rule,
            } => {
                let fill = match fill_rule {
                    FillRule::NonZero => Fill::NonZero,
                    FillRule::EvenOdd => Fill::EvenOdd,
                };
                scene.fill(fill, transform, *color, None, path);
            }

            PaintCommand::Stroke {
                path,
                color,
                width,
                line_cap,
                line_join,
                dash_pattern,
                dash_offset,
            } => {
                let cap = convert_line_cap(*line_cap);
                let mut stroke = Stroke::new(*width);
                stroke.start_cap = cap;
                stroke.end_cap = cap;
                stroke.join = convert_line_join(*line_join);
                if !dash_pattern.is_empty() {
                    stroke = stroke.with_dashes(*dash_offset, dash_pattern.iter().copied());
                }
                scene.stroke(&stroke, transform, *color, None, path);
            }

            PaintCommand::Line {
                start,
                end,
                width,
                color,
                line_cap,
            } => {
                let line = Line::new(*start, *end);
                let cap = convert_line_cap(*line_cap);
                let mut stroke = Stroke::new(*width);
                stroke.start_cap = cap;
                stroke.end_cap = cap;
                scene.stroke(&stroke, transform, *color, None, &line);
            }

            PaintCommand::Rect {
                rect,
                fill,
                stroke,
                stroke_width,
                corner_radius,
            } => {
                if let Some(radius) = corner_radius {
                    let rounded = RoundedRect::from_rect(*rect, *radius);
                    if let Some(fill_color) = fill {
                        scene.fill(Fill::NonZero, transform, *fill_color, None, &rounded);
                    }
                    if let Some(stroke_color) = stroke {
                        let stroke_style = Stroke::new(*stroke_width);
                        scene.stroke(&stroke_style, transform, *stroke_color, None, &rounded);
                    }
                } else {
                    if let Some(fill_color) = fill {
                        scene.fill(Fill::NonZero, transform, *fill_color, None, rect);
                    }
                    if let Some(stroke_color) = stroke {
                        let stroke_style = Stroke::new(*stroke_width);
                        scene.stroke(&stroke_style, transform, *stroke_color, None, rect);
                    }
                }
            }

            PaintCommand::Circle {
                center,
                radius,
                fill,
                stroke,
                stroke_width,
            } => {
                let circle = Circle::new(*center, *radius);
                if let Some(fill_color) = fill {
                    scene.fill(Fill::NonZero, transform, *fill_color, None, &circle);
                }
                if let Some(stroke_color) = stroke {
                    let stroke_style = Stroke::new(*stroke_width);
                    scene.stroke(&stroke_style, transform, *stroke_color, None, &circle);
                }
            }

            PaintCommand::Ellipse {
                center,
                radius_x,
                radius_y,
                fill,
                stroke,
                stroke_width,
            } => {
                let ellipse = Ellipse::new(*center, (*radius_x, *radius_y), 0.0);
                if let Some(fill_color) = fill {
                    scene.fill(Fill::NonZero, transform, *fill_color, None, &ellipse);
                }
                if let Some(stroke_color) = stroke {
                    let stroke_style = Stroke::new(*stroke_width);
                    scene.stroke(&stroke_style, transform, *stroke_color, None, &ellipse);
                }
            }

            PaintCommand::Glyph {
                codepoint,
                position,
                size,
                color,
            } => {
                self.render_glyph(scene, *codepoint, *position, *size, *color, transform);
            }

            PaintCommand::Text {
                text,
                font_family,
                font_size,
                position,
                color,
                anchor,
                weight: _,
                style: _,
            } => {
                self.render_text(
                    scene,
                    text,
                    font_family,
                    *font_size,
                    *position,
                    *color,
                    *anchor,
                    transform,
                );
            }
        }
    }

    /// Render a SMuFL glyph.
    ///
    /// Uses the font's outline data to render the actual glyph shape.
    /// Falls back to a placeholder rectangle if font is not available.
    ///
    /// Note: SMuFL fonts are designed so that 1 em = 4 staff spaces (spatium).
    /// The layout system passes `spatium` as the glyph size, so we multiply by 4
    /// to get the correct font size for rendering.
    fn render_glyph(
        &self,
        scene: &mut impl PaintScene,
        codepoint: char,
        position: Point,
        size: f64,
        color: Color,
        transform: Affine,
    ) {
        // SMuFL: 1 em = 4 staff spaces, so font_size = spatium * 4
        let font_size = size * 4.0;

        // Try to render actual glyph outline from font
        if let Some(font) = self.font
            && let Some(path) = self.get_glyph_path(font, codepoint, font_size)
        {
            // Font outlines are Y-up, but screen coordinates are Y-down
            // Apply position and flip Y axis
            let glyph_transform = transform
                * Affine::translate(position.to_vec2())
                * Affine::scale_non_uniform(1.0, -1.0);

            scene.fill(Fill::NonZero, glyph_transform, color, None, &path);
            return;
        }

        // Fallback: draw placeholder rectangle (visual size matches glyph)
        let placeholder = Rect::new(
            position.x,
            position.y - font_size,
            position.x + font_size * 0.5,
            position.y,
        );
        scene.fill(Fill::NonZero, transform, color, None, &placeholder);
    }

    /// Get the BezPath for a glyph from the font.
    fn get_glyph_path(&self, font: &SMuFLFont<'_>, codepoint: char, size: f64) -> Option<BezPath> {
        let font_ref = font.font();

        // Look up glyph ID from codepoint
        let cmap = font_ref.charmap();
        let glyph_id = cmap.map(codepoint)?;

        // Get outline glyphs
        let outline_glyphs = font_ref.outline_glyphs();
        let outline = outline_glyphs.get(glyph_id)?;

        // Draw the glyph outline at the specified size
        let settings = DrawSettings::unhinted(Size::new(size as f32), LocationRef::default());

        let mut pen = VelloPen::new();
        outline.draw(settings, &mut pen).ok()?;

        Some(pen.build())
    }

    /// Render text using Vello's draw_glyphs API.
    ///
    /// Uses fonts in this priority order:
    /// 1. Font registered by family name in the font registry
    /// 2. Default text font set via `with_text_font()`
    /// 3. Fallback to placeholder rectangles
    ///
    /// The position specifies the anchor point of the text, which depends on the anchor type:
    /// - `Start`: left edge of first character (baseline origin)
    /// - `Middle`: horizontal center of text
    /// - `End`: right edge of last character
    ///
    /// Note: Vello's draw_glyphs uses font_size in pixels and glyph positions relative
    /// to the transform origin. Since we work in a transformed coordinate system (points),
    /// we need to:
    /// 1. Transform the text position to screen coordinates
    /// 2. Scale the font size by the transform's scale factor
    /// 3. Use only a translation transform for the text (to avoid double-scaling glyph positions)
    #[allow(clippy::too_many_arguments)]
    fn render_text(
        &self,
        scene: &mut impl PaintScene,
        text: &str,
        font_family: &str,
        font_size: f64,
        position: Point,
        color: Color,
        anchor: TextAnchor,
        transform: Affine,
    ) {
        // Extract scale factor from the transform matrix
        // For a transform [a, b, c, d, e, f], the scale is approximately sqrt(a*a + b*b)
        let coeffs = transform.as_coeffs();
        let scale_x = (coeffs[0] * coeffs[0] + coeffs[1] * coeffs[1]).sqrt();

        // Transform the position to screen coordinates (includes pan, zoom, DPI scale)
        let screen_position = transform * position;

        // Scale the font size from scene units (points) to screen units (pixels)
        let screen_font_size = font_size * scale_x;

        // Try to find the font: first check registry by family name, then fall back to default text font
        let font_data = self
            .font_registry
            .get(font_family)
            .or(self.text_font.as_ref());

        // Try to render with the resolved font
        if let Some(font_data) = font_data
            && let Some(font_ref) = to_font_ref(font_data)
        {
            // Use screen font size for glyph metrics so advances match the rendered size
            let size = skrifa::instance::Size::new(screen_font_size as f32);
            let charmap = font_ref.charmap();
            let glyph_metrics = font_ref.glyph_metrics(size, LocationRef::default());

            // Build glyph list with positions relative to origin
            let mut glyphs = Vec::new();
            let mut pen_x = 0.0_f32;

            for ch in text.chars() {
                if ch == '\n' {
                    continue; // Skip newlines for single-line text
                }
                let gid = charmap.map(ch).unwrap_or_default();
                let advance = glyph_metrics.advance_width(gid).unwrap_or_default();

                glyphs.push(Glyph {
                    id: gid.to_u32(),
                    x: pen_x,
                    y: 0.0, // Baseline is at y=0 relative to position
                });

                pen_x += advance;
            }

            // Calculate anchor offset based on total text width
            let total_width = pen_x as f64;
            let anchor_offset = match anchor {
                TextAnchor::Start => 0.0,
                TextAnchor::Middle => -total_width / 2.0,
                TextAnchor::End => -total_width,
            };

            // Use translation-only transform at the screen position with anchor offset
            // This avoids double-scaling the glyph positions
            let text_transform = Affine::translate(
                (screen_position + kurbo::Vec2::new(anchor_offset, 0.0)).to_vec2(),
            );

            scene.draw_glyphs(
                font_data,
                screen_font_size as f32,
                false,             // hint
                &[],               // normalized_coords
                kurbo::Vec2::ZERO, // embolden
                Fill::NonZero,     // style
                color,             // brush
                1.0,               // brush_alpha
                text_transform,    // transform
                None,              // glyph_transform
                glyphs.into_iter(),
            );

            return;
        }

        // Fallback: draw placeholder rectangles
        let char_width = font_size * 0.6;
        let char_height = font_size * 0.8;
        let total_width = text.chars().count() as f64 * char_width;
        let anchor_offset = match anchor {
            TextAnchor::Start => 0.0,
            TextAnchor::Middle => -total_width / 2.0,
            TextAnchor::End => -total_width,
        };

        for (i, _ch) in text.chars().enumerate() {
            let x = position.x + anchor_offset + i as f64 * char_width;
            let rect = Rect::new(
                x,
                position.y - char_height,
                x + char_width * 0.8,
                position.y,
            );
            scene.fill(Fill::NonZero, transform, color, None, &rect);
        }
    }
}

/// Convert Font to FontRef for skrifa operations.
fn to_font_ref(font: &FontData) -> Option<FontRef<'_>> {
    let file_ref = FileRef::new(font.data.as_ref()).ok()?;
    match file_ref {
        FileRef::Font(font) => Some(font),
        FileRef::Collection(collection) => collection.get(font.index).ok(),
    }
}

impl<'a> Default for VelloSceneRenderer<'a> {
    fn default() -> Self {
        Self::new()
    }
}

/// Convert our LineCap to Vello's Cap.
fn convert_line_cap(cap: LineCap) -> kurbo::Cap {
    match cap {
        LineCap::Butt => kurbo::Cap::Butt,
        LineCap::Round => kurbo::Cap::Round,
        LineCap::Square => kurbo::Cap::Square,
    }
}

/// Convert our LineJoin to Vello's Join.
fn convert_line_join(join: LineJoin) -> kurbo::Join {
    match join {
        LineJoin::Miter => kurbo::Join::Miter,
        LineJoin::Round => kurbo::Join::Round,
        LineJoin::Bevel => kurbo::Join::Bevel,
    }
}

/// Builder for creating rendering output from a scene graph.
///
/// Provides convenience methods for common rendering operations.
pub struct SceneRenderBuilder {
    config: SceneRenderConfig,
}

impl SceneRenderBuilder {
    /// Create a new render builder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: SceneRenderConfig::default(),
        }
    }

    /// Set the spatium (staff space) in pixels.
    #[must_use]
    pub fn spatium(mut self, spatium: f64) -> Self {
        self.config.spatium = spatium;
        self
    }

    /// Set the music font family name.
    #[must_use]
    pub fn music_font(mut self, family: impl Into<String>) -> Self {
        self.config.music_font_family = family.into();
        self
    }

    /// Enable debug bounding boxes.
    #[must_use]
    pub fn show_debug_boxes(mut self, show: bool) -> Self {
        self.config.show_bounding_boxes = show;
        self
    }

    /// Build the renderer.
    #[must_use]
    pub fn build<'a>(self) -> VelloSceneRenderer<'a> {
        VelloSceneRenderer::with_config(self.config)
    }
}

impl Default for SceneRenderBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engraver::scene::id::{ElementType, SemanticId};
    use crate::engraver::scene::node::SceneNode;

    #[test]
    fn test_renderer_creation() {
        let renderer = VelloSceneRenderer::new();
        assert_eq!(renderer.config.spatium, 10.0);
    }

    #[test]
    fn test_renderer_with_config() {
        let config = SceneRenderConfig {
            spatium: 20.0,
            show_bounding_boxes: true,
            ..Default::default()
        };
        let renderer = VelloSceneRenderer::with_config(config);
        assert_eq!(renderer.config.spatium, 20.0);
        assert!(renderer.config.show_bounding_boxes);
    }

    #[test]
    fn test_transform_stack() {
        let mut renderer = VelloSceneRenderer::new();

        // Initial transform is identity
        assert_eq!(renderer.current_transform(), Affine::IDENTITY);

        // Push a translation
        renderer.push_transform(Affine::translate((10.0, 20.0)));
        let t = renderer.current_transform();
        let expected = Affine::translate((10.0, 20.0));
        assert_eq!(t.as_coeffs(), expected.as_coeffs());

        // Push another translation
        renderer.push_transform(Affine::translate((5.0, 5.0)));
        let t2 = renderer.current_transform();
        // Combined should be (15, 25)
        let pt = t2 * Point::ORIGIN;
        assert!((pt.x - 15.0).abs() < 0.001);
        assert!((pt.y - 25.0).abs() < 0.001);

        // Pop
        renderer.pop_transform();
        let t3 = renderer.current_transform();
        let pt2 = t3 * Point::ORIGIN;
        assert!((pt2.x - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_render_empty_scene() {
        let mut scene = anyrender::NullScenePainter::new();
        let mut renderer = VelloSceneRenderer::new();

        let node = SceneNode::group(SemanticId::page(1));
        renderer.render(&mut scene, &node);

        // Should complete without errors
    }

    #[test]
    fn test_render_with_commands() {
        let mut scene = anyrender::NullScenePainter::new();
        let mut renderer = VelloSceneRenderer::new();

        let node = SceneNode::leaf(
            SemanticId::new(ElementType::Note, 1),
            vec![
                PaintCommand::filled_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK),
                PaintCommand::line(
                    Point::new(0.0, 0.0),
                    Point::new(10.0, 10.0),
                    Color::BLACK,
                    1.0,
                ),
            ],
        );

        renderer.render(&mut scene, &node);
    }

    #[test]
    fn test_render_nested_nodes() {
        let mut scene = anyrender::NullScenePainter::new();
        let mut renderer = VelloSceneRenderer::new();

        let mut parent = SceneNode::group(SemanticId::measure(1));
        parent.transform = Affine::translate((100.0, 100.0));

        let child = SceneNode::leaf(
            SemanticId::new(ElementType::Note, 1),
            vec![PaintCommand::filled_circle(
                Point::new(0.0, 0.0),
                5.0,
                Color::BLACK,
            )],
        );
        parent.add_child(child);

        renderer.render(&mut scene, &parent);
    }

    #[test]
    fn test_invisible_nodes_skipped() {
        let mut scene = anyrender::NullScenePainter::new();
        let mut renderer = VelloSceneRenderer::new();

        let mut node = SceneNode::leaf(
            SemanticId::new(ElementType::Note, 1),
            vec![PaintCommand::filled_rect(
                Rect::new(0.0, 0.0, 100.0, 100.0),
                Color::BLACK,
            )],
        );
        node.visible = false;

        renderer.render(&mut scene, &node);
        // Should complete without rendering anything
    }

    #[test]
    fn test_render_builder() {
        let renderer = SceneRenderBuilder::new()
            .spatium(15.0)
            .music_font("Bravura Text")
            .show_debug_boxes(true)
            .build();

        assert_eq!(renderer.config.spatium, 15.0);
        assert_eq!(renderer.config.music_font_family, "Bravura Text");
        assert!(renderer.config.show_bounding_boxes);
    }

    #[test]
    fn test_line_cap_conversion() {
        assert!(matches!(convert_line_cap(LineCap::Butt), kurbo::Cap::Butt));
        assert!(matches!(
            convert_line_cap(LineCap::Round),
            kurbo::Cap::Round
        ));
        assert!(matches!(
            convert_line_cap(LineCap::Square),
            kurbo::Cap::Square
        ));
    }

    #[test]
    fn test_line_join_conversion() {
        assert!(matches!(
            convert_line_join(LineJoin::Miter),
            kurbo::Join::Miter
        ));
        assert!(matches!(
            convert_line_join(LineJoin::Round),
            kurbo::Join::Round
        ));
        assert!(matches!(
            convert_line_join(LineJoin::Bevel),
            kurbo::Join::Bevel
        ));
    }

    #[test]
    fn test_render_all_paint_commands() {
        let mut scene = anyrender::NullScenePainter::new();
        let mut renderer = VelloSceneRenderer::new();

        let mut path = BezPath::new();
        path.move_to(Point::new(0.0, 0.0));
        path.line_to(Point::new(10.0, 0.0));
        path.line_to(Point::new(10.0, 10.0));
        path.close_path();

        let commands = vec![
            PaintCommand::filled_path(path.clone(), Color::BLACK),
            PaintCommand::stroked_path(path.clone(), Color::BLACK, 2.0),
            PaintCommand::line(Point::ORIGIN, Point::new(10.0, 10.0), Color::BLACK, 1.0),
            PaintCommand::filled_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK),
            PaintCommand::stroked_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK, 1.0),
            PaintCommand::rounded_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK, 2.0),
            PaintCommand::filled_circle(Point::new(5.0, 5.0), 5.0, Color::BLACK),
            PaintCommand::Circle {
                center: Point::new(5.0, 5.0),
                radius: 5.0,
                fill: None,
                stroke: Some(Color::BLACK),
                stroke_width: 1.0,
            },
            PaintCommand::Ellipse {
                center: Point::new(5.0, 5.0),
                radius_x: 10.0,
                radius_y: 5.0,
                fill: Some(Color::BLACK),
                stroke: None,
                stroke_width: 0.0,
            },
            PaintCommand::glyph('\u{E0A4}', Point::new(10.0, 10.0), 20.0, Color::BLACK),
            PaintCommand::text("Test", "Arial", 12.0, Point::new(0.0, 20.0), Color::BLACK),
        ];

        let node = SceneNode::anonymous_leaf(commands);
        renderer.render(&mut scene, &node);
    }
}
