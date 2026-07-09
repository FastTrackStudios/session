//! Rendering primitives for music notation.
//!
//! This module provides shared vertex types, geometry helpers, and shaders
//! used for rendering music notation with WGPU.

use bytemuck::{Pod, Zeroable};

// ============================================================================
// Vertex Types
// ============================================================================

/// Vertex with position and color for basic geometry (lines, rectangles, glyphs)
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 2],
    pub color: [f32; 4],
}

impl Vertex {
    /// Vertex attributes for WGPU pipeline
    pub const ATTRIBS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];

    /// Vertex buffer layout descriptor
    #[must_use]
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// Vertex for SDF-based rounded rectangles - pixel-perfect at any zoom level
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct SdfRectVertex {
    /// Position in NDC
    pub position: [f32; 2],
    /// Rectangle center in pixels
    pub rect_center: [f32; 2],
    /// Rectangle half-size (from center to corner) in pixels
    pub rect_half_size: [f32; 2],
    /// Corner radius in pixels
    pub corner_radius: f32,
    /// Border width (0 = filled, >0 = stroked)
    pub border_width: f32,
    /// Fill/border color
    pub color: [f32; 4],
}

impl SdfRectVertex {
    /// Vertex buffer layout descriptor
    #[must_use]
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<SdfRectVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x2,
                }, // position
                wgpu::VertexAttribute {
                    offset: 8,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                }, // rect_center
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x2,
                }, // rect_half_size
                wgpu::VertexAttribute {
                    offset: 24,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32,
                }, // corner_radius
                wgpu::VertexAttribute {
                    offset: 28,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Float32,
                }, // border_width
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32x4,
                }, // color
            ],
        }
    }
}

/// Camera/view transform uniform for zoom and pan
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct CameraUniform {
    /// Combined transform: [scale_x, scale_y, offset_x, offset_y]
    pub transform: [f32; 4],
    /// Canvas resolution: [width, height, unused, unused]
    pub resolution: [f32; 4],
}

impl CameraUniform {
    /// Create identity camera (no zoom/pan) with default resolution
    #[must_use]
    pub fn new() -> Self {
        Self {
            transform: [1.0, 1.0, 0.0, 0.0],
            resolution: [1200.0, 900.0, 0.0, 0.0],
        }
    }

    /// Create camera from zoom and pan values with resolution
    #[must_use]
    pub fn from_zoom_pan(zoom: f32, pan_x: f32, pan_y: f32) -> Self {
        Self {
            transform: [zoom, zoom, pan_x, pan_y],
            resolution: [1200.0, 900.0, 0.0, 0.0],
        }
    }

    /// Create camera with full parameters including resolution
    #[must_use]
    pub fn with_resolution(zoom: f32, pan_x: f32, pan_y: f32, width: f32, height: f32) -> Self {
        Self {
            transform: [zoom, zoom, pan_x, pan_y],
            resolution: [width, height, 0.0, 0.0],
        }
    }
}

impl Default for CameraUniform {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Geometry Helpers
// ============================================================================

/// Convert pixel coordinates to normalized device coordinates (-1 to 1)
#[must_use]
pub fn px_to_ndc(x: f32, y: f32, width: f32, height: f32) -> [f32; 2] {
    [
        (x / width) * 2.0 - 1.0,
        1.0 - (y / height) * 2.0, // Flip Y for screen coordinates
    ]
}

/// Create a line as two triangles (a thin rectangle)
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn create_line(
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    thickness: f32,
    color: [f32; 4],
    width: f32,
    height: f32,
) -> [Vertex; 6] {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let len = (dx * dx + dy * dy).sqrt();
    let nx = -dy / len * thickness * 0.5;
    let ny = dx / len * thickness * 0.5;

    let p1 = px_to_ndc(x1 + nx, y1 + ny, width, height);
    let p2 = px_to_ndc(x1 - nx, y1 - ny, width, height);
    let p3 = px_to_ndc(x2 + nx, y2 + ny, width, height);
    let p4 = px_to_ndc(x2 - nx, y2 - ny, width, height);

    [
        Vertex {
            position: p1,
            color,
        },
        Vertex {
            position: p2,
            color,
        },
        Vertex {
            position: p3,
            color,
        },
        Vertex {
            position: p2,
            color,
        },
        Vertex {
            position: p4,
            color,
        },
        Vertex {
            position: p3,
            color,
        },
    ]
}

/// Create a filled rectangle
#[must_use]
pub fn create_rect(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: [f32; 4],
    canvas_width: f32,
    canvas_height: f32,
) -> [Vertex; 6] {
    let p1 = px_to_ndc(x, y, canvas_width, canvas_height);
    let p2 = px_to_ndc(x + w, y, canvas_width, canvas_height);
    let p3 = px_to_ndc(x, y + h, canvas_width, canvas_height);
    let p4 = px_to_ndc(x + w, y + h, canvas_width, canvas_height);

    [
        Vertex {
            position: p1,
            color,
        },
        Vertex {
            position: p3,
            color,
        },
        Vertex {
            position: p2,
            color,
        },
        Vertex {
            position: p3,
            color,
        },
        Vertex {
            position: p4,
            color,
        },
        Vertex {
            position: p2,
            color,
        },
    ]
}

/// Create an SDF rounded rectangle (6 vertices for 2 triangles forming a quad)
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn create_sdf_rounded_rect(
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    corner_radius: f32,
    border_width: f32,
    color: [f32; 4],
    canvas_width: f32,
    canvas_height: f32,
) -> [SdfRectVertex; 6] {
    // Add padding for SDF antialiasing
    let padding = 2.0;
    let px = x - padding;
    let py = y - padding;
    let pw = w + padding * 2.0;
    let ph = h + padding * 2.0;

    let ndc = |px: f32, py: f32| -> [f32; 2] {
        [
            (px / canvas_width) * 2.0 - 1.0,
            1.0 - (py / canvas_height) * 2.0,
        ]
    };

    let rect_center = [x + w / 2.0, y + h / 2.0];
    let rect_half_size = [w / 2.0, h / 2.0];

    let p1 = ndc(px, py);
    let p2 = ndc(px + pw, py);
    let p3 = ndc(px, py + ph);
    let p4 = ndc(px + pw, py + ph);

    let make_vertex = |pos: [f32; 2]| SdfRectVertex {
        position: pos,
        rect_center,
        rect_half_size,
        corner_radius,
        border_width,
        color,
    };

    [
        make_vertex(p1),
        make_vertex(p3),
        make_vertex(p2),
        make_vertex(p3),
        make_vertex(p4),
        make_vertex(p2),
    ]
}

// ============================================================================
// Shaders
// ============================================================================

/// Main shader for geometry (lines, rectangles, glyphs) with camera transform
pub const SHADER_SOURCE: &str = r#"
struct Camera {
    transform: vec4<f32>,  // scale_x, scale_y, offset_x, offset_y
    resolution: vec4<f32>, // width, height, unused, unused (for consistency)
}

@group(0) @binding(0)
var<uniform> camera: Camera;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    // Apply zoom (scale) and pan (offset)
    let scaled = input.position * camera.transform.xy;
    let transformed = scaled + camera.transform.zw;
    output.position = vec4<f32>(transformed, 0.0, 1.0);
    output.color = input.color;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;
}
"#;

/// SDF shader for pixel-perfect rounded rectangles at any zoom level
pub const SDF_SHADER_SOURCE: &str = r#"
struct Camera {
    transform: vec4<f32>,  // scale_x, scale_y, offset_x, offset_y
    resolution: vec4<f32>, // width, height, unused, unused
}

@group(0) @binding(0)
var<uniform> camera: Camera;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) rect_center: vec2<f32>,
    @location(2) rect_half_size: vec2<f32>,
    @location(3) corner_radius: f32,
    @location(4) border_width: f32,
    @location(5) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) rect_center: vec2<f32>,
    @location(1) rect_half_size: vec2<f32>,
    @location(2) corner_radius: f32,
    @location(3) border_width: f32,
    @location(4) color: vec4<f32>,
    @location(5) pixel_pos: vec2<f32>,
    @location(6) zoom: f32,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Apply camera transform (same as main shader)
    let scaled = in.position * camera.transform.xy;
    let transformed = scaled + camera.transform.zw;
    out.clip_position = vec4<f32>(transformed, 0.0, 1.0);

    // Pass through rect parameters for fragment shader
    out.rect_center = in.rect_center;
    out.rect_half_size = in.rect_half_size;
    out.corner_radius = in.corner_radius;
    out.border_width = in.border_width;
    out.color = in.color;
    out.zoom = camera.transform.x;  // Pass zoom level to fragment shader

    // Calculate pixel position from NDC position (using resolution from camera uniform)
    out.pixel_pos = (in.position + 1.0) * 0.5 * camera.resolution.xy;
    out.pixel_pos.y = camera.resolution.y - out.pixel_pos.y;  // Flip Y

    return out;
}

// Signed Distance Field for rounded rectangle
fn sdf_rounded_rect(p: vec2<f32>, center: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let d = abs(p - center) - half_size + radius;
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0) - radius;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let dist = sdf_rounded_rect(
        in.pixel_pos,
        in.rect_center,
        in.rect_half_size,
        in.corner_radius
    );

    var color = in.color;

    // Scale the antialiasing width inversely with zoom for crisp edges at all zoom levels
    // At zoom 1.0, aa_width = 0.5 (standard). At zoom 2.0, aa_width = 0.25, etc.
    let aa_width = 0.5 / in.zoom;

    if in.border_width > 0.0 {
        // Stroked rectangle
        let inner_dist = dist + in.border_width;
        let outer_alpha = 1.0 - smoothstep(-aa_width, aa_width, dist);
        let inner_alpha = smoothstep(-aa_width, aa_width, inner_dist);
        color.a *= outer_alpha * inner_alpha;
    } else {
        // Filled rectangle
        color.a *= 1.0 - smoothstep(-aa_width, aa_width, dist);
    }

    return color;
}
"#;

// ============================================================================
// Pipeline Creation Helpers
// ============================================================================

/// Create the camera bind group layout
#[must_use]
pub fn create_camera_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Camera Bind Group Layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

/// Create the main geometry render pipeline
#[must_use]
pub fn create_main_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    camera_bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Music Notation Shader"),
        source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Music Pipeline Layout"),
        bind_group_layouts: &[Some(camera_bind_group_layout)],
        immediate_size: 0,
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Music Pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[Vertex::desc()],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(format.into())],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

// ============================================================================
// Texture Blit for Retained Rendering
// ============================================================================

/// Vertex for texture blitting (fullscreen quad with camera transform)
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct BlitVertex {
    /// Position in NDC (-1 to 1)
    pub position: [f32; 2],
    /// UV coordinates (0 to 1)
    pub uv: [f32; 2],
}

impl BlitVertex {
    /// Vertex attributes for WGPU pipeline
    pub const ATTRIBS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2];

    /// Vertex buffer layout descriptor
    #[must_use]
    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<BlitVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// Create vertices for a fullscreen quad for texture blitting
#[must_use]
pub fn create_fullscreen_quad() -> [BlitVertex; 6] {
    [
        // First triangle
        BlitVertex {
            position: [-1.0, -1.0],
            uv: [0.0, 1.0],
        },
        BlitVertex {
            position: [1.0, -1.0],
            uv: [1.0, 1.0],
        },
        BlitVertex {
            position: [-1.0, 1.0],
            uv: [0.0, 0.0],
        },
        // Second triangle
        BlitVertex {
            position: [1.0, -1.0],
            uv: [1.0, 1.0],
        },
        BlitVertex {
            position: [1.0, 1.0],
            uv: [1.0, 0.0],
        },
        BlitVertex {
            position: [-1.0, 1.0],
            uv: [0.0, 0.0],
        },
    ]
}

/// Shader for texture blitting with camera transform
pub const BLIT_SHADER_SOURCE: &str = r#"
struct Camera {
    transform: vec4<f32>,  // scale_x, scale_y, offset_x, offset_y
    resolution: vec4<f32>, // width, height, unused, unused
}

@group(0) @binding(0)
var<uniform> camera: Camera;

@group(1) @binding(0)
var scene_texture: texture_2d<f32>;

@group(1) @binding(1)
var scene_sampler: sampler;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    // Apply camera transform to positions
    let scaled = input.position * camera.transform.xy;
    let transformed = scaled + camera.transform.zw;
    output.position = vec4<f32>(transformed, 0.0, 1.0);
    output.uv = input.uv;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(scene_texture, scene_sampler, input.uv);
}
"#;

/// Create the texture blit bind group layout for scene texture
#[must_use]
pub fn create_blit_texture_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Blit Texture Bind Group Layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

/// Create the texture blit pipeline
#[must_use]
pub fn create_blit_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    camera_bind_group_layout: &wgpu::BindGroupLayout,
    texture_bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Blit Shader"),
        source: wgpu::ShaderSource::Wgsl(BLIT_SHADER_SOURCE.into()),
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Blit Pipeline Layout"),
        bind_group_layouts: &[
            Some(camera_bind_group_layout),
            Some(texture_bind_group_layout),
        ],
        immediate_size: 0,
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Blit Pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[BlitVertex::desc()],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(format.into())],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Create the SDF render pipeline for rounded rectangles
#[must_use]
pub fn create_sdf_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    camera_bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("SDF Shader"),
        source: wgpu::ShaderSource::Wgsl(SDF_SHADER_SOURCE.into()),
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("SDF Pipeline Layout"),
        bind_group_layouts: &[Some(camera_bind_group_layout)],
        immediate_size: 0,
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("SDF Pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[SdfRectVertex::desc()],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}
