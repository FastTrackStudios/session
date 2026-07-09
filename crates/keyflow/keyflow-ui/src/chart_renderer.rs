//! Chart Layout Manager
//!
//! Manages the chart layout and rendering pipeline for desktop use.
//! Ported from the web app's renderer.rs, adapted for native rendering
//! (no WASM canvas methods — produces `vello::Scene` for the app to render).

use crate::signals::{PageMeta, PreviewMode, SystemMeta};
use anyrender::{ImageRenderer, PaintScene, Scene as RecordedScene};
use anyrender::{Paint, recording::RenderCommand};
#[cfg(feature = "anyrender_vello")]
type OffscreenRenderer = anyrender_vello::VelloImageRenderer;
#[cfg(not(feature = "anyrender_vello"))]
type OffscreenRenderer = anyrender::NullImageRenderer;

#[cfg(feature = "anyrender_vello")]
fn new_offscreen_renderer(width: u32, height: u32) -> OffscreenRenderer {
    anyrender_vello::VelloImageRenderer::new(width, height)
}
#[cfg(not(feature = "anyrender_vello"))]
fn new_offscreen_renderer(_width: u32, _height: u32) -> OffscreenRenderer {
    anyrender::NullImageRenderer::new()
}
use keyflow::engraver::export::{PdfSerializer, SvgExportConfig, SvgSerializer};
use keyflow::engraver::fonts::ChartFontBundle;
use keyflow::engraver::layout::chart::cursor::{ChartCursor, CursorState, HighlightCommand, Rgba};
use keyflow::engraver::layout::chart::{
    BeatPosition, Breakpoint, ChartLayoutConfig, ChartLayoutEngine, ChartLayoutResult, LayoutMode,
};
use keyflow::engraver::renderer::cursor_renderer::render_cursor_commands;
use keyflow::engraver::renderer::scene_renderer::SceneRenderBuilder;
use keyflow::engraver::scene::node::{SceneNode, metadata_keys};
use keyflow::engraver::style::MStyle;
use keyflow::{Chart, ChartPosition};
use kurbo::{Affine, Point, Rect};
use peniko::{ImageAlphaType, ImageBrush, ImageData, ImageFormat, ImageQuality};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use vello::peniko::{Color, Compose, Fill};

/// Screen DPI for rendering.
const SCREEN_DPI: f64 = 96.0;
/// Points per inch (typographical standard).
const POINTS_PER_INCH: f64 = 72.0;
/// DPI scaling factor: converts points to screen pixels.
pub const DPI_SCALE: f64 = SCREEN_DPI / POINTS_PER_INCH;

/// US Letter page dimensions in points (8.5" x 11").
///
/// Kept as constants for ergonomic call sites; values mirror
/// [`engraver::model::PaperSize::Letter`] / `PaperSize::A4` to one decimal.
pub const LETTER_WIDTH: f64 = 612.0;
pub const LETTER_HEIGHT: f64 = 792.0;
/// A4 page dimensions in points.
pub const A4_WIDTH: f64 = 595.0;
pub const A4_HEIGHT: f64 = 842.0;

/// Convert a time signature denominator to ticks per beat.
///
/// Uses standard 480 PPQ (ticks per quarter note):
/// - denominator 2 (half note) = 960 ticks
/// - denominator 4 (quarter note) = 480 ticks
/// - denominator 8 (eighth note) = 240 ticks
fn ticks_per_beat_for_denom(denominator: u8) -> i64 {
    match denominator {
        2 => 960,
        4 => 480,
        8 => 240,
        _ => 480,
    }
}

/// Classify a viewport width and pick the breakpoint, accounting for zoom.
///
/// Zoom > 1 makes content larger, so the *effective* viewport shrinks —
/// a zoomed-in tablet should layout like a phone.
fn responsive_breakpoint(viewport_points: f64, zoom: f64) -> Breakpoint {
    let effective = viewport_points / zoom.max(0.25);
    Breakpoint::from_viewport_pt(effective)
}

pub fn layout_mode_for_preview(
    preview_mode: PreviewMode,
    viewport_width: f64,
    zoom: f64,
) -> (LayoutMode, ChartLayoutConfig) {
    let raw_points = viewport_width / DPI_SCALE;
    let viewport_points = LayoutMode::sanitize_dim(raw_points, LETTER_WIDTH).max(240.0);
    match preview_mode {
        PreviewMode::Snippet => (
            LayoutMode::snippet(viewport_points),
            ChartLayoutConfig::snippet().with_page_offsets(true),
        ),
        PreviewMode::Page => (
            LayoutMode::paginated_letter(),
            ChartLayoutConfig::master_rhythm().with_page_offsets(true),
        ),
        PreviewMode::Responsive => {
            // Vertical-only scroll: page width snaps to viewport so nothing
            // overflows horizontally. ContinuousScroll has no page boundary,
            // so content reflows as one infinite column — the right shape
            // for a phone/tablet preview that grows downward only.
            let breakpoint = responsive_breakpoint(viewport_points, zoom);
            (
                LayoutMode::ContinuousScroll {
                    width: viewport_points,
                },
                ChartLayoutConfig::responsive_for(breakpoint),
            )
        }
    }
}

/// Result of a scene graph hit-test.
///
/// Contains the musical position metadata from the deepest matching node,
/// plus the resolved absolute tick from the `BeatPosition` array.
#[derive(Debug, Clone)]
pub struct SceneHitResult {
    /// The musical position stored on the hit node.
    pub position: ChartPosition,
    /// Absolute tick resolved from the BeatPosition array.
    pub absolute_tick: i64,
    /// World-space bounding box of the hit node.
    pub bounds: Rect,
    /// Element type (e.g., "chord", "slash", "rest").
    pub element_type: Option<String>,
}

/// Walk a SceneNode tree and collect all nodes whose world-space bounds
/// contain `point` and that carry `ChartPosition` metadata.
///
/// Results are ordered deepest-first (leaf nodes before parents), so
/// the first result is the most specific hit.
fn hit_test_scene_recursive<'a>(
    node: &'a SceneNode,
    point: Point,
    parent_transform: Affine,
    results: &mut Vec<(&'a SceneNode, Affine)>,
) {
    if !node.visible {
        return;
    }

    let world_transform = parent_transform * node.transform;

    // Recurse into children first (depth-first, so deeper nodes come first)
    for child in &node.children {
        hit_test_scene_recursive(child, point, world_transform, results);
    }

    // Check if this node has chart position metadata and its bounds contain the point
    if node.has_metadata(metadata_keys::CHART_POSITION) {
        let world_bounds = world_transform.transform_rect_bbox(node.bounds);
        // Expand bounds slightly for easier clicking (music symbols can be small)
        let padded = world_bounds.inflate(2.0, 4.0);
        if padded.contains(point) {
            results.push((node, world_transform));
        }
    }
}

fn subtree_world_bounds(node: &SceneNode, parent_transform: Affine) -> Option<Rect> {
    if !node.visible {
        return None;
    }

    let world_transform = parent_transform * node.transform;
    let mut acc: Option<Rect> = if node.bounds.width() > 0.0 || node.bounds.height() > 0.0 {
        Some(world_transform.transform_rect_bbox(node.bounds))
    } else {
        None
    };

    for child in &node.children {
        if let Some(child_bounds) = subtree_world_bounds(child, world_transform) {
            acc = Some(match acc {
                Some(current) => current.union(child_bounds),
                None => child_bounds,
            });
        }
    }

    acc
}

fn is_coarse_command(cmd: &RenderCommand) -> bool {
    match cmd {
        RenderCommand::GlyphRun(g) => g.font_size >= 10.0,
        _ => true,
    }
}

fn can_merge_glyph_runs(
    a: &anyrender::recording::GlyphRunCommand,
    b: &anyrender::recording::GlyphRunCommand,
) -> bool {
    a.font_data == b.font_data
        && a.font_size == b.font_size
        && a.hint == b.hint
        && a.normalized_coords == b.normalized_coords
        && a.style == b.style
        && a.brush == b.brush
        && a.brush_alpha == b.brush_alpha
        && a.transform == b.transform
        && a.glyph_transform == b.glyph_transform
}

fn pack_adjacent_draw_commands(commands: &mut Vec<RenderCommand>) {
    if commands.len() < 2 {
        return;
    }

    let max_merged_glyphs = std::env::var("KEYFLOW_GLYPH_MERGE_MAX")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(4096)
        .max(256);

    let mut packed: Vec<RenderCommand> = Vec::with_capacity(commands.len());
    for cmd in commands.drain(..) {
        match cmd {
            RenderCommand::GlyphRun(next) => {
                if let Some(RenderCommand::GlyphRun(prev)) = packed.last_mut() {
                    if can_merge_glyph_runs(prev, &next)
                        && prev.glyphs.len() + next.glyphs.len() <= max_merged_glyphs
                    {
                        prev.glyphs.extend(next.glyphs);
                        continue;
                    }
                }
                packed.push(RenderCommand::GlyphRun(next));
            }
            other => packed.push(other),
        }
    }
    *commands = packed;
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
struct TransformedPageKey {
    page_number: u32,
    fragment: PageFragmentKind,
    a: i32,
    b: i32,
    c: i32,
    d: i32,
    tx: i32,
    ty: i32,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
enum PageFragmentKind {
    Full,
    Coarse,
    Base,
    Detail,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
enum PageLodTier {
    Low,
    Medium,
    High,
}

impl PageLodTier {
    fn default_scale(self) -> f64 {
        match self {
            Self::Low => 0.45,
            Self::Medium => 0.60,
            Self::High => 0.74,
        }
    }

    fn default_quality(self) -> ImageQuality {
        match self {
            Self::Low => ImageQuality::Low,
            Self::Medium => ImageQuality::Medium,
            Self::High => ImageQuality::High,
        }
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
enum PageLodLayer {
    Full,
    Detail,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
struct PageLodKey {
    page_number: u32,
    tier: PageLodTier,
    layer: PageLodLayer,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
struct ViewStaticKey {
    width: u32,
    height: u32,
    layout_hash: u64,
    ox: i32,
    oy: i32,
    a: i32,
    b: i32,
    c: i32,
    d: i32,
    tx: i32,
    ty: i32,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
struct TransformStabilityKey {
    a: i32,
    b: i32,
    c: i32,
    d: i32,
    tx: i32,
    ty: i32,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
struct ViewCullKey {
    width: u32,
    height: u32,
    transform: TransformStabilityKey,
}

fn quantize_transform(v: f64) -> i32 {
    (v * 1024.0).round() as i32
}

fn transformed_page_key(
    page_number: u32,
    fragment: PageFragmentKind,
    transform: Affine,
) -> TransformedPageKey {
    let [a, b, c, d, tx, ty] = transform.as_coeffs();
    TransformedPageKey {
        page_number,
        fragment,
        a: quantize_transform(a),
        b: quantize_transform(b),
        c: quantize_transform(c),
        d: quantize_transform(d),
        tx: quantize_transform(tx),
        ty: quantize_transform(ty),
    }
}

fn view_static_key(
    width: f64,
    height: f64,
    layout_hash: u64,
    offset: Affine,
    transform: Affine,
) -> ViewStaticKey {
    let [_, _, _, _, ox, oy] = offset.as_coeffs();
    let [a, b, c, d, tx, ty] = transform.as_coeffs();
    ViewStaticKey {
        width: width.round().max(1.0) as u32,
        height: height.round().max(1.0) as u32,
        layout_hash,
        ox: quantize_transform(ox),
        oy: quantize_transform(oy),
        a: quantize_transform(a),
        b: quantize_transform(b),
        c: quantize_transform(c),
        d: quantize_transform(d),
        tx: quantize_transform(tx),
        ty: quantize_transform(ty),
    }
}

fn transform_stability_key(transform: Affine) -> TransformStabilityKey {
    let [a, b, c, d, tx, ty] = transform.as_coeffs();
    // Coarser quantization than transformed-page cache:
    // enough to detect "visually stable" camera states.
    let q = |v: f64| (v * 256.0).round() as i32;
    TransformStabilityKey {
        a: q(a),
        b: q(b),
        c: q(c),
        d: q(d),
        tx: q(tx),
        ty: q(ty),
    }
}

fn replay_recorded_scene(
    target: &mut impl PaintScene,
    recorded: &RecordedScene,
    scene_transform: Affine,
) {
    if scene_transform == Affine::IDENTITY {
        for cmd in &recorded.commands {
            match cmd {
                RenderCommand::PushLayer(cmd) => {
                    target.push_layer(cmd.blend, cmd.alpha, cmd.transform, &cmd.clip)
                }
                RenderCommand::PushClipLayer(cmd) => {
                    target.push_clip_layer(cmd.transform, &cmd.clip)
                }
                RenderCommand::PopLayer => target.pop_layer(),
                RenderCommand::Stroke(cmd) => target.stroke(
                    &cmd.style,
                    cmd.transform,
                    match cmd.brush {
                        Paint::Solid(alpha_color) => Paint::Solid(alpha_color),
                        Paint::Gradient(ref gradient) => Paint::Gradient(gradient),
                        Paint::Image(ref image) => Paint::Image(image.as_ref()),
                        Paint::Resource(id) => Paint::Resource(id),
                        Paint::Custom(ref custom) => Paint::Custom(custom.as_ref()),
                    },
                    cmd.brush_transform,
                    &cmd.shape,
                ),
                RenderCommand::Fill(cmd) => target.fill(
                    cmd.fill,
                    cmd.transform,
                    match cmd.brush {
                        Paint::Solid(alpha_color) => Paint::Solid(alpha_color),
                        Paint::Gradient(ref gradient) => Paint::Gradient(gradient),
                        Paint::Image(ref image) => Paint::Image(image.as_ref()),
                        Paint::Resource(id) => Paint::Resource(id),
                        Paint::Custom(ref custom) => Paint::Custom(custom.as_ref()),
                    },
                    cmd.brush_transform,
                    &cmd.shape,
                ),
                RenderCommand::GlyphRun(cmd) => target.draw_glyphs(
                    &cmd.font_data,
                    cmd.font_size,
                    cmd.hint,
                    &cmd.normalized_coords,
                    cmd.embolden,
                    &cmd.style,
                    match cmd.brush {
                        Paint::Solid(alpha_color) => Paint::Solid(alpha_color),
                        Paint::Gradient(ref gradient) => Paint::Gradient(gradient),
                        Paint::Image(ref image) => Paint::Image(image.as_ref()),
                        Paint::Resource(id) => Paint::Resource(id),
                        Paint::Custom(ref custom) => Paint::Custom(custom.as_ref()),
                    },
                    cmd.brush_alpha,
                    cmd.transform,
                    cmd.glyph_transform,
                    cmd.glyphs.iter().copied(),
                ),
                RenderCommand::BoxShadow(cmd) => target.draw_box_shadow(
                    cmd.transform,
                    cmd.rect,
                    cmd.brush,
                    cmd.radius,
                    cmd.std_dev,
                ),
            }
        }
        return;
    }

    for cmd in &recorded.commands {
        match cmd {
            RenderCommand::PushLayer(cmd) => target.push_layer(
                cmd.blend,
                cmd.alpha,
                scene_transform * cmd.transform,
                &cmd.clip,
            ),
            RenderCommand::PushClipLayer(cmd) => {
                target.push_clip_layer(scene_transform * cmd.transform, &cmd.clip)
            }
            RenderCommand::PopLayer => target.pop_layer(),
            RenderCommand::Stroke(cmd) => target.stroke(
                &cmd.style,
                scene_transform * cmd.transform,
                match cmd.brush {
                    Paint::Solid(alpha_color) => Paint::Solid(alpha_color),
                    Paint::Gradient(ref gradient) => Paint::Gradient(gradient),
                    Paint::Image(ref image) => Paint::Image(image.as_ref()),
                    Paint::Resource(id) => Paint::Resource(id),
                    Paint::Custom(ref custom) => Paint::Custom(custom.as_ref()),
                },
                cmd.brush_transform,
                &cmd.shape,
            ),
            RenderCommand::Fill(cmd) => target.fill(
                cmd.fill,
                scene_transform * cmd.transform,
                match cmd.brush {
                    Paint::Solid(alpha_color) => Paint::Solid(alpha_color),
                    Paint::Gradient(ref gradient) => Paint::Gradient(gradient),
                    Paint::Image(ref image) => Paint::Image(image.as_ref()),
                    Paint::Resource(id) => Paint::Resource(id),
                    Paint::Custom(ref custom) => Paint::Custom(custom.as_ref()),
                },
                cmd.brush_transform,
                &cmd.shape,
            ),
            RenderCommand::GlyphRun(cmd) => target.draw_glyphs(
                &cmd.font_data,
                cmd.font_size,
                cmd.hint,
                &cmd.normalized_coords,
                cmd.embolden,
                &cmd.style,
                match cmd.brush {
                    Paint::Solid(alpha_color) => Paint::Solid(alpha_color),
                    Paint::Gradient(ref gradient) => Paint::Gradient(gradient),
                    Paint::Image(ref image) => Paint::Image(image.as_ref()),
                    Paint::Resource(id) => Paint::Resource(id),
                    Paint::Custom(ref custom) => Paint::Custom(custom.as_ref()),
                },
                cmd.brush_alpha,
                scene_transform * cmd.transform,
                cmd.glyph_transform,
                cmd.glyphs.iter().copied(),
            ),
            RenderCommand::BoxShadow(cmd) => target.draw_box_shadow(
                scene_transform * cmd.transform,
                cmd.rect,
                cmd.brush,
                cmd.radius,
                cmd.std_dev,
            ),
        }
    }
}

/// Chart layout and rendering engine for desktop.
///
/// Manages fonts, layout engine, and Vello scene rendering.
/// Produces `vello::Scene` objects that the app renders to its WGPU surface.
///
/// # Caching Strategy
///
/// Two cache levels to avoid redundant work:
/// 1. **Parse cache** (`cached_chart`): Keyed by source text hash. Avoids re-parsing
///    (~10-20ms) when only the layout mode or viewport changes.
/// 2. **Layout cache** (`layout_result`): Keyed by (source, preview_mode) hash. Avoids
///    re-layout (~440-500ms) when the same chart is re-rendered.
///
/// Rendering uses the `PaintScene` trait abstraction (via anyrender) so the
/// chart rendering pipeline is backend-agnostic.
pub struct ChartLayoutManager {
    /// Font bundle (single source of truth for all chart fonts).
    font_bundle: ChartFontBundle,
    /// Layout engine.
    layout_engine: ChartLayoutEngine,
    /// Cached layout result.
    layout_result: Option<ChartLayoutResult>,
    /// Last layout hash — covers (source, preview_mode) for layout invalidation.
    last_chart_hash: u64,
    /// Cached parsed chart — rebuilt only when source text changes.
    cached_chart: Option<Chart>,
    /// Hash of just the source text (for parse cache invalidation).
    last_source_hash: u64,
    /// Last preview mode (affects fit-to-width calculation).
    last_preview_mode: PreviewMode,
    /// Renderer-agnostic cursor for computing highlight commands.
    cursor: ChartCursor,
    /// Last computed cursor state (cached to avoid recomputing every frame when tick hasn't changed).
    cached_cursor_state: Option<CursorState>,
    /// The tick value used to compute the cached cursor state.
    cached_cursor_tick: i64,
    /// Cached static chart fragments by page number.
    ///
    /// Rebuilt only when layout changes. Per-page fragments let us submit only
    /// visible pages each frame instead of the entire chart.
    cached_page_fragments: HashMap<u32, RecordedScene>,
    /// Base page geometry layer (staff, lines, page furniture, large glyphs).
    cached_page_base_fragments: HashMap<u32, RecordedScene>,
    /// Fine detail layer (small glyph/text).
    cached_page_detail_fragments: HashMap<u32, RecordedScene>,
    /// Coarse subset of static chart commands for far zoom on focused page.
    cached_page_coarse_fragments: HashMap<u32, RecordedScene>,
    /// Static page fragments with transform pre-applied (keyed by page+transform).
    cached_transformed_fragments: HashMap<TransformedPageKey, RecordedScene>,
    transformed_fragment_order: VecDeque<TransformedPageKey>,
    transformed_fragment_capacity: usize,
    /// Low-detail rasterized pages for non-focused pages at far zoom.
    cached_page_lod_images: HashMap<PageLodKey, ImageBrush>,
    /// Offscreen renderer for generating low-detail page rasters.
    page_lod_renderer: Option<OffscreenRenderer>,
    /// Cached full static viewport image for settled camera states.
    cached_view_static_image: Option<ImageBrush>,
    cached_view_static_key: Option<ViewStaticKey>,
    last_view_static_key: Option<ViewStaticKey>,
    view_static_stable_frames: u8,
    view_static_renderer: Option<OffscreenRenderer>,
    enable_view_static_cache: bool,
    enable_pretransformed_fragments: bool,
    last_transform_key: Option<TransformStabilityKey>,
    transform_stable_frames: u8,
    page_geometry: HashMap<u32, (f64, f64, f64, f64)>,
    last_cull_key: Option<ViewCullKey>,
    cached_visible_pages: Vec<u32>,
    cached_focus_page: Option<u32>,
}

impl ChartLayoutManager {
    /// Render only the static chart layer (background + chart glyphs).
    ///
    /// This should be used when cursor/hover overlays are rendered separately
    /// to avoid rebuilding static content on every transport tick.
    pub fn render_static_layer_to_scene(
        &mut self,
        scene: &mut impl PaintScene,
        width: f64,
        height: f64,
        offset: Affine,
        transform: Affine,
    ) {
        if !self.enable_view_static_cache {
            self.render_static_layer_vector(scene, width, height, offset, transform);
            return;
        }

        let view_key = view_static_key(width, height, self.last_chart_hash, offset, transform);
        if self.cached_view_static_key == Some(view_key) {
            if let Some(image) = self.cached_view_static_image.as_ref() {
                scene.draw_image(image.as_ref(), Affine::IDENTITY);
                return;
            }
        }

        if self.last_view_static_key == Some(view_key) {
            self.view_static_stable_frames = self.view_static_stable_frames.saturating_add(1);
        } else {
            self.last_view_static_key = Some(view_key);
            self.view_static_stable_frames = 1;
            self.cached_view_static_key = None;
            self.cached_view_static_image = None;
        }

        // Don't build the offscreen cache while the camera is actively changing.
        // Once the view is stable for >=2 frames, build and reuse.
        if self.view_static_stable_frames >= 2 {
            if let Some(image) =
                self.ensure_view_static_image(width, height, offset, transform, view_key)
            {
                scene.draw_image(image.as_ref(), Affine::IDENTITY);
                return;
            }
        }

        self.render_static_layer_vector(scene, width, height, offset, transform);
    }

    fn render_static_layer_vector(
        &mut self,
        scene: &mut impl PaintScene,
        width: f64,
        height: f64,
        offset: Affine,
        transform: Affine,
    ) {
        // Fill background (gray workspace) — varies with viewport size.
        scene.fill(
            Fill::NonZero,
            offset,
            Color::from_rgb8(55, 65, 81),
            None,
            &Rect::new(0.0, 0.0, width, height),
        );

        if self.layout_result.is_none() {
            return;
        }

        let clip_rect = Rect::new(0.0, 0.0, width, height);
        scene.push_layer(Compose::SrcOver, 1.0, offset, &clip_rect);

        let (visible_pages, focus_page) =
            self.visible_and_focus_for_viewport(width, height, transform);
        let visible_page_count = visible_pages.len();
        let use_multi_page_lod = visible_page_count >= 3;
        let scene_transform = offset * transform;
        let use_pretransformed_fragments =
            self.enable_pretransformed_fragments && self.transform_is_stable(scene_transform);
        let [a, b, c, d, _, _] = scene_transform.as_coeffs();
        let effective_scale = (a * d - b * c).abs().sqrt();
        for page_number in visible_pages {
            let page_geom = self.page_geometry.get(&page_number).copied().or_else(|| {
                self.layout_result
                    .as_ref()
                    .and_then(|l| l.pages.iter().find(|p| p.number == page_number))
                    .map(|p| (p.x_offset, p.y_offset, p.width, p.height))
            });
            let Some((page_x, page_y, page_w, page_h)) = page_geom else {
                continue;
            };
            let projected_page_width = page_w * effective_scale;
            let is_focused = focus_page.is_some_and(|focused| focused == page_number);

            if let Some(tier) = self.lod_tier_for_page(
                projected_page_width,
                visible_page_count,
                use_multi_page_lod,
                is_focused,
            ) {
                let image_transform = |image: &ImageBrush| {
                    scene_transform
                        * Affine::translate((page_x, page_y))
                        * Affine::scale_non_uniform(
                            page_w / image.image.width as f64,
                            page_h / image.image.height as f64,
                        )
                };

                match tier {
                    // Lowest tier: draw a full-page LOD raster.
                    PageLodTier::Low => {
                        if let Some(image) =
                            self.ensure_page_lod_image(page_number, tier, PageLodLayer::Full)
                        {
                            scene.draw_image(image.as_ref(), image_transform(image));
                            continue;
                        }
                    }
                    // Mid/high tier: keep geometry vector-sharp, rasterize only fine detail text/glyphs.
                    PageLodTier::Medium | PageLodTier::High => {
                        self.draw_page_fragment(
                            scene,
                            page_number,
                            PageFragmentKind::Base,
                            scene_transform,
                            use_pretransformed_fragments,
                        );
                        if let Some(image) =
                            self.ensure_page_lod_image(page_number, tier, PageLodLayer::Detail)
                        {
                            scene.draw_image(image.as_ref(), image_transform(image));
                        } else {
                            self.draw_page_fragment(
                                scene,
                                page_number,
                                PageFragmentKind::Detail,
                                scene_transform,
                                use_pretransformed_fragments,
                            );
                        }
                        continue;
                    }
                }
            }

            let prefer_coarse = use_multi_page_lod && is_focused && effective_scale < 0.55;
            let fragment_kind = if prefer_coarse {
                PageFragmentKind::Coarse
            } else {
                PageFragmentKind::Full
            };
            self.draw_page_fragment(
                scene,
                page_number,
                fragment_kind,
                scene_transform,
                use_pretransformed_fragments,
            );
        }

        scene.pop_layer();
    }

    fn transform_is_stable(&mut self, scene_transform: Affine) -> bool {
        let key = transform_stability_key(scene_transform);
        if self.last_transform_key == Some(key) {
            self.transform_stable_frames = self.transform_stable_frames.saturating_add(1);
        } else {
            self.last_transform_key = Some(key);
            self.transform_stable_frames = 1;
        }
        self.transform_stable_frames >= 2
    }

    fn draw_page_fragment(
        &mut self,
        scene: &mut impl PaintScene,
        page_number: u32,
        kind: PageFragmentKind,
        scene_transform: Affine,
        use_pretransformed_fragments: bool,
    ) {
        if use_pretransformed_fragments {
            if let Some(fragment) =
                self.page_fragment_for_draw_transformed(page_number, kind, scene_transform)
            {
                replay_recorded_scene(scene, fragment, Affine::IDENTITY);
            }
            return;
        }

        if let Some(fragment) = self.page_fragment_for_kind(page_number, kind) {
            replay_recorded_scene(scene, fragment, scene_transform);
        }
    }

    fn ensure_view_static_image(
        &mut self,
        width: f64,
        height: f64,
        offset: Affine,
        transform: Affine,
        key: ViewStaticKey,
    ) -> Option<&ImageBrush> {
        if self.cached_view_static_key == Some(key) {
            return self.cached_view_static_image.as_ref();
        }

        let image_width = key.width.max(1);
        let image_height = key.height.max(1);

        let mut static_scene = RecordedScene::new();
        self.render_static_layer_vector(&mut static_scene, width, height, offset, transform);

        let renderer = self
            .view_static_renderer
            .get_or_insert_with(|| new_offscreen_renderer(image_width, image_height));
        renderer.resize(image_width, image_height);

        let mut pixels = Vec::new();
        renderer.render_to_vec(
            |painter| replay_recorded_scene(painter, &static_scene, Affine::IDENTITY),
            &mut pixels,
        );

        let image_data = ImageData {
            data: pixels.into(),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: image_width,
            height: image_height,
        };
        let brush = ImageBrush::new(image_data).with_quality(ImageQuality::High);
        self.cached_view_static_image = Some(brush);
        self.cached_view_static_key = Some(key);
        self.cached_view_static_image.as_ref()
    }

    /// Render only the dynamic overlay layer (hover + cursor).
    ///
    /// Draws on top of a previously-rendered static layer.
    #[allow(clippy::too_many_arguments)]
    pub fn render_overlay_layer_to_scene(
        &mut self,
        scene: &mut impl PaintScene,
        width: f64,
        height: f64,
        offset: Affine,
        transform: Affine,
        cursor_tick: Option<i64>,
        hover_point: Option<(f64, f64)>,
    ) {
        if self.layout_result.is_none() {
            return;
        }

        let clip_rect = Rect::new(0.0, 0.0, width, height);
        scene.push_layer(Compose::SrcOver, 1.0, offset, &clip_rect);

        if let Some((scene_x, scene_y)) = hover_point {
            self.render_hover_highlight(scene, scene_x, scene_y, offset * transform);
        }

        if let Some(tick) = cursor_tick {
            self.update_cursor(tick);
            if let Some(ref state) = self.cached_cursor_state {
                let font = self.font_bundle.smufl_font();
                render_cursor_commands(scene, &state.commands, offset * transform, Some(font));
            }
        }

        scene.pop_layer();
    }

    /// Create a new chart layout manager with embedded fonts.
    pub fn new() -> Result<Self, String> {
        let font_bundle = ChartFontBundle::new()?;

        let style = Box::leak(Box::new(MStyle::new()));
        let layout_engine = font_bundle.create_layout_engine(style);

        Ok(Self {
            font_bundle,
            layout_engine,
            layout_result: None,
            last_chart_hash: 0,
            cached_chart: None,
            last_source_hash: 0,
            last_preview_mode: PreviewMode::Page,
            cursor: ChartCursor::default(),
            cached_cursor_state: None,
            cached_cursor_tick: i64::MIN,
            cached_page_fragments: HashMap::new(),
            cached_page_base_fragments: HashMap::new(),
            cached_page_detail_fragments: HashMap::new(),
            cached_page_coarse_fragments: HashMap::new(),
            cached_transformed_fragments: HashMap::new(),
            transformed_fragment_order: VecDeque::new(),
            transformed_fragment_capacity: std::env::var("KEYFLOW_TRANSFORM_CACHE_MAX")
                .ok()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(2048)
                .max(64),
            cached_page_lod_images: HashMap::new(),
            page_lod_renderer: None,
            cached_view_static_image: None,
            cached_view_static_key: None,
            last_view_static_key: None,
            view_static_stable_frames: 0,
            view_static_renderer: None,
            enable_view_static_cache: std::env::var("KEYFLOW_VIEW_STATIC_CACHE")
                .ok()
                .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true")),
            enable_pretransformed_fragments: std::env::var("KEYFLOW_PRETRANSFORM_CACHE")
                .ok()
                .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true")),
            last_transform_key: None,
            transform_stable_frames: 0,
            page_geometry: HashMap::new(),
            last_cull_key: None,
            cached_visible_pages: Vec::new(),
            cached_focus_page: None,
        })
    }

    /// Parse source and layout the chart, using caches at each level.
    ///
    /// This is the main entry point for the layout effect. It:
    /// 1. Checks the layout hash — if (source, snippet_mode) haven't changed, returns false
    /// 2. Checks the parse cache — if source hasn't changed, reuses the cached Chart
    /// 3. Runs the layout engine and caches the result
    ///
    /// Returns `Ok(true)` if the layout was updated, `Ok(false)` if skipped (no change).
    pub fn parse_and_layout(
        &mut self,
        source: &str,
        viewport_width: f64,
        snippet_mode: bool,
    ) -> Result<bool, String> {
        let preview_mode = if snippet_mode {
            PreviewMode::Snippet
        } else {
            PreviewMode::Page
        };
        self.parse_and_layout_with_preview_mode(source, viewport_width, preview_mode, 1.0)
    }

    /// Parse source and layout the chart using an explicit preview mode.
    pub fn parse_and_layout_with_preview_mode(
        &mut self,
        source: &str,
        viewport_width: f64,
        preview_mode: PreviewMode,
        zoom: f64,
    ) -> Result<bool, String> {
        // Check layout hash — skip everything if nothing changed
        let chart_hash = self.compute_chart_hash(source, preview_mode, viewport_width, zoom);
        if self.layout_result.is_some() && chart_hash == self.last_chart_hash {
            return Ok(false);
        }

        // Check parse cache — only re-parse if source text changed
        let source_hash = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            source.hash(&mut hasher);
            hasher.finish()
        };

        if self.cached_chart.is_none() || source_hash != self.last_source_hash {
            let chart = keyflow::parse(source).map_err(|e| format!("{}", e))?;
            self.cached_chart = Some(chart);
            self.last_source_hash = source_hash;
        }

        // Layout using the cached chart
        let chart = self.cached_chart.as_ref().unwrap();
        let (mode, config) = layout_mode_for_preview(preview_mode, viewport_width, zoom);

        let result = self
            .layout_engine
            .layout_chart_with_config(chart, &mode, &config);

        self.layout_result = Some(result);
        self.last_chart_hash = chart_hash;
        self.last_preview_mode = preview_mode;
        self.cached_cursor_state = None; // Invalidate cursor cache
        self.cached_cursor_tick = i64::MIN;
        self.cached_page_fragments.clear();
        self.cached_page_base_fragments.clear();
        self.cached_page_detail_fragments.clear();
        self.cached_page_coarse_fragments.clear();
        self.cached_transformed_fragments.clear();
        self.transformed_fragment_order.clear();
        self.cached_page_lod_images.clear();
        self.page_lod_renderer = None;
        self.cached_view_static_image = None;
        self.cached_view_static_key = None;
        self.last_view_static_key = None;
        self.view_static_stable_frames = 0;
        self.view_static_renderer = None;
        self.last_transform_key = None;
        self.transform_stable_frames = 0;
        self.last_cull_key = None;
        self.cached_visible_pages.clear();
        self.cached_focus_page = None;
        self.rebuild_page_geometry_cache();

        Ok(true)
    }

    /// Layout a chart with a specified mode.
    ///
    /// # Arguments
    /// * `chart` - The parsed chart to layout
    /// * `source` - The original source text (used for cache invalidation)
    /// * `viewport_width` - Width of the viewport in CSS pixels
    /// * `snippet_mode` - If true, use snippet mode. If false, use A4 paginated.
    pub fn layout_chart_with_mode(
        &mut self,
        chart: &Chart,
        source: &str,
        viewport_width: f64,
        snippet_mode: bool,
    ) {
        let preview_mode = if snippet_mode {
            PreviewMode::Snippet
        } else {
            PreviewMode::Page
        };
        self.layout_chart_with_preview_mode(chart, source, viewport_width, preview_mode, 1.0);
    }

    /// Layout a chart with an explicit preview mode.
    pub fn layout_chart_with_preview_mode(
        &mut self,
        chart: &Chart,
        source: &str,
        viewport_width: f64,
        preview_mode: PreviewMode,
        zoom: f64,
    ) {
        let chart_hash = self.compute_chart_hash(source, preview_mode, viewport_width, zoom);

        // Skip if already laid out with same content
        if self.layout_result.is_some() && chart_hash == self.last_chart_hash {
            return;
        }

        let (mode, config) = layout_mode_for_preview(preview_mode, viewport_width, zoom);

        let result = self
            .layout_engine
            .layout_chart_with_config(chart, &mode, &config);

        self.layout_result = Some(result);
        self.last_chart_hash = chart_hash;
        self.last_preview_mode = preview_mode;
        self.cached_cursor_state = None; // Invalidate cursor cache
        self.cached_cursor_tick = i64::MIN;
        self.cached_page_fragments.clear();
        self.cached_page_base_fragments.clear();
        self.cached_page_detail_fragments.clear();
        self.cached_page_coarse_fragments.clear();
        self.cached_transformed_fragments.clear();
        self.transformed_fragment_order.clear();
        self.cached_page_lod_images.clear();
        self.page_lod_renderer = None;
        self.cached_view_static_image = None;
        self.cached_view_static_key = None;
        self.last_view_static_key = None;
        self.view_static_stable_frames = 0;
        self.view_static_renderer = None;
        self.last_transform_key = None;
        self.transform_stable_frames = 0;
        self.last_cull_key = None;
        self.cached_visible_pages.clear();
        self.cached_focus_page = None;
        self.rebuild_page_geometry_cache();
    }

    fn visible_pages_for_viewport(&self, width: f64, height: f64, transform: Affine) -> Vec<u32> {
        let Some(layout) = self.layout_result.as_ref() else {
            return Vec::new();
        };

        let [a, b, c, d, _, _] = transform.as_coeffs();
        let det = a * d - b * c;
        if det.abs() < 1e-9 {
            return layout.pages.iter().map(|p| p.number).collect();
        }

        let inv = transform.inverse();
        let p0 = inv * Point::new(0.0, 0.0);
        let p1 = inv * Point::new(width, 0.0);
        let p2 = inv * Point::new(0.0, height);
        let p3 = inv * Point::new(width, height);

        let min_x = p0.x.min(p1.x).min(p2.x).min(p3.x);
        let max_x = p0.x.max(p1.x).max(p2.x).max(p3.x);
        let min_y = p0.y.min(p1.y).min(p2.y).min(p3.y);
        let max_y = p0.y.max(p1.y).max(p2.y).max(p3.y);

        let viewport_scene = Rect::new(min_x, min_y, max_x, max_y).inflate(32.0, 32.0);

        let mut pages: Vec<u32> = layout
            .pages
            .iter()
            .filter_map(|page| {
                let page_rect = Rect::new(
                    page.x_offset,
                    page.y_offset,
                    page.x_offset + page.width,
                    page.y_offset + page.height,
                );
                if page_rect.intersect(viewport_scene).area() > 0.0 {
                    Some(page.number)
                } else {
                    None
                }
            })
            .collect();

        if pages.is_empty() && !layout.pages.is_empty() {
            pages.push(layout.pages[0].number);
        }
        pages
    }

    fn rebuild_page_geometry_cache(&mut self) {
        self.page_geometry.clear();
        if let Some(layout) = self.layout_result.as_ref() {
            for page in &layout.pages {
                self.page_geometry.insert(
                    page.number,
                    (page.x_offset, page.y_offset, page.width, page.height),
                );
            }
        }
    }

    fn visible_and_focus_for_viewport(
        &mut self,
        width: f64,
        height: f64,
        transform: Affine,
    ) -> (Vec<u32>, Option<u32>) {
        let key = ViewCullKey {
            width: width.round().max(1.0) as u32,
            height: height.round().max(1.0) as u32,
            transform: transform_stability_key(transform),
        };
        if self.last_cull_key == Some(key) {
            return (self.cached_visible_pages.clone(), self.cached_focus_page);
        }

        let visible_pages = self.visible_pages_for_viewport(width, height, transform);
        let focus_page = self.focused_page_for_viewport(width, height, transform);
        self.last_cull_key = Some(key);
        self.cached_visible_pages = visible_pages.clone();
        self.cached_focus_page = focus_page;
        (visible_pages, focus_page)
    }

    fn focused_page_for_viewport(&self, width: f64, height: f64, transform: Affine) -> Option<u32> {
        let layout = self.layout_result.as_ref()?;
        if layout.pages.is_empty() {
            return None;
        }

        let [a, b, c, d, _, _] = transform.as_coeffs();
        let det = a * d - b * c;
        if det.abs() < 1e-9 {
            return layout.pages.first().map(|p| p.number);
        }

        let inv = transform.inverse();
        let center_scene = inv * Point::new(width * 0.5, height * 0.5);

        layout
            .pages
            .iter()
            .find(|p| {
                center_scene.x >= p.x_offset
                    && center_scene.x <= p.x_offset + p.width
                    && center_scene.y >= p.y_offset
                    && center_scene.y <= p.y_offset + p.height
            })
            .map(|p| p.number)
            .or_else(|| {
                layout
                    .pages
                    .iter()
                    .min_by(|a, b| {
                        let da = (a.x_offset - center_scene.x).abs();
                        let db = (b.x_offset - center_scene.x).abs();
                        da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                    })
                    .map(|p| p.number)
            })
    }

    /// Ensure we have a cached static fragment for a specific page.
    fn ensure_page_static_fragment(&mut self, page_number: u32) -> Option<&RecordedScene> {
        if self.cached_page_fragments.contains_key(&page_number) {
            return self.cached_page_fragments.get(&page_number);
        }

        let layout = self.layout_result.as_ref()?;
        let page = layout.pages.iter().find(|p| p.number == page_number)?;
        let started = web_time::Instant::now();
        let page_rect = Rect::new(
            page.x_offset,
            page.y_offset,
            page.x_offset + page.width,
            page.y_offset + page.height,
        );
        let mut page_roots: Vec<(&SceneNode, Affine)> = Vec::new();
        for child in &layout.scene.children {
            if !child.visible {
                continue;
            }
            let intersects = subtree_world_bounds(child, Affine::IDENTITY)
                .is_some_and(|b| b.intersect(page_rect).area() > 0.0);
            if intersects {
                page_roots.push((child, Affine::IDENTITY));
            }
        }

        // Fallback: render full scene if the geometric page selection found nothing.
        if page_roots.is_empty() {
            page_roots.push((&layout.scene, Affine::IDENTITY));
        }

        let mut fragment = RecordedScene::new();

        let base_renderer = SceneRenderBuilder::new().spatium(5.0).build();
        let mut renderer = self.font_bundle.configure_renderer(base_renderer);
        for (node, parent_transform) in page_roots {
            renderer.render_with_transform(&mut fragment, node, parent_transform);
        }

        let commands_before_pack = fragment.commands.len();
        pack_adjacent_draw_commands(&mut fragment.commands);
        let commands_after_pack = fragment.commands.len();

        tracing::info!(
            "Chart static fragment rebuilt: page={}, commands={} -> {}, time={:.2}ms",
            page_number,
            commands_before_pack,
            commands_after_pack,
            started.elapsed().as_secs_f64() * 1000.0
        );

        let mut coarse = RecordedScene::new();
        coarse.commands = fragment
            .commands
            .iter()
            .filter(|cmd| is_coarse_command(cmd))
            .cloned()
            .collect();

        let mut base = RecordedScene::new();
        base.commands = fragment
            .commands
            .iter()
            .filter(|cmd| !matches!(cmd, RenderCommand::GlyphRun(g) if g.font_size < 10.0))
            .cloned()
            .collect();

        let mut detail = RecordedScene::new();
        detail.commands = fragment
            .commands
            .iter()
            .filter(|cmd| matches!(cmd, RenderCommand::GlyphRun(g) if g.font_size < 10.0))
            .cloned()
            .collect();

        self.cached_page_fragments.insert(page_number, fragment);
        self.cached_page_base_fragments.insert(page_number, base);
        self.cached_page_detail_fragments
            .insert(page_number, detail);
        self.cached_page_coarse_fragments
            .insert(page_number, coarse);
        self.cached_page_fragments.get(&page_number)
    }

    fn page_fragment_for_kind(
        &mut self,
        page_number: u32,
        kind: PageFragmentKind,
    ) -> Option<&RecordedScene> {
        self.ensure_page_static_fragment(page_number)?;
        match kind {
            PageFragmentKind::Full => self.cached_page_fragments.get(&page_number),
            PageFragmentKind::Coarse => self
                .cached_page_coarse_fragments
                .get(&page_number)
                .or_else(|| self.cached_page_fragments.get(&page_number)),
            PageFragmentKind::Base => self
                .cached_page_base_fragments
                .get(&page_number)
                .or_else(|| self.cached_page_fragments.get(&page_number)),
            PageFragmentKind::Detail => self.cached_page_detail_fragments.get(&page_number),
        }
    }

    fn page_fragment_for_draw_transformed(
        &mut self,
        page_number: u32,
        kind: PageFragmentKind,
        scene_transform: Affine,
    ) -> Option<&RecordedScene> {
        let key = transformed_page_key(page_number, kind, scene_transform);
        if self.cached_transformed_fragments.contains_key(&key) {
            return self.cached_transformed_fragments.get(&key);
        }

        let mut transformed = RecordedScene::new();
        {
            let base_fragment = self.page_fragment_for_kind(page_number, kind)?;
            replay_recorded_scene(&mut transformed, base_fragment, scene_transform);
        }

        self.insert_transformed_fragment(key, transformed);
        self.cached_transformed_fragments.get(&key)
    }

    fn insert_transformed_fragment(&mut self, key: TransformedPageKey, value: RecordedScene) {
        if let std::collections::hash_map::Entry::Occupied(mut e) =
            self.cached_transformed_fragments.entry(key)
        {
            e.insert(value);
            return;
        }

        while self.cached_transformed_fragments.len() >= self.transformed_fragment_capacity {
            if let Some(oldest) = self.transformed_fragment_order.pop_front() {
                self.cached_transformed_fragments.remove(&oldest);
            } else {
                break;
            }
        }

        self.transformed_fragment_order.push_back(key);
        self.cached_transformed_fragments.insert(key, value);
    }

    fn lod_tier_for_page(
        &self,
        projected_page_width: f64,
        visible_page_count: usize,
        use_multi_page_lod: bool,
        is_focused: bool,
    ) -> Option<PageLodTier> {
        if !use_multi_page_lod {
            return None;
        }

        // In very dense views, prioritize throughput for non-focused pages.
        if visible_page_count >= 4 && !is_focused {
            return Some(PageLodTier::Low);
        }

        // Keep focused page vector when it's large on screen.
        let focused_vector_cutoff = 860.0;
        let unfocused_vector_cutoff = 1120.0;
        let vector_cutoff = if is_focused {
            focused_vector_cutoff
        } else {
            unfocused_vector_cutoff
        };
        if projected_page_width >= vector_cutoff {
            return None;
        }

        if projected_page_width < 360.0 {
            Some(PageLodTier::Low)
        } else if projected_page_width < 560.0 {
            Some(PageLodTier::Medium)
        } else {
            Some(PageLodTier::High)
        }
    }

    fn ensure_page_lod_image(
        &mut self,
        page_number: u32,
        tier: PageLodTier,
        layer: PageLodLayer,
    ) -> Option<&ImageBrush> {
        let key = PageLodKey {
            page_number,
            tier,
            layer,
        };
        if self.cached_page_lod_images.contains_key(&key) {
            return self.cached_page_lod_images.get(&key);
        }

        let (page_x, page_y, page_w, page_h) = {
            let layout = self.layout_result.as_ref()?;
            let page = layout.pages.iter().find(|p| p.number == page_number)?;
            (page.x_offset, page.y_offset, page.width, page.height)
        };

        let fragment_kind = match layer {
            PageLodLayer::Full => PageFragmentKind::Full,
            PageLodLayer::Detail => PageFragmentKind::Detail,
        };
        let fragment = self
            .page_fragment_for_kind(page_number, fragment_kind)?
            .clone();

        let lod_scale = std::env::var("KEYFLOW_PAGE_LOD_SCALE")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(tier.default_scale())
            .clamp(0.15, 0.8);
        let image_width = (page_w * lod_scale).round().max(64.0) as u32;
        let image_height = (page_h * lod_scale).round().max(64.0) as u32;

        let renderer = self
            .page_lod_renderer
            .get_or_insert_with(|| new_offscreen_renderer(image_width, image_height));
        renderer.resize(image_width, image_height);

        let sx = image_width as f64 / page_w;
        let sy = image_height as f64 / page_h;
        let page_to_image =
            Affine::scale_non_uniform(sx, sy) * Affine::translate((-page_x, -page_y));

        let mut pixels = Vec::new();
        renderer.render_to_vec(
            |painter| replay_recorded_scene(painter, &fragment, page_to_image),
            &mut pixels,
        );

        let image_data = ImageData {
            data: pixels.into(),
            format: ImageFormat::Rgba8,
            alpha_type: ImageAlphaType::Alpha,
            width: image_width,
            height: image_height,
        };
        let lod_quality = match std::env::var("KEYFLOW_PAGE_LOD_QUALITY")
            .unwrap_or_else(|_| "medium".to_string())
            .to_ascii_lowercase()
            .as_str()
        {
            "low" => ImageQuality::Low,
            "high" => ImageQuality::High,
            _ => tier.default_quality(),
        };
        let brush = ImageBrush::new(image_data).with_quality(lod_quality);
        self.cached_page_lod_images.insert(key, brush);
        self.cached_page_lod_images.get(&key)
    }

    /// Render the chart to a Vello scene.
    ///
    /// Uses a cached vello Scene to avoid re-walking the scene graph every frame.
    /// The cached scene is built once at identity transform when the layout changes,
    /// then composited into the output scene with `Scene::append(cached, transform)`
    /// for instant pan/zoom (~1-2ms vs ~67ms for full re-render).
    ///
    /// If `cursor_tick` is `Some`, the cursor highlight commands are rendered on top
    /// of the chart using the same transform. The cursor is NOT part of the cached
    /// scene because it changes independently of the layout.
    ///
    /// If `hover_point` is `Some((scene_x, scene_y))`, a blue highlight is rendered
    /// on the nearest beat to that scene coordinate.
    #[allow(clippy::too_many_arguments)]
    pub fn render_to_scene(
        &mut self,
        scene: &mut impl PaintScene,
        width: f64,
        height: f64,
        offset: Affine,
        transform: Affine,
        cursor_tick: Option<i64>,
        hover_point: Option<(f64, f64)>,
    ) {
        self.render_static_layer_to_scene(scene, width, height, offset, transform);
        self.render_overlay_layer_to_scene(
            scene,
            width,
            height,
            offset,
            transform,
            cursor_tick,
            hover_point,
        );
    }

    /// Get the cached layout result.
    pub fn layout_result(&self) -> Option<&ChartLayoutResult> {
        self.layout_result.as_ref()
    }

    /// Get the current layout hash (for change detection).
    pub fn last_chart_hash(&self) -> u64 {
        self.last_chart_hash
    }

    /// Get the content dimensions in points (with 20pt padding).
    pub fn get_content_dimensions(&self) -> Option<(f64, f64)> {
        self.layout_result.as_ref().map(|layout| {
            let bounds = layout.scene.compute_bounds();
            if bounds.width() > 0.0 && bounds.height() > 0.0 {
                (bounds.x1 + 20.0, bounds.y1 + 20.0)
            } else {
                (A4_WIDTH, A4_HEIGHT)
            }
        })
    }

    /// Compute the base scale factor to fit the chart content within the viewport.
    ///
    /// Returns a scale that converts from layout points to physical pixels such that
    /// the chart content fits horizontally within the viewport with padding.
    ///
    /// - **Paginated mode**: fits a single A4 page width (595pt)
    /// - **Snippet mode**: fits the actual scene content width
    pub fn fit_to_width_scale(&self, viewport_width_physical: f64, device_pixel_ratio: f64) -> f64 {
        let padding_physical = 40.0 * device_pixel_ratio; // 20 CSS px padding on each side
        let available = viewport_width_physical - padding_physical;
        if available <= 0.0 {
            return DPI_SCALE * device_pixel_ratio;
        }

        // In paginated mode, fit to the single page width (not the full multi-page scene).
        // In snippet mode, fit to the actual content width.
        let content_width = if self.last_preview_mode == PreviewMode::Snippet {
            // Snippet: use actual scene bounds.
            self.get_content_dimensions()
                .map(|(w, _)| w)
                .unwrap_or(LETTER_WIDTH)
        } else {
            // Paginated/responsive: use the first page width.
            self.layout_result
                .as_ref()
                .and_then(|layout| layout.pages.first().map(|p| p.width))
                .unwrap_or(LETTER_WIDTH)
        };

        let scale = available / content_width;
        // Clamp: don't scale up beyond natural DPI×dpr (1:1 point-to-pixel mapping)
        scale.min(DPI_SCALE * device_pixel_ratio)
    }

    /// Extract lightweight page+system metadata from the current layout result.
    ///
    /// Returns `None` if no layout has been computed. The metadata is used by
    /// the UI for page navigation and semantic zoom calculations.
    pub fn page_metadata(&self) -> Option<Vec<PageMeta>> {
        self.layout_result.as_ref().map(|result| {
            result
                .pages
                .iter()
                .map(|page| PageMeta {
                    number: page.number,
                    x_offset: page.x_offset,
                    y_offset: page.y_offset,
                    width: page.width,
                    height: page.height,
                    systems: page
                        .systems
                        .iter()
                        .map(|sys| SystemMeta {
                            y: sys.y,
                            height: sys.height,
                        })
                        .collect(),
                })
                .collect()
        })
    }

    /// Export the current paginated layout as a multi-page vector PDF.
    ///
    /// The layout must already be computed in paginated mode (`snippet_mode = false`).
    /// Returns a complete PDF document as bytes.
    pub fn export_pdf_bytes(&self) -> Result<Vec<u8>, String> {
        let layout = self
            .layout_result
            .as_ref()
            .ok_or_else(|| "No chart layout available to export".to_string())?;

        if layout.pages.is_empty() {
            return Err("No pages available to export".to_string());
        }

        let mut svg_pages = Vec::with_capacity(layout.pages.len());
        for page in &layout.pages {
            let config =
                SvgExportConfig::for_page(page.x_offset, page.y_offset, page.width, page.height)
                    .with_embedded_font(
                        "Bravura",
                        self.font_bundle.symbol_font_data().as_ref().clone(),
                    )
                    .with_embedded_font(
                        "MuseJazzText",
                        self.font_bundle.text_font_data().as_ref().clone(),
                    )
                    .with_embedded_font(
                        "FreeSans",
                        self.font_bundle.aux_font_data().as_ref().clone(),
                    );

            let mut serializer = SvgSerializer::new(config);
            svg_pages.push(serializer.serialize(&layout.scene));
        }

        let symbol_font = self.font_bundle.symbol_font_data();
        let text_font = self.font_bundle.text_font_data();
        let aux_font = self.font_bundle.aux_font_data();

        PdfSerializer::serialize_from_svg(
            &svg_pages,
            &[
                ("Bravura", symbol_font.as_slice()),
                ("MuseJazzText", text_font.as_slice()),
                ("FreeSans", aux_font.as_slice()),
            ],
        )
        .map_err(|e| format!("Failed to export PDF: {e}"))
    }

    /// Export the current paginated layout as per-page SVG documents.
    ///
    /// The layout must already be computed in paginated mode (`snippet_mode = false`).
    /// Returns one SVG string per page.
    pub fn export_svg_pages(&self) -> Result<Vec<String>, String> {
        let layout = self
            .layout_result
            .as_ref()
            .ok_or_else(|| "No chart layout available to export".to_string())?;

        if layout.pages.is_empty() {
            return Err("No pages available to export".to_string());
        }

        let mut pages = Vec::with_capacity(layout.pages.len());
        for page in &layout.pages {
            let config =
                SvgExportConfig::for_page(page.x_offset, page.y_offset, page.width, page.height)
                    .with_embedded_font(
                        "Bravura",
                        self.font_bundle.symbol_font_data().as_ref().clone(),
                    )
                    .with_embedded_font(
                        "MuseJazzText",
                        self.font_bundle.text_font_data().as_ref().clone(),
                    )
                    .with_embedded_font(
                        "FreeSans",
                        self.font_bundle.aux_font_data().as_ref().clone(),
                    );
            let mut serializer = SvgSerializer::new(config);
            pages.push(serializer.serialize(&layout.scene));
        }

        Ok(pages)
    }

    /// Determine which page is currently visible given scroll values.
    ///
    /// Pages are laid out **horizontally** (side by side). Converts scroll_x
    /// back to scene coordinates and finds which page's X range contains
    /// the viewport center. Returns 1-indexed page number.
    pub fn current_page_for_scroll(
        &self,
        scroll_x: f64,
        base_scale: f64,
        zoom: f64,
        dpr: f64,
    ) -> u32 {
        let Some(result) = &self.layout_result else {
            return 1;
        };
        if result.pages.is_empty() {
            return 1;
        }

        // Convert scroll_x to scene coordinates.
        // From transform: screen_x = pad - scroll_x * dpr + scene_x * base_scale * zoom
        // At the viewport left (screen_x = pad): scene_x = scroll_x * dpr / (base_scale * zoom)
        let scale = base_scale * zoom;
        if scale <= 0.0 {
            return 1;
        }
        let scene_x_left = (scroll_x * dpr) / scale;

        // Find the page whose X range contains scene_x_left (search from last to first)
        for page in result.pages.iter().rev() {
            if scene_x_left >= page.x_offset {
                return page.number;
            }
        }
        1
    }

    /// Get total page count from current layout.
    pub fn total_pages(&self) -> u32 {
        self.layout_result
            .as_ref()
            .map(|r| r.pages.len() as u32)
            .unwrap_or(1)
    }

    /// Find the page number and system index for a given absolute tick.
    ///
    /// Returns `(page_number, system_index_on_page)` where page_number is 1-indexed.
    /// Uses BeatPosition data to locate the tick, then maps it to the page/system
    /// via page layout metadata.
    pub fn system_for_tick(&self, tick: i64) -> Option<(u32, usize)> {
        let layout = self.layout_result.as_ref()?;

        // Find the BeatPosition closest to (or containing) this tick.
        // beat_positions are sorted by time_start, but we search by absolute_tick.
        let beat = layout
            .beat_positions
            .iter()
            .min_by_key(|bp| (bp.absolute_tick - tick).unsigned_abs())?;

        Some((beat.page, beat.system))
    }

    /// Compute scroll_y to position a specific system at the top of the viewport.
    ///
    /// Given a page number (1-indexed) and system index on that page,
    /// returns the scroll_y value that places that system at the viewport top.
    ///
    /// `base_scale`, `zoom`, and `dpr` are needed to convert from scene coordinates
    /// to scroll coordinates: `scroll_y = scene_y * base_scale * zoom / dpr`.
    pub fn scroll_y_for_system(
        &self,
        page_number: u32,
        system_index: usize,
        base_scale: f64,
        zoom: f64,
        dpr: f64,
    ) -> Option<f64> {
        let layout = self.layout_result.as_ref()?;
        let page = layout.pages.iter().find(|p| p.number == page_number)?;
        let system = page.systems.get(system_index)?;

        // Scene-space Y of the system = page.y_offset + system.y
        let scene_y = page.y_offset + system.y;

        // Convert to scroll_y: from the transform equation
        // screen_y = pad - scroll_y * dpr + scene_y * base_scale * zoom
        // We want screen_y ≈ pad (system at top with a small margin above)
        // → scroll_y = scene_y * base_scale * zoom / dpr
        // Subtract a small margin (e.g. 5pt worth) so system isn't flush against top edge
        let margin_pts = 5.0;
        let scroll_y = (scene_y - margin_pts) * base_scale * zoom / dpr;
        Some(scroll_y.max(0.0))
    }

    /// Compute zoom factor to show N systems in the viewport height.
    ///
    /// Takes the page metadata and picks the first N systems' total height
    /// (including spacing between them) and computes the zoom that fits that
    /// height into `viewport_height_physical` pixels.
    pub fn zoom_for_n_systems(
        &self,
        page_number: u32,
        system_index: usize,
        n: usize,
        base_scale: f64,
        viewport_height_physical: f64,
    ) -> Option<f64> {
        let layout = self.layout_result.as_ref()?;
        let page = layout.pages.iter().find(|p| p.number == page_number)?;

        if page.systems.is_empty() {
            return None;
        }

        // Get the N systems starting from system_index
        let start = system_index.min(page.systems.len().saturating_sub(1));
        let end = (start + n).min(page.systems.len());
        let systems = &page.systems[start..end];

        if systems.is_empty() {
            return None;
        }

        // Total height from top of first system to bottom of last system
        let first_y = systems.first().unwrap().y;
        let last = systems.last().unwrap();
        let content_height = (last.y + last.height) - first_y;

        if content_height <= 0.0 || base_scale <= 0.0 {
            return None;
        }

        // Add padding (10pt above and below)
        let padded_height = content_height + 20.0;

        // Fit N systems vertically: zoom = viewport_height / (content_height * base_scale).
        // This may exceed 1.0, meaning the page is wider than the viewport — the caller
        // is responsible for setting scroll_x/scroll_y to center or left-align the content.
        let zoom = viewport_height_physical / (padded_height * base_scale);
        Some(zoom.clamp(0.1, 8.0))
    }

    // ========================================================================
    // Cursor
    // ========================================================================

    /// Update the cached cursor state for a given tick.
    ///
    /// Skips recomputation if the tick hasn't changed since last call.
    fn update_cursor(&mut self, tick: i64) {
        if tick == self.cached_cursor_tick && self.cached_cursor_state.is_some() {
            return;
        }
        self.cached_cursor_tick = tick;
        self.cached_cursor_state = self
            .layout_result
            .as_ref()
            .and_then(|layout| self.cursor.compute(layout, tick));
    }

    /// Get the current cursor state (if any).
    pub fn cursor_state(&self) -> Option<&CursorState> {
        self.cached_cursor_state.as_ref()
    }

    /// Render a blue hover highlight at (scene_x, scene_y).
    ///
    /// First tries scene graph hit-testing to highlight the exact symbol
    /// under the mouse. Falls back to nearest-beat heuristic if no scene
    /// node was hit directly.
    fn render_hover_highlight(
        &self,
        scene: &mut impl PaintScene,
        scene_x: f64,
        scene_y: f64,
        transform: Affine,
    ) {
        let Some(layout) = &self.layout_result else {
            return;
        };

        // Try scene graph hit-test first for exact symbol highlighting
        if let Some(hit) = self.hit_test_at_point(scene_x, scene_y) {
            // Find the matching BeatPosition for glyph/staff info
            if let Some(beat) = layout.beat_positions.iter().find(|bp| {
                bp.measure == hit.position.measure as usize && bp.beat == hit.position.beat as usize
            }) {
                self.render_beat_hover(scene, beat, scene_x, transform);
                return;
            }
        }

        // Fallback: nearest-beat heuristic
        let page_number = layout
            .pages
            .iter()
            .rev()
            .find(|p| scene_x >= p.x_offset)
            .map(|p| p.number)
            .unwrap_or(1);

        let page_beats = layout.beats_on_page(page_number);
        if page_beats.is_empty() {
            return;
        }

        let mut best = &page_beats[0];
        let mut best_dist = f64::INFINITY;
        for beat in &page_beats {
            let y_dist = if scene_y < beat.staff_y {
                beat.staff_y - scene_y
            } else if scene_y > beat.staff_y + beat.staff_height {
                scene_y - (beat.staff_y + beat.staff_height)
            } else {
                0.0
            };
            let x_center = beat.x + beat.width / 2.0;
            let x_dist = (scene_x - x_center).abs();
            let combined = x_dist + y_dist * 3.0;
            if combined < best_dist {
                best_dist = combined;
                best = beat;
            }
        }

        if best_dist > 80.0 {
            return;
        }

        self.render_beat_hover(scene, best, scene_x, transform);
    }

    /// Render blue hover highlight commands for a specific BeatPosition.
    fn render_beat_hover(
        &self,
        scene: &mut impl PaintScene,
        beat: &BeatPosition,
        scene_x: f64,
        transform: Affine,
    ) {
        let hover_color: Rgba = [60, 130, 255, 255];
        let hover_fill: Rgba = [60, 130, 255, 50];
        let glow_color: Rgba = [60, 130, 255, 100];
        let mut commands = Vec::new();

        // Beat box background
        commands.push(HighlightCommand::FillRoundedRect {
            x: beat.x,
            y: beat.staff_y,
            width: beat.width,
            height: beat.staff_height,
            radius: 2.0,
            color: hover_fill,
        });

        // Vertical line at hover X
        let ext = beat.staff_height * 0.15;
        commands.push(HighlightCommand::StrokeLine {
            x: scene_x.clamp(beat.x, beat.x + beat.width),
            y_top: beat.staff_y - ext,
            y_bottom: beat.staff_y + beat.staff_height + ext,
            color: [60, 130, 255, 120],
            width: 2.0,
        });

        // Notehead glyph highlight
        if let Some(codepoint) = beat.glyph_codepoint {
            let font_size = beat.glyph_size * 4.0;
            let glyph_x = beat.x;
            let glyph_y = beat.staff_y + beat.staff_height * 0.5;

            commands.push(HighlightCommand::StrokeGlyph {
                codepoint,
                font_size,
                x: glyph_x,
                y: glyph_y,
                stroke_width: 4.0,
                color: glow_color,
            });
            commands.push(HighlightCommand::FillGlyph {
                codepoint,
                font_size,
                x: glyph_x,
                y: glyph_y,
                color: hover_color,
            });
        }

        let font = self.font_bundle.smufl_font();
        render_cursor_commands(scene, &commands, transform, Some(font));
    }

    /// Get the chart's time signature as (numerator, denominator).
    ///
    /// Returns the initial/current time signature from the parsed chart,
    /// defaulting to 4/4 if not specified.
    pub fn time_signature(&self) -> (u8, u8) {
        self.cached_chart
            .as_ref()
            .and_then(|chart| {
                chart
                    .time_signature
                    .or(chart.initial_time_signature)
                    .map(|ts| (ts.numerator as u8, ts.denominator as u8))
            })
            .unwrap_or((4, 4))
    }

    /// Compute the musical position string for a given tick.
    ///
    /// Format: "M.B.TTT" where M = measure (1-indexed), B = beat (1-indexed),
    /// TTT = sub-beat ticks within that beat.
    ///
    /// Uses each `BeatPosition`'s own `time_signature` field to derive the
    /// musical beat from `BeatPosition.tick` (in-measure tick offset). This
    /// handles mid-chart time signature changes correctly.
    ///
    /// Examples in 4/4 time (480 ticks/quarter):
    /// - Beat 1: tick=0 → "M.1.000"
    /// - First triplet after beat 1: tick=160 → "M.1.160"
    /// - Beat 2: tick=480 → "M.2.000"
    /// - Eighth note of beat 3: tick=960+240=1200 → "M.3.240"
    pub fn musical_position_for_tick(&self, tick: i64) -> String {
        let Some(layout) = &self.layout_result else {
            return "1.1.000".to_string();
        };

        // Find which beat position contains this tick
        if let Some(beat) = layout.beat_at_tick(tick) {
            let measure_display = beat.measure as i64 + 1; // 1-indexed

            // Use this beat's own time signature for correct per-measure calculation
            let ts = beat.time_signature;
            let ticks_per_beat = ticks_per_beat_for_denom(ts.1);

            // Derive musical beat from the in-measure tick offset, not segment index.
            // beat.tick is the tick offset from measure start (e.g., 0, 160, 320, 480...).
            let in_measure_tick = beat.tick as i64;
            let musical_beat = (in_measure_tick / ticks_per_beat) + 1; // 1-indexed
            let sub_ticks = in_measure_tick % ticks_per_beat;

            // Clamp beat to valid range (1..=numerator)
            let musical_beat = musical_beat.clamp(1, ts.0 as i64);

            format!("{}.{}.{:03}", measure_display, musical_beat, sub_ticks)
        } else if tick < 0 {
            // Count-in territory: negative ticks — use chart-level time signature
            let time_sig = self.time_signature();
            let ticks_per_beat = ticks_per_beat_for_denom(time_sig.1);
            let ticks_per_measure = time_sig.0 as i64 * ticks_per_beat;
            let abs_tick = tick.unsigned_abs() as i64;
            let measure_back = abs_tick / ticks_per_measure;
            let remaining = abs_tick % ticks_per_measure;
            let beat_back = remaining / ticks_per_beat;
            let sub = remaining % ticks_per_beat;
            format!("-{}.{}.{:03}", measure_back + 1, beat_back + 1, sub)
        } else {
            // Beyond layout — use chart-level time signature
            let time_sig = self.time_signature();
            let ticks_per_beat = ticks_per_beat_for_denom(time_sig.1);
            let ticks_per_measure = time_sig.0 as i64 * ticks_per_beat;
            let measure = tick / ticks_per_measure;
            let remaining = tick % ticks_per_measure;
            let beat = remaining / ticks_per_beat;
            let sub = remaining % ticks_per_beat;
            format!("{}.{}.{:03}", measure + 1, beat + 1, sub)
        }
    }

    /// Compute zero-indexed musical coordinates for a given absolute tick.
    ///
    /// Returns `(measure, beat, subdivision)` where `subdivision` is 0..=999.
    pub fn musical_position_at_tick(&self, tick: i64) -> Option<(i32, i32, i32)> {
        let layout = self.layout_result.as_ref()?;
        let beat = layout.beat_at_tick(tick)?;

        let ts = beat.time_signature;
        let ticks_per_beat = ticks_per_beat_for_denom(ts.1);
        if ticks_per_beat <= 0 {
            return None;
        }

        // Recover in-measure tick from the located beat anchor and delta.
        let delta = tick - beat.absolute_tick;
        let in_measure_tick = (beat.tick as i64 + delta).max(0);
        let beat_index = (in_measure_tick / ticks_per_beat).max(0);
        let sub_ticks = (in_measure_tick % ticks_per_beat).max(0);
        let subdivision = ((sub_ticks * 1000) / ticks_per_beat).clamp(0, 999);

        Some((beat.measure as i32, beat_index as i32, subdivision as i32))
    }

    /// Convert zero-indexed musical coordinates to an absolute chart tick.
    ///
    /// Inputs are expected to use DAW-style coordinates: measure and beat are
    /// 0-indexed; subdivision is in the range 0..=999.
    pub fn tick_for_musical_position(
        &self,
        measure: i32,
        beat: i32,
        subdivision: i32,
    ) -> Option<i64> {
        let layout = self.layout_result.as_ref()?;
        if measure < 0 || beat < 0 || !(0..=999).contains(&subdivision) {
            return None;
        }

        let measure = measure as usize;
        let beat = beat as i64;

        let ts = layout
            .beat_positions
            .iter()
            .find(|bp| bp.measure == measure)
            .map(|bp| bp.time_signature)
            .unwrap_or((4, 4));

        let ticks_per_beat = ticks_per_beat_for_denom(ts.1);
        if ticks_per_beat <= 0 {
            return None;
        }

        let sub_ticks = ((subdivision as i64 * ticks_per_beat) / 1000).clamp(0, ticks_per_beat);
        let target_in_measure_tick = beat * ticks_per_beat + sub_ticks;

        let anchor = layout
            .beat_positions
            .iter()
            .filter(|bp| bp.measure == measure)
            .min_by_key(|bp| ((bp.tick as i64) - target_in_measure_tick).abs())?;

        Some(anchor.absolute_tick + (target_in_measure_tick - anchor.tick as i64))
    }

    /// Hit-test the scene graph at a point in scene coordinates.
    ///
    /// Walks the scene node tree, accumulating transforms, and finds the
    /// deepest node with `ChartPosition` metadata whose world-space bounds
    /// contain the point. Then resolves the absolute tick by matching the
    /// position's measure/beat to the `BeatPosition` array.
    ///
    /// Returns `None` if no node with chart position metadata contains the point.
    pub fn hit_test_at_point(&self, scene_x: f64, scene_y: f64) -> Option<SceneHitResult> {
        let layout = self.layout_result.as_ref()?;
        let point = Point::new(scene_x, scene_y);

        // Walk the scene graph to find nodes containing this point
        let mut hits = Vec::new();
        hit_test_scene_recursive(&layout.scene, point, Affine::IDENTITY, &mut hits);

        if hits.is_empty() {
            return None;
        }

        // Take the first (deepest) hit that has a ChartPosition
        let (node, world_transform) = &hits[0];
        let chart_pos: ChartPosition = node.get_json_metadata(metadata_keys::CHART_POSITION)?;
        let world_bounds = world_transform.transform_rect_bbox(node.bounds);
        let element_type = node.get_element_type().map(String::from);

        // Resolve absolute tick from the BeatPosition array by matching measure + beat index.
        // The ChartPosition.beat is the chord index within the measure, which maps
        // to the segment index in the BeatPosition array for that measure.
        let absolute_tick = layout
            .beat_positions
            .iter()
            .find(|bp| {
                bp.measure == chart_pos.measure as usize && bp.beat == chart_pos.beat as usize
            })
            .map(|bp| bp.absolute_tick)
            .unwrap_or_else(|| {
                // Fallback: calculate tick from position
                let ts = self.time_signature();
                chart_pos.calculate_tick(ts.0 as u32)
            });

        Some(SceneHitResult {
            position: chart_pos,
            absolute_tick,
            bounds: world_bounds,
            element_type,
        })
    }

    /// Hit-test: find the tick at a point in scene coordinates.
    ///
    /// First tries scene graph hit-testing for exact symbol matching.
    /// Falls back to nearest-beat heuristic if no scene node was hit
    /// (e.g., clicking in whitespace between symbols).
    pub fn tick_at_scene_point(&self, scene_x: f64, scene_y: f64) -> Option<i64> {
        // Try scene graph hit-test first for precise positioning
        if let Some(hit) = self.hit_test_at_point(scene_x, scene_y) {
            tracing::debug!(
                "Scene hit: {:?} at measure={} beat={} → tick={}",
                hit.element_type,
                hit.position.measure + 1,
                hit.position.beat + 1,
                hit.absolute_tick,
            );
            return Some(hit.absolute_tick);
        }

        // Fallback: nearest-beat heuristic for clicks in whitespace
        self.nearest_beat_at_point(scene_x, scene_y)
    }

    /// Nearest-beat heuristic fallback for click-to-position.
    ///
    /// Finds the closest `BeatPosition` to the given scene coordinates.
    /// Used when scene graph hit-testing doesn't find a direct node hit.
    fn nearest_beat_at_point(&self, scene_x: f64, scene_y: f64) -> Option<i64> {
        let layout = self.layout_result.as_ref()?;

        // Find which page contains this X (pages are horizontal, side by side)
        let page_number = layout
            .pages
            .iter()
            .rev()
            .find(|p| scene_x >= p.x_offset)
            .map(|p| p.number)
            .unwrap_or(1);

        let page_beats = layout.beats_on_page(page_number);
        if page_beats.is_empty() {
            return None;
        }

        // Find the beat whose x range is closest to scene_x
        let mut best = &page_beats[0];
        let mut best_dist = f64::INFINITY;
        for beat in &page_beats {
            let y_dist = if scene_y < beat.staff_y {
                beat.staff_y - scene_y
            } else if scene_y > beat.staff_y + beat.staff_height {
                scene_y - (beat.staff_y + beat.staff_height)
            } else {
                0.0
            };

            let x_center = beat.x + beat.width / 2.0;
            let x_dist = (scene_x - x_center).abs();

            // Weight Y distance more heavily to prefer beats on the correct staff line
            let combined = x_dist + y_dist * 3.0;
            if combined < best_dist {
                best_dist = combined;
                best = beat;
            }
        }

        Some(best.absolute_tick)
    }

    /// Get total ticks from the layout.
    pub fn total_ticks(&self) -> i64 {
        self.layout_result
            .as_ref()
            .map(|l| l.total_ticks())
            .unwrap_or(0)
    }

    // ========================================================================
    // Private helpers
    // ========================================================================

    /// Compute a hash of the chart source and layout mode for cache invalidation.
    fn compute_chart_hash(
        &self,
        source: &str,
        preview_mode: PreviewMode,
        viewport_width: f64,
        zoom: f64,
    ) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        source.hash(&mut hasher);
        preview_mode.hash(&mut hasher);
        match preview_mode {
            PreviewMode::Snippet => {
                ((viewport_width / 16.0).round() as i64).hash(&mut hasher);
            }
            PreviewMode::Responsive => {
                ((viewport_width / 16.0).round() as i64).hash(&mut hasher);
                let viewport_points = (viewport_width / DPI_SCALE).max(240.0);
                responsive_breakpoint(viewport_points, zoom).hash(&mut hasher);
            }
            PreviewMode::Page => {}
        }
        hasher.finish()
    }

    /// Check whether a layout is needed for the given source and mode.
    ///
    /// Returns `true` if the (source, snippet_mode) hash differs from the
    /// last successful layout. Used by the async layout path to avoid
    /// spawning background work when nothing has changed.
    pub fn needs_layout(&self, source: &str, snippet_mode: bool) -> bool {
        let preview_mode = if snippet_mode {
            PreviewMode::Snippet
        } else {
            PreviewMode::Page
        };
        self.needs_layout_for_preview_mode(source, preview_mode, 0.0, 1.0)
    }

    /// Check whether a layout is needed for the given source and preview mode.
    pub fn needs_layout_for_preview_mode(
        &self,
        source: &str,
        preview_mode: PreviewMode,
        viewport_width: f64,
        zoom: f64,
    ) -> bool {
        let chart_hash = self.compute_chart_hash(source, preview_mode, viewport_width, zoom);
        self.layout_result.is_none() || chart_hash != self.last_chart_hash
    }

    /// Get the font data needed to create a background `ChartLayoutEngine`.
    pub fn font_data(&self) -> (Arc<Vec<u8>>, Arc<Vec<u8>>) {
        (
            self.font_bundle.text_font_data().clone(),
            self.font_bundle.symbol_font_data().clone(),
        )
    }

    /// Apply a pre-computed layout result from a background thread.
    ///
    /// This does the same cache invalidation as `parse_and_layout`, but
    /// accepts a chart + result that were computed off the main thread.
    pub fn apply_precomputed_layout(
        &mut self,
        chart: Chart,
        result: ChartLayoutResult,
        source: &str,
        snippet_mode: bool,
    ) {
        let preview_mode = if snippet_mode {
            PreviewMode::Snippet
        } else {
            PreviewMode::Page
        };
        self.apply_precomputed_layout_with_preview_mode(
            chart,
            result,
            source,
            preview_mode,
            0.0,
            1.0,
        );
    }

    /// Apply a pre-computed layout result from a background thread.
    pub fn apply_precomputed_layout_with_preview_mode(
        &mut self,
        chart: Chart,
        result: ChartLayoutResult,
        source: &str,
        preview_mode: PreviewMode,
        viewport_width: f64,
        zoom: f64,
    ) {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        source.hash(&mut hasher);
        let source_hash = hasher.finish();

        self.cached_chart = Some(chart);
        self.last_source_hash = source_hash;
        self.layout_result = Some(result);
        self.last_chart_hash = self.compute_chart_hash(source, preview_mode, viewport_width, zoom);
        self.last_preview_mode = preview_mode;
        self.cached_cursor_state = None;
        self.cached_cursor_tick = i64::MIN;
        self.cached_page_fragments.clear();
        self.cached_page_base_fragments.clear();
        self.cached_page_detail_fragments.clear();
        self.cached_page_coarse_fragments.clear();
        self.cached_transformed_fragments.clear();
        self.transformed_fragment_order.clear();
        self.cached_page_lod_images.clear();
        self.page_lod_renderer = None;
        self.cached_view_static_image = None;
        self.cached_view_static_key = None;
        self.last_view_static_key = None;
        self.view_static_stable_frames = 0;
        self.view_static_renderer = None;
        self.last_transform_key = None;
        self.transform_stable_frames = 0;
        self.last_cull_key = None;
        self.cached_visible_pages.clear();
        self.cached_focus_page = None;
        self.rebuild_page_geometry_cache();
    }
}
