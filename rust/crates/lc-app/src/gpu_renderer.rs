//! Retained wgpu scene composition for normal windowed gameplay.
//!
//! `pixels` 0.13 always uploads its logical CPU pixel buffer before invoking
//! `Pixels::render_with`.  The application therefore constructs `Pixels` with
//! a 1x1 logical buffer and leaves that buffer at 1x1; this renderer records the
//! real game scene into the surface view supplied to the closure.  Source
//! textures stay resident here and only changed dirty rectangles are uploaded.
//!
//! Scene blending happens in a physical-size `Rgba8Unorm` texture.  Keeping the
//! intermediate target non-sRGB preserves LegacyClonk's byte-space blending,
//! while a final shader converts to linear when the window surface is sRGB so
//! the surface encode restores those same bytes. Fixed-function gamma resolves
//! that completed image into a second unorm target; whichever image is final is
//! also the screenshot and deterministic-test readback source.

use lc_graphics::{
    GpuBlend, GpuCommand, GpuGammaMode, GpuPresentation, GpuPrimitiveTopology, GpuSampler,
    GpuScene, GpuTextureFormat, GpuTextureId, GpuTextureResource, GpuVertex, Rect,
};
use pixels::wgpu;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::mpsc;
use thiserror::Error;

const PACKED_VERTEX_FLOATS: usize = 18;
const PACKED_VERTEX_STRIDE: u64 = (PACKED_VERTEX_FLOATS * std::mem::size_of::<f32>()) as u64;
const INITIAL_VERTEX_BUFFER_SIZE: u64 = 4096;
const SOURCE_TEXTURE_CACHE_BUDGET_BYTES: u64 = 256 * 1024 * 1024;
const SOURCE_TEXTURE_CACHE_MAX_ENTRIES: usize = 4096;

const PACKED_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 5] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 16,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 24,
        shader_location: 2,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 40,
        shader_location: 3,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 56,
        shader_location: 4,
    },
];

const QUAD_SHADER: &str = r#"
struct VertexInput {
    @location(0) clip_position: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) modulation: vec4<f32>,
    @location(3) flags: vec4<f32>,
    @location(4) sample_tile: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) modulation: vec4<f32>,
    @location(2) flags: vec4<f32>,
    @location(3) @interpolate(flat) sample_tile: vec4<f32>,
};

@group(0) @binding(0) var gamma_lut: texture_2d<u32>;
@group(1) @binding(0) var image: texture_2d<f32>;
@group(1) @binding(1) var image_sampler: sampler;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = input.clip_position;
    output.uv = input.uv;
    output.modulation = input.modulation;
    output.flags = input.flags;
    output.sample_tile = input.sample_tile;
    return output;
}

fn tiled_texel(image_size: vec2<i32>, tile: vec4<f32>, relative: vec2<i32>) -> vec4<f32> {
    let tile_size = max(i32(tile.z), 1);
    let local = clamp(relative, vec2<i32>(0), vec2<i32>(tile_size - 1));
    let position = vec2<i32>(i32(tile.x), i32(tile.y)) + local;
    if any(position < vec2<i32>(0)) || any(position >= image_size) {
        // C4Surface clears unused C4TexRef storage to 0xffffffff before
        // uploading the logical image.  Its high byte is transparency, so
        // the equivalent opacity-alpha texel is transparent white.
        return vec4<f32>(1.0, 1.0, 1.0, 0.0);
    }
    return textureLoad(image, position, 0);
}

fn sample_native_tile(uv: vec2<f32>, tile: vec4<f32>) -> vec4<f32> {
    let image_size = vec2<i32>(textureDimensions(image));
    let tile_size = max(tile.z, 1.0);
    let source_edge = uv * vec2<f32>(image_size);
    let tile_origin = floor(source_edge / vec2<f32>(tile_size)) * tile_size;
    let native_tile = vec4<f32>(tile_origin, tile_size, tile.w);
    let source = source_edge - vec2<f32>(0.5) - tile_origin;
    let base = vec2<i32>(floor(source));
    let fraction = fract(source);
    let top = mix(
        tiled_texel(image_size, native_tile, base),
        tiled_texel(image_size, native_tile, base + vec2<i32>(1, 0)),
        fraction.x,
    );
    let bottom = mix(
        tiled_texel(image_size, native_tile, base + vec2<i32>(0, 1)),
        tiled_texel(image_size, native_tile, base + vec2<i32>(1, 1)),
        fraction.x,
    );
    return mix(top, bottom, fraction.y);
}

fn gamma_channel(channel: u32, value: f32) -> f32 {
    let index = min(u32(clamp(value, 0.0, 1.0) * 256.0), 255u);
    let sample = textureLoad(gamma_lut, vec2<i32>(i32(index), i32(channel)), 0).r;
    return f32(sample) / 65535.0;
}

fn apply_gamma(color: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        gamma_channel(0u, color.r),
        gamma_channel(1u, color.g),
        gamma_channel(2u, color.b),
    );
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    var source: vec4<f32>;
    if input.sample_tile.w > 0.5 {
        source = sample_native_tile(input.uv, input.sample_tile);
    } else {
        // The explicit level keeps this branch valid in non-uniform fragment
        // control flow. Source textures have one mip, matching C4TexRef.
        source = textureSampleLevel(image, image_sampler, input.uv, 0.0);
    }
    var rgb = source.rgb;
    var alpha = source.a;
    if input.flags.x > 0.5 {
        rgb = clamp((rgb + input.modulation.rgb) * 2.0 - 1.0, vec3<f32>(0.0), vec3<f32>(1.0));
    } else {
        rgb = clamp(rgb * input.modulation.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
        alpha = clamp(alpha - input.modulation.a, 0.0, 1.0);
    }
    if input.flags.y > 0.5 {
        rgb = apply_gamma(rgb);
    }
    return vec4<f32>(rgb, alpha);
}
"#;

const LANDSCAPE_SHADER: &str = r#"
struct VertexInput {
    @location(0) clip_position: vec4<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) modulation: vec4<f32>,
    @location(3) liquid_scale: vec4<f32>,
    @location(4) phase_gamma: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) modulation: vec4<f32>,
    @location(2) liquid_scale: vec2<f32>,
    @location(3) phase_gamma: vec4<f32>,
};

@group(0) @binding(0) var gamma_lut: texture_2d<u32>;
@group(1) @binding(0) var base_image: texture_2d<f32>;
@group(1) @binding(1) var liquid_mask: texture_2d<f32>;
@group(1) @binding(2) var liquid_image: texture_2d<f32>;
@group(1) @binding(3) var base_sampler: sampler;
@group(1) @binding(4) var liquid_sampler: sampler;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = input.clip_position;
    output.uv = input.uv;
    output.modulation = input.modulation;
    output.liquid_scale = input.liquid_scale.xy;
    output.phase_gamma = input.phase_gamma;
    return output;
}

fn gamma_channel(channel: u32, value: f32) -> f32 {
    let index = min(u32(clamp(value, 0.0, 1.0) * 256.0), 255u);
    let sample = textureLoad(gamma_lut, vec2<i32>(i32(index), i32(channel)), 0).r;
    return f32(sample) / 65535.0;
}

fn apply_gamma(color: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        gamma_channel(0u, color.r),
        gamma_channel(1u, color.g),
        gamma_channel(2u, color.b),
    );
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let source = textureSample(base_image, base_sampler, input.uv);
    let mask = textureSample(liquid_mask, base_sampler, input.uv).r;
    let liquid = textureSample(liquid_image, liquid_sampler, input.uv * input.liquid_scale).rgb - vec3<f32>(0.5);
    let delta = dot(liquid, input.phase_gamma.rgb) * mask;
    var rgb = clamp(source.rgb + vec3<f32>(delta), vec3<f32>(0.0), vec3<f32>(1.0));
    rgb = rgb * input.modulation.rgb;
    let alpha = clamp(source.a - input.modulation.a, 0.0, 1.0);
    if input.phase_gamma.a > 0.5 {
        rgb = apply_gamma(rgb);
    }
    return vec4<f32>(rgb, alpha);
}
"#;

const SOLID_SHADER: &str = r#"
struct VertexInput {
    @location(0) clip_position: vec4<f32>,
    @location(1) unused_uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) flags: vec4<f32>,
    @location(4) unused: vec4<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) flags: vec4<f32>,
};

@group(0) @binding(0) var gamma_lut: texture_2d<u32>;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;
    output.position = input.clip_position;
    output.color = input.color;
    output.flags = input.flags;
    return output;
}

fn gamma_channel(channel: u32, value: f32) -> f32 {
    let index = min(u32(clamp(value, 0.0, 1.0) * 256.0), 255u);
    let sample = textureLoad(gamma_lut, vec2<i32>(i32(index), i32(channel)), 0).r;
    return f32(sample) / 65535.0;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    var rgb = input.color.rgb;
    if input.flags.x > 0.5 {
        rgb = vec3<f32>(
            gamma_channel(0u, rgb.r),
            gamma_channel(1u, rgb.g),
            gamma_channel(2u, rgb.b),
        );
    }
    return vec4<f32>(rgb, input.color.a);
}
"#;

const PRESENT_SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;
@group(1) @binding(0) var gamma_lut: texture_2d<u32>;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    var output: VertexOutput;
    if index == 0u {
        output.position = vec4<f32>(-1.0, -1.0, 0.0, 1.0);
        output.uv = vec2<f32>(0.0, 1.0);
    } else if index == 1u {
        output.position = vec4<f32>(3.0, -1.0, 0.0, 1.0);
        output.uv = vec2<f32>(2.0, 1.0);
    } else {
        output.position = vec4<f32>(-1.0, 3.0, 0.0, 1.0);
        output.uv = vec2<f32>(0.0, -1.0);
    }
    return output;
}

fn srgb_to_linear(color: vec3<f32>) -> vec3<f32> {
    let low = color / 12.92;
    let high = pow((color + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    return select(high, low, color <= vec3<f32>(0.04045));
}

fn gamma_channel(channel: u32, value: f32) -> f32 {
    let index = min(u32(clamp(value, 0.0, 1.0) * 256.0), 255u);
    let sample = textureLoad(gamma_lut, vec2<i32>(i32(index), i32(channel)), 0).r;
    return f32(sample) / 65535.0;
}

fn apply_gamma(color: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        gamma_channel(0u, color.r),
        gamma_channel(1u, color.g),
        gamma_channel(2u, color.b),
    );
}

@fragment
fn fs_linear(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(image, image_sampler, input.uv);
}

@fragment
fn fs_srgb(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(image, image_sampler, input.uv);
    return vec4<f32>(srgb_to_linear(color.rgb), color.a);
}

@fragment
fn fs_monitor_linear(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(image, image_sampler, input.uv);
    return vec4<f32>(apply_gamma(color.rgb), color.a);
}

@fragment
fn fs_monitor_srgb(input: VertexOutput) -> @location(0) vec4<f32> {
    let color = textureSample(image, image_sampler, input.uv);
    return vec4<f32>(srgb_to_linear(apply_gamma(color.rgb)), color.a);
}
"#;

#[derive(Debug, Error)]
pub enum GpuRendererError {
    #[error("invalid GPU presentation: logical={logical:?}, physical={physical:?}, scale={scale}, crop_top={crop_top}")]
    InvalidPresentation {
        logical: [u32; 2],
        physical: [u32; 2],
        scale: f32,
        crop_top: u32,
    },
    #[error("texture {id:?} has invalid {format:?} data for extent {extent:?}: expected {expected:?} bytes, got {actual}")]
    InvalidTextureData {
        id: GpuTextureId,
        format: GpuTextureFormat,
        extent: [u32; 2],
        expected: Option<usize>,
        actual: usize,
    },
    #[error("texture {0:?} occurs more than once in one scene")]
    DuplicateTexture(GpuTextureId),
    #[error("dirty rectangle {rect:?} is outside texture {id:?} extent {extent:?}")]
    InvalidDirtyRect {
        id: GpuTextureId,
        rect: Rect,
        extent: [u32; 2],
    },
    #[error("draw command references missing texture {0:?}")]
    MissingTexture(GpuTextureId),
    #[error("draw command expected texture {id:?} to be {expected:?}, found {actual:?}")]
    TextureFormatMismatch {
        id: GpuTextureId,
        expected: GpuTextureFormat,
        actual: GpuTextureFormat,
    },
    #[error("scalar owner masks must be lowered to explicit painter-ordered RGBA quad passes")]
    OwnerMaskNotLowered,
    #[error("landscape liquid animation requires both a mask and a liquid texture")]
    IncompleteLandscapeLiquid,
    #[error("{topology:?} received {vertices} vertices")]
    InvalidPrimitiveVertexCount {
        topology: GpuPrimitiveTopology,
        vertices: usize,
    },
    #[error("non-finite GPU vertex or presentation coordinate")]
    NonFiniteCoordinate,
    #[error("GPU vertex stream exceeds wgpu's u32 draw range")]
    VertexRangeOverflow,
    #[error("GPU readback size overflow")]
    ReadbackSizeOverflow,
    #[error("GPU readback callback was dropped")]
    ReadbackCallbackDropped,
    #[error("GPU readback mapping failed: {0}")]
    ReadbackMap(String),
}

/// Per-frame evidence that source retention and dirty updates are working.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuRendererStats {
    pub resident_source_textures: usize,
    pub created_source_textures: usize,
    pub full_upload_bytes: u64,
    pub dirty_upload_bytes: u64,
    pub draw_calls: usize,
    pub composition_recreated: bool,
}

#[derive(Debug)]
struct CachedTexture {
    revision: u64,
    extent: [u32; 2],
    format: GpuTextureFormat,
    byte_len: u64,
    last_used_epoch: u64,
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct QuadBindingKey {
    texture: GpuTextureId,
    sampler: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct LandscapeBindingKey {
    base: GpuTextureId,
    mask: Option<GpuTextureId>,
    liquid: Option<GpuTextureId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Scissor {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Clone, Copy, Debug)]
enum DrawKind {
    Quad(QuadBindingKey),
    Landscape(LandscapeBindingKey),
    Solid(GpuPrimitiveTopology),
}

#[derive(Clone, Debug)]
struct DrawCall {
    vertices: Range<u32>,
    scissor: Scissor,
    blend: GpuBlend,
    kind: DrawKind,
}

#[derive(Debug)]
struct CompositionTarget {
    extent: [u32; 2],
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    present_bind_group: wgpu::BindGroup,
    gamma_resolved_texture: wgpu::Texture,
    gamma_resolved_view: wgpu::TextureView,
    gamma_resolved_present_bind_group: wgpu::BindGroup,
}

/// A submitted, padded texture-to-buffer copy.
#[derive(Debug)]
pub struct GpuReadbackTicket {
    buffer: wgpu::Buffer,
    extent: [u32; 2],
    unpadded_bytes_per_row: usize,
    padded_bytes_per_row: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuReadbackFrame {
    pub extent: [u32; 2],
    pub rgba: Vec<u8>,
}

impl GpuReadbackTicket {
    /// Wait for a copy already submitted by `Pixels::render_with` (or a test
    /// queue submission), remove WebGPU row padding, and return tightly packed
    /// physical RGBA pixels.
    pub fn read(self, device: &wgpu::Device) -> Result<GpuReadbackFrame, GpuRendererError> {
        let slice = self.buffer.slice(..);
        let (sender, receiver) = mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device.poll(wgpu::Maintain::Wait);
        let result = receiver
            .recv()
            .map_err(|_| GpuRendererError::ReadbackCallbackDropped)?;
        result.map_err(|error| GpuRendererError::ReadbackMap(error.to_string()))?;

        let mapped = slice.get_mapped_range();
        let output_len = self
            .unpadded_bytes_per_row
            .checked_mul(self.extent[1] as usize)
            .ok_or(GpuRendererError::ReadbackSizeOverflow)?;
        let mut rgba = Vec::with_capacity(output_len);
        for row in mapped
            .chunks(self.padded_bytes_per_row)
            .take(self.extent[1] as usize)
        {
            rgba.extend_from_slice(&row[..self.unpadded_bytes_per_row]);
        }
        drop(mapped);
        self.buffer.unmap();
        Ok(GpuReadbackFrame {
            extent: self.extent,
            rgba,
        })
    }
}

/// Device-owned retained texture cache and scene pipelines.
///
/// A wgpu `Device` has no public stable identity.  If the application rebuilds
/// `Pixels` after a device loss, it must call [`Self::recreate`] with the new
/// device before recording another frame.  Ordinary surface resize only
/// recreates the physical composition target and leaves source textures live.
pub struct RetainedGpuRenderer {
    surface_format: wgpu::TextureFormat,
    generation: u64,
    texture_epoch: u64,
    textures: HashMap<GpuTextureId, CachedTexture>,
    quad_bind_groups: HashMap<QuadBindingKey, wgpu::BindGroup>,
    landscape_bind_groups: HashMap<LandscapeBindingKey, wgpu::BindGroup>,

    gamma_texture: wgpu::Texture,
    _gamma_view: wgpu::TextureView,
    gamma_bind_group: wgpu::BindGroup,
    gamma_revision: Option<u64>,

    quad_bind_group_layout: wgpu::BindGroupLayout,
    landscape_bind_group_layout: wgpu::BindGroupLayout,
    present_bind_group_layout: wgpu::BindGroupLayout,
    quad_replace_pipeline: wgpu::RenderPipeline,
    quad_normal_pipeline: wgpu::RenderPipeline,
    quad_additive_pipeline: wgpu::RenderPipeline,
    landscape_pipeline: wgpu::RenderPipeline,
    solid_replace_pipelines: [wgpu::RenderPipeline; 3],
    solid_normal_pipelines: [wgpu::RenderPipeline; 3],
    solid_additive_pipelines: [wgpu::RenderPipeline; 3],
    monitor_gamma_pipeline: wgpu::RenderPipeline,
    present_pipeline: wgpu::RenderPipeline,

    nearest_sampler: wgpu::Sampler,
    linear_sampler: wgpu::Sampler,
    repeat_nearest_sampler: wgpu::Sampler,
    present_sampler: wgpu::Sampler,
    _fallback_mask_texture: wgpu::Texture,
    fallback_mask_view: wgpu::TextureView,
    _fallback_liquid_texture: wgpu::Texture,
    fallback_liquid_view: wgpu::TextureView,

    vertex_buffer: wgpu::Buffer,
    vertex_buffer_size: u64,
    vertex_scratch: Vec<PackedVertex>,
    draw_call_scratch: Vec<DrawCall>,
    composition: Option<CompositionTarget>,
    last_presented_monitor_gamma: Option<bool>,
    last_stats: GpuRendererStats,
}

impl RetainedGpuRenderer {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        Self::build(device, queue, surface_format, 1)
    }

    fn build(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        generation: u64,
    ) -> Self {
        let gamma_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("lc_gpu_gamma_layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                }],
            });
        let gamma_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lc_gpu_gamma_lut"),
            size: wgpu::Extent3d {
                width: 256,
                height: 3,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R16Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let gamma_view = gamma_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let gamma_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lc_gpu_gamma_bind_group"),
            layout: &gamma_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&gamma_view),
            }],
        });

        let quad_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("lc_gpu_quad_layout"),
                entries: &[
                    texture_layout_entry(0, wgpu::TextureSampleType::Float { filterable: true }),
                    sampler_layout_entry(1),
                ],
            });
        let landscape_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("lc_gpu_landscape_layout"),
                entries: &[
                    texture_layout_entry(0, wgpu::TextureSampleType::Float { filterable: true }),
                    texture_layout_entry(1, wgpu::TextureSampleType::Float { filterable: true }),
                    texture_layout_entry(2, wgpu::TextureSampleType::Float { filterable: true }),
                    sampler_layout_entry(3),
                    sampler_layout_entry(4),
                ],
            });
        let present_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("lc_gpu_present_layout"),
                entries: &[
                    texture_layout_entry(0, wgpu::TextureSampleType::Float { filterable: true }),
                    sampler_layout_entry(1),
                ],
            });

        let quad_shader = shader(device, "lc_gpu_quad_shader", QUAD_SHADER);
        let landscape_shader = shader(device, "lc_gpu_landscape_shader", LANDSCAPE_SHADER);
        let solid_shader = shader(device, "lc_gpu_solid_shader", SOLID_SHADER);
        let present_shader = shader(device, "lc_gpu_present_shader", PRESENT_SHADER);

        let quad_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lc_gpu_quad_pipeline_layout"),
            bind_group_layouts: &[&gamma_bind_group_layout, &quad_bind_group_layout],
            push_constant_ranges: &[],
        });
        let landscape_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("lc_gpu_landscape_pipeline_layout"),
                bind_group_layouts: &[&gamma_bind_group_layout, &landscape_bind_group_layout],
                push_constant_ranges: &[],
            });
        let solid_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("lc_gpu_solid_pipeline_layout"),
                bind_group_layouts: &[&gamma_bind_group_layout],
                push_constant_ranges: &[],
            });
        let present_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("lc_gpu_present_pipeline_layout"),
                bind_group_layouts: &[&present_bind_group_layout],
                push_constant_ranges: &[],
            });
        let monitor_gamma_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("lc_gpu_monitor_gamma_pipeline_layout"),
                bind_group_layouts: &[&present_bind_group_layout, &gamma_bind_group_layout],
                push_constant_ranges: &[],
            });

        let quad_replace_pipeline = scene_pipeline(
            device,
            "lc_gpu_quad_replace",
            &quad_pipeline_layout,
            &quad_shader,
            wgpu::PrimitiveTopology::TriangleList,
            GpuBlend::Replace,
        );
        let quad_normal_pipeline = scene_pipeline(
            device,
            "lc_gpu_quad_normal",
            &quad_pipeline_layout,
            &quad_shader,
            wgpu::PrimitiveTopology::TriangleList,
            GpuBlend::Normal,
        );
        let quad_additive_pipeline = scene_pipeline(
            device,
            "lc_gpu_quad_additive",
            &quad_pipeline_layout,
            &quad_shader,
            wgpu::PrimitiveTopology::TriangleList,
            GpuBlend::Additive,
        );
        let landscape_pipeline = scene_pipeline(
            device,
            "lc_gpu_landscape",
            &landscape_pipeline_layout,
            &landscape_shader,
            wgpu::PrimitiveTopology::TriangleList,
            GpuBlend::Normal,
        );
        let solid_replace_pipelines = solid_pipelines(
            device,
            &solid_pipeline_layout,
            &solid_shader,
            GpuBlend::Replace,
        );
        let solid_normal_pipelines = solid_pipelines(
            device,
            &solid_pipeline_layout,
            &solid_shader,
            GpuBlend::Normal,
        );
        let solid_additive_pipelines = solid_pipelines(
            device,
            &solid_pipeline_layout,
            &solid_shader,
            GpuBlend::Additive,
        );
        let monitor_gamma_pipeline = present_pipeline(
            device,
            &monitor_gamma_pipeline_layout,
            &present_shader,
            wgpu::TextureFormat::Rgba8Unorm,
            true,
        );
        let present_pipeline = present_pipeline(
            device,
            &present_pipeline_layout,
            &present_shader,
            surface_format,
            false,
        );

        let nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lc_gpu_nearest_clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lc_gpu_linear_clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let repeat_nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lc_gpu_nearest_repeat"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let present_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lc_gpu_present_nearest"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let (fallback_mask_texture, fallback_mask_view) = fallback_texture(
            device,
            queue,
            "lc_gpu_empty_liquid_mask",
            wgpu::TextureFormat::R8Unorm,
            &[0],
        );
        let (fallback_liquid_texture, fallback_liquid_view) = fallback_texture(
            device,
            queue,
            "lc_gpu_neutral_liquid",
            wgpu::TextureFormat::Rgba8Unorm,
            &[128, 128, 128, 255],
        );
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lc_gpu_scene_vertices"),
            size: INITIAL_VERTEX_BUFFER_SIZE,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            surface_format,
            generation,
            texture_epoch: 0,
            textures: HashMap::new(),
            quad_bind_groups: HashMap::new(),
            landscape_bind_groups: HashMap::new(),
            gamma_texture,
            _gamma_view: gamma_view,
            gamma_bind_group,
            gamma_revision: None,
            quad_bind_group_layout,
            landscape_bind_group_layout,
            present_bind_group_layout,
            quad_replace_pipeline,
            quad_normal_pipeline,
            quad_additive_pipeline,
            landscape_pipeline,
            solid_replace_pipelines,
            solid_normal_pipelines,
            solid_additive_pipelines,
            monitor_gamma_pipeline,
            present_pipeline,
            nearest_sampler,
            linear_sampler,
            repeat_nearest_sampler,
            present_sampler,
            _fallback_mask_texture: fallback_mask_texture,
            fallback_mask_view,
            _fallback_liquid_texture: fallback_liquid_texture,
            fallback_liquid_view,
            vertex_buffer,
            vertex_buffer_size: INITIAL_VERTEX_BUFFER_SIZE,
            vertex_scratch: Vec::new(),
            draw_call_scratch: Vec::new(),
            composition: None,
            last_presented_monitor_gamma: None,
            last_stats: GpuRendererStats::default(),
        }
    }

    pub fn recreate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
    ) {
        let generation = self.generation.wrapping_add(1).max(1);
        *self = Self::build(device, queue, surface_format, generation);
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_format
    }

    pub fn last_stats(&self) -> GpuRendererStats {
        self.last_stats
    }

    /// Encodes a copy of the most recently presented composition before the
    /// next render pass overwrites its retained target.
    pub fn readback_last_presentation(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
    ) -> Result<Option<GpuReadbackTicket>, GpuRendererError> {
        let (Some(composition), Some(monitor_gamma)) =
            (self.composition.as_ref(), self.last_presented_monitor_gamma)
        else {
            return Ok(None);
        };
        let texture = if monitor_gamma {
            &composition.gamma_resolved_texture
        } else {
            &composition.texture
        };
        encode_readback(device, encoder, texture, composition.extent).map(Some)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
        scene: &GpuScene,
        presentation: &GpuPresentation,
        request_readback: bool,
    ) -> Result<Option<GpuReadbackTicket>, GpuRendererError> {
        validate_presentation(scene, presentation)?;
        self.last_stats = GpuRendererStats::default();
        self.texture_epoch = self.texture_epoch.wrapping_add(1).max(1);
        self.sync_gamma(queue, scene);
        self.sync_textures(device, queue, &scene.textures)?;

        let (vertices, calls) = self.build_draw_stream(scene, presentation)?;
        let vertex_bytes = packed_vertex_bytes(&vertices);
        self.ensure_bind_groups(device, &calls)?;
        let mut used_quad_bindings = HashSet::new();
        let mut used_landscape_bindings = HashSet::new();
        for call in &calls {
            match call.kind {
                DrawKind::Quad(key) => {
                    used_quad_bindings.insert(key);
                }
                DrawKind::Landscape(key) => {
                    used_landscape_bindings.insert(key);
                }
                DrawKind::Solid(_) => {}
            }
        }
        // Bind groups are cheap to recreate and can otherwise grow with every
        // historical combination of retained textures. Keep only bindings
        // reachable by this frame; source textures themselves follow the
        // larger bounded LRU below and survive temporary invisibility.
        self.quad_bind_groups
            .retain(|key, _| used_quad_bindings.contains(key));
        self.landscape_bind_groups
            .retain(|key, _| used_landscape_bindings.contains(key));
        self.ensure_vertex_buffer(device, vertex_bytes.len())?;
        if !vertex_bytes.is_empty() {
            queue.write_buffer(&self.vertex_buffer, 0, &vertex_bytes);
        }
        self.last_stats.draw_calls = calls.len();
        self.last_stats.resident_source_textures = self.textures.len();

        self.ensure_composition(device, presentation.physical_extent);
        let composition = self.composition.as_ref().expect("composition was created");
        let clear = scene.clear;
        {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &composition.view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(clear.r) / 255.0,
                        g: f64::from(clear.g) / 255.0,
                        b: f64::from(clear.b) / 255.0,
                        a: f64::from(clear.a) / 255.0,
                    }),
                    store: true,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lc_gpu_scene_pass"),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
            });
            if !calls.is_empty() {
                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                pass.set_bind_group(0, &self.gamma_bind_group, &[]);
            }
            for call in &calls {
                pass.set_scissor_rect(
                    call.scissor.x,
                    call.scissor.y,
                    call.scissor.width,
                    call.scissor.height,
                );
                match call.kind {
                    DrawKind::Quad(key) => {
                        pass.set_pipeline(match call.blend {
                            GpuBlend::Replace => &self.quad_replace_pipeline,
                            GpuBlend::Normal => &self.quad_normal_pipeline,
                            GpuBlend::Additive => &self.quad_additive_pipeline,
                        });
                        pass.set_bind_group(
                            1,
                            self.quad_bind_groups
                                .get(&key)
                                .expect("quad binding was prepared"),
                            &[],
                        );
                    }
                    DrawKind::Landscape(key) => {
                        pass.set_pipeline(&self.landscape_pipeline);
                        pass.set_bind_group(
                            1,
                            self.landscape_bind_groups
                                .get(&key)
                                .expect("landscape binding was prepared"),
                            &[],
                        );
                    }
                    DrawKind::Solid(topology) => {
                        pass.set_pipeline(self.solid_pipeline(call.blend, topology));
                    }
                }
                pass.draw(call.vertices.clone(), 0..1);
            }
        }

        if scene.gamma_mode.monitor_postpass() {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &composition.gamma_resolved_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: true,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lc_gpu_monitor_gamma_pass"),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
            });
            pass.set_pipeline(&self.monitor_gamma_pipeline);
            pass.set_bind_group(0, &composition.present_bind_group, &[]);
            pass.set_bind_group(1, &self.gamma_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        let (presented_texture, presented_bind_group) = if scene.gamma_mode.monitor_postpass() {
            (
                &composition.gamma_resolved_texture,
                &composition.gamma_resolved_present_bind_group,
            )
        } else {
            (&composition.texture, &composition.present_bind_group)
        };
        self.last_presented_monitor_gamma = Some(scene.gamma_mode.monitor_postpass());
        let readback = request_readback
            .then(|| encode_readback(device, encoder, presented_texture, composition.extent))
            .transpose()?;

        {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: surface_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: true,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lc_gpu_present_pass"),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
            });
            pass.set_pipeline(&self.present_pipeline);
            pass.set_bind_group(0, presented_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        self.vertex_scratch = vertices;
        self.draw_call_scratch = calls;
        Ok(readback)
    }

    fn sync_gamma(&mut self, queue: &wgpu::Queue, scene: &GpuScene) {
        if self.gamma_revision == Some(scene.gamma.revision) {
            return;
        }
        let mut bytes = Vec::with_capacity(256 * 3 * 2);
        for channel in scene.gamma.channels.iter() {
            for value in channel {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.gamma_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(256 * 2),
                rows_per_image: Some(3),
            },
            wgpu::Extent3d {
                width: 256,
                height: 3,
                depth_or_array_layers: 1,
            },
        );
        self.gamma_revision = Some(scene.gamma.revision);
    }

    fn sync_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &[GpuTextureResource],
    ) -> Result<(), GpuRendererError> {
        let mut live = HashSet::with_capacity(resources.len());
        let mut replaced = HashSet::new();
        for resource in resources {
            if !live.insert(resource.id) {
                return Err(GpuRendererError::DuplicateTexture(resource.id));
            }
            if !resource.is_valid() {
                return Err(GpuRendererError::InvalidTextureData {
                    id: resource.id,
                    format: resource.format,
                    extent: resource.extent,
                    expected: resource.expected_len(),
                    actual: resource.pixels.len(),
                });
            }

            let recreate = self.textures.get(&resource.id).is_none_or(|cached| {
                cached.extent != resource.extent || cached.format != resource.format
            });
            if recreate {
                let texture = create_source_texture(device, resource);
                upload_full(queue, &texture, resource);
                self.last_stats.created_source_textures += 1;
                self.last_stats.full_upload_bytes = self
                    .last_stats
                    .full_upload_bytes
                    .saturating_add(resource.pixels.len() as u64);
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                self.textures.insert(
                    resource.id,
                    CachedTexture {
                        revision: resource.revision,
                        extent: resource.extent,
                        format: resource.format,
                        byte_len: resource.pixels.len() as u64,
                        last_used_epoch: self.texture_epoch,
                        _texture: texture,
                        view,
                    },
                );
                replaced.insert(resource.id);
                continue;
            }

            let cached = self
                .textures
                .get_mut(&resource.id)
                .expect("non-recreated texture exists");
            cached.last_used_epoch = self.texture_epoch;
            if cached.revision == resource.revision {
                // A recorder may encounter the same retained surface more than
                // once and repeat the delta that was already consumed above.
                continue;
            }

            let can_apply_delta =
                !resource.dirty.is_empty() && resource.base_revision == Some(cached.revision);
            if !can_apply_delta || dirty_upload_prefers_full(resource) {
                // The CPU backing is complete.  If presentation skipped one
                // or more produced revisions (or this delta has no declared
                // base), replace the GPU contents rather than layering a
                // newer dirty rectangle over stale texels.
                upload_full(queue, &cached._texture, resource);
                self.last_stats.full_upload_bytes = self
                    .last_stats
                    .full_upload_bytes
                    .saturating_add(resource.pixels.len() as u64);
            } else {
                for &rect in &resource.dirty {
                    validate_dirty(resource, rect)?;
                    if rect.width == 0 || rect.height == 0 {
                        continue;
                    }
                    upload_dirty(queue, &cached._texture, resource, rect);
                    let bytes = u64::from(rect.width)
                        .saturating_mul(u64::from(rect.height))
                        .saturating_mul(resource.format.bytes_per_pixel() as u64);
                    self.last_stats.dirty_upload_bytes =
                        self.last_stats.dirty_upload_bytes.saturating_add(bytes);
                }
            }
            cached.revision = resource.revision;
        }

        let mut retained_bytes = self.textures.values().fold(0_u64, |total, texture| {
            total.saturating_add(texture.byte_len)
        });
        if retained_bytes > SOURCE_TEXTURE_CACHE_BUDGET_BYTES
            || self.textures.len() > SOURCE_TEXTURE_CACHE_MAX_ENTRIES
        {
            let mut eviction_candidates = self
                .textures
                .iter()
                .filter(|(id, _)| !live.contains(id))
                .map(|(id, texture)| (*id, texture.last_used_epoch, texture.byte_len))
                .collect::<Vec<_>>();
            eviction_candidates.sort_by_key(|(id, epoch, _)| (*epoch, *id));
            for (id, _, bytes) in eviction_candidates {
                if retained_bytes <= SOURCE_TEXTURE_CACHE_BUDGET_BYTES
                    && self.textures.len() <= SOURCE_TEXTURE_CACHE_MAX_ENTRIES
                {
                    break;
                }
                if self.textures.remove(&id).is_some() {
                    retained_bytes = retained_bytes.saturating_sub(bytes);
                }
            }
        }
        let retained = self.textures.keys().copied().collect::<HashSet<_>>();
        self.quad_bind_groups
            .retain(|key, _| retained.contains(&key.texture) && !replaced.contains(&key.texture));
        self.landscape_bind_groups.retain(|key, _| {
            [Some(key.base), key.mask, key.liquid]
                .into_iter()
                .flatten()
                .all(|id| retained.contains(&id) && !replaced.contains(&id))
        });
        Ok(())
    }

    fn build_draw_stream(
        &mut self,
        scene: &GpuScene,
        presentation: &GpuPresentation,
    ) -> Result<(Vec<PackedVertex>, Vec<DrawCall>), GpuRendererError> {
        let mut vertices = std::mem::take(&mut self.vertex_scratch);
        let mut calls = std::mem::take(&mut self.draw_call_scratch);
        vertices.clear();
        calls.clear();
        calls.reserve(scene.commands.len());
        for command in &scene.commands {
            match command {
                GpuCommand::Quad {
                    texture,
                    owner_mask,
                    vertices: quad,
                    clip,
                    blend,
                    base_mod2,
                    owner_mod2: _,
                    sampler,
                    gamma,
                } => {
                    if owner_mask.is_some() {
                        return Err(GpuRendererError::OwnerMaskNotLowered);
                    }
                    self.require_format(*texture, GpuTextureFormat::Rgba8)?;
                    let Some(scissor) = physical_scissor(*clip, presentation)? else {
                        continue;
                    };
                    let start = vertex_count(&vertices)?;
                    for index in [0, 1, 2, 2, 1, 3] {
                        let vertex = quad[index];
                        append_vertex(
                            &mut vertices,
                            packed_quad_vertex(
                                vertex,
                                *base_mod2,
                                fragment_gamma_flag(scene.gamma_mode, *gamma),
                                presentation,
                            )?,
                        );
                    }
                    let end = vertex_count(&vertices)?;
                    calls.push(DrawCall {
                        vertices: start..end,
                        scissor,
                        blend: *blend,
                        kind: DrawKind::Quad(QuadBindingKey {
                            texture: *texture,
                            sampler: sampler_key(*sampler),
                        }),
                    });
                }
                GpuCommand::Landscape {
                    base,
                    liquid_mask,
                    liquid,
                    vertices: quad,
                    clip,
                    phase,
                    gamma,
                } => {
                    self.require_format(*base, GpuTextureFormat::Rgba8)?;
                    if liquid_mask.is_some() != liquid.is_some() {
                        return Err(GpuRendererError::IncompleteLandscapeLiquid);
                    }
                    if let Some(mask) = liquid_mask {
                        self.require_format(*mask, GpuTextureFormat::R8)?;
                    }
                    if let Some(liquid) = liquid {
                        self.require_format(*liquid, GpuTextureFormat::Rgba8)?;
                    }
                    let Some(scissor) = physical_scissor(*clip, presentation)? else {
                        continue;
                    };
                    let base_extent = self
                        .textures
                        .get(base)
                        .expect("base was format-checked")
                        .extent;
                    let liquid_scale = liquid.map_or([1.0, 1.0], |id| {
                        let extent = self
                            .textures
                            .get(&id)
                            .expect("liquid was format-checked")
                            .extent;
                        [
                            base_extent[0] as f32 / extent[0] as f32,
                            base_extent[1] as f32 / extent[1] as f32,
                        ]
                    });
                    let start = vertex_count(&vertices)?;
                    for index in [0, 1, 2, 2, 1, 3] {
                        append_vertex(
                            &mut vertices,
                            packed_landscape_vertex(
                                quad[index],
                                liquid_scale,
                                *phase,
                                fragment_gamma_flag(scene.gamma_mode, *gamma),
                                presentation,
                            )?,
                        );
                    }
                    let end = vertex_count(&vertices)?;
                    calls.push(DrawCall {
                        vertices: start..end,
                        scissor,
                        blend: GpuBlend::Normal,
                        kind: DrawKind::Landscape(LandscapeBindingKey {
                            base: *base,
                            mask: *liquid_mask,
                            liquid: *liquid,
                        }),
                    });
                }
                GpuCommand::Solid {
                    vertices: solid,
                    topology,
                    clip,
                    blend,
                    gamma,
                } => {
                    validate_primitive_count(*topology, solid.len())?;
                    if solid.is_empty() {
                        continue;
                    }
                    let Some(scissor) = physical_scissor(*clip, presentation)? else {
                        continue;
                    };
                    let draw_topology = match topology {
                        // wgpu points are always one physical pixel, whereas
                        // C++ uses glPointSize(Application.GetScale()).  The
                        // producers place points at logical pixel centers, so
                        // expand them to logical 1x1 quads before projection.
                        GpuPrimitiveTopology::PointList => GpuPrimitiveTopology::TriangleList,
                        // C++ scales line width too.  No retained-scene
                        // producer currently emits LineList; preserve the
                        // native topology until geometry expansion is needed.
                        GpuPrimitiveTopology::LineList => GpuPrimitiveTopology::LineList,
                        GpuPrimitiveTopology::TriangleList => GpuPrimitiveTopology::TriangleList,
                    };
                    let start = vertex_count(&vertices)?;
                    for vertex in solid {
                        if !vertex.color.iter().all(|value| value.is_finite()) {
                            return Err(GpuRendererError::NonFiniteCoordinate);
                        }
                        if *topology == GpuPrimitiveTopology::PointList {
                            let [x, y, w] = vertex.position;
                            for [offset_x, offset_y] in [
                                [-0.5, -0.5],
                                [0.5, -0.5],
                                [-0.5, 0.5],
                                [-0.5, 0.5],
                                [0.5, -0.5],
                                [0.5, 0.5],
                            ] {
                                append_vertex(
                                    &mut vertices,
                                    packed_solid_vertex(
                                        [x + offset_x * w, y + offset_y * w, w],
                                        vertex.color,
                                        fragment_gamma_flag(scene.gamma_mode, *gamma),
                                        presentation,
                                    )?,
                                );
                            }
                        } else {
                            append_vertex(
                                &mut vertices,
                                packed_solid_vertex(
                                    vertex.position,
                                    vertex.color,
                                    fragment_gamma_flag(scene.gamma_mode, *gamma),
                                    presentation,
                                )?,
                            );
                        }
                    }
                    let end = vertex_count(&vertices)?;
                    calls.push(DrawCall {
                        vertices: start..end,
                        scissor,
                        blend: *blend,
                        kind: DrawKind::Solid(draw_topology),
                    });
                }
            }
        }
        Ok((vertices, calls))
    }

    fn require_format(
        &self,
        id: GpuTextureId,
        expected: GpuTextureFormat,
    ) -> Result<(), GpuRendererError> {
        let texture = self
            .textures
            .get(&id)
            .ok_or(GpuRendererError::MissingTexture(id))?;
        if texture.format != expected {
            return Err(GpuRendererError::TextureFormatMismatch {
                id,
                expected,
                actual: texture.format,
            });
        }
        Ok(())
    }

    fn ensure_bind_groups(
        &mut self,
        device: &wgpu::Device,
        calls: &[DrawCall],
    ) -> Result<(), GpuRendererError> {
        for call in calls {
            match call.kind {
                DrawKind::Quad(key) if !self.quad_bind_groups.contains_key(&key) => {
                    let texture = self
                        .textures
                        .get(&key.texture)
                        .ok_or(GpuRendererError::MissingTexture(key.texture))?;
                    let sampler = if key.sampler == sampler_key(GpuSampler::Nearest) {
                        &self.nearest_sampler
                    } else {
                        &self.linear_sampler
                    };
                    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("lc_gpu_quad_bind_group"),
                        layout: &self.quad_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&texture.view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(sampler),
                            },
                        ],
                    });
                    self.quad_bind_groups.insert(key, bind_group);
                }
                DrawKind::Landscape(key) if !self.landscape_bind_groups.contains_key(&key) => {
                    let base = self
                        .textures
                        .get(&key.base)
                        .ok_or(GpuRendererError::MissingTexture(key.base))?;
                    let mask = match key.mask {
                        Some(id) => {
                            &self
                                .textures
                                .get(&id)
                                .ok_or(GpuRendererError::MissingTexture(id))?
                                .view
                        }
                        None => &self.fallback_mask_view,
                    };
                    let liquid = match key.liquid {
                        Some(id) => {
                            &self
                                .textures
                                .get(&id)
                                .ok_or(GpuRendererError::MissingTexture(id))?
                                .view
                        }
                        None => &self.fallback_liquid_view,
                    };
                    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("lc_gpu_landscape_bind_group"),
                        layout: &self.landscape_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&base.view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(mask),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::TextureView(liquid),
                            },
                            wgpu::BindGroupEntry {
                                binding: 3,
                                resource: wgpu::BindingResource::Sampler(&self.nearest_sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 4,
                                resource: wgpu::BindingResource::Sampler(
                                    &self.repeat_nearest_sampler,
                                ),
                            },
                        ],
                    });
                    self.landscape_bind_groups.insert(key, bind_group);
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn ensure_vertex_buffer(
        &mut self,
        device: &wgpu::Device,
        required: usize,
    ) -> Result<(), GpuRendererError> {
        let required =
            u64::try_from(required).map_err(|_| GpuRendererError::VertexRangeOverflow)?;
        if required <= self.vertex_buffer_size {
            return Ok(());
        }
        let size = required
            .checked_next_power_of_two()
            .ok_or(GpuRendererError::VertexRangeOverflow)?
            .max(INITIAL_VERTEX_BUFFER_SIZE);
        self.vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lc_gpu_scene_vertices"),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.vertex_buffer_size = size;
        Ok(())
    }

    fn ensure_composition(&mut self, device: &wgpu::Device, extent: [u32; 2]) {
        if self
            .composition
            .as_ref()
            .is_some_and(|composition| composition.extent == extent)
        {
            return;
        }
        let create_target = |texture_label, bind_group_label| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(texture_label),
                size: wgpu::Extent3d {
                    width: extent[0],
                    height: extent[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(bind_group_label),
                layout: &self.present_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.present_sampler),
                    },
                ],
            });
            (texture, view, bind_group)
        };
        let (texture, view, present_bind_group) =
            create_target("lc_gpu_physical_composition", "lc_gpu_present_bind_group");
        let (gamma_resolved_texture, gamma_resolved_view, gamma_resolved_present_bind_group) =
            create_target(
                "lc_gpu_monitor_gamma_composition",
                "lc_gpu_monitor_gamma_present_bind_group",
            );
        self.composition = Some(CompositionTarget {
            extent,
            texture,
            view,
            present_bind_group,
            gamma_resolved_texture,
            gamma_resolved_view,
            gamma_resolved_present_bind_group,
        });
        self.last_stats.composition_recreated = true;
    }

    fn solid_pipeline(
        &self,
        blend: GpuBlend,
        topology: GpuPrimitiveTopology,
    ) -> &wgpu::RenderPipeline {
        let index = topology_index(topology);
        match blend {
            GpuBlend::Replace => &self.solid_replace_pipelines[index],
            GpuBlend::Normal => &self.solid_normal_pipelines[index],
            GpuBlend::Additive => &self.solid_additive_pipelines[index],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct PackedVertex {
    clip: [f32; 4],
    uv: [f32; 2],
    data0: [f32; 4],
    data1: [f32; 4],
    data2: [f32; 4],
}

fn packed_quad_vertex(
    vertex: GpuVertex,
    mod2: bool,
    gamma: bool,
    presentation: &GpuPresentation,
) -> Result<PackedVertex, GpuRendererError> {
    if !vertex
        .uv
        .iter()
        .chain(vertex.modulation.iter())
        .chain(vertex.sample_tile.iter())
        .all(|value| value.is_finite())
    {
        return Err(GpuRendererError::NonFiniteCoordinate);
    }
    Ok(PackedVertex {
        clip: clip_position(vertex.position, presentation)?,
        uv: vertex.uv,
        data0: vertex.modulation,
        data1: [flag(mod2), flag(gamma), 0.0, 0.0],
        data2: vertex.sample_tile,
    })
}

fn packed_landscape_vertex(
    vertex: GpuVertex,
    liquid_scale: [f32; 2],
    phase: [f32; 3],
    gamma: bool,
    presentation: &GpuPresentation,
) -> Result<PackedVertex, GpuRendererError> {
    if !vertex
        .uv
        .iter()
        .chain(vertex.modulation.iter())
        .chain(liquid_scale.iter())
        .chain(phase.iter())
        .all(|value| value.is_finite())
    {
        return Err(GpuRendererError::NonFiniteCoordinate);
    }
    Ok(PackedVertex {
        clip: clip_position(vertex.position, presentation)?,
        uv: vertex.uv,
        data0: vertex.modulation,
        data1: [liquid_scale[0], liquid_scale[1], 0.0, 0.0],
        data2: [phase[0], phase[1], phase[2], flag(gamma)],
    })
}

fn packed_solid_vertex(
    position: [f32; 3],
    color: [f32; 4],
    gamma: bool,
    presentation: &GpuPresentation,
) -> Result<PackedVertex, GpuRendererError> {
    Ok(PackedVertex {
        clip: clip_position(position, presentation)?,
        uv: [0.0, 0.0],
        data0: color,
        data1: [flag(gamma), 0.0, 0.0, 0.0],
        data2: [0.0; 4],
    })
}

fn clip_position(
    position: [f32; 3],
    presentation: &GpuPresentation,
) -> Result<[f32; 4], GpuRendererError> {
    if !position.iter().all(|value| value.is_finite()) {
        return Err(GpuRendererError::NonFiniteCoordinate);
    }
    let [x, y, w] = position;
    let width = presentation.physical_extent[0] as f32;
    let height = presentation.physical_extent[1] as f32;
    let scale = presentation.scale;
    let crop = presentation.crop_top as f32;
    let clip = [
        2.0 * x * scale / width - w,
        w - 2.0 * (y * scale - crop * w) / height,
        0.0,
        w,
    ];
    clip.iter()
        .all(|value| value.is_finite())
        .then_some(clip)
        .ok_or(GpuRendererError::NonFiniteCoordinate)
}

fn physical_scissor(
    clip: Option<Rect>,
    presentation: &GpuPresentation,
) -> Result<Option<Scissor>, GpuRendererError> {
    let [width, height] = presentation.physical_extent;
    let Some(clip) = clip else {
        return Ok(Some(Scissor {
            x: 0,
            y: 0,
            width,
            height,
        }));
    };
    let scale = f64::from(presentation.scale);
    let crop = f64::from(presentation.crop_top);
    let left = (f64::from(clip.x) * scale).floor();
    let top = (f64::from(clip.y) * scale - crop).floor();
    let right = ((f64::from(clip.x) + f64::from(clip.width)) * scale).ceil();
    let bottom = ((f64::from(clip.y) + f64::from(clip.height)) * scale - crop).ceil();
    if ![left, top, right, bottom]
        .iter()
        .all(|value| value.is_finite())
    {
        return Err(GpuRendererError::NonFiniteCoordinate);
    }
    let left = left.clamp(0.0, f64::from(width)) as u32;
    let top = top.clamp(0.0, f64::from(height)) as u32;
    let right = right.clamp(0.0, f64::from(width)) as u32;
    let bottom = bottom.clamp(0.0, f64::from(height)) as u32;
    if right <= left || bottom <= top {
        return Ok(None);
    }
    Ok(Some(Scissor {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }))
}

fn validate_presentation(
    scene: &GpuScene,
    presentation: &GpuPresentation,
) -> Result<(), GpuRendererError> {
    if scene.logical_extent.contains(&0)
        || presentation.physical_extent.contains(&0)
        || !presentation.scale.is_finite()
        || presentation.scale <= 0.0
    {
        return Err(GpuRendererError::InvalidPresentation {
            logical: scene.logical_extent,
            physical: presentation.physical_extent,
            scale: presentation.scale,
            crop_top: presentation.crop_top,
        });
    }
    Ok(())
}

fn validate_dirty(resource: &GpuTextureResource, rect: Rect) -> Result<(), GpuRendererError> {
    let right = i64::from(rect.x) + i64::from(rect.width);
    let bottom = i64::from(rect.y) + i64::from(rect.height);
    if rect.x < 0
        || rect.y < 0
        || right > i64::from(resource.extent[0])
        || bottom > i64::from(resource.extent[1])
    {
        return Err(GpuRendererError::InvalidDirtyRect {
            id: resource.id,
            rect,
            extent: resource.extent,
        });
    }
    Ok(())
}

fn dirty_upload_prefers_full(resource: &GpuTextureResource) -> bool {
    if resource.dirty.len() <= 1 {
        return false;
    }
    let full_pixels = u64::from(resource.extent[0]).saturating_mul(u64::from(resource.extent[1]));
    let dirty_pixels = resource.dirty.iter().fold(0_u64, |total, rect| {
        total.saturating_add(u64::from(rect.width).saturating_mul(u64::from(rect.height)))
    });
    // Queue::write_texture creates one staging write per rectangle. Once the
    // changed regions cover most of a retained texture, one contiguous copy
    // is both smaller in call overhead and close enough in byte volume to the
    // native renderer's coalesced locked-surface upload.
    dirty_pixels.saturating_mul(4) >= full_pixels.saturating_mul(3)
}

fn validate_primitive_count(
    topology: GpuPrimitiveTopology,
    vertices: usize,
) -> Result<(), GpuRendererError> {
    let valid = match topology {
        GpuPrimitiveTopology::TriangleList => vertices % 3 == 0,
        GpuPrimitiveTopology::LineList => vertices % 2 == 0,
        GpuPrimitiveTopology::PointList => true,
    };
    if valid {
        Ok(())
    } else {
        Err(GpuRendererError::InvalidPrimitiveVertexCount { topology, vertices })
    }
}

fn vertex_count(vertices: &[PackedVertex]) -> Result<u32, GpuRendererError> {
    u32::try_from(vertices.len()).map_err(|_| GpuRendererError::VertexRangeOverflow)
}

fn append_vertex(vertices: &mut Vec<PackedVertex>, vertex: PackedVertex) {
    vertices.push(vertex);
}

fn packed_vertex_bytes(vertices: &[PackedVertex]) -> &[u8] {
    const {
        assert!(std::mem::size_of::<PackedVertex>() == PACKED_VERTEX_STRIDE as usize);
    }
    // SAFETY: `PackedVertex` is `repr(C)`, contains only contiguous `f32`
    // arrays, and the size assertion above excludes padding. Reading any
    // initialized object representation as bytes is valid for the upload.
    unsafe {
        std::slice::from_raw_parts(
            vertices.as_ptr().cast::<u8>(),
            std::mem::size_of_val(vertices),
        )
    }
}

fn sampler_key(sampler: GpuSampler) -> u8 {
    match sampler {
        GpuSampler::Nearest => 0,
        GpuSampler::Linear => 1,
    }
}

const fn flag(value: bool) -> f32 {
    if value {
        1.0
    } else {
        0.0
    }
}

const fn fragment_gamma_flag(mode: GpuGammaMode, command_gamma: bool) -> bool {
    mode.fragment_lookup() && command_gamma
}

fn topology_index(topology: GpuPrimitiveTopology) -> usize {
    match topology {
        GpuPrimitiveTopology::TriangleList => 0,
        GpuPrimitiveTopology::LineList => 1,
        GpuPrimitiveTopology::PointList => 2,
    }
}

fn wgpu_topology(topology: GpuPrimitiveTopology) -> wgpu::PrimitiveTopology {
    match topology {
        GpuPrimitiveTopology::TriangleList => wgpu::PrimitiveTopology::TriangleList,
        GpuPrimitiveTopology::LineList => wgpu::PrimitiveTopology::LineList,
        GpuPrimitiveTopology::PointList => wgpu::PrimitiveTopology::PointList,
    }
}

fn create_source_texture(device: &wgpu::Device, resource: &GpuTextureResource) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("lc_gpu_retained_source"),
        size: wgpu::Extent3d {
            width: resource.extent[0],
            height: resource.extent[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: texture_format(resource.format),
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn texture_format(format: GpuTextureFormat) -> wgpu::TextureFormat {
    match format {
        GpuTextureFormat::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
        GpuTextureFormat::R8 => wgpu::TextureFormat::R8Unorm,
    }
}

fn upload_full(queue: &wgpu::Queue, texture: &wgpu::Texture, resource: &GpuTextureResource) {
    let bytes_per_row = resource.extent[0] * resource.format.bytes_per_pixel() as u32;
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &resource.pixels,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(bytes_per_row),
            rows_per_image: Some(resource.extent[1]),
        },
        wgpu::Extent3d {
            width: resource.extent[0],
            height: resource.extent[1],
            depth_or_array_layers: 1,
        },
    );
}

fn upload_dirty(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    resource: &GpuTextureResource,
    rect: Rect,
) {
    let bytes_per_pixel = resource.format.bytes_per_pixel();
    let bytes_per_row = resource.extent[0] as usize * bytes_per_pixel;
    let offset = rect.y as usize * bytes_per_row + rect.x as usize * bytes_per_pixel;
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: rect.x as u32,
                y: rect.y as u32,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        &resource.pixels,
        wgpu::ImageDataLayout {
            offset: offset as u64,
            bytes_per_row: Some(bytes_per_row as u32),
            rows_per_image: Some(resource.extent[1]),
        },
        wgpu::Extent3d {
            width: rect.width,
            height: rect.height,
            depth_or_array_layers: 1,
        },
    );
}

fn encode_readback(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    texture: &wgpu::Texture,
    extent: [u32; 2],
) -> Result<GpuReadbackTicket, GpuRendererError> {
    let unpadded = usize::try_from(extent[0])
        .ok()
        .and_then(|width| width.checked_mul(4))
        .ok_or(GpuRendererError::ReadbackSizeOverflow)?;
    let alignment = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize;
    let padded = unpadded
        .checked_add(alignment - 1)
        .map(|value| value / alignment * alignment)
        .ok_or(GpuRendererError::ReadbackSizeOverflow)?;
    let size = padded
        .checked_mul(extent[1] as usize)
        .and_then(|size| u64::try_from(size).ok())
        .ok_or(GpuRendererError::ReadbackSizeOverflow)?;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("lc_gpu_readback"),
        size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::ImageCopyTexture {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::ImageCopyBuffer {
            buffer: &buffer,
            layout: wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(extent[1]),
            },
        },
        wgpu::Extent3d {
            width: extent[0],
            height: extent[1],
            depth_or_array_layers: 1,
        },
    );
    Ok(GpuReadbackTicket {
        buffer,
        extent,
        unpadded_bytes_per_row: unpadded,
        padded_bytes_per_row: padded,
    })
}

fn fallback_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    format: wgpu::TextureFormat,
    bytes: &[u8],
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(bytes.len() as u32),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn texture_layout_entry(
    binding: u32,
    sample_type: wgpu::TextureSampleType,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_layout_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn shader(device: &wgpu::Device, label: &str, source: &'static str) -> wgpu::ShaderModule {
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(source)),
    })
}

fn packed_vertex_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: PACKED_VERTEX_STRIDE,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &PACKED_VERTEX_ATTRIBUTES,
    }
}

fn scene_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    topology: wgpu::PrimitiveTopology,
    blend: GpuBlend,
) -> wgpu::RenderPipeline {
    let vertex_layouts = [packed_vertex_layout()];
    let targets = [Some(wgpu::ColorTargetState {
        format: wgpu::TextureFormat::Rgba8Unorm,
        blend: Some(blend_state(blend)),
        write_mask: wgpu::ColorWrites::ALL,
    })];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: "vs_main",
            buffers: &vertex_layouts,
        },
        primitive: wgpu::PrimitiveState {
            topology,
            cull_mode: None,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: "fs_main",
            targets: &targets,
        }),
        multiview: None,
    })
}

fn solid_pipelines(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    blend: GpuBlend,
) -> [wgpu::RenderPipeline; 3] {
    [
        GpuPrimitiveTopology::TriangleList,
        GpuPrimitiveTopology::LineList,
        GpuPrimitiveTopology::PointList,
    ]
    .map(|topology| {
        scene_pipeline(
            device,
            match (blend, topology) {
                (GpuBlend::Replace, GpuPrimitiveTopology::TriangleList) => {
                    "lc_gpu_solid_replace_triangles"
                }
                (GpuBlend::Replace, GpuPrimitiveTopology::LineList) => "lc_gpu_solid_replace_lines",
                (GpuBlend::Replace, GpuPrimitiveTopology::PointList) => {
                    "lc_gpu_solid_replace_points"
                }
                (GpuBlend::Normal, GpuPrimitiveTopology::TriangleList) => {
                    "lc_gpu_solid_normal_triangles"
                }
                (GpuBlend::Normal, GpuPrimitiveTopology::LineList) => "lc_gpu_solid_normal_lines",
                (GpuBlend::Normal, GpuPrimitiveTopology::PointList) => "lc_gpu_solid_normal_points",
                (GpuBlend::Additive, GpuPrimitiveTopology::TriangleList) => {
                    "lc_gpu_solid_additive_triangles"
                }
                (GpuBlend::Additive, GpuPrimitiveTopology::LineList) => {
                    "lc_gpu_solid_additive_lines"
                }
                (GpuBlend::Additive, GpuPrimitiveTopology::PointList) => {
                    "lc_gpu_solid_additive_points"
                }
            },
            layout,
            shader,
            wgpu_topology(topology),
            blend,
        )
    })
}

fn present_pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    surface_format: wgpu::TextureFormat,
    monitor_gamma: bool,
) -> wgpu::RenderPipeline {
    let targets = [Some(wgpu::ColorTargetState {
        format: surface_format,
        blend: None,
        write_mask: wgpu::ColorWrites::ALL,
    })];
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(if monitor_gamma {
            "lc_gpu_monitor_gamma_pipeline"
        } else {
            "lc_gpu_present_pipeline"
        }),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: "vs_main",
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: match (monitor_gamma, surface_format.is_srgb()) {
                (false, false) => "fs_linear",
                (false, true) => "fs_srgb",
                (true, false) => "fs_monitor_linear",
                (true, true) => "fs_monitor_srgb",
            },
            targets: &targets,
        }),
        multiview: None,
    })
}

fn blend_state(blend: GpuBlend) -> wgpu::BlendState {
    match blend {
        GpuBlend::Replace => wgpu::BlendState::REPLACE,
        GpuBlend::Normal => wgpu::BlendState::ALPHA_BLENDING,
        GpuBlend::Additive => wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_graphics::{
        Color, GammaRamp, GpuGammaLut, GpuSolidVertex, GpuTextureResource, PixelFormat, Surface,
    };
    use lc_gui::{ImageData, Rect as GuiRect};
    use std::sync::Arc;

    #[test]
    fn fragmented_large_texture_delta_prefers_one_full_upload() {
        let resource = GpuTextureResource {
            id: GpuTextureId::fresh(),
            extent: [100, 100],
            revision: 2,
            base_revision: Some(1),
            format: GpuTextureFormat::Rgba8,
            pixels: Arc::from(vec![0; 100 * 100 * 4].into_boxed_slice()),
            dirty: vec![Rect::new(0, 0, 100, 40), Rect::new(0, 40, 100, 40)],
        };
        assert!(dirty_upload_prefers_full(&resource));

        let sparse = GpuTextureResource {
            dirty: vec![Rect::new(1, 1, 2, 2), Rect::new(90, 90, 2, 2)],
            ..resource
        };
        assert!(!dirty_upload_prefers_full(&sparse));
    }

    #[test]
    fn packed_vertex_upload_is_exactly_eighteen_native_floats() {
        let vertex = PackedVertex {
            clip: [0.0, 1.0, 2.0, 3.0],
            uv: [4.0, 5.0],
            data0: [6.0, 7.0, 8.0, 9.0],
            data1: [10.0, 11.0, 12.0, 13.0],
            data2: [14.0, 15.0, 16.0, 17.0],
        };
        let vertices = [vertex];
        let bytes = packed_vertex_bytes(&vertices);
        assert_eq!(bytes.len(), PACKED_VERTEX_STRIDE as usize);
        let expected = (0..PACKED_VERTEX_FLOATS)
            .flat_map(|value| (value as f32).to_ne_bytes())
            .collect::<Vec<_>>();
        assert_eq!(bytes, expected);
    }

    #[test]
    fn disabled_gamma_mode_clears_requested_fragment_gamma_flag() {
        assert!(fragment_gamma_flag(GpuGammaMode::Fragment, true));
        assert!(!fragment_gamma_flag(GpuGammaMode::Disabled, true));
        assert!(!fragment_gamma_flag(GpuGammaMode::Monitor, true));
    }

    const LOGICAL: [u32; 2] = [8, 6];
    const CLEAR: [u8; 4] = [10, 20, 30, 255];
    const HALF: [u8; 4] = [80, 120, 200, 128];
    const MAGENTA: [u8; 4] = [220, 30, 180, 255];
    const CYAN: [u8; 4] = [20, 190, 210, 255];
    const SOLID: [u8; 4] = [20, 220, 40, 255];
    const POINT: [u8; 4] = [240, 180, 20, 255];
    const LANDSCAPE_LEFT: [u8; 4] = [100, 50, 25, 255];
    const LANDSCAPE_RIGHT: [u8; 4] = [40, 80, 120, 255];
    const OWNER_BASE: [u8; 4] = [80, 120, 160, 255];
    const OWNER_OVERLAY: [u8; 4] = [200, 100, 50, 128];

    #[derive(Clone, Copy)]
    struct SceneTextureIds {
        mutable: GpuTextureId,
        half: GpuTextureId,
        black: GpuTextureId,
        magenta: GpuTextureId,
        cyan: GpuTextureId,
        landscape_base: GpuTextureId,
        liquid_mask: GpuTextureId,
        liquid: GpuTextureId,
        owner_base: GpuTextureId,
        owner_overlay: GpuTextureId,
    }

    #[test]
    fn gpu_renderer_matches_cpu_reference_frame() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build Tokio runtime for wgpu adapter discovery");
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            dx12_shader_compiler: wgpu::Dx12Compiler::default(),
        });
        let adapter = runtime
            .block_on(async {
                let primary = instance
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::HighPerformance,
                        compatible_surface: None,
                        force_fallback_adapter: false,
                    })
                    .await;
                if primary.is_some() {
                    primary
                } else {
                    instance
                        .request_adapter(&wgpu::RequestAdapterOptions {
                            power_preference: wgpu::PowerPreference::LowPower,
                            compatible_surface: None,
                            force_fallback_adapter: true,
                        })
                        .await
                }
            })
            .unwrap_or_else(|| {
                panic!(
                    "gpu_renderer_matches_cpu_reference_frame requires a working wgpu adapter; \
                     no hardware or fallback adapter was available for Backends::all()"
                )
            });
        let adapter_info = adapter.get_info();
        eprintln!(
            "GPU parity adapter: {} ({:?}, {:?})",
            adapter_info.name, adapter_info.backend, adapter_info.device_type
        );
        let descriptor = wgpu::DeviceDescriptor {
            label: Some("lc_gpu_parity_test_device"),
            features: wgpu::Features::empty(),
            limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        };
        let (device, queue) = runtime
            .block_on(adapter.request_device(&descriptor, None))
            .expect("request wgpu device for retained renderer parity test");
        device.push_error_scope(wgpu::ErrorFilter::Validation);

        let ids = SceneTextureIds {
            mutable: GpuTextureId::fresh(),
            half: GpuTextureId::fresh(),
            black: GpuTextureId::fresh(),
            magenta: GpuTextureId::fresh(),
            cyan: GpuTextureId::fresh(),
            landscape_base: GpuTextureId::fresh(),
            liquid_mask: GpuTextureId::fresh(),
            liquid: GpuTextureId::fresh(),
            owner_base: GpuTextureId::fresh(),
            owner_overlay: GpuTextureId::fresh(),
        };
        let initial_mutable = [100, 50, 25, 255];
        let mut scene = representative_scene(ids, initial_mutable);
        let mut renderer =
            RetainedGpuRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);

        let initial = render_readback(
            &mut renderer,
            &device,
            &queue,
            &scene,
            &GpuPresentation::identity(LOGICAL[0], LOGICAL[1]),
        );
        let validation = runtime.block_on(device.pop_error_scope());
        assert!(
            validation.is_none(),
            "initial device frame reported wgpu validation error: {validation:?}"
        );
        device.push_error_scope(wgpu::ErrorFilter::Validation);
        assert_eq!(
            initial.rgba,
            expected_frame(LOGICAL, initial_mutable, &scene.gamma),
            "initial retained GPU frame must match the local CPU oracle"
        );
        assert_eq!(renderer.last_stats().created_source_textures, 10);
        assert_eq!(renderer.last_stats().full_upload_bytes, 46);
        assert_eq!(renderer.last_stats().dirty_upload_bytes, 0);
        assert!(renderer.last_stats().composition_recreated);
        assert_eq!(
            readback_last_presentation(&renderer, &device, &queue),
            initial,
            "save capture must read the retained frame before a later render overwrites it",
        );

        let monitor_ramp = GammaRamp::from_control_points([0x102030, 0x708090, 0xd0e0f0]);
        let mut raw_scene = scene.clone();
        raw_scene.gamma = GpuGammaLut::from_ramp(&monitor_ramp);
        raw_scene.gamma_mode = GpuGammaMode::Disabled;
        let raw = render_readback(
            &mut renderer,
            &device,
            &queue,
            &raw_scene,
            &GpuPresentation::identity(LOGICAL[0], LOGICAL[1]),
        );
        let mut expected_monitor = raw.rgba.clone();
        monitor_ramp.apply_to_rgba_bytes(&mut expected_monitor);
        let mut monitor_scene = raw_scene;
        monitor_scene.gamma_mode = GpuGammaMode::Monitor;
        let monitor = render_readback(
            &mut renderer,
            &device,
            &queue,
            &monitor_scene,
            &GpuPresentation::identity(LOGICAL[0], LOGICAL[1]),
        );
        assert_ne!(monitor.rgba, raw.rgba);
        assert_eq!(
            monitor.rgba, expected_monitor,
            "monitor gamma must resolve the complete composition before readback",
        );
        assert_eq!(
            readback_last_presentation(&renderer, &device, &queue),
            monitor,
            "previous-frame capture must retain the presented monitor-gamma target",
        );

        let hidden = GpuScene {
            logical_extent: LOGICAL,
            clear: Color::new(CLEAR[0], CLEAR[1], CLEAR[2], CLEAR[3]),
            gamma: scene.gamma.clone(),
            gamma_mode: scene.gamma_mode,
            textures: Vec::new(),
            commands: Vec::new(),
        };
        let _ = render_readback(
            &mut renderer,
            &device,
            &queue,
            &hidden,
            &GpuPresentation::identity(LOGICAL[0], LOGICAL[1]),
        );
        assert_eq!(
            renderer.last_stats().resident_source_textures,
            10,
            "temporarily hidden C4Surface textures stay resident"
        );
        let visible_again = render_readback(
            &mut renderer,
            &device,
            &queue,
            &scene,
            &GpuPresentation::identity(LOGICAL[0], LOGICAL[1]),
        );
        assert_eq!(visible_again.rgba, initial.rgba);
        assert_eq!(renderer.last_stats().created_source_textures, 0);
        assert_eq!(renderer.last_stats().full_upload_bytes, 0);

        let scaled = render_readback(
            &mut renderer,
            &device,
            &queue,
            &scene,
            &GpuPresentation {
                physical_extent: [LOGICAL[0] * 2, LOGICAL[1] * 2],
                scale: 2.0,
                crop_top: 0,
            },
        );
        for y in 8..10 {
            for x in 12..14 {
                assert_eq!(
                    readback_pixel(&scaled, x, y),
                    POINT,
                    "a logical point must cover a 2x2 physical block at scale 2"
                );
            }
        }
        assert_eq!(readback_pixel(&scaled, 14, 8), CLEAR);

        // Exercise the real frontend CPU reference and retained capture for a
        // 5x3 selects four-pixel C4TexRefs: the right column and bottom row
        // therefore expose both native tile seams and 0xffffffff padding.
        // A whole-image sampler or transparent-black padding disagrees with
        // this real frontend CPU reference along those partial edges.
        let tiled_pixels = (0_u8..15)
            .flat_map(|value| {
                [
                    20_u8.wrapping_add(value.wrapping_mul(11)),
                    30_u8.wrapping_add(value.wrapping_mul(17)),
                    40_u8.wrapping_add(value.wrapping_mul(23)),
                    255,
                ]
            })
            .collect::<Vec<_>>();
        let tiled_image = ImageData::new(5, 3, tiled_pixels);
        let tiled_rect = GuiRect::new(0.0, 0.0, 10.0, 6.0);
        let mut cpu_tiled = Surface::new(10, 6, PixelFormat::Rgba8888);
        lc_frontend::draw_image_bilinear(&mut cpu_tiled, &tiled_rect, &tiled_image, None);
        let mut gpu_tiled = Surface::new(10, 6, PixelFormat::Rgba8888);
        gpu_tiled.begin_gpu_scene_capture();
        lc_frontend::draw_image_bilinear(&mut gpu_tiled, &tiled_rect, &tiled_image, None);
        let tiled_scene = gpu_tiled
            .take_gpu_scene_capture()
            .expect("linear draw remains captured")
            .into_scene([10, 6], Color::transparent(), &GammaRamp::standard());
        assert_eq!(tiled_scene.commands.len(), 1);
        let tiled_gpu = render_readback(
            &mut renderer,
            &device,
            &queue,
            &tiled_scene,
            &GpuPresentation::identity(10, 6),
        );
        assert_eq!(tiled_gpu.rgba.len(), cpu_tiled.pixels().len());
        for (index, (&actual, &expected)) in
            tiled_gpu.rgba.iter().zip(cpu_tiled.pixels()).enumerate()
        {
            if index % 4 == 3 {
                assert_eq!(actual, expected, "tile alpha byte {index}");
            } else {
                assert!(
                    actual.abs_diff(expected) <= 1,
                    "tile color byte {index}: GPU {actual}, CPU {expected}"
                );
            }
        }

        let resized_extent = [10, 8];
        let resized = render_readback(
            &mut renderer,
            &device,
            &queue,
            &scene,
            &GpuPresentation::identity(resized_extent[0], resized_extent[1]),
        );
        assert_eq!(
            resized.rgba,
            expected_frame(resized_extent, initial_mutable, &scene.gamma),
            "physical resize must preserve scene coordinates and content"
        );
        assert_eq!(renderer.last_stats().created_source_textures, 0);
        assert_eq!(renderer.last_stats().full_upload_bytes, 0);
        assert_eq!(renderer.last_stats().dirty_upload_bytes, 0);
        assert!(renderer.last_stats().composition_recreated);

        let updated_mutable = [200, 10, 20, 255];
        let mutable = scene
            .textures
            .iter_mut()
            .find(|resource| resource.id == ids.mutable)
            .expect("mutable retained resource");
        mutable.revision = 1;
        mutable.base_revision = Some(0);
        mutable.pixels = Arc::from(updated_mutable);
        mutable.dirty = vec![Rect::new(0, 0, 1, 1)];

        let dirty = render_readback(
            &mut renderer,
            &device,
            &queue,
            &scene,
            &GpuPresentation::identity(resized_extent[0], resized_extent[1]),
        );
        assert_eq!(
            dirty.rgba,
            expected_frame(resized_extent, updated_mutable, &scene.gamma),
            "one dirty texel must update every use without a full upload"
        );
        assert_eq!(renderer.last_stats().created_source_textures, 0);
        assert_eq!(renderer.last_stats().full_upload_bytes, 0);
        assert_eq!(renderer.last_stats().dirty_upload_bytes, 4);
        assert!(!renderer.last_stats().composition_recreated);

        let validation = runtime.block_on(device.pop_error_scope());
        assert!(
            validation.is_none(),
            "first device reported wgpu validation error: {validation:?}"
        );

        let (replacement_device, replacement_queue) = runtime
            .block_on(adapter.request_device(&descriptor, None))
            .expect("request replacement wgpu device");
        replacement_device.push_error_scope(wgpu::ErrorFilter::Validation);
        let previous_generation = renderer.generation();
        renderer.recreate(
            &replacement_device,
            &replacement_queue,
            wgpu::TextureFormat::Rgba8Unorm,
        );
        assert_ne!(renderer.generation(), previous_generation);
        let recreated = render_readback(
            &mut renderer,
            &replacement_device,
            &replacement_queue,
            &scene,
            &GpuPresentation::identity(resized_extent[0], resized_extent[1]),
        );
        assert_eq!(
            recreated.rgba,
            expected_frame(resized_extent, updated_mutable, &scene.gamma),
            "device recreation must regenerate every retained source from complete backing"
        );
        assert_eq!(renderer.last_stats().created_source_textures, 10);
        assert_eq!(renderer.last_stats().full_upload_bytes, 46);
        assert_eq!(renderer.last_stats().dirty_upload_bytes, 0);
        assert!(renderer.last_stats().composition_recreated);
        let validation = runtime.block_on(replacement_device.pop_error_scope());
        assert!(
            validation.is_none(),
            "replacement device reported wgpu validation error: {validation:?}"
        );
    }

    fn representative_scene(ids: SceneTextureIds, mutable: [u8; 4]) -> GpuScene {
        let identity = [1.0, 1.0, 1.0, 0.0];
        let mut commands = vec![
            GpuCommand::Quad {
                texture: ids.mutable,
                owner_mask: None,
                vertices: quad(0.0, 0.0, 2.0, 2.0, 1.0, identity),
                clip: None,
                blend: GpuBlend::Replace,
                base_mod2: false,
                owner_mod2: false,
                sampler: GpuSampler::Nearest,
                gamma: false,
            },
            GpuCommand::Quad {
                texture: ids.half,
                owner_mask: None,
                vertices: quad(2.0, 0.0, 4.0, 2.0, 1.0, identity),
                clip: None,
                blend: GpuBlend::Normal,
                base_mod2: false,
                owner_mod2: false,
                sampler: GpuSampler::Nearest,
                gamma: false,
            },
            GpuCommand::Quad {
                texture: ids.half,
                owner_mask: None,
                vertices: quad(4.0, 0.0, 6.0, 2.0, 1.0, identity),
                clip: None,
                blend: GpuBlend::Additive,
                base_mod2: false,
                owner_mod2: false,
                sampler: GpuSampler::Nearest,
                gamma: false,
            },
            GpuCommand::Quad {
                texture: ids.black,
                owner_mask: None,
                vertices: quad(
                    6.0,
                    0.0,
                    8.0,
                    2.0,
                    1.0,
                    [127.0 / 255.0, 127.0 / 255.0, 127.0 / 255.0, 0.0],
                ),
                clip: None,
                blend: GpuBlend::Replace,
                base_mod2: true,
                owner_mod2: false,
                sampler: GpuSampler::Nearest,
                gamma: true,
            },
            GpuCommand::Quad {
                texture: ids.magenta,
                owner_mask: None,
                vertices: quad(0.0, 2.0, 4.0, 4.0, 1.0, identity),
                clip: Some(Rect::new(1, 3, 2, 1)),
                blend: GpuBlend::Replace,
                base_mod2: false,
                owner_mod2: false,
                sampler: GpuSampler::Nearest,
                gamma: false,
            },
            GpuCommand::Quad {
                texture: ids.cyan,
                owner_mask: None,
                // A constant source texel keeps the output unambiguous while
                // unequal positive W values exercise perspective-correct
                // captured transforms all the way through the backend.
                vertices: projective_quad(4.0, 2.0, 6.0, 4.0, [1.0, 1.5, 2.0, 2.5], identity),
                clip: None,
                blend: GpuBlend::Replace,
                base_mod2: false,
                owner_mod2: false,
                sampler: GpuSampler::Nearest,
                gamma: false,
            },
            GpuCommand::Landscape {
                base: ids.landscape_base,
                liquid_mask: Some(ids.liquid_mask),
                liquid: Some(ids.liquid),
                // Two source texels stretched over four destination pixels:
                // native nearest sampling repeats each texel exactly twice.
                vertices: quad(0.0, 4.0, 4.0, 6.0, 1.0, identity),
                clip: None,
                phase: [0.25, 0.0, 0.0],
                gamma: false,
            },
            GpuCommand::Quad {
                texture: ids.owner_base,
                owner_mask: None,
                vertices: modulated_quad(
                    6.0,
                    2.0,
                    8.0,
                    4.0,
                    [
                        [1.0, 1.0, 1.0, 0.0],
                        [0.5, 1.0, 1.0, 0.0],
                        [1.0, 0.5, 1.0, 0.0],
                        [0.5, 0.5, 1.0, 0.0],
                    ],
                ),
                clip: None,
                blend: GpuBlend::Normal,
                base_mod2: false,
                owner_mod2: false,
                sampler: GpuSampler::Nearest,
                gamma: false,
            },
            // ColorByOwner surfaces are lowered to a complete base pass and
            // then a complete owner pass. Vary the red owner modulation at
            // the same corners to prove spatial FoW interpolation survives.
            GpuCommand::Quad {
                texture: ids.owner_overlay,
                owner_mask: None,
                vertices: modulated_quad(
                    6.0,
                    2.0,
                    8.0,
                    4.0,
                    [
                        [1.0, 0.0, 0.0, 0.0],
                        [0.5, 0.0, 0.0, 0.0],
                        [1.0, 0.0, 0.0, 0.0],
                        [0.5, 0.0, 0.0, 0.0],
                    ],
                ),
                clip: None,
                blend: GpuBlend::Normal,
                base_mod2: false,
                owner_mod2: false,
                sampler: GpuSampler::Nearest,
                gamma: false,
            },
        ];
        let color = rgba_f32(SOLID);
        commands.push(GpuCommand::Solid {
            vertices: vec![
                solid_vertex(4.0, 4.0, color),
                solid_vertex(6.0, 4.0, color),
                solid_vertex(4.0, 6.0, color),
                solid_vertex(4.0, 6.0, color),
                solid_vertex(6.0, 4.0, color),
                solid_vertex(6.0, 6.0, color),
            ],
            topology: GpuPrimitiveTopology::TriangleList,
            clip: None,
            blend: GpuBlend::Replace,
            gamma: false,
        });
        commands.push(GpuCommand::Solid {
            // Producers encode logical pixel centers.  Non-unit W proves the
            // point expansion preserves homogeneous coordinates.
            vertices: vec![solid_vertex_w(6.5, 4.5, 2.0, rgba_f32(POINT))],
            topology: GpuPrimitiveTopology::PointList,
            clip: None,
            blend: GpuBlend::Replace,
            gamma: false,
        });

        GpuScene {
            logical_extent: LOGICAL,
            clear: Color::new(CLEAR[0], CLEAR[1], CLEAR[2], CLEAR[3]),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
            gamma_mode: GpuGammaMode::Fragment,
            textures: vec![
                rgba_resource(ids.mutable, mutable),
                rgba_resource(ids.half, HALF),
                rgba_resource(ids.black, [0, 0, 0, 255]),
                rgba_resource(ids.magenta, MAGENTA),
                rgba_resource(ids.cyan, CYAN),
                rgba_resource_2x1(ids.landscape_base, LANDSCAPE_LEFT, LANDSCAPE_RIGHT),
                GpuTextureResource {
                    id: ids.liquid_mask,
                    extent: [2, 1],
                    revision: 0,
                    base_revision: None,
                    format: GpuTextureFormat::R8,
                    pixels: Arc::from([255, 0]),
                    dirty: Vec::new(),
                },
                rgba_resource_2x1(ids.liquid, [255, 128, 128, 255], [0, 128, 128, 255]),
                rgba_resource(ids.owner_base, OWNER_BASE),
                rgba_resource(ids.owner_overlay, OWNER_OVERLAY),
            ],
            commands,
        }
    }

    fn rgba_resource(id: GpuTextureId, pixel: [u8; 4]) -> GpuTextureResource {
        GpuTextureResource {
            id,
            extent: [1, 1],
            revision: 0,
            base_revision: None,
            format: GpuTextureFormat::Rgba8,
            pixels: Arc::from(pixel),
            dirty: Vec::new(),
        }
    }

    fn rgba_resource_2x1(id: GpuTextureId, left: [u8; 4], right: [u8; 4]) -> GpuTextureResource {
        let mut pixels = Vec::with_capacity(8);
        pixels.extend_from_slice(&left);
        pixels.extend_from_slice(&right);
        GpuTextureResource {
            id,
            extent: [2, 1],
            revision: 0,
            base_revision: None,
            format: GpuTextureFormat::Rgba8,
            pixels: Arc::from(pixels),
            dirty: Vec::new(),
        }
    }

    fn quad(
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
        w: f32,
        modulation: [f32; 4],
    ) -> [GpuVertex; 4] {
        [
            GpuVertex::new([left * w, top * w, w], [0.0, 0.0], modulation),
            GpuVertex::new([right * w, top * w, w], [1.0, 0.0], modulation),
            GpuVertex::new([left * w, bottom * w, w], [0.0, 1.0], modulation),
            GpuVertex::new([right * w, bottom * w, w], [1.0, 1.0], modulation),
        ]
    }

    fn projective_quad(
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
        w: [f32; 4],
        modulation: [f32; 4],
    ) -> [GpuVertex; 4] {
        [
            GpuVertex::new([left * w[0], top * w[0], w[0]], [0.0, 0.0], modulation),
            GpuVertex::new([right * w[1], top * w[1], w[1]], [1.0, 0.0], modulation),
            GpuVertex::new([left * w[2], bottom * w[2], w[2]], [0.0, 1.0], modulation),
            GpuVertex::new([right * w[3], bottom * w[3], w[3]], [1.0, 1.0], modulation),
        ]
    }

    fn modulated_quad(
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
        modulation: [[f32; 4]; 4],
    ) -> [GpuVertex; 4] {
        [
            GpuVertex::new([left, top, 1.0], [0.0, 0.0], modulation[0]),
            GpuVertex::new([right, top, 1.0], [1.0, 0.0], modulation[1]),
            GpuVertex::new([left, bottom, 1.0], [0.0, 1.0], modulation[2]),
            GpuVertex::new([right, bottom, 1.0], [1.0, 1.0], modulation[3]),
        ]
    }

    fn solid_vertex(x: f32, y: f32, color: [f32; 4]) -> GpuSolidVertex {
        solid_vertex_w(x, y, 1.0, color)
    }

    fn solid_vertex_w(x: f32, y: f32, w: f32, color: [f32; 4]) -> GpuSolidVertex {
        GpuSolidVertex {
            position: [x * w, y * w, w],
            color,
        }
    }

    fn rgba_f32(color: [u8; 4]) -> [f32; 4] {
        color.map(|component| f32::from(component) / 255.0)
    }

    fn render_readback(
        renderer: &mut RetainedGpuRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &GpuScene,
        presentation: &GpuPresentation,
    ) -> GpuReadbackFrame {
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lc_gpu_parity_test_surface"),
            size: wgpu::Extent3d {
                width: presentation.physical_extent[0],
                height: presentation.physical_extent[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let target_view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("lc_gpu_parity_test_encoder"),
        });
        let ticket = renderer
            .render(
                device,
                queue,
                &mut encoder,
                &target_view,
                scene,
                presentation,
                true,
            )
            .expect("encode retained GPU scene")
            .expect("request readback ticket");
        queue.submit(Some(encoder.finish()));
        ticket.read(device).expect("map retained GPU frame")
    }

    fn readback_last_presentation(
        renderer: &RetainedGpuRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> GpuReadbackFrame {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("lc_gpu_previous_frame_test_encoder"),
        });
        let ticket = renderer
            .readback_last_presentation(device, &mut encoder)
            .expect("encode previous retained GPU frame")
            .expect("previous retained GPU frame exists");
        queue.submit(Some(encoder.finish()));
        ticket
            .read(device)
            .expect("map previous retained GPU frame")
    }

    fn expected_frame(extent: [u32; 2], mutable: [u8; 4], gamma: &GpuGammaLut) -> Vec<u8> {
        let mut frame = vec![0; extent[0] as usize * extent[1] as usize * 4];
        for pixel in frame.chunks_exact_mut(4) {
            pixel.copy_from_slice(&CLEAR);
        }
        fill(&mut frame, extent, 0, 0, 2, 2, mutable);
        fill(&mut frame, extent, 2, 0, 4, 2, alpha_over(HALF, CLEAR));
        fill(&mut frame, extent, 4, 0, 6, 2, additive(HALF, CLEAR));
        let gamma_black = [
            gamma_byte(gamma.channels[0][0]),
            gamma_byte(gamma.channels[1][0]),
            gamma_byte(gamma.channels[2][0]),
            255,
        ];
        fill(&mut frame, extent, 6, 0, 8, 2, gamma_black);
        fill(&mut frame, extent, 1, 3, 3, 4, MAGENTA);
        fill(&mut frame, extent, 4, 2, 6, 4, CYAN);
        for y in 2..4 {
            for x in 6..8 {
                let u = (x as f32 + 0.5 - 6.0) / 2.0;
                let v = (y as f32 + 0.5 - 2.0) / 2.0;
                let base = [
                    (f32::from(OWNER_BASE[0]) * (1.0 - 0.5 * u)).round() as u8,
                    (f32::from(OWNER_BASE[1]) * (1.0 - 0.5 * v)).round() as u8,
                    OWNER_BASE[2],
                    255,
                ];
                let owner_red = f32::from(OWNER_OVERLAY[0]) * (1.0 - 0.5 * u);
                let alpha = f32::from(OWNER_OVERLAY[3]) / 255.0;
                let owner = [
                    (owner_red * alpha + f32::from(base[0]) * (1.0 - alpha)).round() as u8,
                    (f32::from(base[1]) * (1.0 - alpha)).round() as u8,
                    (f32::from(base[2]) * (1.0 - alpha)).round() as u8,
                    255,
                ];
                fill(&mut frame, extent, x, y, x + 1, y + 1, owner);
            }
        }
        // StdGL collapses the noise dot product to one scalar, then adds that
        // same value to all three landscape channels.
        let animated_channel = |channel: u8| {
            ((f32::from(channel) / 255.0 + 0.125).clamp(0.0, 1.0) * 255.0).round() as u8
        };
        fill(
            &mut frame,
            extent,
            0,
            4,
            2,
            6,
            [
                animated_channel(LANDSCAPE_LEFT[0]),
                animated_channel(LANDSCAPE_LEFT[1]),
                animated_channel(LANDSCAPE_LEFT[2]),
                LANDSCAPE_LEFT[3],
            ],
        );
        // The second nearest mask texel is zero, so the right base texel is
        // repeated without liquid animation.
        fill(&mut frame, extent, 2, 4, 4, 6, LANDSCAPE_RIGHT);
        fill(&mut frame, extent, 4, 4, 6, 6, SOLID);
        fill(&mut frame, extent, 6, 4, 7, 5, POINT);
        frame
    }

    fn readback_pixel(frame: &GpuReadbackFrame, x: u32, y: u32) -> [u8; 4] {
        assert!(x < frame.extent[0] && y < frame.extent[1]);
        let offset = (y as usize * frame.extent[0] as usize + x as usize) * 4;
        frame.rgba[offset..offset + 4]
            .try_into()
            .expect("one readback RGBA pixel")
    }

    fn fill(
        frame: &mut [u8],
        extent: [u32; 2],
        left: u32,
        top: u32,
        right: u32,
        bottom: u32,
        color: [u8; 4],
    ) {
        for y in top..bottom.min(extent[1]) {
            for x in left..right.min(extent[0]) {
                let offset = (y as usize * extent[0] as usize + x as usize) * 4;
                frame[offset..offset + 4].copy_from_slice(&color);
            }
        }
    }

    fn alpha_over(source: [u8; 4], destination: [u8; 4]) -> [u8; 4] {
        let alpha = f32::from(source[3]) / 255.0;
        let channel = |index: usize| {
            (f32::from(source[index]) * alpha + f32::from(destination[index]) * (1.0 - alpha))
                .round() as u8
        };
        [channel(0), channel(1), channel(2), 255]
    }

    fn additive(source: [u8; 4], destination: [u8; 4]) -> [u8; 4] {
        let alpha = f32::from(source[3]) / 255.0;
        let channel = |index: usize| {
            (f32::from(destination[index]) + f32::from(source[index]) * alpha)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        [channel(0), channel(1), channel(2), destination[3]]
    }

    fn gamma_byte(value: u16) -> u8 {
        ((u32::from(value) + 128) / 257) as u8
    }
}
