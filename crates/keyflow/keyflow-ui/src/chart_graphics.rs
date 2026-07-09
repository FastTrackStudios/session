//! Chart graphics context for GPU-accelerated chart rendering.
//!
//! Provides platform-specific rendering backends:
//! - **Desktop** (`desktop-panels`): `VelloWindowRenderer` with WGPU behind a transparent WebView
//! - **WASM** (`wasm-panels`): `vello_hybrid::WebGlRenderer` rendering to an HTML canvas

// ─── Desktop backend (VelloWindowRenderer + WGPU) ───────────────────────────

#[cfg(feature = "desktop-panels")]
mod desktop {
    use std::sync::Arc;

    use anyrender::WindowRenderer;
    use anyrender_vello::{VelloRendererOptions, VelloWindowRenderer};
    use dioxus::desktop::tao::window::Window;
    use peniko::Color;
    use vello::AaConfig;

    /// Chart graphics context wrapping anyrender's VelloWindowRenderer.
    ///
    /// Renders charts to the window's WGPU surface, with the Dioxus WebView
    /// overlaid on top as a transparent child window.
    pub struct ChartGraphics {
        renderer: VelloWindowRenderer,
        width: u32,
        height: u32,
    }

    impl ChartGraphics {
        fn antialiasing_from_env(var: &str, default: &str) -> AaConfig {
            match std::env::var("KEYFLOW_AA")
                .or_else(|_| std::env::var(var))
                .unwrap_or_else(|_| default.to_string())
                .to_ascii_lowercase()
                .as_str()
            {
                "msaa16" => AaConfig::Msaa16,
                "msaa8" => AaConfig::Msaa8,
                _ => AaConfig::Area,
            }
        }

        /// Create a new ChartGraphics context for the given window.
        ///
        /// Initializes the Vello GPU renderer via anyrender.
        /// The window should have transparency enabled for the hybrid overlay to work.
        pub fn new(window: Arc<Window>, width: u32, height: u32) -> Self {
            let current_aa = Self::antialiasing_from_env("KEYFLOW_AA_QUALITY", "msaa8");
            tracing::info!("ChartGraphics AA mode: {:?}", current_aa);

            let mut renderer = VelloWindowRenderer::with_options(VelloRendererOptions {
                base_color: Color::TRANSPARENT,
                antialiasing_method: current_aa,
                ..Default::default()
            });

            renderer.resume(window.clone(), width, height, || {});

            Self {
                renderer,
                width,
                height,
            }
        }

        /// Use lower AA while actively interacting (pan/zoom), higher AA when idle.
        pub fn set_interaction_active(&mut self, active: bool) {
            // Runtime renderer hot-swap can disrupt WebView layering in hybrid mode.
            // Keep fixed AA for stability; runtime switching can be reintroduced behind
            // a safer surface reconfiguration path.
            let _ = active;
        }

        /// Resize the rendering surface.
        pub fn resize(&mut self, width: u32, height: u32) {
            self.width = width;
            self.height = height;
            self.renderer.set_size(width, height);
        }

        /// Check if the renderer is active.
        pub fn is_active(&self) -> bool {
            self.renderer.is_active()
        }

        /// Render using a drawing function.
        pub fn render<F>(&mut self, draw_fn: F)
        where
            F: FnOnce(&mut <VelloWindowRenderer as WindowRenderer>::ScenePainter<'_>),
        {
            self.renderer.render(draw_fn);
        }

        /// Get the current surface dimensions.
        pub fn size(&self) -> (u32, u32) {
            (self.width, self.height)
        }

        /// Render chart content through the anyrender PaintScene abstraction.
        pub fn render_chart<F>(&mut self, draw_fn: F)
        where
            F: FnOnce(&mut <VelloWindowRenderer as WindowRenderer>::ScenePainter<'_>),
        {
            self.renderer.render(draw_fn);
        }
    }

    impl Drop for ChartGraphics {
        fn drop(&mut self) {
            self.renderer.suspend();
        }
    }
}

#[cfg(feature = "desktop-panels")]
pub use desktop::ChartGraphics;

// ─── WASM backend (vello_hybrid WebGlRenderer) ─────────────────────────────

#[cfg(all(feature = "wasm-panels", target_arch = "wasm32"))]
mod wasm {
    use anyrender_vello_hybrid::{WebGlImageManager, WebGlScenePainter};
    use rustc_hash::FxHashMap;
    use vello_hybrid::{RenderSettings, RenderSize, Scene, WebGlRenderer};
    use web_sys::HtmlCanvasElement;

    /// Chart graphics context wrapping vello_hybrid's WebGlRenderer.
    ///
    /// Renders charts to an HTML `<canvas>` element via WebGL2.
    /// Uses the same `anyrender::PaintScene` abstraction as the desktop backend,
    /// so `ChartLayoutManager::render_to_scene()` works identically.
    pub struct ChartGraphics {
        renderer: WebGlRenderer,
        scene: Scene,
        cached_images: FxHashMap<u64, vello_common::paint::ImageId>,
        width: u32,
        height: u32,
    }

    impl ChartGraphics {
        /// Create a new ChartGraphics context targeting an HTML canvas element.
        ///
        /// Initializes a WebGL2 rendering context from the canvas.
        pub fn new_web(canvas: &HtmlCanvasElement, width: u32, height: u32) -> Self {
            let renderer = WebGlRenderer::new(canvas);
            let scene = Scene::new_with(width as u16, height as u16, RenderSettings::default());
            tracing::info!("ChartGraphics WebGL: {}x{}", width, height);

            Self {
                renderer,
                scene,
                cached_images: FxHashMap::default(),
                width,
                height,
            }
        }

        /// Resize the rendering surface.
        pub fn resize(&mut self, width: u32, height: u32) {
            self.width = width;
            self.height = height;
            self.scene = Scene::new_with(width as u16, height as u16, RenderSettings::default());
        }

        /// Check if the renderer is active (always true for WebGL once created).
        pub fn is_active(&self) -> bool {
            true
        }

        /// Get the current surface dimensions.
        pub fn size(&self) -> (u32, u32) {
            (self.width, self.height)
        }

        /// Render chart content through the anyrender PaintScene abstraction.
        ///
        /// The draw function receives a `WebGlScenePainter` which implements
        /// `anyrender::PaintScene`, making it compatible with `ChartLayoutManager`.
        pub fn render_chart<F>(&mut self, draw_fn: F)
        where
            F: FnOnce(&mut WebGlScenePainter<'_>),
        {
            // Build the scene via the PaintScene-compatible painter.
            // WebGlImageManager borrows &mut renderer, so scope it.
            {
                let image_manager =
                    WebGlImageManager::new(&mut self.renderer, &mut self.cached_images);
                let mut painter = WebGlScenePainter::new(&mut self.scene, image_manager);
                draw_fn(&mut painter);
            }

            // Render the built scene to the canvas via WebGL2.
            let render_size = RenderSize {
                width: self.width,
                height: self.height,
            };
            if let Err(e) = self.renderer.render(&self.scene, &render_size) {
                tracing::error!("WebGL chart render failed: {:?}", e);
            }

            self.scene.reset();
        }
    }
}

#[cfg(all(feature = "wasm-panels", target_arch = "wasm32"))]
pub use wasm::ChartGraphics;
