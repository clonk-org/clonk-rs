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

use clonk_graphics::{
    ClipperProjection, GpuBlend, GpuCommand, GpuGammaMode, GpuPresentation, GpuPrimitiveTopology,
    GpuSampler, GpuScene, GpuSolidAlphaMode, GpuSolidVertex, GpuTextureFormat, GpuTextureId,
    GpuTextureResource, GpuVertex, Rect,
};
use pixels::wgpu;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::{mpsc, Arc, Mutex};
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

/// Recovery decision published by the wgpu 0.16 uncaptured-error hook.
///
/// That wgpu release has no public `Device::lost` future or callback. Native
/// `DeviceError::Lost` values which reach ordinary resource operations are
/// instead formatted as validation errors, so this monitor recognizes that
/// specific diagnostic. Loss reported directly by `Queue::submit` or
/// `Device::poll` is converted to an upstream panic; the application catches
/// that one documented loss diagnostic at the presentation boundary and
/// routes it through the same full-device recreation path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetainedGpuRendererHealth {
    Healthy,
    RecreateRequired {
        reason: RetainedGpuRecreateReason,
        detail: String,
    },
    Fatal {
        reason: RetainedGpuFatalReason,
        detail: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetainedGpuRecreateReason {
    DeviceLost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetainedGpuFatalReason {
    OutOfMemory,
    Validation,
}

#[derive(Clone, Debug)]
struct RetainedGpuHealthMonitor {
    state: Arc<Mutex<RetainedGpuRendererHealth>>,
}

impl RetainedGpuHealthMonitor {
    fn install(device: &wgpu::Device) -> Self {
        let state = Arc::new(Mutex::new(RetainedGpuRendererHealth::Healthy));
        let callback_state = Arc::clone(&state);
        device.on_uncaptured_error(Box::new(move |error| {
            let health = classify_uncaptured_wgpu_error(&error);
            tracing::error!(%error, ?health, "uncaptured retained GPU device error");
            record_renderer_health(&callback_state, health);
        }));
        Self { state }
    }

    fn current(&self) -> RetainedGpuRendererHealth {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

fn classify_uncaptured_wgpu_error(error: &wgpu::Error) -> RetainedGpuRendererHealth {
    match error {
        wgpu::Error::OutOfMemory { .. } => RetainedGpuRendererHealth::Fatal {
            reason: RetainedGpuFatalReason::OutOfMemory,
            detail: error.to_string(),
        },
        wgpu::Error::Validation { description, .. } => {
            classify_wgpu_validation_description(description)
        }
    }
}

fn classify_wgpu_validation_description(description: &str) -> RetainedGpuRendererHealth {
    let normalized = description.to_ascii_lowercase();
    if normalized.contains("device is lost")
        || normalized.contains("device was lost")
        || normalized.contains("device has been lost")
    {
        RetainedGpuRendererHealth::RecreateRequired {
            reason: RetainedGpuRecreateReason::DeviceLost,
            detail: description.to_owned(),
        }
    } else {
        RetainedGpuRendererHealth::Fatal {
            reason: RetainedGpuFatalReason::Validation,
            detail: description.to_owned(),
        }
    }
}

fn record_renderer_health(
    state: &Mutex<RetainedGpuRendererHealth>,
    reported: RetainedGpuRendererHealth,
) {
    let mut current = state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let replace = matches!(&*current, RetainedGpuRendererHealth::Healthy)
        || matches!(&reported, RetainedGpuRendererHealth::Fatal { .. })
            && !matches!(&*current, RetainedGpuRendererHealth::Fatal { .. });
    if replace {
        *current = reported;
    }
}

#[derive(Debug, Error)]
pub enum GpuRendererError {
    #[error("retained GPU composition requires at least one ordered scene layer")]
    NoSceneLayers,
    #[error("GPU layer {layer} uses physical extent {actual:?}, expected {expected:?}")]
    LayerPhysicalExtentMismatch {
        layer: usize,
        expected: [u32; 2],
        actual: [u32; 2],
    },
    #[error("GPU layer {layer} uses a different gamma ramp or placement mode")]
    LayerGammaMismatch { layer: usize },
    #[error("GPU layers publish conflicting complete backing for texture {0:?}")]
    LayerTextureConflict(GpuTextureId),
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
    #[error("texture {id:?} publishes dirty data without advancing revision {revision}")]
    DirtyRevisionNotAdvanced { id: GpuTextureId, revision: u64 },
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
    #[error("retained GPU device recreation required after {reason:?}: {detail}")]
    DeviceRecreationRequired {
        reason: RetainedGpuRecreateReason,
        detail: String,
    },
    #[error("retained GPU device is unusable after {reason:?}: {detail}")]
    DeviceFatal {
        reason: RetainedGpuFatalReason,
        detail: String,
    },
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

/// One painter-ordered retained scene and its coordinate transform.
///
/// Layers share a physical composition target. A logical game/UI layer uses
/// the application's scaled/cropped presentation, while scale-native text or
/// another physical-resolution overlay uses `GpuPresentation::identity` for
/// the same physical extent. This keeps the native order without forcing all
/// layers through one coordinate space.
#[derive(Clone, Copy, Debug)]
pub struct GpuSceneLayer<'a> {
    pub scene: &'a GpuScene,
    pub presentation: GpuPresentation,
}

impl<'a> GpuSceneLayer<'a> {
    pub const fn new(scene: &'a GpuScene, presentation: GpuPresentation) -> Self {
        Self {
            scene,
            presentation,
        }
    }
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

/// C++ installs one rounded viewport and clip-relative projection every time
/// the primary clipper changes. Keep those two halves together so the draw
/// geometry and hardware scissor cannot disagree at fractional scales.
#[derive(Clone, Copy, Debug)]
struct DrawProjection {
    clipper: ClipperProjection,
    physical_extent: [u32; 2],
    line_width: f32,
    scissor: Scissor,
}

#[derive(Clone, Copy, Debug)]
enum DrawKind {
    Quad(QuadBindingKey),
    Landscape(LandscapeBindingKey),
    Solid { alpha_mode: GpuSolidAlphaMode },
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
    health: RetainedGpuHealthMonitor,
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
    solid_replace_pipeline: wgpu::RenderPipeline,
    solid_over_normal_pipeline: wgpu::RenderPipeline,
    solid_non_separate_normal_pipeline: wgpu::RenderPipeline,
    solid_additive_pipeline: wgpu::RenderPipeline,
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
        let health = RetainedGpuHealthMonitor::install(device);
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
            GpuSolidAlphaMode::SourceOver,
        );
        let quad_normal_pipeline = scene_pipeline(
            device,
            "lc_gpu_quad_normal",
            &quad_pipeline_layout,
            &quad_shader,
            wgpu::PrimitiveTopology::TriangleList,
            GpuBlend::Normal,
            GpuSolidAlphaMode::SourceOver,
        );
        let quad_additive_pipeline = scene_pipeline(
            device,
            "lc_gpu_quad_additive",
            &quad_pipeline_layout,
            &quad_shader,
            wgpu::PrimitiveTopology::TriangleList,
            GpuBlend::Additive,
            GpuSolidAlphaMode::SourceOver,
        );
        let landscape_pipeline = scene_pipeline(
            device,
            "lc_gpu_landscape",
            &landscape_pipeline_layout,
            &landscape_shader,
            wgpu::PrimitiveTopology::TriangleList,
            GpuBlend::Normal,
            GpuSolidAlphaMode::SourceOver,
        );
        // Point and line commands are expanded to physical triangle quads
        // before submission, so every solid uses one TriangleList pipeline.
        let solid_replace_pipeline = scene_pipeline(
            device,
            "lc_gpu_solid_replace",
            &solid_pipeline_layout,
            &solid_shader,
            wgpu::PrimitiveTopology::TriangleList,
            GpuBlend::Replace,
            GpuSolidAlphaMode::SourceOver,
        );
        let solid_over_normal_pipeline = scene_pipeline(
            device,
            "lc_gpu_solid_over_normal",
            &solid_pipeline_layout,
            &solid_shader,
            wgpu::PrimitiveTopology::TriangleList,
            GpuBlend::Normal,
            GpuSolidAlphaMode::SourceOver,
        );
        let solid_non_separate_normal_pipeline = scene_pipeline(
            device,
            "lc_gpu_solid_non_separate_normal",
            &solid_pipeline_layout,
            &solid_shader,
            wgpu::PrimitiveTopology::TriangleList,
            GpuBlend::Normal,
            GpuSolidAlphaMode::NonSeparate,
        );
        let solid_additive_pipeline = scene_pipeline(
            device,
            "lc_gpu_solid_additive",
            &solid_pipeline_layout,
            &solid_shader,
            wgpu::PrimitiveTopology::TriangleList,
            GpuBlend::Additive,
            GpuSolidAlphaMode::SourceOver,
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
            health,
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
            solid_replace_pipeline,
            solid_over_normal_pipeline,
            solid_non_separate_normal_pipeline,
            solid_additive_pipeline,
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

    /// Rebuild every device-owned object after the application has replaced
    /// the `Pixels` device and queue.
    ///
    /// `pixels` already reconfigures and retries an acquired surface once, so
    /// an ordinary `SurfaceError::Lost`/`Outdated` does not require this. Call
    /// this only after constructing a replacement `Pixels`; the next validated
    /// scene carries complete CPU backing for every referenced texture and
    /// therefore repopulates this empty cache without a CPU-frame fallback.
    pub fn recreate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
    ) -> u64 {
        let generation = self.generation.wrapping_add(1).max(1);
        *self = Self::build(device, queue, surface_format, generation);
        generation
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn health(&self) -> RetainedGpuRendererHealth {
        self.health.current()
    }

    /// Refuse further work after an observed device fault. A recognized native
    /// device-loss diagnostic is recoverable by rebuilding `Pixels` and calling
    /// [`Self::recreate`]; validation and OOM failures remain fatal.
    pub fn check_health(&self) -> Result<(), GpuRendererError> {
        match self.health() {
            RetainedGpuRendererHealth::Healthy => Ok(()),
            RetainedGpuRendererHealth::RecreateRequired { reason, detail } => {
                Err(GpuRendererError::DeviceRecreationRequired { reason, detail })
            }
            RetainedGpuRendererHealth::Fatal { reason, detail } => {
                Err(GpuRendererError::DeviceFatal { reason, detail })
            }
        }
    }

    /// Validate a scene as a self-contained recovery unit before touching GPU
    /// state. In particular, command resources must be declared in this scene,
    /// even if an earlier frame left a texture with the same id in the cache.
    pub fn validate_scene(
        scene: &GpuScene,
        presentation: &GpuPresentation,
    ) -> Result<(), GpuRendererError> {
        validate_scene(scene, presentation)
    }

    /// Validate ordered logical/physical layers without touching device state.
    pub fn validate_layers(layers: &[GpuSceneLayer<'_>]) -> Result<(), GpuRendererError> {
        validate_layers(layers).map(drop)
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
        let layer = GpuSceneLayer::new(scene, *presentation);
        self.render_layers(
            device,
            queue,
            encoder,
            surface_view,
            std::slice::from_ref(&layer),
            request_readback,
        )
    }

    /// Compose ordered scenes that may use different coordinate spaces into
    /// one physical frame, then apply monitor gamma/readback/presentation once.
    #[allow(clippy::too_many_arguments)]
    pub fn render_layers(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
        layers: &[GpuSceneLayer<'_>],
        request_readback: bool,
    ) -> Result<Option<GpuReadbackTicket>, GpuRendererError> {
        self.check_health()?;
        let resources = validate_layers(layers)?;
        let base = layers.first().ok_or(GpuRendererError::NoSceneLayers)?;
        let scene = base.scene;
        self.last_stats = GpuRendererStats::default();
        self.texture_epoch = self.texture_epoch.wrapping_add(1).max(1);
        self.sync_gamma(queue, scene);
        self.sync_textures(device, queue, &resources)?;

        let (vertices, calls) = self.build_layered_draw_stream(layers)?;
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
                DrawKind::Solid { .. } => {}
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
            queue.write_buffer(&self.vertex_buffer, 0, vertex_bytes);
        }
        self.last_stats.draw_calls = calls.len();
        self.last_stats.resident_source_textures = self.textures.len();

        self.ensure_composition(device, base.presentation.physical_extent);
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
            self.encode_draw_calls(&mut pass, &calls);
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
        self.check_health()?;
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
            match texture_upload_plan(Some(cached.revision), resource) {
                TextureUploadPlan::Unchanged => {
                    // A recorder may encounter the same retained surface more
                    // than once and repeat the delta already consumed above.
                    continue;
                }
                TextureUploadPlan::Full => {
                    // The CPU backing is complete. If presentation skipped one
                    // or more produced revisions (including time spent in
                    // another application mode), replace the GPU contents
                    // rather than applying a delta to stale texels.
                    upload_full(queue, &cached._texture, resource);
                    self.last_stats.full_upload_bytes = self
                        .last_stats
                        .full_upload_bytes
                        .saturating_add(resource.pixels.len() as u64);
                }
                TextureUploadPlan::Dirty => {
                    for &rect in &resource.dirty {
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

    fn build_layered_draw_stream(
        &mut self,
        layers: &[GpuSceneLayer<'_>],
    ) -> Result<(Vec<PackedVertex>, Vec<DrawCall>), GpuRendererError> {
        let mut vertices = std::mem::take(&mut self.vertex_scratch);
        let mut calls = std::mem::take(&mut self.draw_call_scratch);
        vertices.clear();
        calls.clear();
        calls.reserve(layers.iter().map(|layer| layer.scene.commands.len()).sum());
        for layer in layers {
            self.append_draw_stream(layer.scene, &layer.presentation, &mut vertices, &mut calls)?;
        }
        Ok((vertices, calls))
    }

    fn append_draw_stream(
        &self,
        scene: &GpuScene,
        presentation: &GpuPresentation,
        vertices: &mut Vec<PackedVertex>,
        calls: &mut Vec<DrawCall>,
    ) -> Result<(), GpuRendererError> {
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
                    let Some(projection) =
                        draw_projection(*clip, scene.logical_extent, presentation)?
                    else {
                        continue;
                    };
                    let start = vertex_count(vertices)?;
                    for index in [0, 1, 2, 2, 1, 3] {
                        let vertex = quad[index];
                        append_vertex(
                            vertices,
                            packed_quad_vertex(
                                vertex,
                                *base_mod2,
                                fragment_gamma_flag(scene.gamma_mode, *gamma),
                                &projection,
                            )?,
                        );
                    }
                    let end = vertex_count(vertices)?;
                    calls.push(DrawCall {
                        vertices: start..end,
                        scissor: projection.scissor,
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
                    let Some(projection) =
                        draw_projection(*clip, scene.logical_extent, presentation)?
                    else {
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
                    let start = vertex_count(vertices)?;
                    for index in [0, 1, 2, 2, 1, 3] {
                        append_vertex(
                            vertices,
                            packed_landscape_vertex(
                                quad[index],
                                liquid_scale,
                                *phase,
                                fragment_gamma_flag(scene.gamma_mode, *gamma),
                                &projection,
                            )?,
                        );
                    }
                    let end = vertex_count(vertices)?;
                    calls.push(DrawCall {
                        vertices: start..end,
                        scissor: projection.scissor,
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
                    alpha_mode,
                    clip,
                    blend,
                    gamma,
                } => {
                    validate_primitive_count(*topology, solid.len())?;
                    if solid.is_empty() {
                        continue;
                    }
                    let Some(projection) =
                        draw_projection(*clip, scene.logical_extent, presentation)?
                    else {
                        continue;
                    };
                    let start = vertex_count(vertices)?;
                    if !solid
                        .iter()
                        .flat_map(|vertex| vertex.color)
                        .all(f32::is_finite)
                    {
                        return Err(GpuRendererError::NonFiniteCoordinate);
                    }
                    match topology {
                        GpuPrimitiveTopology::PointList => {
                            for vertex in solid {
                                if let Some(point) = packed_point_rect(
                                    *vertex,
                                    fragment_gamma_flag(scene.gamma_mode, *gamma),
                                    &projection,
                                )? {
                                    vertices.extend(point);
                                }
                            }
                        }
                        GpuPrimitiveTopology::LineList => {
                            for pair in solid.chunks_exact(2) {
                                vertices.extend(packed_line_fragments(
                                    pair[0],
                                    pair[1],
                                    fragment_gamma_flag(scene.gamma_mode, *gamma),
                                    &projection,
                                )?);
                            }
                        }
                        GpuPrimitiveTopology::TriangleList => {
                            for vertex in solid {
                                append_vertex(
                                    vertices,
                                    packed_solid_vertex(
                                        vertex.position,
                                        vertex.color,
                                        fragment_gamma_flag(scene.gamma_mode, *gamma),
                                        &projection,
                                    )?,
                                );
                            }
                        }
                    }
                    let end = vertex_count(vertices)?;
                    if start != end {
                        calls.push(DrawCall {
                            vertices: start..end,
                            scissor: projection.scissor,
                            blend: *blend,
                            kind: DrawKind::Solid {
                                alpha_mode: *alpha_mode,
                            },
                        });
                    }
                }
            }
        }
        Ok(())
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
        alpha_mode: GpuSolidAlphaMode,
    ) -> &wgpu::RenderPipeline {
        match blend {
            GpuBlend::Replace => &self.solid_replace_pipeline,
            GpuBlend::Normal => match alpha_mode {
                GpuSolidAlphaMode::SourceOver => &self.solid_over_normal_pipeline,
                GpuSolidAlphaMode::NonSeparate => &self.solid_non_separate_normal_pipeline,
            },
            // The CPU reference preserves destination alpha for every
            // additive producer, so one additive state serves both modes.
            GpuBlend::Additive => &self.solid_additive_pipeline,
        }
    }

    fn encode_draw_calls<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        calls: &'pass [DrawCall],
    ) {
        for call in calls {
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
                DrawKind::Solid { alpha_mode } => {
                    pass.set_pipeline(self.solid_pipeline(call.blend, alpha_mode));
                }
            }
            pass.draw(call.vertices.clone(), 0..1);
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
    projection: &DrawProjection,
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
        clip: clip_position(vertex.position, projection)?,
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
    projection: &DrawProjection,
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
        clip: clip_position(vertex.position, projection)?,
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
    projection: &DrawProjection,
) -> Result<PackedVertex, GpuRendererError> {
    Ok(PackedVertex {
        clip: clip_position(position, projection)?,
        uv: [0.0, 0.0],
        data0: color,
        data1: [flag(gamma), 0.0, 0.0, 0.0],
        data2: [0.0; 4],
    })
}

fn rounded_raster_width(projection: &DrawProjection) -> i64 {
    let maximum = projection.physical_extent.into_iter().max().unwrap_or(1);
    projection
        .line_width
        .round()
        .max(1.0)
        .min(maximum.max(1) as f32) as i64
}

fn floor_i64(value: f64) -> Result<i64, GpuRendererError> {
    if !value.is_finite() || value < i64::MIN as f64 || value >= i64::MAX as f64 {
        return Err(GpuRendererError::NonFiniteCoordinate);
    }
    Ok(value.floor() as i64)
}

fn packed_point_rect(
    point: GpuSolidVertex,
    gamma: bool,
    projection: &DrawProjection,
) -> Result<Option<[PackedVertex; 6]>, GpuRendererError> {
    let [logical_x, logical_y, logical_w] = point.position;
    if logical_w == 0.0 {
        return Err(GpuRendererError::NonFiniteCoordinate);
    }
    if logical_w < 0.0 {
        return Ok(None);
    }
    // Preserve the renderer's finite clip-coordinate validation even when
    // the point center will subsequently be clipped away.
    let _ = clip_position(point.position, projection)?;
    let center_x = f64::from(logical_x) / f64::from(logical_w);
    let center_y = f64::from(logical_y) / f64::from(logical_w);
    let logical_clip = projection.clipper.logical_clip();
    let clip_left = f64::from(logical_clip.x);
    let clip_top = f64::from(logical_clip.y);
    let clip_right = clip_left + f64::from(logical_clip.width);
    let clip_bottom = clip_top + f64::from(logical_clip.height);
    // GL clips the point vertex before applying PointSize. A wide point whose
    // center is outside the clip volume must not leak back through the scissor;
    // centers exactly on a clip plane remain inside.
    if center_x < clip_left
        || center_x > clip_right
        || center_y < clip_top
        || center_y > clip_bottom
    {
        return Ok(None);
    }
    let [x, top] = projected_physical_position(point.position, projection)?;
    let height = f64::from(projection.physical_extent[1]);
    let gl_y = height - top;
    let width = rounded_raster_width(projection);
    let half = width / 2;
    // OpenGL 2.1 section 3.3.1 centers odd point widths on the truncated
    // window coordinate and even widths on floor(window + 1/2). Work in GL's
    // bottom-up coordinates, then reflect the aligned rectangle once.
    let (gl_left, gl_bottom) = if width % 2 == 0 {
        (
            floor_i64(x + 0.5)?.saturating_sub(half),
            floor_i64(gl_y + 0.5)?.saturating_sub(half),
        )
    } else {
        (
            floor_i64(x)?.saturating_sub(half),
            floor_i64(gl_y)?.saturating_sub(half),
        )
    };
    let gl_right = gl_left.saturating_add(width);
    let gl_top = gl_bottom.saturating_add(width);
    let framebuffer_height = i64::from(projection.physical_extent[1]);
    let left = gl_left;
    let right = gl_right;
    let top = framebuffer_height.saturating_sub(gl_top);
    let bottom = framebuffer_height.saturating_sub(gl_bottom);
    let scissor_right = i64::from(projection.scissor.x) + i64::from(projection.scissor.width);
    let scissor_bottom = i64::from(projection.scissor.y) + i64::from(projection.scissor.height);
    if right <= i64::from(projection.scissor.x)
        || left >= scissor_right
        || bottom <= i64::from(projection.scissor.y)
        || top >= scissor_bottom
    {
        return Ok(None);
    }
    let positions = [
        [left as f64, top as f64],
        [right as f64, top as f64],
        [left as f64, bottom as f64],
        [left as f64, bottom as f64],
        [right as f64, top as f64],
        [right as f64, bottom as f64],
    ];
    Ok(Some([
        packed_solid_physical_vertex(positions[0], point.color, gamma, projection)?,
        packed_solid_physical_vertex(positions[1], point.color, gamma, projection)?,
        packed_solid_physical_vertex(positions[2], point.color, gamma, projection)?,
        packed_solid_physical_vertex(positions[3], point.color, gamma, projection)?,
        packed_solid_physical_vertex(positions[4], point.color, gamma, projection)?,
        packed_solid_physical_vertex(positions[5], point.color, gamma, projection)?,
    ]))
}

fn next_down(value: f64) -> f64 {
    if value.is_nan() || value == f64::NEG_INFINITY {
        return value;
    }
    if value == 0.0 {
        return -f64::from_bits(1);
    }
    let bits = value.to_bits();
    if value > 0.0 {
        f64::from_bits(bits - 1)
    } else {
        f64::from_bits(bits + 1)
    }
}

fn perturb_down(value: f64, amount: f64) -> f64 {
    let perturbed = value - amount;
    if perturbed < value {
        perturbed
    } else {
        next_down(value)
    }
}

fn l1_distance(point: [f64; 2], center: [f64; 2]) -> f64 {
    (point[0] - center[0]).abs() + (point[1] - center[1]).abs()
}

fn segment_diamond_distance(start: [f64; 2], end: [f64; 2], center: [f64; 2]) -> f64 {
    let delta = [end[0] - start[0], end[1] - start[1]];
    let mut minimum = l1_distance(start, center).min(l1_distance(end, center));
    for axis in 0..2 {
        if delta[axis] != 0.0 {
            let t = ((center[axis] - start[axis]) / delta[axis]).clamp(0.0, 1.0);
            let point = [start[0] + delta[0] * t, start[1] + delta[1] * t];
            minimum = minimum.min(l1_distance(point, center));
        }
    }
    minimum
}

fn clip_directed_line(
    start: [f64; 2],
    end: [f64; 2],
    bounds: [f64; 4],
) -> Option<([f64; 2], [f64; 2])> {
    let delta = [end[0] - start[0], end[1] - start[1]];
    let mut enter = 0.0_f64;
    let mut exit = 1.0_f64;
    for (p, q) in [
        (-delta[0], start[0] - bounds[0]),
        (delta[0], bounds[1] - start[0]),
        (-delta[1], start[1] - bounds[2]),
        (delta[1], bounds[3] - start[1]),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
            continue;
        }
        let ratio = q / p;
        if p < 0.0 {
            enter = enter.max(ratio);
        } else {
            exit = exit.min(ratio);
        }
        if enter > exit {
            return None;
        }
    }
    if enter >= exit {
        return None;
    }
    Some((
        [start[0] + delta[0] * enter, start[1] + delta[1] * enter],
        [start[0] + delta[0] * exit, start[1] + delta[1] * exit],
    ))
}

fn line_color_at_parameter(
    start: GpuSolidVertex,
    end: GpuSolidVertex,
    t: f64,
) -> Result<[f32; 4], GpuRendererError> {
    let start_w = f64::from(start.position[2]);
    let end_w = f64::from(end.position[2]);
    let denominator = (1.0 - t) / start_w + t / end_w;
    if !denominator.is_finite() || denominator == 0.0 {
        return Err(GpuRendererError::NonFiniteCoordinate);
    }
    let color = std::array::from_fn(|channel| {
        (((1.0 - t) * f64::from(start.color[channel]) / start_w
            + t * f64::from(end.color[channel]) / end_w)
            / denominator) as f32
    });
    color
        .iter()
        .all(|value| value.is_finite())
        .then_some(color)
        .ok_or(GpuRendererError::NonFiniteCoordinate)
}

fn walk_aliased_line_fragments(
    start: GpuSolidVertex,
    end: GpuSolidVertex,
    projection: &DrawProjection,
    mut emit: impl FnMut(i64, i64, f64) -> Result<(), GpuRendererError>,
) -> Result<u64, GpuRendererError> {
    let [start_x, start_top] = projected_physical_position(start.position, projection)?;
    let [end_x, end_top] = projected_physical_position(end.position, projection)?;
    let framebuffer_height = f64::from(projection.physical_extent[1]);
    let original_start = [start_x, framebuffer_height - start_top];
    let original_end = [end_x, framebuffer_height - end_top];
    let physical_clip = projection.clipper.physical_clip();
    let clip_left = f64::from(physical_clip.x);
    let clip_right = clip_left + f64::from(physical_clip.width);
    let clip_top = f64::from(physical_clip.y);
    let clip_bottom = clip_top + f64::from(physical_clip.height);
    let Some((clipped_start, clipped_end)) = clip_directed_line(
        original_start,
        original_end,
        [
            clip_left,
            clip_right,
            framebuffer_height - clip_bottom,
            framebuffer_height - clip_top,
        ],
    ) else {
        return Ok(0);
    };
    let delta = [
        clipped_end[0] - clipped_start[0],
        clipped_end[1] - clipped_start[1],
    ];
    if delta == [0.0, 0.0] {
        return Ok(0);
    }
    let x_major = delta[0].abs() >= delta[1].abs();
    let line_width = rounded_raster_width(projection);
    let minor_offset = (line_width - 1) as f64 * 0.5;
    let mut base_start = clipped_start;
    let mut base_end = clipped_end;
    let mut attribute_start = original_start;
    let mut attribute_end = original_end;
    let minor_axis = usize::from(x_major);
    base_start[minor_axis] -= minor_offset;
    base_end[minor_axis] -= minor_offset;
    attribute_start[minor_axis] -= minor_offset;
    attribute_end[minor_axis] -= minor_offset;

    // Section 3.4.1 defines the ideal tie break by translating both endpoints
    // by (-epsilon, -epsilon^2) in GL window coordinates. Inputs originate as
    // f32; this bias is below one f32 ulp at unit magnitude, while next_down
    // keeps the epsilon^2 term observable at large physical coordinates.
    const EPSILON: f64 = f32::EPSILON as f64 * 0.25;
    let epsilon_squared = EPSILON * EPSILON;
    let raster_start = [
        perturb_down(base_start[0], EPSILON),
        perturb_down(base_start[1], epsilon_squared),
    ];
    let raster_end = [
        perturb_down(base_end[0], EPSILON),
        perturb_down(base_end[1], epsilon_squared),
    ];
    let major_axis = usize::from(!x_major);
    let major_delta = raster_end[major_axis] - raster_start[major_axis];
    let (clip_start, clip_end) = if x_major {
        (
            i64::from(projection.scissor.x),
            i64::from(projection.scissor.x) + i64::from(projection.scissor.width),
        )
    } else {
        let height = i64::from(projection.physical_extent[1]);
        (
            height - i64::from(projection.scissor.y) - i64::from(projection.scissor.height),
            height - i64::from(projection.scissor.y),
        )
    };
    let segment_min = raster_start[major_axis].min(raster_end[major_axis]);
    let segment_max = raster_start[major_axis].max(raster_end[major_axis]);
    let first_major = (segment_min.floor() - 1.0)
        .max(clip_start as f64)
        .min(clip_end as f64) as i64;
    let end_major = (segment_max.ceil() + 1.0)
        .max(clip_start as f64)
        .min(clip_end as f64) as i64;
    if first_major >= end_major {
        return Ok(0);
    }

    let span = u64::try_from(end_major - first_major)
        .map_err(|_| GpuRendererError::VertexRangeOverflow)?;
    let mut fragment_count = 0_u64;
    for offset in 0..span {
        let offset = i64::try_from(offset).map_err(|_| GpuRendererError::VertexRangeOverflow)?;
        let major_pixel = if major_delta > 0.0 {
            first_major + offset
        } else {
            end_major - 1 - offset
        };
        let major_center = major_pixel as f64 + 0.5;
        let guess_t = ((major_center - raster_start[major_axis]) / major_delta).clamp(0.0, 1.0);
        let guess_minor = raster_start[minor_axis]
            + (raster_end[minor_axis] - raster_start[minor_axis]) * guess_t;
        let guess_pixel = guess_minor.floor() as i64;
        let mut base_fragment = None::<(i64, f64)>;
        for minor_pixel in guess_pixel.saturating_sub(2)..=guess_pixel.saturating_add(2) {
            let center = if x_major {
                [major_center, minor_pixel as f64 + 0.5]
            } else {
                [minor_pixel as f64 + 0.5, major_center]
            };
            if l1_distance(raster_end, center) < 0.5 {
                continue;
            }
            let distance = segment_diamond_distance(raster_start, raster_end, center);
            if distance < 0.5
                && base_fragment.is_none_or(|(_, best_distance)| distance < best_distance)
            {
                base_fragment = Some((minor_pixel, distance));
            }
        }
        let Some((base_minor, _)) = base_fragment else {
            continue;
        };
        let base_center = if x_major {
            [major_center, base_minor as f64 + 0.5]
        } else {
            [base_minor as f64 + 0.5, major_center]
        };
        let attribute_delta = [
            attribute_end[0] - attribute_start[0],
            attribute_end[1] - attribute_start[1],
        ];
        let denominator =
            attribute_delta[0] * attribute_delta[0] + attribute_delta[1] * attribute_delta[1];
        let t = ((base_center[0] - attribute_start[0]) * attribute_delta[0]
            + (base_center[1] - attribute_start[1]) * attribute_delta[1])
            / denominator;

        for width_offset in 0..line_width {
            let replicated_minor = base_minor.saturating_add(width_offset);
            let (x, gl_y) = if x_major {
                (major_pixel, replicated_minor)
            } else {
                (replicated_minor, major_pixel)
            };
            let y = i64::from(projection.physical_extent[1]) - 1 - gl_y;
            let scissor_right =
                i64::from(projection.scissor.x) + i64::from(projection.scissor.width);
            let scissor_bottom =
                i64::from(projection.scissor.y) + i64::from(projection.scissor.height);
            if x < i64::from(projection.scissor.x)
                || x >= scissor_right
                || y < i64::from(projection.scissor.y)
                || y >= scissor_bottom
            {
                continue;
            }
            fragment_count = fragment_count
                .checked_add(1)
                .ok_or(GpuRendererError::VertexRangeOverflow)?;
            emit(x, y, t)?;
        }
    }
    Ok(fragment_count)
}

fn packed_line_fragments(
    start: GpuSolidVertex,
    end: GpuSolidVertex,
    gamma: bool,
    projection: &DrawProjection,
) -> Result<Vec<PackedVertex>, GpuRendererError> {
    // OpenGL 2.1 section 3.4 rasterizes an aliased x-major line into at
    // most one fragment per physical column (one per row for y-major), omits
    // the directed final fragment, and implements a wide line by replicating
    // that base fragment in the minor direction. An oriented rectangle is
    // observably wrong: it is direction-invariant and can cover two pixels in
    // one major column on a diagonal. Generate the exact half-open fragment
    // stream, then lower each physical fragment to a 1x1 triangle pair.
    let mut packed = Vec::new();
    walk_aliased_line_fragments(start, end, projection, |x, y, t| {
        let color = line_color_at_parameter(start, end, t)?;
        let left = x as f64;
        let top = y as f64;
        for position in [
            [left, top],
            [left + 1.0, top],
            [left, top + 1.0],
            [left, top + 1.0],
            [left + 1.0, top],
            [left + 1.0, top + 1.0],
        ] {
            packed.push(packed_solid_physical_vertex(
                position, color, gamma, projection,
            )?);
        }
        Ok(())
    })?;
    Ok(packed)
}

fn projected_physical_position(
    position: [f32; 3],
    projection: &DrawProjection,
) -> Result<[f64; 2], GpuRendererError> {
    if !position.iter().all(|value| value.is_finite()) || position[2] == 0.0 {
        return Err(GpuRendererError::NonFiniteCoordinate);
    }
    let logical_x = f64::from(position[0] / position[2]);
    let logical_y = f64::from(position[1] / position[2]);
    let (physical_x, physical_y) = projection.clipper.logical_to_physical(logical_x, logical_y);
    [physical_x, physical_y]
        .iter()
        .all(|value| value.is_finite())
        .then_some([physical_x, physical_y])
        .ok_or(GpuRendererError::NonFiniteCoordinate)
}

fn packed_solid_physical_vertex(
    position: [f64; 2],
    color: [f32; 4],
    gamma: bool,
    projection: &DrawProjection,
) -> Result<PackedVertex, GpuRendererError> {
    if !position.iter().all(|value| value.is_finite())
        || !color.iter().all(|value| value.is_finite())
    {
        return Err(GpuRendererError::NonFiniteCoordinate);
    }
    let width = f64::from(projection.physical_extent[0]);
    let height = f64::from(projection.physical_extent[1]);
    let clip = [
        (2.0 * position[0] / width - 1.0) as f32,
        (1.0 - 2.0 * position[1] / height) as f32,
        0.0,
        1.0,
    ];
    if !clip.iter().all(|value| value.is_finite()) {
        return Err(GpuRendererError::NonFiniteCoordinate);
    }
    Ok(PackedVertex {
        clip,
        uv: [0.0, 0.0],
        data0: color,
        data1: [flag(gamma), 0.0, 0.0, 0.0],
        data2: [0.0; 4],
    })
}

fn clip_position(
    position: [f32; 3],
    projection: &DrawProjection,
) -> Result<[f32; 4], GpuRendererError> {
    if !position.iter().all(|value| value.is_finite()) {
        return Err(GpuRendererError::NonFiniteCoordinate);
    }
    let [x, y, w] = position;
    let logical = projection.clipper.logical_clip();
    let physical = projection.clipper.physical_clip();
    let (scale_x, scale_y) = projection.clipper.scale();
    let x = f64::from(x);
    let y = f64::from(y);
    let w = f64::from(w);

    // Preserve homogeneous W while applying the affine mapping installed by
    // gluOrtho2D over this command's logical clip. The rounded viewport extent
    // intentionally supplies independent X/Y scales.
    let physical_x = f64::from(physical.x) * w + (x - f64::from(logical.x) * w) * scale_x;
    let physical_y = f64::from(physical.y) * w + (y - f64::from(logical.y) * w) * scale_y;
    let width = f64::from(projection.physical_extent[0]);
    let height = f64::from(projection.physical_extent[1]);
    let clip64 = [
        2.0 * physical_x / width - w,
        w - 2.0 * physical_y / height,
        0.0,
        w,
    ];
    let clip = clip64.map(|value| value as f32);
    clip.iter()
        .all(|value| value.is_finite())
        .then_some(clip)
        .ok_or(GpuRendererError::NonFiniteCoordinate)
}

fn draw_projection(
    clip: Option<Rect>,
    logical_extent: [u32; 2],
    presentation: &GpuPresentation,
) -> Result<Option<DrawProjection>, GpuRendererError> {
    let logical_clip =
        clip.unwrap_or_else(|| Rect::new(0, 0, logical_extent[0], logical_extent[1]));
    let viewport_height = ((logical_extent[1] as f32) * presentation.scale)
        .ceil()
        .clamp(0.0, u32::MAX as f32) as u32;
    let projection_height = viewport_height.saturating_sub(presentation.crop_top);
    let clipper = ClipperProjection::new(
        presentation.scale,
        (logical_extent[0], logical_extent[1]),
        projection_height,
        logical_clip,
    );
    let Some(scissor) = physical_scissor(clipper.physical_clip(), presentation.physical_extent)
    else {
        return Ok(None);
    };
    Ok(Some(DrawProjection {
        clipper,
        physical_extent: presentation.physical_extent,
        line_width: presentation.scale,
        scissor,
    }))
}

fn physical_scissor(clip: Rect, extent: [u32; 2]) -> Option<Scissor> {
    let [width, height] = extent;
    let left = i64::from(clip.x).clamp(0, i64::from(width));
    let top = i64::from(clip.y).clamp(0, i64::from(height));
    let right = (i64::from(clip.x) + i64::from(clip.width)).clamp(0, i64::from(width));
    let bottom = (i64::from(clip.y) + i64::from(clip.height)).clamp(0, i64::from(height));
    if right <= left || bottom <= top {
        return None;
    }
    Some(Scissor {
        x: left as u32,
        y: top as u32,
        width: (right - left) as u32,
        height: (bottom - top) as u32,
    })
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

fn validate_scene(
    scene: &GpuScene,
    presentation: &GpuPresentation,
) -> Result<(), GpuRendererError> {
    validate_presentation(scene, presentation)?;

    let mut resources = HashMap::with_capacity(scene.textures.len());
    for resource in &scene.textures {
        if resources.insert(resource.id, resource).is_some() {
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
        if !resource.dirty.is_empty() && resource.base_revision == Some(resource.revision) {
            return Err(GpuRendererError::DirtyRevisionNotAdvanced {
                id: resource.id,
                revision: resource.revision,
            });
        }
        for &rect in &resource.dirty {
            validate_dirty(resource, rect)?;
        }
    }

    let mut packed_vertices = 0_u64;
    for command in &scene.commands {
        match command {
            GpuCommand::Quad {
                texture,
                owner_mask,
                vertices,
                clip,
                ..
            } => {
                if owner_mask.is_some() {
                    return Err(GpuRendererError::OwnerMaskNotLowered);
                }
                require_declared_format(&resources, *texture, GpuTextureFormat::Rgba8)?;
                let projection = draw_projection(*clip, scene.logical_extent, presentation)?;
                for vertex in vertices {
                    validate_gpu_vertex(vertex, projection.as_ref())?;
                }
                packed_vertices = packed_vertices.saturating_add(6);
            }
            GpuCommand::Landscape {
                base,
                liquid_mask,
                liquid,
                vertices,
                clip,
                phase,
                ..
            } => {
                require_declared_format(&resources, *base, GpuTextureFormat::Rgba8)?;
                if liquid_mask.is_some() != liquid.is_some() {
                    return Err(GpuRendererError::IncompleteLandscapeLiquid);
                }
                if let Some(mask) = liquid_mask {
                    require_declared_format(&resources, *mask, GpuTextureFormat::R8)?;
                }
                if let Some(liquid) = liquid {
                    require_declared_format(&resources, *liquid, GpuTextureFormat::Rgba8)?;
                }
                if !phase.iter().all(|value| value.is_finite()) {
                    return Err(GpuRendererError::NonFiniteCoordinate);
                }
                let projection = draw_projection(*clip, scene.logical_extent, presentation)?;
                for vertex in vertices {
                    validate_gpu_vertex(vertex, projection.as_ref())?;
                }
                packed_vertices = packed_vertices.saturating_add(6);
            }
            GpuCommand::Solid {
                vertices,
                topology,
                clip,
                ..
            } => {
                validate_primitive_count(*topology, vertices.len())?;
                let projection = draw_projection(*clip, scene.logical_extent, presentation)?;
                let mut expanded_line_vertices = 0_u64;
                for vertex in vertices {
                    if !vertex
                        .position
                        .iter()
                        .chain(vertex.color.iter())
                        .all(|value| value.is_finite())
                    {
                        return Err(GpuRendererError::NonFiniteCoordinate);
                    }
                    if *topology == GpuPrimitiveTopology::PointList {
                        if let Some(projection) = projection.as_ref() {
                            let _ = packed_point_rect(*vertex, false, projection)?;
                        }
                    } else if *topology == GpuPrimitiveTopology::TriangleList {
                        if let Some(projection) = projection.as_ref() {
                            let _ = clip_position(vertex.position, projection)?;
                        }
                    }
                }
                if *topology == GpuPrimitiveTopology::LineList {
                    for pair in vertices.chunks_exact(2) {
                        if pair[0].position[2] == 0.0 || pair[1].position[2] == 0.0 {
                            return Err(GpuRendererError::NonFiniteCoordinate);
                        }
                        if let Some(projection) = projection.as_ref() {
                            let fragment_count = walk_aliased_line_fragments(
                                pair[0],
                                pair[1],
                                projection,
                                |_, _, _| Ok(()),
                            )?;
                            expanded_line_vertices = expanded_line_vertices
                                .checked_add(
                                    fragment_count
                                        .checked_mul(6)
                                        .ok_or(GpuRendererError::VertexRangeOverflow)?,
                                )
                                .ok_or(GpuRendererError::VertexRangeOverflow)?;
                        }
                    }
                }
                let count = u64::try_from(vertices.len())
                    .map_err(|_| GpuRendererError::VertexRangeOverflow)?;
                let expanded = match topology {
                    GpuPrimitiveTopology::PointList => count.saturating_mul(6),
                    GpuPrimitiveTopology::LineList => expanded_line_vertices,
                    GpuPrimitiveTopology::TriangleList => count,
                };
                packed_vertices = packed_vertices.saturating_add(expanded);
            }
        }
        if packed_vertices > u64::from(u32::MAX) {
            return Err(GpuRendererError::VertexRangeOverflow);
        }
    }
    Ok(())
}

#[inline]
fn arc_slice_contents_equal<T: PartialEq>(left: &Arc<[T]>, right: &Arc<[T]>) -> bool {
    Arc::ptr_eq(left, right) || left.as_ref() == right.as_ref()
}

fn validate_layers(
    layers: &[GpuSceneLayer<'_>],
) -> Result<Vec<GpuTextureResource>, GpuRendererError> {
    let first = layers.first().ok_or(GpuRendererError::NoSceneLayers)?;
    let physical_extent = first.presentation.physical_extent;
    let gamma_mode = first.scene.gamma_mode;
    let gamma_revision = first.scene.gamma.revision;
    let gamma_channels = &first.scene.gamma.channels;
    let mut resources = HashMap::<GpuTextureId, GpuTextureResource>::new();

    for (index, layer) in layers.iter().enumerate() {
        validate_scene(layer.scene, &layer.presentation)?;
        if layer.presentation.physical_extent != physical_extent {
            return Err(GpuRendererError::LayerPhysicalExtentMismatch {
                layer: index,
                expected: physical_extent,
                actual: layer.presentation.physical_extent,
            });
        }
        if layer.scene.gamma_mode != gamma_mode
            || layer.scene.gamma.revision != gamma_revision
            || layer.scene.gamma.channels.as_ref() != gamma_channels.as_ref()
        {
            return Err(GpuRendererError::LayerGammaMismatch { layer: index });
        }

        for resource in &layer.scene.textures {
            match resources.entry(resource.id) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert(resource.clone());
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    let current = entry.get_mut();
                    if current.extent != resource.extent
                        || current.format != resource.format
                        || current.revision != resource.revision
                        || !arc_slice_contents_equal(&current.pixels, &resource.pixels)
                    {
                        return Err(GpuRendererError::LayerTextureConflict(resource.id));
                    }

                    // Texture id + revision is the producer's content identity.
                    // Preserve a usable delta when only one capture consumed it;
                    // incompatible deltas fall back to the complete backing.
                    match (current.dirty.is_empty(), resource.dirty.is_empty()) {
                        (true, false) => {
                            current.base_revision = resource.base_revision;
                            current.dirty.clone_from(&resource.dirty);
                        }
                        (false, false)
                            if current.base_revision != resource.base_revision
                                || current.dirty != resource.dirty =>
                        {
                            current.base_revision = None;
                            current.dirty.clear();
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    let mut resources = resources.into_values().collect::<Vec<_>>();
    resources.sort_by_key(|resource| resource.id);
    Ok(resources)
}

fn require_declared_format(
    resources: &HashMap<GpuTextureId, &GpuTextureResource>,
    id: GpuTextureId,
    expected: GpuTextureFormat,
) -> Result<(), GpuRendererError> {
    let resource = resources
        .get(&id)
        .ok_or(GpuRendererError::MissingTexture(id))?;
    if resource.format != expected {
        return Err(GpuRendererError::TextureFormatMismatch {
            id,
            expected,
            actual: resource.format,
        });
    }
    Ok(())
}

fn validate_gpu_vertex(
    vertex: &GpuVertex,
    projection: Option<&DrawProjection>,
) -> Result<(), GpuRendererError> {
    vertex
        .position
        .iter()
        .chain(vertex.uv.iter())
        .chain(vertex.modulation.iter())
        .chain(vertex.owner_modulation.iter())
        .chain(vertex.sample_tile.iter())
        .all(|value| value.is_finite())
        .then_some(())
        .ok_or(GpuRendererError::NonFiniteCoordinate)?;
    if let Some(projection) = projection {
        let _ = clip_position(vertex.position, projection)?;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TextureUploadPlan {
    Unchanged,
    Full,
    Dirty,
}

fn texture_upload_plan(
    cached_revision: Option<u64>,
    resource: &GpuTextureResource,
) -> TextureUploadPlan {
    let Some(cached_revision) = cached_revision else {
        return TextureUploadPlan::Full;
    };
    if cached_revision == resource.revision {
        return TextureUploadPlan::Unchanged;
    }
    if resource.dirty.is_empty()
        || resource.base_revision != Some(cached_revision)
        || dirty_upload_prefers_full(resource)
    {
        TextureUploadPlan::Full
    } else {
        TextureUploadPlan::Dirty
    }
}

fn validate_primitive_count(
    topology: GpuPrimitiveTopology,
    vertices: usize,
) -> Result<(), GpuRendererError> {
    let valid = match topology {
        GpuPrimitiveTopology::TriangleList => vertices.is_multiple_of(3),
        GpuPrimitiveTopology::LineList => vertices.is_multiple_of(2),
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
    alpha_mode: GpuSolidAlphaMode,
) -> wgpu::RenderPipeline {
    let vertex_layouts = [packed_vertex_layout()];
    let targets = [Some(wgpu::ColorTargetState {
        format: wgpu::TextureFormat::Rgba8Unorm,
        blend: Some(blend_state(blend, alpha_mode)),
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

/// wgpu translation of the deterministic CPU-reference blend equations.
///
/// Native GL never reads framebuffer alpha back (no `GL_DST_ALPHA` factor in
/// CStdGL), so the CPU oracle's destination-alpha conventions are
/// authoritative: normal primitive draws keep source-over alpha, sampled
/// fragment recovery shares the non-separate colour factors, and additive
/// draws preserve destination alpha entirely.
fn blend_state(blend: GpuBlend, alpha_mode: GpuSolidAlphaMode) -> wgpu::BlendState {
    match blend {
        GpuBlend::Replace => wgpu::BlendState::REPLACE,
        GpuBlend::Normal => match alpha_mode {
            GpuSolidAlphaMode::SourceOver => wgpu::BlendState::ALPHA_BLENDING,
            GpuSolidAlphaMode::NonSeparate => {
                let component = wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::SrcAlpha,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                };
                wgpu::BlendState {
                    color: component,
                    alpha: component,
                }
            }
        },
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
    use clonk_graphics::{
        Color, GammaRamp, GpuGammaLut, GpuSolidVertex, GpuTextureResource, PixelFormat, Surface,
    };
    use clonk_gui::{ImageData, Rect as GuiRect};
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
    fn source_over_normal_blend_matches_cpu_reference_alpha() {
        let blend = blend_state(GpuBlend::Normal, GpuSolidAlphaMode::SourceOver);

        assert_eq!(blend, wgpu::BlendState::ALPHA_BLENDING);
        assert_eq!(blend.color.src_factor, wgpu::BlendFactor::SrcAlpha);
        assert_eq!(blend.color.dst_factor, wgpu::BlendFactor::OneMinusSrcAlpha);
        assert_eq!(blend.alpha.src_factor, wgpu::BlendFactor::One);
        assert_eq!(blend.alpha.dst_factor, wgpu::BlendFactor::OneMinusSrcAlpha);
    }

    #[test]
    fn non_separate_normal_blend_shares_color_factors_with_alpha() {
        let blend = blend_state(GpuBlend::Normal, GpuSolidAlphaMode::NonSeparate);

        assert_eq!(blend.color, blend.alpha);
        assert_eq!(blend.color.src_factor, wgpu::BlendFactor::SrcAlpha);
        assert_eq!(blend.color.dst_factor, wgpu::BlendFactor::OneMinusSrcAlpha);
        assert_eq!(blend.color.operation, wgpu::BlendOperation::Add);
    }

    #[test]
    fn additive_blend_preserves_destination_alpha_for_both_modes() {
        for alpha_mode in [
            GpuSolidAlphaMode::SourceOver,
            GpuSolidAlphaMode::NonSeparate,
        ] {
            let blend = blend_state(GpuBlend::Additive, alpha_mode);

            assert_eq!(blend.color.src_factor, wgpu::BlendFactor::SrcAlpha);
            assert_eq!(blend.color.dst_factor, wgpu::BlendFactor::One);
            assert_eq!(blend.color.operation, wgpu::BlendOperation::Add);
            assert_eq!(blend.alpha.src_factor, wgpu::BlendFactor::Zero);
            assert_eq!(blend.alpha.dst_factor, wgpu::BlendFactor::One);
            assert_eq!(blend.alpha.operation, wgpu::BlendOperation::Add);
        }
    }

    #[test]
    fn device_health_distinguishes_recoverable_loss_from_fatal_validation() {
        let lost = classify_wgpu_validation_description(
            "Queue::submit failed because the Parent device is lost",
        );
        assert!(matches!(
            lost,
            RetainedGpuRendererHealth::RecreateRequired {
                reason: RetainedGpuRecreateReason::DeviceLost,
                ..
            }
        ));

        let validation = classify_wgpu_validation_description("invalid bind group layout");
        assert!(matches!(
            validation,
            RetainedGpuRendererHealth::Fatal {
                reason: RetainedGpuFatalReason::Validation,
                ..
            }
        ));
    }

    #[test]
    fn fatal_device_health_supersedes_a_pending_recreation() {
        let state = Mutex::new(RetainedGpuRendererHealth::Healthy);
        record_renderer_health(
            &state,
            RetainedGpuRendererHealth::RecreateRequired {
                reason: RetainedGpuRecreateReason::DeviceLost,
                detail: "lost".to_owned(),
            },
        );
        record_renderer_health(
            &state,
            RetainedGpuRendererHealth::Fatal {
                reason: RetainedGpuFatalReason::OutOfMemory,
                detail: "oom".to_owned(),
            },
        );
        record_renderer_health(
            &state,
            RetainedGpuRendererHealth::RecreateRequired {
                reason: RetainedGpuRecreateReason::DeviceLost,
                detail: "later loss".to_owned(),
            },
        );

        assert!(matches!(
            &*state.lock().expect("health state"),
            RetainedGpuRendererHealth::Fatal {
                reason: RetainedGpuFatalReason::OutOfMemory,
                ..
            }
        ));
    }

    #[test]
    fn fractional_clipper_rounds_viewport_then_projects_relative_coordinates() {
        let presentation = GpuPresentation {
            physical_extent: [5, 4],
            scale: 1.5,
            crop_top: 1,
        };
        let projection = draw_projection(Some(Rect::new(1, 1, 2, 1)), [4, 3], &presentation)
            .expect("valid fractional presentation")
            .expect("clip intersects the physical framebuffer");

        assert_eq!(projection.clipper.logical_clip(), Rect::new(1, 1, 2, 1));
        assert_eq!(projection.clipper.physical_clip(), Rect::new(1, 1, 3, 2));
        assert_eq!(
            projection.scissor,
            Scissor {
                x: 1,
                y: 1,
                width: 3,
                height: 2,
            }
        );
        assert_eq!(
            clip_position([2.0, 1.0, 1.0], &projection).unwrap(),
            [0.0, 0.5, 0.0, 1.0]
        );
        assert_eq!(
            clip_position([4.0, 2.0, 2.0], &projection).unwrap(),
            [0.0, 1.0, 0.0, 2.0],
            "homogeneous W must preserve the same Euclidean coordinate"
        );
    }

    #[test]
    fn wide_point_is_center_clipped_before_physical_rasterization() {
        let presentation = GpuPresentation {
            physical_extent: [8, 8],
            scale: 2.0,
            crop_top: 0,
        };
        let projection = draw_projection(Some(Rect::new(0, 0, 2, 2)), [4, 4], &presentation)
            .expect("valid point presentation")
            .expect("point clip intersects the framebuffer");
        let color = [1.0; 4];

        assert!(
            packed_point_rect(solid_vertex(0.0, 1.0, color), false, &projection)
                .expect("left clip-plane point")
                .is_some()
        );
        assert!(
            packed_point_rect(solid_vertex(2.0, 1.0, color), false, &projection)
                .expect("right clip-plane point")
                .is_some()
        );
        assert!(
            packed_point_rect(solid_vertex(-0.001, 1.0, color), false, &projection)
                .expect("outside left point")
                .is_none()
        );
        assert!(
            packed_point_rect(solid_vertex(2.001, 1.0, color), false, &projection)
                .expect("outside right point")
                .is_none()
        );
    }

    #[test]
    fn logical_line_pair_expands_to_cpp_application_scale_in_physical_space() {
        let presentation = GpuPresentation {
            physical_extent: [12, 8],
            scale: 2.0,
            crop_top: 0,
        };
        let projection = draw_projection(None, [6, 4], &presentation)
            .expect("valid line presentation")
            .expect("line clip intersects the framebuffer");
        let color = [1.0, 0.0, 0.0, 1.0];
        let expanded = packed_line_fragments(
            solid_vertex(1.5, 1.5, color),
            solid_vertex(4.5, 1.5, color),
            false,
            &projection,
        )
        .expect("expand line pair");
        let physical = |vertex: PackedVertex| {
            [
                ((f64::from(vertex.clip[0]) + 1.0) * 6.0).round(),
                ((1.0 - f64::from(vertex.clip[1])) * 4.0).round(),
            ]
        };

        let mut origins = expanded
            .chunks_exact(6)
            .map(|fragment| physical(fragment[0]))
            .collect::<Vec<_>>();
        origins.sort_by(|left, right| left.partial_cmp(right).expect("finite physical origin"));
        let mut expected = (2..8)
            .flat_map(|x| (2..4).map(move |y| [f64::from(x), f64::from(y)]))
            .collect::<Vec<_>>();
        expected.sort_by(|left, right| left.partial_cmp(right).expect("finite expected origin"));
        assert_eq!(origins, expected);
        assert_eq!(expanded.len(), 6 * 2 * 6);
        assert!(expanded.iter().all(|vertex| vertex.data0 == color));
    }

    #[test]
    fn diagonal_line_color_uses_cpp_window_space_projection_parameter() {
        let presentation = GpuPresentation::identity(5, 4);
        let projection = draw_projection(None, [5, 4], &presentation)
            .expect("valid line presentation")
            .expect("line clip intersects the framebuffer");
        let expanded = packed_line_fragments(
            solid_vertex(0.5, 0.5, [0.0, 0.0, 0.0, 1.0]),
            solid_vertex(4.5, 2.5, [1.0, 0.0, 0.0, 1.0]),
            false,
            &projection,
        )
        .expect("expand diagonal line");
        let physical = |vertex: PackedVertex| {
            [
                ((f64::from(vertex.clip[0]) + 1.0) * 2.5).round(),
                ((1.0 - f64::from(vertex.clip[1])) * 2.0).round(),
            ]
        };
        let fragment = expanded
            .chunks_exact(6)
            .find(|fragment| physical(fragment[0]) == [1.0, 1.0])
            .expect("slope-one-half line covers physical pixel (1,1)");
        assert!((fragment[0].data0[0] - 0.3).abs() < 1.0e-6);
    }

    #[test]
    fn line_clipping_preserves_directed_entry_and_exit_endpoints() {
        let presentation = GpuPresentation::identity(6, 4);
        let projection = draw_projection(Some(Rect::new(2, 0, 2, 4)), [6, 4], &presentation)
            .expect("valid clipped-line presentation")
            .expect("line clip intersects the framebuffer");
        let color = [1.0; 4];
        let collect = |start, end| {
            let mut fragments = Vec::new();
            walk_aliased_line_fragments(
                solid_vertex(start, 1.5, color),
                solid_vertex(end, 1.5, color),
                &projection,
                |x, y, _| {
                    fragments.push((x, y));
                    Ok(())
                },
            )
            .expect("walk clipped line");
            fragments
        };

        assert_eq!(collect(0.5, 5.5), vec![(2, 1)]);
        assert_eq!(collect(5.5, 0.5), vec![(3, 1), (2, 1)]);
        assert!(collect(0.5, 2.0).is_empty(), "clip-point degeneracy");
    }

    #[test]
    fn disabled_gamma_mode_clears_requested_fragment_gamma_flag() {
        assert!(fragment_gamma_flag(GpuGammaMode::Fragment, true));
        assert!(!fragment_gamma_flag(GpuGammaMode::Disabled, true));
        assert!(!fragment_gamma_flag(GpuGammaMode::Monitor, true));
    }

    #[test]
    fn recovery_validation_requires_every_command_texture_in_the_current_scene() {
        let texture = GpuTextureId::fresh();
        let identity = [1.0, 1.0, 1.0, 0.0];
        let mut scene = GpuScene {
            logical_extent: [2, 2],
            clear: Color::transparent(),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
            gamma_mode: GpuGammaMode::Fragment,
            textures: Vec::new(),
            commands: vec![GpuCommand::Quad {
                texture,
                owner_mask: None,
                vertices: quad(0.0, 0.0, 2.0, 2.0, 1.0, identity),
                clip: None,
                blend: GpuBlend::Normal,
                base_mod2: false,
                owner_mod2: false,
                sampler: GpuSampler::Nearest,
                gamma: false,
            }],
        };
        assert!(matches!(
            RetainedGpuRenderer::validate_scene(&scene, &GpuPresentation::identity(2, 2)),
            Err(GpuRendererError::MissingTexture(id)) if id == texture
        ));

        scene
            .textures
            .push(rgba_resource(texture, [10, 20, 30, 255]));
        assert!(
            RetainedGpuRenderer::validate_scene(&scene, &GpuPresentation::identity(2, 2)).is_ok()
        );
    }

    #[test]
    fn recovery_validation_checks_all_deltas_before_gpu_mutation() {
        let id = GpuTextureId::fresh();
        let mut resource = rgba_resource_2x1(id, [0, 0, 0, 255], [1, 1, 1, 255]);
        resource.revision = 4;
        resource.base_revision = Some(4);
        resource.dirty = vec![Rect::new(0, 0, 1, 1)];
        let mut scene = GpuScene {
            logical_extent: [2, 1],
            clear: Color::transparent(),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
            gamma_mode: GpuGammaMode::Fragment,
            textures: vec![resource],
            commands: Vec::new(),
        };
        assert!(matches!(
            RetainedGpuRenderer::validate_scene(&scene, &GpuPresentation::identity(2, 1)),
            Err(GpuRendererError::DirtyRevisionNotAdvanced {
                id: invalid,
                revision: 4
            }) if invalid == id
        ));

        scene.textures[0].base_revision = Some(3);
        scene.textures[0].dirty = vec![Rect::new(2, 0, 1, 1)];
        assert!(matches!(
            RetainedGpuRenderer::validate_scene(&scene, &GpuPresentation::identity(2, 1)),
            Err(GpuRendererError::InvalidDirtyRect { id: invalid, .. }) if invalid == id
        ));
    }

    #[test]
    fn recovery_validation_rejects_projection_overflow_before_gpu_mutation() {
        let scene = GpuScene {
            logical_extent: [2, 2],
            clear: Color::transparent(),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
            gamma_mode: GpuGammaMode::Fragment,
            textures: Vec::new(),
            commands: vec![GpuCommand::Solid {
                vertices: vec![GpuSolidVertex {
                    position: [f32::MAX, 0.5, 1.0],
                    color: [1.0, 1.0, 1.0, 1.0],
                    outer_modulation: clonk_graphics::GpuSolidOuterModulation::PackedC4,
                }],
                topology: GpuPrimitiveTopology::PointList,
                alpha_mode: GpuSolidAlphaMode::SourceOver,
                clip: None,
                blend: GpuBlend::Replace,
                gamma: false,
            }],
        };
        assert!(matches!(
            RetainedGpuRenderer::validate_scene(
                &scene,
                &GpuPresentation {
                    physical_extent: [2, 2],
                    scale: 2.0,
                    crop_top: 0,
                }
            ),
            Err(GpuRendererError::NonFiniteCoordinate)
        ));
    }

    #[test]
    fn layered_validation_rejects_conflicting_complete_texture_backing() {
        let id = GpuTextureId::fresh();
        let scene = |pixel| GpuScene {
            logical_extent: [1, 1],
            clear: Color::transparent(),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
            gamma_mode: GpuGammaMode::Fragment,
            textures: vec![rgba_resource(id, pixel)],
            commands: Vec::new(),
        };
        let first = scene([1, 2, 3, 255]);
        let second = scene([4, 5, 6, 255]);
        let presentation = GpuPresentation::identity(1, 1);
        assert!(matches!(
            RetainedGpuRenderer::validate_layers(&[
                GpuSceneLayer::new(&first, presentation),
                GpuSceneLayer::new(&second, presentation),
            ]),
            Err(GpuRendererError::LayerTextureConflict(conflict)) if conflict == id
        ));
    }

    #[test]
    fn shared_arc_backing_skips_content_comparison() {
        #[derive(Debug)]
        struct ComparisonMustNotRun;

        impl PartialEq for ComparisonMustNotRun {
            fn eq(&self, _other: &Self) -> bool {
                panic!("shared Arc backing must bypass element comparison");
            }
        }

        let shared: Arc<[ComparisonMustNotRun]> =
            Arc::from(vec![ComparisonMustNotRun].into_boxed_slice());
        assert!(arc_slice_contents_equal(&shared, &shared));

        let first: Arc<[u8]> = Arc::from([1, 2, 3, 4]);
        let equal: Arc<[u8]> = Arc::from([1, 2, 3, 4]);
        let different: Arc<[u8]> = Arc::from([1, 2, 3, 5]);
        assert!(arc_slice_contents_equal(&first, &equal));
        assert!(!arc_slice_contents_equal(&first, &different));
    }

    #[test]
    fn mode_and_device_generation_gaps_choose_safe_texture_uploads() {
        let id = GpuTextureId::fresh();
        let mut resource = rgba_resource_2x1(id, [1, 2, 3, 255], [4, 5, 6, 255]);
        resource.revision = 1;
        resource.base_revision = Some(0);
        resource.dirty = vec![Rect::new(1, 0, 1, 1)];

        assert_eq!(
            texture_upload_plan(None, &resource),
            TextureUploadPlan::Full,
            "a replacement device has no retained texture to patch"
        );
        assert_eq!(
            texture_upload_plan(Some(0), &resource),
            TextureUploadPlan::Dirty
        );
        assert_eq!(
            texture_upload_plan(Some(1), &resource),
            TextureUploadPlan::Unchanged
        );

        // Mode transitions may suppress presentation for several producer
        // revisions. A delta based on revision 2 cannot patch cached revision
        // 1, but its complete backing remains a safe full upload.
        resource.revision = 3;
        resource.base_revision = Some(2);
        assert_eq!(
            texture_upload_plan(Some(1), &resource),
            TextureUploadPlan::Full
        );
        assert_eq!(
            texture_upload_plan(Some(2), &resource),
            TextureUploadPlan::Dirty
        );
    }

    #[test]
    fn layered_presentations_preserve_physical_painter_order() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build Tokio runtime for layered renderer test");
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
            .expect("layered renderer test requires a working wgpu adapter");
        let descriptor = wgpu::DeviceDescriptor {
            label: Some("lc_gpu_layered_test_device"),
            features: wgpu::Features::empty(),
            limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        };
        let (device, queue) = runtime
            .block_on(adapter.request_device(&descriptor, None))
            .expect("request layered renderer test device");
        device.push_error_scope(wgpu::ErrorFilter::Validation);

        let gamma = GpuGammaLut::from_ramp(&GammaRamp::standard());
        let base = GpuScene {
            logical_extent: [4, 3],
            clear: Color::opaque(10, 20, 30),
            gamma: gamma.clone(),
            gamma_mode: GpuGammaMode::Disabled,
            textures: Vec::new(),
            commands: vec![GpuCommand::Solid {
                vertices: vec![solid_vertex(2.5, 1.5, rgba_f32(POINT))],
                topology: GpuPrimitiveTopology::PointList,
                alpha_mode: GpuSolidAlphaMode::SourceOver,
                clip: None,
                blend: GpuBlend::Replace,
                gamma: false,
            }],
        };
        let physical_text = GpuScene {
            logical_extent: [8, 6],
            clear: Color::transparent(),
            gamma,
            gamma_mode: GpuGammaMode::Disabled,
            textures: Vec::new(),
            commands: vec![GpuCommand::Solid {
                vertices: vec![solid_vertex(5.5, 2.5, rgba_f32(MAGENTA))],
                topology: GpuPrimitiveTopology::PointList,
                alpha_mode: GpuSolidAlphaMode::SourceOver,
                clip: None,
                blend: GpuBlend::Replace,
                gamma: false,
            }],
        };
        let physical_extent = [8, 6];
        let layers = [
            GpuSceneLayer::new(
                &base,
                GpuPresentation {
                    physical_extent,
                    scale: 2.0,
                    crop_top: 0,
                },
            ),
            GpuSceneLayer::new(
                &physical_text,
                GpuPresentation::identity(physical_extent[0], physical_extent[1]),
            ),
        ];
        let mut renderer =
            RetainedGpuRenderer::new(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        assert_eq!(renderer.health(), RetainedGpuRendererHealth::Healthy);
        let frame = render_layers_readback(&mut renderer, &device, &queue, &layers);

        assert_eq!(
            readback_pixel(&frame, 4, 2),
            POINT,
            "the scaled logical point covers a 2x2 physical block"
        );
        assert_eq!(
            readback_pixel(&frame, 5, 2),
            MAGENTA,
            "the later identity-space layer paints one native pixel over it"
        );
        assert_eq!(readback_pixel(&frame, 6, 2), [10, 20, 30, 255]);
        let validation = runtime.block_on(device.pop_error_scope());
        assert!(
            validation.is_none(),
            "layered renderer reported wgpu validation error: {validation:?}"
        );
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

        let point_scene = GpuScene {
            logical_extent: LOGICAL,
            clear: Color::new(CLEAR[0], CLEAR[1], CLEAR[2], CLEAR[3]),
            gamma: scene.gamma.clone(),
            gamma_mode: GpuGammaMode::Disabled,
            textures: Vec::new(),
            commands: vec![scene.commands.last().expect("point command").clone()],
        };
        for (label, presentation, x_range, y_range) in [
            (
                "scale-1.5",
                GpuPresentation {
                    physical_extent: [12, 9],
                    scale: 1.5,
                    crop_top: 0,
                },
                9..11,
                6..8,
            ),
            (
                "scale-2",
                GpuPresentation {
                    physical_extent: [16, 12],
                    scale: 2.0,
                    crop_top: 0,
                },
                12..14,
                8..10,
            ),
        ] {
            let point =
                render_readback(&mut renderer, &device, &queue, &point_scene, &presentation);
            for y in 0..presentation.physical_extent[1] {
                for x in 0..presentation.physical_extent[0] {
                    let expected = if x_range.contains(&x) && y_range.contains(&y) {
                        POINT
                    } else {
                        CLEAR
                    };
                    assert_eq!(
                        readback_pixel(&point, x, y),
                        expected,
                        "{label} even-width GL point footprint ({x}, {y})"
                    );
                }
            }
        }

        let line_scene = GpuScene {
            logical_extent: [6, 4],
            clear: Color::new(CLEAR[0], CLEAR[1], CLEAR[2], CLEAR[3]),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
            gamma_mode: GpuGammaMode::Disabled,
            textures: Vec::new(),
            commands: vec![GpuCommand::Solid {
                // DrawLineDw adds 0.5 before GL_LINES and installs
                // glLineWidth(Application.GetScale()).
                vertices: vec![
                    solid_vertex(1.5, 1.5, rgba_f32(SOLID)),
                    solid_vertex(4.5, 1.5, rgba_f32(SOLID)),
                ],
                topology: GpuPrimitiveTopology::LineList,
                alpha_mode: GpuSolidAlphaMode::SourceOver,
                clip: None,
                blend: GpuBlend::Replace,
                gamma: false,
            }],
        };
        let scaled_line = render_readback(
            &mut renderer,
            &device,
            &queue,
            &line_scene,
            &GpuPresentation {
                physical_extent: [12, 8],
                scale: 2.0,
                crop_top: 0,
            },
        );
        for y in 0..8 {
            for x in 0..12 {
                let expected = if (2..8).contains(&x) && (2..4).contains(&y) {
                    SOLID
                } else {
                    CLEAR
                };
                assert_eq!(
                    readback_pixel(&scaled_line, x, y),
                    expected,
                    "scale-two C++ line footprint ({x}, {y})"
                );
            }
        }

        let mut reverse_line_scene = line_scene.clone();
        let GpuCommand::Solid { vertices, .. } = &mut reverse_line_scene.commands[0] else {
            unreachable!("line fixture is solid");
        };
        vertices.reverse();
        let scaled_reverse_line = render_readback(
            &mut renderer,
            &device,
            &queue,
            &reverse_line_scene,
            &GpuPresentation {
                physical_extent: [12, 8],
                scale: 2.0,
                crop_top: 0,
            },
        );
        for y in 0..8 {
            for x in 0..12 {
                let expected = if (3..9).contains(&x) && (2..4).contains(&y) {
                    SOLID
                } else {
                    CLEAR
                };
                assert_eq!(
                    readback_pixel(&scaled_reverse_line, x, y),
                    expected,
                    "reverse scale-two C++ line footprint ({x}, {y})"
                );
            }
        }

        let diagonal_line_scene = GpuScene {
            logical_extent: [5, 4],
            clear: Color::new(CLEAR[0], CLEAR[1], CLEAR[2], CLEAR[3]),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
            gamma_mode: GpuGammaMode::Disabled,
            textures: Vec::new(),
            commands: vec![GpuCommand::Solid {
                vertices: vec![
                    solid_vertex(0.5, 0.5, rgba_f32(SOLID)),
                    solid_vertex(4.5, 2.5, rgba_f32(SOLID)),
                ],
                topology: GpuPrimitiveTopology::LineList,
                alpha_mode: GpuSolidAlphaMode::SourceOver,
                clip: None,
                blend: GpuBlend::Replace,
                gamma: false,
            }],
        };
        let negative_diagonal_line_scene = GpuScene {
            commands: vec![GpuCommand::Solid {
                vertices: vec![
                    solid_vertex(0.5, 2.5, rgba_f32(SOLID)),
                    solid_vertex(4.5, 0.5, rgba_f32(SOLID)),
                ],
                topology: GpuPrimitiveTopology::LineList,
                alpha_mode: GpuSolidAlphaMode::SourceOver,
                clip: None,
                blend: GpuBlend::Replace,
                gamma: false,
            }],
            ..diagonal_line_scene.clone()
        };
        for (label, scene, expected_pixels) in [
            (
                "forward",
                diagonal_line_scene.clone(),
                [(0, 0), (1, 1), (2, 1), (3, 2)],
            ),
            (
                "reverse",
                {
                    let mut scene = diagonal_line_scene.clone();
                    let GpuCommand::Solid { vertices, .. } = &mut scene.commands[0] else {
                        unreachable!("diagonal fixture is solid");
                    };
                    vertices.reverse();
                    scene
                },
                [(4, 2), (3, 2), (2, 1), (1, 1)],
            ),
            (
                "negative forward",
                negative_diagonal_line_scene.clone(),
                [(0, 2), (1, 1), (2, 1), (3, 0)],
            ),
            (
                "negative reverse",
                {
                    let mut scene = negative_diagonal_line_scene;
                    let GpuCommand::Solid { vertices, .. } = &mut scene.commands[0] else {
                        unreachable!("negative diagonal fixture is solid");
                    };
                    vertices.reverse();
                    scene
                },
                [(4, 0), (3, 0), (2, 1), (1, 1)],
            ),
        ] {
            let diagonal = render_readback(
                &mut renderer,
                &device,
                &queue,
                &scene,
                &GpuPresentation::identity(5, 4),
            );
            for y in 0..4 {
                for x in 0..5 {
                    let expected = if expected_pixels.contains(&(x, y)) {
                        SOLID
                    } else {
                        CLEAR
                    };
                    assert_eq!(
                        readback_pixel(&diagonal, x, y),
                        expected,
                        "{label} slope-one-half C++ line footprint ({x}, {y})"
                    );
                }
            }
        }

        let frame_scene = GpuScene {
            logical_extent: [6, 6],
            clear: Color::transparent(),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
            gamma_mode: GpuGammaMode::Disabled,
            textures: Vec::new(),
            commands: vec![GpuCommand::Solid {
                vertices: [
                    (1.5, 1.5),
                    (4.5, 1.5),
                    (4.5, 1.5),
                    (4.5, 4.5),
                    (4.5, 4.5),
                    (1.5, 4.5),
                    (1.5, 4.5),
                    (1.5, 1.5),
                ]
                .into_iter()
                .map(|(x, y)| solid_vertex(x, y, rgba_f32([255, 0, 0, 128])))
                .collect(),
                topology: GpuPrimitiveTopology::LineList,
                alpha_mode: GpuSolidAlphaMode::SourceOver,
                clip: None,
                blend: GpuBlend::Normal,
                gamma: false,
            }],
        };
        let frame = render_readback(
            &mut renderer,
            &device,
            &queue,
            &frame_scene,
            &GpuPresentation::identity(6, 6),
        );
        for y in 0..6 {
            for x in 0..6 {
                let on_frame = (1..=4).contains(&x)
                    && (1..=4).contains(&y)
                    && (x == 1 || x == 4 || y == 1 || y == 4);
                assert_eq!(
                    readback_pixel(&frame, x, y),
                    if on_frame { [128, 0, 0, 128] } else { [0; 4] },
                    "directed DrawFrameDw corner ownership ({x}, {y})"
                );
            }
        }

        let translucent_point_scene = GpuScene {
            logical_extent: [1, 1],
            clear: Color::new(0, 0, 0, 128),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
            gamma_mode: GpuGammaMode::Disabled,
            textures: Vec::new(),
            commands: vec![GpuCommand::Solid {
                vertices: vec![solid_vertex(0.5, 0.5, rgba_f32([200, 100, 50, 64]))],
                topology: GpuPrimitiveTopology::PointList,
                alpha_mode: GpuSolidAlphaMode::SourceOver,
                clip: None,
                blend: GpuBlend::Normal,
                gamma: false,
            }],
        };
        let translucent_point = render_readback(
            &mut renderer,
            &device,
            &queue,
            &translucent_point_scene,
            &GpuPresentation::identity(1, 1),
        );
        assert_eq!(
            readback_pixel(&translucent_point, 0, 0),
            [50, 25, 13, 160],
            "translucent points keep the CPU reference's source-over alpha"
        );

        let additive_scene = GpuScene {
            logical_extent: [1, 1],
            clear: Color::new(0, 0, 0, 192),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
            gamma_mode: GpuGammaMode::Disabled,
            textures: Vec::new(),
            commands: vec![GpuCommand::Solid {
                vertices: vec![solid_vertex(0.5, 0.5, rgba_f32([200, 100, 50, 64]))],
                topology: GpuPrimitiveTopology::PointList,
                alpha_mode: GpuSolidAlphaMode::SourceOver,
                clip: None,
                blend: GpuBlend::Additive,
                gamma: false,
            }],
        };
        let additive = render_readback(
            &mut renderer,
            &device,
            &queue,
            &additive_scene,
            &GpuPresentation::identity(1, 1),
        );
        assert_eq!(
            readback_pixel(&additive, 0, 0),
            [50, 25, 13, 192],
            "additive points preserve destination alpha like the CPU reference"
        );

        let additive_filled_scene = GpuScene {
            logical_extent: [1, 1],
            clear: Color::new(0, 0, 0, 192),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
            gamma_mode: GpuGammaMode::Disabled,
            textures: Vec::new(),
            commands: vec![GpuCommand::Solid {
                vertices: vec![
                    solid_vertex(0.0, 0.0, rgba_f32([200, 100, 50, 64])),
                    solid_vertex(1.0, 0.0, rgba_f32([200, 100, 50, 64])),
                    solid_vertex(0.0, 1.0, rgba_f32([200, 100, 50, 64])),
                    solid_vertex(0.0, 1.0, rgba_f32([200, 100, 50, 64])),
                    solid_vertex(1.0, 0.0, rgba_f32([200, 100, 50, 64])),
                    solid_vertex(1.0, 1.0, rgba_f32([200, 100, 50, 64])),
                ],
                topology: GpuPrimitiveTopology::TriangleList,
                alpha_mode: GpuSolidAlphaMode::SourceOver,
                clip: None,
                blend: GpuBlend::Additive,
                gamma: false,
            }],
        };
        let additive_filled = render_readback(
            &mut renderer,
            &device,
            &queue,
            &additive_filled_scene,
            &GpuPresentation::identity(1, 1),
        );
        assert_eq!(
            readback_pixel(&additive_filled, 0, 0),
            [50, 25, 13, 192],
            "filled additive draws preserve destination alpha like the CPU reference"
        );

        // CStdGL rounds this command's 2x1 logical clip to a 3x2 viewport,
        // then projects relative to that rounded viewport. Absolute x*scale
        // projection would start half a pixel later and miss this footprint.
        // The translucent draw also pins the source-over destination alpha.
        let fractional_clear = [0, 0, 255, 255];
        let fractional_source = rgba_f32([255, 0, 0, 128]);
        let fractional_scene = GpuScene {
            logical_extent: [4, 3],
            clear: Color::new(
                fractional_clear[0],
                fractional_clear[1],
                fractional_clear[2],
                fractional_clear[3],
            ),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
            gamma_mode: GpuGammaMode::Disabled,
            textures: Vec::new(),
            commands: vec![GpuCommand::Solid {
                vertices: vec![
                    solid_vertex(1.0, 1.0, fractional_source),
                    solid_vertex(3.0, 1.0, fractional_source),
                    solid_vertex(1.0, 2.0, fractional_source),
                    solid_vertex(1.0, 2.0, fractional_source),
                    solid_vertex(3.0, 1.0, fractional_source),
                    solid_vertex(3.0, 2.0, fractional_source),
                ],
                topology: GpuPrimitiveTopology::TriangleList,
                alpha_mode: GpuSolidAlphaMode::SourceOver,
                clip: Some(Rect::new(1, 1, 2, 1)),
                blend: GpuBlend::Normal,
                gamma: false,
            }],
        };
        let fractional = render_readback(
            &mut renderer,
            &device,
            &queue,
            &fractional_scene,
            &GpuPresentation {
                physical_extent: [5, 4],
                scale: 1.5,
                crop_top: 1,
            },
        );
        for y in 0..4 {
            for x in 0..5 {
                let expected = if (1..4).contains(&x) && (1..3).contains(&y) {
                    [128, 0, 127, 255]
                } else {
                    fractional_clear
                };
                assert_eq!(
                    readback_pixel(&fractional, x, y),
                    expected,
                    "fractional clipper pixel ({x}, {y})"
                );
            }
        }

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
        clonk_frontend::draw_image_bilinear(&mut cpu_tiled, &tiled_rect, &tiled_image, None);
        let mut gpu_tiled = Surface::new(10, 6, PixelFormat::Rgba8888);
        gpu_tiled.begin_gpu_scene_capture();
        clonk_frontend::draw_image_bilinear(&mut gpu_tiled, &tiled_rect, &tiled_image, None);
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
        let resize_generation = renderer.generation();
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
        assert_eq!(
            renderer.generation(),
            resize_generation,
            "surface resize recreates only the composition target"
        );

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
        let replacement_generation = renderer.recreate(
            &replacement_device,
            &replacement_queue,
            wgpu::TextureFormat::Rgba8Unorm,
        );
        assert_eq!(renderer.generation(), replacement_generation);
        assert_ne!(replacement_generation, previous_generation);
        assert_eq!(renderer.health(), RetainedGpuRendererHealth::Healthy);
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
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: None,
            blend: GpuBlend::Replace,
            gamma: false,
        });
        commands.push(GpuCommand::Solid {
            // Producers encode logical pixel centers.  Non-unit W proves the
            // point expansion preserves homogeneous coordinates.
            vertices: vec![solid_vertex_w(6.5, 4.5, 2.0, rgba_f32(POINT))],
            topology: GpuPrimitiveTopology::PointList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
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
            outer_modulation: clonk_graphics::GpuSolidOuterModulation::PackedC4,
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
        let layer = GpuSceneLayer::new(scene, *presentation);
        render_layers_readback(renderer, device, queue, std::slice::from_ref(&layer))
    }

    fn render_layers_readback(
        renderer: &mut RetainedGpuRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layers: &[GpuSceneLayer<'_>],
    ) -> GpuReadbackFrame {
        let extent = layers
            .first()
            .expect("at least one retained test layer")
            .presentation
            .physical_extent;
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lc_gpu_parity_test_surface"),
            size: wgpu::Extent3d {
                width: extent[0],
                height: extent[1],
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
            .render_layers(device, queue, &mut encoder, &target_view, layers, true)
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
        let output_alpha = (f32::from(source[3]) + f32::from(destination[3]) * (1.0 - alpha))
            .round()
            .clamp(0.0, 255.0) as u8;
        [channel(0), channel(1), channel(2), output_alpha]
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
