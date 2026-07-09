//! WGPU/Vello renderer for music notation.
//!
//! This module provides GPU-accelerated rendering of music notation
//! using Vello for 2D vector graphics.
//!
//! ## Components
//!
//! - [`VelloSceneRenderer`] - Renders scene graph to Vello for GPU output
//! - [`EngraverRenderer`] - High-level renderer API
//! - [`primitives`] - Low-level WGPU vertex types and shaders

// region:    --- Modules

pub mod canvas2d;
#[cfg(feature = "engraver-example")]
pub mod context;
pub mod cursor_renderer;
pub mod primitives;
pub mod scene_renderer;

// endregion: --- Modules

// region:    --- Re-exports

pub use canvas2d::{Canvas2D, Color as Canvas2DColor, Rect as Canvas2DRect, Vertex2D};
pub use cursor_renderer::render_cursor_commands;
pub use primitives::{
    BLIT_SHADER_SOURCE, BlitVertex, CameraUniform, SDF_SHADER_SOURCE, SHADER_SOURCE, SdfRectVertex,
    Vertex, create_blit_pipeline, create_blit_texture_bind_group_layout,
    create_camera_bind_group_layout, create_fullscreen_quad, create_line, create_main_pipeline,
    create_rect, create_sdf_pipeline, create_sdf_rounded_rect, px_to_ndc,
};
pub use scene_renderer::{SceneRenderBuilder, SceneRenderConfig, VelloSceneRenderer};

#[cfg(feature = "engraver-example")]
pub use context::VelloRenderContext;

// endregion: --- Re-exports

// region:    --- RenderConfig

use peniko::Color;

/// Renderer configuration.
#[derive(Debug, Clone)]
pub struct RenderConfig {
    /// Background color
    pub background_color: Color,
    /// Staff line color
    pub staff_color: Color,
    /// Note and symbol color
    pub foreground_color: Color,
    /// Selection highlight color
    pub selection_color: Color,
    /// Staff space size in pixels
    pub staff_space: f64,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            background_color: Color::WHITE,
            staff_color: Color::BLACK,
            foreground_color: Color::BLACK,
            selection_color: Color::from_rgb8(0, 120, 215),
            staff_space: 10.0,
        }
    }
}

// endregion: --- RenderConfig

// The high-level `EngraverRenderer` stub was removed. The real chart rendering
// pipeline goes `layout_chart` → `SceneNode` → `VelloSceneRenderer`. Use
// [`scene_renderer::SceneRenderBuilder`] / [`VelloSceneRenderer`] directly.
