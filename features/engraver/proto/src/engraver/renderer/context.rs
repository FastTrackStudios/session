//! Reusable GPU infrastructure for Vello-based rendering.
//!
//! Provides a `VelloRenderContext` that encapsulates all the wgpu and vello
//! setup required for rendering, allowing examples and applications to focus
//! on their domain-specific logic.

use std::sync::Arc;

use vello::Scene;
use wgpu::{
    DeviceDescriptor, Features, Instance, InstanceDescriptor, RequestAdapterOptions,
    TextureDescriptor, TextureDimension, TextureUsages, TextureViewDescriptor,
};
use winit::window::Window;

use crate::engraver::error::{Error, Result};

// region:    --- VelloRenderContext

/// GPU infrastructure for Vello rendering.
///
/// Encapsulates wgpu device, queue, surface, and Vello renderer setup.
/// Use this to avoid duplicating GPU initialization across examples.
///
/// # Example
///
/// ```ignore
/// let render_ctx = VelloRenderContext::new(window);
///
/// // On resize:
/// render_ctx.resize(new_width, new_height);
///
/// // On redraw:
/// let mut scene = vello::Scene::new();
/// // ... build scene ...
/// render_ctx.render(&scene);
/// ```
pub struct VelloRenderContext {
    pub window: Arc<Window>,
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub vello_renderer: vello::Renderer,
    pub render_texture: wgpu::Texture,
    pub blitter: wgpu::util::TextureBlitter,
}

impl VelloRenderContext {
    /// Create a new render context from a window.
    ///
    /// This performs all GPU initialization synchronously using pollster.
    ///
    /// # Errors
    /// Returns `Error::GpuDevice` if surface creation, adapter request,
    /// device request, or Vello renderer construction fails.
    pub fn new(window: Arc<Window>) -> Result<Self> {
        // wgpu 29: InstanceDescriptor lost Default; the display-handle-aware
        // constructor isn't needed since the surface is created from the
        // window right below.
        let instance = Instance::new(InstanceDescriptor::new_without_display_handle());
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| Error::GpuDevice(format!("create surface: {e}")))?;

        let adapter = pollster::block_on(instance.request_adapter(&RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(|e| Error::GpuDevice(format!("request adapter: {e}")))?;

        let (device, queue) = pollster::block_on(adapter.request_device(&DeviceDescriptor {
            required_features: Features::empty(),
            ..Default::default()
        }))
        .map_err(|e| Error::GpuDevice(format!("request device: {e}")))?;

        let size = window.inner_size();

        // Get preferred surface format (prefer non-sRGB for Vello)
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // Create Vello renderer
        let vello_renderer = vello::Renderer::new(&device, vello::RendererOptions::default())
            .map_err(|e| Error::GpuDevice(format!("create vello renderer: {e}")))?;

        // Create intermediate render texture (Rgba8Unorm for Vello's compute shaders)
        let render_texture = Self::create_render_texture(&device, config.width, config.height);

        // Create blitter for copying from intermediate texture to surface
        let blitter = wgpu::util::TextureBlitter::new(&device, surface_format);

        Ok(Self {
            window,
            surface,
            device,
            queue,
            config,
            vello_renderer,
            render_texture,
            blitter,
        })
    }

    /// Resize the render context for a new window size.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.config.width = width;
            self.config.height = height;
            self.surface.configure(&self.device, &self.config);
            self.render_texture = Self::create_render_texture(&self.device, width, height);
        }
    }

    /// Render a Vello scene to the surface.
    ///
    /// # Errors
    /// Returns `Error::GpuDevice` if acquiring the surface texture or
    /// running the Vello render pass fails.
    pub fn render(&mut self, scene: &Scene) -> Result<()> {
        // wgpu 29: get_current_texture returns an enum instead of a Result.
        // Suboptimal still renders; every other variant skips/fails the frame.
        let output = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            other => {
                return Err(Error::GpuDevice(format!("get surface texture: {other:?}")));
            }
        };

        let render_view = self
            .render_texture
            .create_view(&TextureViewDescriptor::default());
        let surface_view = output
            .texture
            .create_view(&TextureViewDescriptor::default());

        self.vello_renderer
            .render_to_texture(
                &self.device,
                &self.queue,
                scene,
                &render_view,
                &vello::RenderParams {
                    base_color: peniko::Color::WHITE,
                    width: self.config.width,
                    height: self.config.height,
                    antialiasing_method: vello::AaConfig::Msaa16,
                },
            )
            .map_err(|e| Error::GpuDevice(format!("vello render: {e}")))?;

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Blit Encoder"),
            });
        self.blitter
            .copy(&self.device, &mut encoder, &render_view, &surface_view);
        self.queue.submit(std::iter::once(encoder.finish()));

        output.present();
        Ok(())
    }

    /// Get the current viewport dimensions.
    #[must_use]
    pub fn viewport_size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Request a redraw of the window.
    pub fn request_redraw(&self) {
        self.window.request_redraw();
    }

    fn create_render_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
        device.create_texture(&TextureDescriptor {
            label: Some("Vello Render Texture"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: TextureUsages::STORAGE_BINDING | TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    }
}

// endregion: --- VelloRenderContext
