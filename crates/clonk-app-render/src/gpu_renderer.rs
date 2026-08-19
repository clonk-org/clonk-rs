//! Retained wgpu scene composition for normal windowed gameplay.
//!
//! `pixels` 0.17 always uploads its logical CPU pixel buffer before invoking
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
    ClipperProjection, GpuBlend, GpuCommand, GpuGammaMode, GpuObjectSprite, GpuOuterModulation,
    GpuPresentation, GpuPrimitiveTopology, GpuSampler, GpuScene, GpuSolidAlphaMode, GpuSolidVertex,
    GpuSpriteQuad, GpuTextureFormat, GpuTextureId, GpuTextureResource, GpuVertex, Rect,
};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;
use wgpu::util::DeviceExt;

const PACKED_VERTEX_FLOATS: usize = 18;
const PACKED_VERTEX_STRIDE: u64 = (PACKED_VERTEX_FLOATS * std::mem::size_of::<f32>()) as u64;
const PACKED_QUAD_INSTANCE_FLOATS: usize = 58;
const PACKED_QUAD_INSTANCE_STRIDE: u64 =
    (PACKED_QUAD_INSTANCE_FLOATS * std::mem::size_of::<f32>()) as u64;
const PACKED_SPRITE_INSTANCE_STRIDE: u64 =
    (8 * std::mem::size_of::<f32>() + 2 * std::mem::size_of::<u32>()) as u64;
const PACKED_OBJECT_SPRITE_INSTANCE_STRIDE: u64 =
    (17 * std::mem::size_of::<f32>() + 5 * std::mem::size_of::<u32>()) as u64;
const PACKED_SOLID_RECT_INSTANCE_STRIDE: u64 =
    (8 * std::mem::size_of::<f32>() + std::mem::size_of::<u32>()) as u64;
const PACKED_LANDSCAPE_INSTANCE_STRIDE: u64 =
    (13 * std::mem::size_of::<f32>() + 5 * std::mem::size_of::<u32>()) as u64;
const LANDSCAPE_INSTANCE_BYTE_BUDGET: u64 = 96;
const LANDSCAPE_FLAG_GAMMA: u32 = 1 << 0;
const LANDSCAPE_FLAG_SMOOTH: u32 = 1 << 1;
const LANDSCAPE_SHAPE_SHIFT: u32 = 2;
/// A covered physical pixel may not cost more than this to describe. The
/// triangle-pair lowering it replaces spent 432 bytes on the same one pixel.
const SOLID_RECT_INSTANCE_BYTE_BUDGET: u64 = 40;
const SOLID_RECT_FLAG_GAMMA: u32 = 1;
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

const PACKED_QUAD_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 15] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 16,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 32,
        shader_location: 2,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 48,
        shader_location: 3,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 64,
        shader_location: 4,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 80,
        shader_location: 5,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 96,
        shader_location: 6,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 112,
        shader_location: 7,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 128,
        shader_location: 8,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 144,
        shader_location: 9,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 160,
        shader_location: 10,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 176,
        shader_location: 11,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 192,
        shader_location: 12,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 208,
        shader_location: 13,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 224,
        shader_location: 14,
    },
];

const PACKED_SPRITE_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 4] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 16,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Uint32,
        offset: 32,
        shader_location: 2,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Uint32,
        offset: 36,
        shader_location: 3,
    },
];

const PACKED_OBJECT_SPRITE_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 8] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 12,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 24,
        shader_location: 2,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 36,
        shader_location: 3,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 48,
        shader_location: 4,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Uint32x4,
        offset: 64,
        shader_location: 5,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32,
        offset: 80,
        shader_location: 6,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Uint32,
        offset: 84,
        shader_location: 7,
    },
];

const PACKED_SOLID_RECT_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 3] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 16,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Uint32,
        offset: 32,
        shader_location: 2,
    },
];

const PACKED_LANDSCAPE_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 6] = [
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 0,
        shader_location: 0,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x4,
        offset: 16,
        shader_location: 1,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Uint32x4,
        offset: 32,
        shader_location: 2,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x2,
        offset: 48,
        shader_location: 3,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Float32x3,
        offset: 56,
        shader_location: 4,
    },
    wgpu::VertexAttribute {
        format: wgpu::VertexFormat::Uint32,
        offset: 68,
        shader_location: 5,
    },
];

// The fragment stage is SOLID_SHADER's, unchanged: a covered pixel resolves
// the same gamma ramp whichever stage assembled its rectangle. Only the vertex
// stage differs, expanding one instance into the shared unit quad. Colour and
// flags interpolate flat because every corner of a fragment already shared
// them.
const SOLID_RECT_SHADER: &str = r#"
struct VertexInput {
    @location(0) clip_rect: vec4<f32>,
    @location(1) color: vec4<f32>,
    @location(2) packed_flags: u32,
    @builtin(vertex_index) vertex_index: u32,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) color: vec4<f32>,
    @location(1) @interpolate(flat) flags: vec2<f32>,
};

@group(0) @binding(0) var gamma_lut: texture_2d<u32>;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let right = input.vertex_index == 1u || input.vertex_index == 3u;
    let bottom = input.vertex_index >= 2u;
    var output: VertexOutput;
    output.position = vec4<f32>(
        select(input.clip_rect.x, input.clip_rect.z, right),
        select(input.clip_rect.y, input.clip_rect.w, bottom),
        0.0,
        1.0,
    );
    output.color = input.color;
    output.flags = vec2<f32>(
        select(0.0, 1.0, (input.packed_flags & 1u) != 0u),
        select(0.0, 1.0, (input.packed_flags & 2u) != 0u),
    );
    return output;
}

fn gamma_channel(channel: u32, value: f32) -> f32 {
    let index = min(u32(clamp(value, 0.0, 1.0) * 256.0), 255u);
    let sample = textureLoad(gamma_lut, vec2<i32>(i32(index), i32(channel)), 0).r;
    return f32(sample) / 65535.0;
}

fn dither_offset(position: vec2<f32>) -> f32 {
    let uniform_noise = fract(52.9829189 * fract(dot(position, vec2<f32>(0.06711056, 0.00583715))));
    let triangular = select(
        1.0 - sqrt(2.0 - 2.0 * uniform_noise),
        sqrt(2.0 * uniform_noise) - 1.0,
        uniform_noise < 0.5,
    );
    return triangular / 255.0;
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
    if input.flags.y > 0.5 {
        rgb = clamp(rgb + vec3<f32>(dither_offset(input.position.xy)), vec3<f32>(0.0), vec3<f32>(1.0));
    }
    return vec4<f32>(rgb, input.color.a);
}
"#;

const QUAD_SHADER: &str = r#"
struct VertexInput {
    @location(0) clip_position_0: vec4<f32>,
    @location(1) clip_position_1: vec4<f32>,
    @location(2) clip_position_2: vec4<f32>,
    @location(3) clip_position_3: vec4<f32>,
    @location(4) uv_01: vec4<f32>,
    @location(5) uv_23: vec4<f32>,
    @location(6) modulation_0: vec4<f32>,
    @location(7) modulation_1: vec4<f32>,
    @location(8) modulation_2: vec4<f32>,
    @location(9) modulation_3: vec4<f32>,
    @location(10) sample_tile_0: vec4<f32>,
    @location(11) sample_tile_1: vec4<f32>,
    @location(12) sample_tile_2: vec4<f32>,
    @location(13) sample_tile_3: vec4<f32>,
    @location(14) flags: vec2<f32>,
    @builtin(vertex_index) vertex_index: u32,
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
    let positions = array<vec4<f32>, 4>(
        input.clip_position_0,
        input.clip_position_1,
        input.clip_position_2,
        input.clip_position_3,
    );
    let uvs = array<vec2<f32>, 4>(
        input.uv_01.xy,
        input.uv_01.zw,
        input.uv_23.xy,
        input.uv_23.zw,
    );
    let modulations = array<vec4<f32>, 4>(
        input.modulation_0,
        input.modulation_1,
        input.modulation_2,
        input.modulation_3,
    );
    let sample_tiles = array<vec4<f32>, 4>(
        input.sample_tile_0,
        input.sample_tile_1,
        input.sample_tile_2,
        input.sample_tile_3,
    );
    let corner = input.vertex_index;
    var output: VertexOutput;
    output.position = positions[corner];
    output.uv = uvs[corner];
    output.modulation = modulations[corner];
    output.flags = vec4<f32>(input.flags, 0.0, 0.0);
    output.sample_tile = sample_tiles[corner];
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

const SPRITE_SHADER: &str = r#"
struct VertexInput {
    @location(0) clip_rect: vec4<f32>,
    @location(1) uv_rect: vec4<f32>,
    @location(2) packed_modulation: u32,
    @location(3) packed_flags: u32,
    @builtin(vertex_index) vertex_index: u32,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) modulation: vec4<f32>,
    @location(2) flags: vec2<f32>,
};

@group(0) @binding(0) var gamma_lut: texture_2d<u32>;
@group(1) @binding(0) var image: texture_2d<f32>;
@group(1) @binding(1) var image_sampler: sampler;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let right = input.vertex_index == 1u || input.vertex_index == 3u;
    let bottom = input.vertex_index >= 2u;
    let red = f32((input.packed_modulation >> 16u) & 255u) / 255.0;
    let green = f32((input.packed_modulation >> 8u) & 255u) / 255.0;
    let blue = f32(input.packed_modulation & 255u) / 255.0;
    let transparency = f32(input.packed_modulation >> 24u) / 255.0;
    var output: VertexOutput;
    output.position = vec4<f32>(
        select(input.clip_rect.x, input.clip_rect.z, right),
        select(input.clip_rect.y, input.clip_rect.w, bottom),
        0.0,
        1.0,
    );
    output.uv = vec2<f32>(
        select(input.uv_rect.x, input.uv_rect.z, right),
        select(input.uv_rect.y, input.uv_rect.w, bottom),
    );
    output.modulation = vec4<f32>(red, green, blue, transparency);
    output.flags = vec2<f32>(
        select(0.0, 1.0, (input.packed_flags & 1u) != 0u),
        select(0.0, 1.0, (input.packed_flags & 2u) != 0u),
    );
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
    let source = textureSampleLevel(image, image_sampler, input.uv, 0.0);
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

const OBJECT_SPRITE_SHADER: &str = r#"
struct VertexInput {
    @location(0) clip_position_0: vec3<f32>,
    @location(1) clip_position_1: vec3<f32>,
    @location(2) clip_position_2: vec3<f32>,
    @location(3) clip_position_3: vec3<f32>,
    @location(4) uv_rect: vec4<f32>,
    @location(5) packed_modulation: vec4<u32>,
    @location(6) sample_tile_size: f32,
    @location(7) packed_flags: u32,
    @builtin(vertex_index) vertex_index: u32,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) modulation: vec4<f32>,
    @location(2) @interpolate(flat) sample_tile_size: f32,
    @location(3) @interpolate(flat) packed_flags: u32,
};

@group(0) @binding(0) var gamma_lut: texture_2d<u32>;
@group(1) @binding(0) var image: texture_2d<f32>;
@group(1) @binding(1) var image_sampler: sampler;
@group(1) @binding(2) var owner_image: texture_2d<f32>;

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let positions = array<vec3<f32>, 4>(
        input.clip_position_0,
        input.clip_position_1,
        input.clip_position_2,
        input.clip_position_3,
    );
    let right = input.vertex_index == 1u || input.vertex_index == 3u;
    let bottom = input.vertex_index >= 2u;
    let packed = input.packed_modulation[input.vertex_index];
    let red = f32((packed >> 16u) & 255u) / 255.0;
    let green = f32((packed >> 8u) & 255u) / 255.0;
    let blue = f32(packed & 255u) / 255.0;
    let transparency = f32(packed >> 24u) / 255.0;
    let position = positions[input.vertex_index];
    var output: VertexOutput;
    output.position = vec4<f32>(position.xy, 0.0, position.z);
    output.uv = vec2<f32>(
        select(input.uv_rect.x, input.uv_rect.z, right),
        select(input.uv_rect.y, input.uv_rect.w, bottom),
    );
    output.modulation = vec4<f32>(red, green, blue, transparency);
    output.sample_tile_size = input.sample_tile_size;
    output.packed_flags = input.packed_flags;
    return output;
}

fn tiled_texel(image_size: vec2<i32>, tile_origin: vec2<f32>, tile_size: f32, relative: vec2<i32>, owner_layer: bool) -> vec4<f32> {
    let size = max(i32(tile_size), 1);
    let local = clamp(relative, vec2<i32>(0), vec2<i32>(size - 1));
    let position = vec2<i32>(tile_origin) + local;
    if any(position < vec2<i32>(0)) || any(position >= image_size) {
        return vec4<f32>(1.0, 1.0, 1.0, 0.0);
    }
    if owner_layer {
        return textureLoad(owner_image, position, 0);
    }
    return textureLoad(image, position, 0);
}

fn sample_native_tile(uv: vec2<f32>, tile_size: f32, owner_layer: bool) -> vec4<f32> {
    var image_size = vec2<i32>(textureDimensions(image));
    if owner_layer {
        image_size = vec2<i32>(textureDimensions(owner_image));
    }
    let size = max(tile_size, 1.0);
    let source_edge = uv * vec2<f32>(image_size);
    let tile_origin = floor(source_edge / vec2<f32>(size)) * size;
    let source = source_edge - vec2<f32>(0.5) - tile_origin;
    let base = vec2<i32>(floor(source));
    let fraction = fract(source);
    let top = mix(
        tiled_texel(image_size, tile_origin, size, base, owner_layer),
        tiled_texel(image_size, tile_origin, size, base + vec2<i32>(1, 0), owner_layer),
        fraction.x,
    );
    let bottom = mix(
        tiled_texel(image_size, tile_origin, size, base + vec2<i32>(0, 1), owner_layer),
        tiled_texel(image_size, tile_origin, size, base + vec2<i32>(1, 1), owner_layer),
        fraction.x,
    );
    return mix(top, bottom, fraction.y);
}

fn sample_nearest(uv: vec2<f32>, owner_layer: bool) -> vec4<f32> {
    var image_size = vec2<i32>(textureDimensions(image));
    if owner_layer {
        image_size = vec2<i32>(textureDimensions(owner_image));
    }
    let source = vec2<i32>(floor(uv * vec2<f32>(image_size)));
    if owner_layer {
        return textureLoad(owner_image, clamp(source, vec2<i32>(0), image_size - vec2<i32>(1)), 0);
    }
    return textureLoad(image, clamp(source, vec2<i32>(0), image_size - vec2<i32>(1)), 0);
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
    let linear = (input.packed_flags & 2u) != 0u;
    let owner_layer = (input.packed_flags & 32u) != 0u;
    var source: vec4<f32>;
    if linear {
        source = sample_native_tile(input.uv, input.sample_tile_size, owner_layer);
    } else {
        source = sample_nearest(input.uv, owner_layer);
    }
    var rgb = source.rgb;
    var alpha = source.a;
    if (input.packed_flags & 1u) != 0u {
        rgb = clamp((rgb + input.modulation.rgb) * 2.0 - 1.0, vec3<f32>(0.0), vec3<f32>(1.0));
    } else {
        rgb = clamp(rgb * input.modulation.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
        alpha = clamp(alpha - input.modulation.a, 0.0, 1.0);
    }
    if (input.packed_flags & 16u) != 0u {
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

struct CompactVertexInput {
    @builtin(vertex_index) vertex_index: u32,
    @location(0) clip_rect: vec4<f32>,
    @location(1) uv_rect: vec4<f32>,
    @location(2) packed_modulation: vec4<u32>,
    @location(3) liquid_scale: vec2<f32>,
    @location(4) phase: vec3<f32>,
    @location(5) flags: u32,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) modulation: vec4<f32>,
    @location(2) liquid_scale: vec4<f32>,
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
    output.liquid_scale = input.liquid_scale;
    output.phase_gamma = input.phase_gamma;
    return output;
}

fn unpack_c4_modulation(packed: u32) -> vec4<f32> {
    return vec4<f32>(
        f32((packed >> 16u) & 255u) / 255.0,
        f32((packed >> 8u) & 255u) / 255.0,
        f32(packed & 255u) / 255.0,
        f32(packed >> 24u) / 255.0,
    );
}

@vertex
fn vs_compact(input: CompactVertexInput) -> VertexOutput {
    let shape = input.flags >> 2u;
    var position = vec2<f32>(input.clip_rect.x, input.clip_rect.y);
    var uv = vec2<f32>(input.uv_rect.x, input.uv_rect.y);
    switch input.vertex_index {
        case 1u: {
            position = vec2<f32>(input.clip_rect.z, input.clip_rect.y);
            uv = vec2<f32>(input.uv_rect.z, input.uv_rect.y);
        }
        case 2u: {
            position = vec2<f32>(input.clip_rect.x, input.clip_rect.w);
            uv = vec2<f32>(input.uv_rect.x, input.uv_rect.w);
        }
        case 3u: {
            position = vec2<f32>(input.clip_rect.z, input.clip_rect.w);
            uv = vec2<f32>(input.uv_rect.z, input.uv_rect.w);
        }
        default: {}
    }
    if shape == 1u && input.vertex_index == 3u {
        position = vec2<f32>(input.clip_rect.x, input.clip_rect.w);
        uv = vec2<f32>(input.uv_rect.x, input.uv_rect.w);
    }
    if shape == 2u {
        if input.vertex_index == 0u {
            position = vec2<f32>(input.clip_rect.x, input.clip_rect.w);
            uv = vec2<f32>(input.uv_rect.x, input.uv_rect.w);
        } else if input.vertex_index >= 2u {
            position = vec2<f32>(input.clip_rect.z, input.clip_rect.w);
            uv = vec2<f32>(input.uv_rect.z, input.uv_rect.w);
        }
    }

    var output: VertexOutput;
    output.position = vec4<f32>(position, 0.0, 1.0);
    output.uv = uv;
    output.modulation = unpack_c4_modulation(input.packed_modulation[input.vertex_index]);
    output.liquid_scale = vec4<f32>(
        input.liquid_scale,
        select(0.0, 1.0, (input.flags & 2u) != 0u),
        0.0,
    );
    output.phase_gamma = vec4<f32>(
        input.phase,
        select(0.0, 1.0, (input.flags & 1u) != 0u),
    );
    return output;
}

// Alpha-weighted bilinear reconstruction of the landscape cache.
//
// Sky texels are RGBA(0,0,0,0) against opaque material, so an ordinary
// bilinear tap drags black into every silhouette and rings the terrain with a
// grey halo. Weighting colour by coverage takes the colour only from texels
// that have any, while alpha still ramps across the boundary — which is what
// turns a magnified 1-game-pixel step into an antialiased edge.
fn landscape_texel(coordinate: vec2<f32>, last: vec2<f32>) -> vec4<f32> {
    let clamped = clamp(coordinate, vec2<f32>(0.0), last);
    let sample = textureLoad(base_image, vec2<i32>(clamped), 0);
    return vec4<f32>(sample.rgb * sample.a, sample.a);
}

fn sample_landscape_smooth(uv: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(base_image, 0));
    let texel = uv * size - vec2<f32>(0.5);
    let origin = floor(texel);
    let weight = texel - origin;
    let last = size - vec2<f32>(1.0);
    let top = mix(
        landscape_texel(origin, last),
        landscape_texel(origin + vec2<f32>(1.0, 0.0), last),
        weight.x,
    );
    let bottom = mix(
        landscape_texel(origin + vec2<f32>(0.0, 1.0), last),
        landscape_texel(origin + vec2<f32>(1.0, 1.0), last),
        weight.x,
    );
    let accumulated = mix(top, bottom, weight.y);
    if accumulated.a <= 0.0 {
        return vec4<f32>(0.0);
    }
    return vec4<f32>(accumulated.rgb / accumulated.a, accumulated.a);
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
    var source = textureSample(base_image, base_sampler, input.uv);
    if input.liquid_scale.z > 0.5 {
        source = sample_landscape_smooth(input.uv);
    }
    let mask = textureSample(liquid_mask, base_sampler, input.uv).r;
    let liquid = textureSample(liquid_image, liquid_sampler, input.uv * input.liquid_scale.xy).rgb - vec3<f32>(0.5);
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

// Interleaved gradient noise (Jimenez 2014), remapped to a triangular PDF
// spanning one 8-bit step. Adding it before the framebuffer quantizes turns
// a hard band boundary into a stochastic one; the mean is unchanged, so the
// dithered gradient is closer to the exact ramp than the banded one is.
fn dither_offset(position: vec2<f32>) -> f32 {
    let uniform_noise = fract(52.9829189 * fract(dot(position, vec2<f32>(0.06711056, 0.00583715))));
    let triangular = select(
        1.0 - sqrt(2.0 - 2.0 * uniform_noise),
        sqrt(2.0 * uniform_noise) - 1.0,
        uniform_noise < 0.5,
    );
    return triangular / 255.0;
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
    if input.flags.y > 0.5 {
        rgb = clamp(rgb + vec3<f32>(dither_offset(input.position.xy)), vec3<f32>(0.0), vec3<f32>(1.0));
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

/// Area ("box") reduction of a presented composition, reproducing
/// `clonk_graphics::surface::downsample_rgba_box` byte for byte.
///
/// The destination is an integer (`Rgba8Uint`) attachment, so the accumulated
/// bytes reach the readback buffer without a normalization round trip.
const PRESENTATION_REDUCE_SHADER: &str = r#"
struct ReduceParams {
    source_extent: vec2<u32>,
    dest_extent: vec2<u32>,
};

@group(0) @binding(0) var source_image: texture_2d<f32>;
@group(0) @binding(1) var<uniform> params: ReduceParams;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    if index == 0u {
        return vec4<f32>(-1.0, -1.0, 0.0, 1.0);
    } else if index == 1u {
        return vec4<f32>(3.0, -1.0, 0.0, 1.0);
    }
    return vec4<f32>(-1.0, 3.0, 0.0, 1.0);
}

// Half-open source span of one destination cell. The spans tile the source
// exactly, so every source pixel contributes to exactly one cell; a
// destination wider than the source collapses to one pixel, which reproduces
// the CPU reference's nearest-neighbour magnification.
fn span(index: u32, dest_extent: u32, source_extent: u32) -> vec2<u32> {
    let start = index * source_extent / dest_extent;
    let end = (index + 1u) * source_extent / dest_extent;
    return vec2<u32>(start, min(max(end, start + 1u), source_extent));
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<u32> {
    let cell = vec2<u32>(position.xy);
    let horizontal = span(cell.x, params.dest_extent.x, params.source_extent.x);
    let vertical = span(cell.y, params.dest_extent.y, params.source_extent.y);
    var alpha_sum = 0u;
    var premultiplied = vec3<u32>(0u, 0u, 0u);
    for (var y = vertical.x; y < vertical.y; y = y + 1u) {
        for (var x = horizontal.x; x < horizontal.y; x = x + 1u) {
            // An `Rgba8Unorm` texel is exactly k/255 for an integer k, so
            // rounding recovers the stored byte with no loss.
            let texel = vec4<u32>(round(
                textureLoad(source_image, vec2<i32>(i32(x), i32(y)), 0) * 255.0
            ));
            alpha_sum = alpha_sum + texel.w;
            premultiplied = premultiplied + texel.xyz * texel.w;
        }
    }
    // A fully transparent cell keeps no colour at all, exactly like the CPU
    // reference's zeroed destination pixel.
    if alpha_sum == 0u {
        return vec4<u32>(0u, 0u, 0u, 0u);
    }
    let samples = (horizontal.y - horizontal.x) * (vertical.y - vertical.x);
    // Round half up on both the unpremultiply and the coverage mean.
    let color = min(
        (premultiplied + vec3<u32>(alpha_sum / 2u)) / vec3<u32>(alpha_sum),
        vec3<u32>(255u, 255u, 255u),
    );
    let alpha = min((alpha_sum + samples / 2u) / samples, 255u);
    return vec4<u32>(color, alpha);
}
"#;

/// Recovery decision published by wgpu's device-loss and uncaptured-error hooks.
///
/// Device loss has a dedicated callback. The validation-message check remains
/// as a compatibility fallback for backends which surface a lost parent device
/// through an ordinary resource operation before dispatching that callback.
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
    Internal,
}

#[derive(Clone, Debug)]
struct RetainedGpuHealthMonitor {
    state: Arc<Mutex<RetainedGpuRendererHealth>>,
}

impl RetainedGpuHealthMonitor {
    fn install(device: &wgpu::Device) -> Self {
        let state = Arc::new(Mutex::new(RetainedGpuRendererHealth::Healthy));
        let callback_state = Arc::clone(&state);
        device.on_uncaptured_error(Arc::new(move |error| {
            let health = classify_uncaptured_wgpu_error(&error);
            tracing::error!(%error, ?health, "uncaptured retained GPU device error");
            record_renderer_health(&callback_state, health);
        }));
        let lost_state = Arc::clone(&state);
        device.set_device_lost_callback(move |reason, message| {
            let detail = if message.is_empty() {
                format!("{reason:?}")
            } else {
                format!("{reason:?}: {message}")
            };
            tracing::error!(?reason, %detail, "retained GPU device lost");
            record_renderer_health(
                &lost_state,
                RetainedGpuRendererHealth::RecreateRequired {
                    reason: RetainedGpuRecreateReason::DeviceLost,
                    detail,
                },
            );
        });
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
        wgpu::Error::Internal { description, .. } => RetainedGpuRendererHealth::Fatal {
            reason: RetainedGpuFatalReason::Internal,
            detail: description.to_owned(),
        },
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
    #[error(
        "retained GPU {kind:?} texture {id:?} extent {extent:?} exceeds the device 2D texture limit {max_texture_dimension_2d}"
    )]
    TextureDimensionExceeded {
        kind: RetainedGpuTextureKind,
        id: Option<GpuTextureId>,
        extent: [u32; 2],
        max_texture_dimension_2d: u32,
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
    #[error("shader landscape composition inputs are invalid: {0}")]
    ShaderLandscapeInputs(&'static str),
    #[error("{topology:?} received {vertices} vertices")]
    InvalidPrimitiveVertexCount {
        topology: GpuPrimitiveTopology,
        vertices: usize,
    },
    #[error("non-finite GPU vertex or presentation coordinate")]
    NonFiniteCoordinate,
    #[error("compact {sampler:?} object sprite uses invalid native tile size {sample_tile_size}")]
    InvalidObjectSpriteSampleTile {
        sampler: GpuSampler,
        sample_tile_size: f32,
    },
    #[error("compact object sprite uses reserved packed flags {flags:#x}")]
    InvalidObjectSpriteFlags { flags: u32 },
    #[error("compact owner-layer object sprite has no companion owner texture")]
    ObjectOwnerLayerWithoutTexture,
    #[error(
        "compact object texture pair has different extents: {texture:?}={texture_extent:?}, {owner_texture:?}={owner_extent:?}"
    )]
    ObjectTextureExtentMismatch {
        texture: GpuTextureId,
        owner_texture: GpuTextureId,
        texture_extent: [u32; 2],
        owner_extent: [u32; 2],
    },
    #[error("replacement compact object batch changes outer-modulation policy at sprite {sprite}")]
    MixedReplaceObjectOuterModulation { sprite: usize },
    #[error("GPU vertex stream exceeds wgpu's u32 draw range")]
    VertexRangeOverflow,
    #[error("GPU readback size overflow")]
    ReadbackSizeOverflow,
    #[error("GPU readback callback was dropped")]
    ReadbackCallbackDropped,
    #[error("GPU readback mapping failed: {0}")]
    ReadbackMap(String),
    #[error("GPU readback polling failed: {0}")]
    ReadbackPoll(String),
    #[error("GPU timestamp readback polling failed: {0}")]
    TimestampPoll(String),
    #[error("GPU timestamp drain completed with {pending} pending frame(s)")]
    TimestampDrainIncomplete { pending: usize },
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

/// A retained 2D texture created by this renderer.
///
/// Source and shader-composer textures can fall back to the existing CPU
/// presentation path. The composition target has the same physical extent as
/// that CPU presentation, so it is reported separately rather than promising
/// a fallback that the device cannot display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetainedGpuTextureKind {
    Source,
    Composition,
    ShaderLandscapeIndex,
    ShaderLandscapeShading,
    ShaderLandscapeAtlas,
    ShaderLandscapeOutput,
}

impl RetainedGpuTextureKind {
    pub const fn supports_cpu_fallback(self) -> bool {
        !matches!(self, Self::Composition)
    }
}

/// CPU wall-clock intervals inside one retained-renderer invocation.
///
/// These are host-side stages, not GPU execution time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuRendererCpuStages {
    pub validation: Duration,
    pub texture_synchronization: Duration,
    pub stream_packing_upload: Duration,
    pub command_encoding: Duration,
}

impl GpuRendererCpuStages {
    pub fn total(self) -> Duration {
        self.validation
            .saturating_add(self.texture_synchronization)
            .saturating_add(self.stream_packing_upload)
            .saturating_add(self.command_encoding)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuTimestampPass {
    ShaderLandscape,
    Scene,
    MonitorGamma,
    Presentation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct GpuTimestampQueryPair {
    pass: GpuTimestampPass,
    begin: u32,
    end: u32,
}

impl GpuTimestampQueryPair {
    const fn new(pass: GpuTimestampPass, begin: u32, end: u32) -> Self {
        Self { pass, begin, end }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuTimestampSample {
    pub pass: GpuTimestampPass,
    pub begin_tick: u64,
    pub end_tick: u64,
    /// `None` whenever raw timing evidence is not safe to interpret.
    pub duration_ns: Option<f64>,
    pub validity: GpuTimestampSampleValidity,
}

/// Machine-readable disposition for a raw timestamp pair.
///
/// Invalid samples remain in the completed frame so benchmark artifacts retain
/// the device evidence, but consumers must reject every value except `Valid`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuTimestampSampleValidity {
    Valid,
    InvalidPeriod,
    CounterRollover,
    InvalidDuration,
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum GpuTimestampDecodeError {
    #[error("GPU timestamp query index {index} is absent from {available} values")]
    MissingQuery { index: u32, available: usize },
}

fn decode_timestamp_frame(
    period_ns: f32,
    pairs: &[GpuTimestampQueryPair],
    ticks: &[u64],
) -> Result<Vec<GpuTimestampSample>, GpuTimestampDecodeError> {
    pairs
        .iter()
        .map(|pair| {
            let begin = ticks.get(pair.begin as usize).copied().ok_or(
                GpuTimestampDecodeError::MissingQuery {
                    index: pair.begin,
                    available: ticks.len(),
                },
            )?;
            let end = ticks.get(pair.end as usize).copied().ok_or(
                GpuTimestampDecodeError::MissingQuery {
                    index: pair.end,
                    available: ticks.len(),
                },
            )?;
            let (validity, duration_ns) = if !period_ns.is_finite() || period_ns <= 0.0 {
                (GpuTimestampSampleValidity::InvalidPeriod, None)
            } else if let Some(elapsed) = end.checked_sub(begin) {
                let duration_ns = elapsed as f64 * f64::from(period_ns);
                if duration_ns.is_finite() {
                    (GpuTimestampSampleValidity::Valid, Some(duration_ns))
                } else {
                    (GpuTimestampSampleValidity::InvalidDuration, None)
                }
            } else {
                (GpuTimestampSampleValidity::CounterRollover, None)
            };
            Ok(GpuTimestampSample {
                pass: pair.pass,
                begin_tick: begin,
                end_tick: end,
                duration_ns,
                validity,
            })
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct GpuTimestampFrame {
    pub frame_id: u64,
    pub renderer_generation: u64,
    pub timestamp_period_ns: f32,
    pub passes: Vec<GpuTimestampSample>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuTimestampTelemetry {
    pub dropped_frames: u64,
    pub readback_errors: u64,
    pub device_discontinuities: u64,
}

const GPU_TIMESTAMP_QUERY_COUNT: u32 = 8;
const GPU_TIMESTAMP_SLOT_COUNT: usize = 8;
const GPU_TIMESTAMP_BUFFER_SIZE: u64 = GPU_TIMESTAMP_QUERY_COUNT as u64 * 8;
const GPU_TIMESTAMP_COMPLETED_HISTORY_LIMIT: usize = 1_024;
const GPU_TIMESTAMP_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

fn timestamp_drain_poll_type() -> wgpu::PollType {
    wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(GPU_TIMESTAMP_DRAIN_TIMEOUT),
    }
}

struct GpuTimestampSlot {
    query_set: wgpu::QuerySet,
    resolve_buffer: wgpu::Buffer,
    staging_buffer: wgpu::Buffer,
    ready: Arc<Mutex<Option<Result<(), String>>>>,
    in_flight: bool,
    frame_id: u64,
    renderer_generation: u64,
    timestamp_period_ns: f32,
    pairs: Vec<GpuTimestampQueryPair>,
    used_queries: u32,
}

struct ActiveGpuTimestampFrame {
    slot: usize,
    query_set: wgpu::QuerySet,
    resolve_buffer: wgpu::Buffer,
    staging_buffer: wgpu::Buffer,
    ready: Arc<Mutex<Option<Result<(), String>>>>,
    frame_id: u64,
    renderer_generation: u64,
    timestamp_period_ns: f32,
    pairs: Vec<GpuTimestampQueryPair>,
    next_query: u32,
}

impl ActiveGpuTimestampFrame {
    fn reserve(&mut self, pass: GpuTimestampPass) -> GpuTimestampQueryPair {
        debug_assert!(self.next_query + 1 < GPU_TIMESTAMP_QUERY_COUNT);
        let pair = GpuTimestampQueryPair::new(pass, self.next_query, self.next_query + 1);
        self.next_query += 2;
        self.pairs.push(pair);
        pair
    }

    fn timestamp_writes(&self, pair: GpuTimestampQueryPair) -> wgpu::RenderPassTimestampWrites<'_> {
        wgpu::RenderPassTimestampWrites {
            query_set: &self.query_set,
            beginning_of_pass_write_index: Some(pair.begin),
            end_of_pass_write_index: Some(pair.end),
        }
    }
}

struct GpuTimestampProfiler {
    slots: Vec<GpuTimestampSlot>,
}

struct GpuTimestampHistory {
    next_frame_id: u64,
    completed: Vec<GpuTimestampFrame>,
    telemetry: GpuTimestampTelemetry,
}

impl Default for GpuTimestampHistory {
    fn default() -> Self {
        Self {
            next_frame_id: 1,
            completed: Vec::new(),
            telemetry: GpuTimestampTelemetry::default(),
        }
    }
}

impl GpuTimestampHistory {
    fn push_completed(&mut self, frame: GpuTimestampFrame) {
        if self.completed.len() >= GPU_TIMESTAMP_COMPLETED_HISTORY_LIMIT {
            let overflow = self
                .completed
                .len()
                .saturating_add(1)
                .saturating_sub(GPU_TIMESTAMP_COMPLETED_HISTORY_LIMIT);
            self.completed.drain(..overflow);
            self.telemetry.dropped_frames = self
                .telemetry
                .dropped_frames
                .saturating_add(overflow as u64);
        }
        self.completed.push(frame);
    }
}

impl GpuTimestampProfiler {
    fn new(device: &wgpu::Device) -> Option<Self> {
        device
            .features()
            .contains(wgpu::Features::TIMESTAMP_QUERY)
            .then(|| Self {
                slots: (0..GPU_TIMESTAMP_SLOT_COUNT)
                    .map(|slot| {
                        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
                            label: Some("lc_gpu_timestamp_queries"),
                            ty: wgpu::QueryType::Timestamp,
                            count: GPU_TIMESTAMP_QUERY_COUNT,
                        });
                        let resolve_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("lc_gpu_timestamp_resolve"),
                            size: GPU_TIMESTAMP_BUFFER_SIZE,
                            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                            mapped_at_creation: false,
                        });
                        let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some("lc_gpu_timestamp_readback"),
                            size: GPU_TIMESTAMP_BUFFER_SIZE,
                            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                            mapped_at_creation: false,
                        });
                        GpuTimestampSlot {
                            query_set,
                            resolve_buffer,
                            staging_buffer,
                            ready: Arc::new(Mutex::new(None)),
                            in_flight: false,
                            frame_id: slot as u64,
                            renderer_generation: 0,
                            timestamp_period_ns: 0.0,
                            pairs: Vec::with_capacity(4),
                            used_queries: 0,
                        }
                    })
                    .collect(),
            })
    }

    fn lock_ready(
        ready: &Mutex<Option<Result<(), String>>>,
    ) -> std::sync::MutexGuard<'_, Option<Result<(), String>>> {
        ready
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn collect_mapped(&mut self, history: &mut GpuTimestampHistory) {
        for slot in &mut self.slots {
            if !slot.in_flight {
                continue;
            }
            let Some(mapped) = Self::lock_ready(&slot.ready).take() else {
                continue;
            };
            if mapped.is_err() {
                history.telemetry.readback_errors =
                    history.telemetry.readback_errors.saturating_add(1);
                slot.in_flight = false;
                continue;
            }
            let byte_len = u64::from(slot.used_queries) * 8;
            let Ok(mapped) = slot.staging_buffer.slice(0..byte_len).get_mapped_range() else {
                history.telemetry.readback_errors =
                    history.telemetry.readback_errors.saturating_add(1);
                slot.staging_buffer.unmap();
                slot.in_flight = false;
                continue;
            };
            let ticks = mapped
                .chunks_exact(8)
                .map(|bytes| u64::from_ne_bytes(bytes.try_into().expect("eight-byte timestamp")))
                .collect::<Vec<_>>();
            drop(mapped);
            slot.staging_buffer.unmap();
            match decode_timestamp_frame(slot.timestamp_period_ns, &slot.pairs, &ticks) {
                Ok(passes) => {
                    if passes
                        .iter()
                        .any(|sample| sample.validity != GpuTimestampSampleValidity::Valid)
                    {
                        history.telemetry.readback_errors =
                            history.telemetry.readback_errors.saturating_add(1);
                    }
                    history.push_completed(GpuTimestampFrame {
                        frame_id: slot.frame_id,
                        renderer_generation: slot.renderer_generation,
                        timestamp_period_ns: slot.timestamp_period_ns,
                        passes,
                    });
                }
                Err(_) => {
                    history.telemetry.readback_errors =
                        history.telemetry.readback_errors.saturating_add(1);
                }
            }
            slot.in_flight = false;
        }
    }

    fn collect_ready(&mut self, device: &wgpu::Device, history: &mut GpuTimestampHistory) {
        let _ = device.poll(wgpu::PollType::Poll);
        self.collect_mapped(history);
    }

    fn begin_frame(
        &mut self,
        device: &wgpu::Device,
        renderer_generation: u64,
        timestamp_period_ns: f32,
        history: &mut GpuTimestampHistory,
    ) -> Option<ActiveGpuTimestampFrame> {
        self.collect_ready(device, history);
        let Some((slot_index, slot)) = self
            .slots
            .iter()
            .enumerate()
            .find(|(_, slot)| !slot.in_flight)
        else {
            history.telemetry.dropped_frames = history.telemetry.dropped_frames.saturating_add(1);
            return None;
        };
        let frame_id = history.next_frame_id;
        history.next_frame_id = history.next_frame_id.wrapping_add(1).max(1);
        *Self::lock_ready(&slot.ready) = None;
        Some(ActiveGpuTimestampFrame {
            slot: slot_index,
            query_set: slot.query_set.clone(),
            resolve_buffer: slot.resolve_buffer.clone(),
            staging_buffer: slot.staging_buffer.clone(),
            ready: Arc::clone(&slot.ready),
            frame_id,
            renderer_generation,
            timestamp_period_ns,
            pairs: Vec::with_capacity(4),
            next_query: 0,
        })
    }

    fn finish_frame(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        active: ActiveGpuTimestampFrame,
    ) {
        if active.next_query == 0 {
            return;
        }
        let byte_len = u64::from(active.next_query) * 8;
        encoder.resolve_query_set(
            &active.query_set,
            0..active.next_query,
            &active.resolve_buffer,
            0,
        );
        encoder.copy_buffer_to_buffer(
            &active.resolve_buffer,
            0,
            &active.staging_buffer,
            0,
            byte_len,
        );
        let ready = Arc::clone(&active.ready);
        encoder.map_buffer_on_submit(
            &active.staging_buffer,
            wgpu::MapMode::Read,
            0..byte_len,
            move |result| {
                *Self::lock_ready(&ready) = Some(result.map_err(|error| error.to_string()));
            },
        );
        let slot = &mut self.slots[active.slot];
        slot.in_flight = true;
        slot.frame_id = active.frame_id;
        slot.renderer_generation = active.renderer_generation;
        slot.timestamp_period_ns = active.timestamp_period_ns;
        slot.pairs = active.pairs;
        slot.used_queries = active.next_query;
    }

    fn collect_completed(&mut self, device: &wgpu::Device, history: &mut GpuTimestampHistory) {
        self.collect_ready(device, history);
    }

    fn pending_frames(&self) -> usize {
        self.slots.iter().filter(|slot| slot.in_flight).count()
    }

    fn drain(
        &mut self,
        device: &wgpu::Device,
        history: &mut GpuTimestampHistory,
    ) -> Result<(), GpuRendererError> {
        device
            .poll(timestamp_drain_poll_type())
            .map_err(|error| GpuRendererError::TimestampPoll(error.to_string()))?;
        self.collect_mapped(history);
        let pending = self.pending_frames();
        if pending != 0 {
            return Err(GpuRendererError::TimestampDrainIncomplete { pending });
        }
        Ok(())
    }
}

/// Per-frame evidence that source retention and dirty updates are working.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuRendererStats {
    pub cpu_stages: GpuRendererCpuStages,
    pub timestamp_frame_id: Option<u64>,
    pub resident_source_textures: usize,
    pub created_source_textures: usize,
    pub full_upload_calls: usize,
    pub full_upload_bytes: u64,
    pub dirty_upload_calls: usize,
    pub dirty_upload_bytes: u64,
    /// Compatible painter-ordered resource runs, excluding fixed post-processing and presentation passes.
    pub draw_calls: usize,
    pub quad_draw_calls: usize,
    pub sprite_draw_calls: usize,
    pub object_sprite_draw_calls: usize,
    pub landscape_draw_calls: usize,
    pub shader_landscape_draw_calls: usize,
    pub solid_draw_calls: usize,
    pub solid_rect_draw_calls: usize,
    pub monitor_gamma_draw_calls: usize,
    pub presentation_draw_calls: usize,
    /// All GPU draw calls, including monitor-gamma and final presentation passes.
    pub total_draw_calls: usize,
    pub compatible_resource_runs: usize,
    /// Packed vertices shared by landscape quads and generic solid triangles.
    pub generic_vertices: usize,
    pub generic_vertex_upload_bytes: usize,
    pub quad_instances: usize,
    pub sprite_instances: usize,
    pub object_sprite_instances: usize,
    pub landscape_instances: usize,
    /// Physical point and line-fragment rectangles uploaded as compact instances.
    pub solid_rect_instances: usize,
    pub quad_instance_upload_bytes: usize,
    pub sprite_instance_upload_bytes: usize,
    pub object_sprite_upload_bytes: usize,
    pub landscape_instance_upload_bytes: usize,
    pub solid_rect_upload_bytes: usize,
    pub composition_recreated: bool,
    /// Composed shader-landscape outputs created this frame. Zero once the
    /// output is retained, which is also what keeps the quad, object and
    /// landscape bind groups that name it valid.
    pub created_shader_landscape_outputs: usize,
    /// Writes the composer issued for its retained planes, atlas and uniforms.
    ///
    /// Each is a `Queue::write_*` call, so this counts staging writes rather
    /// than compositions. Zero on a frame whose landscape did not change,
    /// which is what makes "an unchanged landscape uploads nothing" an
    /// assertion rather than a reading of the upload code.
    pub shader_landscape_upload_calls: usize,
    pub shader_landscape_upload_bytes: u64,
    /// Output texels the composition pass actually rewrote.
    ///
    /// The whole output on a fresh composition or a catalogue change; only the
    /// dirty rectangle, scaled by the detail factor, when a retained output is
    /// recomposed after a map edit.
    pub shader_landscape_composed_texels: u64,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CachedTextureContents {
    Source,
    ShaderLandscape,
}

#[derive(Debug)]
struct CachedTexture {
    /// CPU resource identity represented by the current GPU view. A shader
    /// landscape keeps this identity even though its actual extent differs.
    revision: u64,
    source_extent: [u32; 2],
    source_format: GpuTextureFormat,
    contents: CachedTextureContents,
    /// Actual GPU view descriptor; downstream sampling uses this extent.
    extent: [u32; 2],
    format: GpuTextureFormat,
    byte_len: u64,
    last_used_epoch: u64,
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl CachedTexture {
    fn source_matches(&self, resource: &GpuTextureResource) -> bool {
        self.revision == resource.revision
            && self.source_extent == resource.extent
            && self.source_format == resource.format
    }

    fn preserves_shader_output(
        &self,
        shader_landscape: bool,
        pending_shader_landscape: Option<GpuTextureId>,
        resource: &GpuTextureResource,
    ) -> bool {
        shader_landscape
            && self.contents == CachedTextureContents::ShaderLandscape
            && (self.source_matches(resource) || pending_shader_landscape == Some(resource.id))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct QuadBindingKey {
    texture: GpuTextureId,
    sampler: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct QuadRunKey {
    binding: QuadBindingKey,
    clip: Option<Rect>,
    blend: GpuBlend,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ObjectBindingKey {
    texture: GpuTextureId,
    owner_texture: Option<GpuTextureId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ObjectRunKey {
    binding: ObjectBindingKey,
    clip: Option<Rect>,
    blend: GpuBlend,
    gamma: bool,
    replace_outer_applies: Option<bool>,
}

fn object_run_key(command: &GpuCommand) -> Option<ObjectRunKey> {
    let GpuCommand::ObjectBatch {
        texture,
        owner_texture,
        sprites,
        clip,
        blend,
        gamma,
    } = command
    else {
        return None;
    };
    Some(ObjectRunKey {
        binding: ObjectBindingKey {
            texture: *texture,
            owner_texture: *owner_texture,
        },
        clip: *clip,
        blend: *blend,
        gamma: *gamma,
        replace_outer_applies: (*blend == GpuBlend::Replace).then(|| {
            sprites
                .first()
                .is_some_and(|sprite| sprite.outer_modulation() != GpuOuterModulation::Ignore)
        }),
    })
}

fn quad_run_key(command: &GpuCommand) -> Option<QuadRunKey> {
    match command {
        GpuCommand::Quad {
            texture,
            owner_mask: None,
            clip,
            blend,
            sampler,
            ..
        } => Some(QuadRunKey {
            binding: QuadBindingKey {
                texture: *texture,
                sampler: sampler_key(*sampler),
            },
            clip: *clip,
            blend: *blend,
        }),
        GpuCommand::SpriteBatch {
            texture,
            clip,
            blend,
            ..
        } => Some(QuadRunKey {
            binding: QuadBindingKey {
                texture: *texture,
                sampler: sampler_key(GpuSampler::Nearest),
            },
            clip: *clip,
            blend: *blend,
        }),
        _ => None,
    }
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
struct SpriteProjection {
    logical: Rect,
    physical: Rect,
    scale: (f64, f64),
    physical_extent: [u32; 2],
}

impl SpriteProjection {
    fn new(projection: &DrawProjection) -> Self {
        Self {
            logical: projection.clipper.logical_clip(),
            physical: projection.clipper.physical_clip(),
            scale: projection.clipper.scale(),
            physical_extent: projection.physical_extent,
        }
    }

    fn clip_rect(self, rect: [f32; 4]) -> Result<[f32; 4], GpuRendererError> {
        let [left, top, right, bottom] = rect;
        if !rect.iter().all(|value| value.is_finite()) {
            return Err(GpuRendererError::NonFiniteCoordinate);
        }
        let clip_x = |x: f32| {
            let physical = f64::from(self.physical.x)
                + (f64::from(x) - f64::from(self.logical.x)) * self.scale.0;
            2.0 * physical / f64::from(self.physical_extent[0]) - 1.0
        };
        let clip_y = |y: f32| {
            let physical = f64::from(self.physical.y)
                + (f64::from(y) - f64::from(self.logical.y)) * self.scale.1;
            1.0 - 2.0 * physical / f64::from(self.physical_extent[1])
        };
        let clip =
            [clip_x(left), clip_y(top), clip_x(right), clip_y(bottom)].map(|value| value as f32);
        clip.iter()
            .all(|value| value.is_finite())
            .then_some(clip)
            .ok_or(GpuRendererError::NonFiniteCoordinate)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DrawKind {
    Quad(QuadBindingKey),
    Sprite(QuadBindingKey),
    ObjectSprite(ObjectRunKey),
    Landscape(LandscapeBindingKey),
    LandscapeInstance(LandscapeBindingKey),
    Solid { alpha_mode: GpuSolidAlphaMode },
    SolidRect { alpha_mode: GpuSolidAlphaMode },
}

#[derive(Clone, Debug)]
struct DrawCall {
    vertices: Range<u32>,
    scissor: Scissor,
    blend: GpuBlend,
    kind: DrawKind,
}

struct BuiltDrawStream {
    vertices: Vec<PackedVertex>,
    quad_instances: Vec<PackedQuadInstance>,
    sprite_instances: Vec<PackedSpriteInstance>,
    object_sprite_instances: Vec<PackedObjectSpriteInstance>,
    landscape_instances: Vec<PackedLandscapeInstance>,
    solid_rect_instances: Vec<PackedSolidRectInstance>,
    calls: Vec<DrawCall>,
}

impl GpuRendererStats {
    /// True when every retained draw is classified exactly once and the
    /// classified scene/fixed passes reconcile with the submitted total.
    pub fn has_exact_draw_call_counts(self) -> bool {
        let classified_scene = self
            .quad_draw_calls
            .saturating_add(self.sprite_draw_calls)
            .saturating_add(self.object_sprite_draw_calls)
            .saturating_add(self.landscape_draw_calls)
            .saturating_add(self.solid_draw_calls)
            .saturating_add(self.solid_rect_draw_calls);
        let classified_total = self
            .draw_calls
            .saturating_add(self.shader_landscape_draw_calls)
            .saturating_add(self.monitor_gamma_draw_calls)
            .saturating_add(self.presentation_draw_calls);
        classified_scene == self.draw_calls
            && self.compatible_resource_runs == self.draw_calls
            && classified_total == self.total_draw_calls
    }

    fn record_full_texture_upload(&mut self, upload: TextureUploadStats) {
        self.full_upload_calls = self.full_upload_calls.saturating_add(upload.calls);
        self.full_upload_bytes = self.full_upload_bytes.saturating_add(upload.bytes);
    }

    fn record_dirty_texture_upload(&mut self, bytes: u64) {
        self.dirty_upload_calls = self.dirty_upload_calls.saturating_add(1);
        self.dirty_upload_bytes = self.dirty_upload_bytes.saturating_add(bytes);
    }

    fn record_draw_stream(&mut self, stream: &BuiltDrawStream) {
        self.generic_vertices = stream.vertices.len();
        self.generic_vertex_upload_bytes = stream.vertices.len() * PACKED_VERTEX_STRIDE as usize;
        self.quad_instances = stream.quad_instances.len();
        self.quad_instance_upload_bytes =
            stream.quad_instances.len() * PACKED_QUAD_INSTANCE_STRIDE as usize;
        self.sprite_instances = stream.sprite_instances.len();
        self.sprite_instance_upload_bytes =
            stream.sprite_instances.len() * PACKED_SPRITE_INSTANCE_STRIDE as usize;
        self.object_sprite_instances = stream.object_sprite_instances.len();
        self.object_sprite_upload_bytes =
            stream.object_sprite_instances.len() * PACKED_OBJECT_SPRITE_INSTANCE_STRIDE as usize;
        self.landscape_instances = stream.landscape_instances.len();
        self.landscape_instance_upload_bytes =
            stream.landscape_instances.len() * PACKED_LANDSCAPE_INSTANCE_STRIDE as usize;
        self.solid_rect_instances = stream.solid_rect_instances.len();
        self.solid_rect_upload_bytes =
            stream.solid_rect_instances.len() * PACKED_SOLID_RECT_INSTANCE_STRIDE as usize;
        self.draw_calls = stream.calls.len();
        self.compatible_resource_runs = stream.calls.len();
        for call in &stream.calls {
            let count = match call.kind {
                DrawKind::Quad(_) => &mut self.quad_draw_calls,
                DrawKind::Sprite(_) => &mut self.sprite_draw_calls,
                DrawKind::ObjectSprite(_) => &mut self.object_sprite_draw_calls,
                DrawKind::Landscape(_) | DrawKind::LandscapeInstance(_) => {
                    &mut self.landscape_draw_calls
                }
                DrawKind::Solid { .. } => &mut self.solid_draw_calls,
                DrawKind::SolidRect { .. } => &mut self.solid_rect_draw_calls,
            };
            *count = count.saturating_add(1);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TextureUploadStats {
    calls: usize,
    bytes: u64,
}

impl TextureUploadStats {
    fn record(&mut self, bytes: usize) {
        self.calls = self.calls.saturating_add(1);
        self.bytes = self.bytes.saturating_add(bytes as u64);
    }

    fn add(&mut self, other: Self) {
        self.calls = self.calls.saturating_add(other.calls);
        self.bytes = self.bytes.saturating_add(other.bytes);
    }
}

impl DrawCall {
    fn push_compatible_quad(calls: &mut Vec<Self>, batch_start: usize, call: Self) {
        let compatible = (calls.len() > batch_start)
            .then(|| calls.last_mut())
            .flatten()
            .filter(|previous| {
                previous.vertices.end == call.vertices.start
                    && previous.scissor == call.scissor
                    && previous.blend == call.blend
                    && previous.kind == call.kind
            });
        if let Some(previous) = compatible {
            // Vertices retain command and triangle order, so fixed-function
            // blending produces the same painter-order result in one draw.
            previous.vertices.end = call.vertices.end;
        } else {
            calls.push(call);
        }
    }
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

/// Retained pipeline and destination for [`RetainedGpuRenderer::readback_last_presentation_reduced`].
///
/// A save thumbnail is 200x150 whatever the frame is. Reading a 4K
/// presentation back only to reduce it on the CPU maps about 31.6 MiB to
/// produce about 117 KiB, so the reduction runs against the presented texture
/// and only its result is copied out.
#[derive(Debug)]
struct PresentationReducer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    target: Option<ReducedPresentationTarget>,
}

#[derive(Debug)]
struct ReducedPresentationTarget {
    extent: [u32; 2],
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl PresentationReducer {
    /// An integer attachment carries the accumulated bytes out untouched; a
    /// normalized one would depend on the backend's unorm rounding.
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Uint;

    fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lc_gpu_presentation_reduce_layout"),
            entries: &[
                texture_layout_entry(0, wgpu::TextureSampleType::Float { filterable: true }),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let module = shader(
            device,
            "lc_gpu_presentation_reduce_shader",
            PRESENTATION_REDUCE_SHADER,
        );
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lc_gpu_presentation_reduce_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let targets = [Some(wgpu::ColorTargetState {
            format: Self::FORMAT,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lc_gpu_presentation_reduce"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            bind_group_layout,
            target: None,
        }
    }

    fn ensure_target(&mut self, device: &wgpu::Device, extent: [u32; 2]) {
        if self
            .target
            .as_ref()
            .is_none_or(|target| target.extent != extent)
        {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("lc_gpu_reduced_presentation"),
                size: wgpu::Extent3d {
                    width: extent[0],
                    height: extent[1],
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: Self::FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.target = Some(ReducedPresentationTarget {
                extent,
                texture,
                view,
            });
        }
    }

    /// Records the reduction of `source` into the retained target.
    ///
    /// Two requests in one frame reuse that target, which is safe because the
    /// caller copies each result out before recording the next pass.
    fn reduce(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        source_extent: [u32; 2],
        dest_extent: [u32; 2],
    ) -> &wgpu::Texture {
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lc_gpu_presentation_reduce_params"),
            contents: u32_bytes(&[
                source_extent[0],
                source_extent[1],
                dest_extent[0],
                dest_extent[1],
            ]),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lc_gpu_presentation_reduce_bind_group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params.as_entire_binding(),
                },
            ],
        });
        self.ensure_target(device, dest_extent);
        let target = self
            .target
            .as_ref()
            .expect("the reduced presentation target was just ensured");
        let attachments = [Some(wgpu::RenderPassColorAttachment {
            view: &target.view,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                store: wgpu::StoreOp::Store,
            },
        })];
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("lc_gpu_presentation_reduce_pass"),
            color_attachments: &attachments,
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind_group, &[]);
        pass.draw(0..3, 0..1);
        drop(pass);
        &target.texture
    }
}

/// Largest source span one destination cell may cover before the shader's
/// `u32` accumulator could overflow.
///
/// A cell of `s` samples accumulates at most `255 * 255 * s` of premultiplied
/// colour and adds `255 * s / 2` to round the unpremultiply half up, so the
/// divisor is `65025 + 127.5` rounded up.
const MAX_REDUCTION_SAMPLES: u32 = u32::MAX / 65153;

/// True when the GPU reduction of `source` to `dest` is provably exact.
fn reduction_accumulator_fits(source: [u32; 2], dest: [u32; 2]) -> bool {
    if source.contains(&0) || dest.contains(&0) {
        return false;
    }
    // A span is `floor((i + 1) * source / dest) - floor(i * source / dest)`,
    // so it never exceeds `ceil(source / dest)`; magnification collapses it to
    // a single source pixel.
    let max_span = |source: u32, dest: u32| {
        if dest < source {
            source.div_ceil(dest)
        } else {
            1
        }
    };
    max_span(source[0], dest[0])
        .checked_mul(max_span(source[1], dest[1]))
        .is_some_and(|samples| samples <= MAX_REDUCTION_SAMPLES)
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
    /// Bytes this ticket will map, including WebGPU's per-row padding.
    pub fn mapped_bytes(&self) -> u64 {
        self.buffer.size()
    }

    /// Wait for a copy already submitted by `Pixels::render_with` (or a test
    /// queue submission), remove WebGPU row padding, and return tightly packed
    /// physical RGBA pixels.
    pub fn read(self, device: &wgpu::Device) -> Result<GpuReadbackFrame, GpuRendererError> {
        let slice = self.buffer.slice(..);
        let (sender, receiver) = mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| GpuRendererError::ReadbackPoll(error.to_string()))?;
        let result = receiver
            .recv()
            .map_err(|_| GpuRendererError::ReadbackCallbackDropped)?;
        result.map_err(|error| GpuRendererError::ReadbackMap(error.to_string()))?;

        let mapped = slice
            .get_mapped_range()
            .map_err(|error| GpuRendererError::ReadbackMap(error.to_string()))?;
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
    object_bind_groups: HashMap<ObjectBindingKey, wgpu::BindGroup>,
    landscape_bind_groups: HashMap<LandscapeBindingKey, wgpu::BindGroup>,

    gamma_texture: wgpu::Texture,
    _gamma_view: wgpu::TextureView,
    gamma_bind_group: wgpu::BindGroup,
    gamma_revision: Option<u64>,

    quad_bind_group_layout: wgpu::BindGroupLayout,
    object_bind_group_layout: wgpu::BindGroupLayout,
    landscape_bind_group_layout: wgpu::BindGroupLayout,
    present_bind_group_layout: wgpu::BindGroupLayout,
    quad_replace_pipeline: wgpu::RenderPipeline,
    quad_normal_pipeline: wgpu::RenderPipeline,
    quad_additive_pipeline: wgpu::RenderPipeline,
    sprite_replace_pipeline: wgpu::RenderPipeline,
    sprite_normal_pipeline: wgpu::RenderPipeline,
    sprite_additive_pipeline: wgpu::RenderPipeline,
    object_sprite_replace_pipeline: wgpu::RenderPipeline,
    object_sprite_normal_pipeline: wgpu::RenderPipeline,
    object_sprite_additive_pipeline: wgpu::RenderPipeline,
    landscape_pipeline: wgpu::RenderPipeline,
    landscape_instance_pipeline: wgpu::RenderPipeline,
    solid_replace_pipeline: wgpu::RenderPipeline,
    solid_over_normal_pipeline: wgpu::RenderPipeline,
    solid_non_separate_normal_pipeline: wgpu::RenderPipeline,
    solid_additive_pipeline: wgpu::RenderPipeline,
    solid_rect_replace_pipeline: wgpu::RenderPipeline,
    solid_rect_over_normal_pipeline: wgpu::RenderPipeline,
    solid_rect_non_separate_normal_pipeline: wgpu::RenderPipeline,
    solid_rect_additive_pipeline: wgpu::RenderPipeline,
    monitor_gamma_pipeline: wgpu::RenderPipeline,
    present_pipeline: wgpu::RenderPipeline,

    nearest_sampler: wgpu::Sampler,
    linear_sampler: wgpu::Sampler,
    /// Trilinear + anisotropic variant used when `mipmaps` is on. Kept beside
    /// the C++-exact sampler so the policy is one bind-time choice.
    linear_mip_sampler: wgpu::Sampler,
    mipmaps: bool,
    smooth_landscape: bool,
    shader_landscape: bool,
    landscape_detail: u32,
    /// Composed by the fragment shader before the next frame draws, replacing
    /// the CPU-composed upload for this texture id. Taken each frame so a stale
    /// plan can never outlive the landscape it describes.
    pending_shader_landscape: Option<(GpuTextureId, clonk_graphics::ShaderLandscapePlan)>,
    landscape_composer: Option<ShaderLandscapeComposer>,
    repeat_nearest_sampler: wgpu::Sampler,
    present_sampler: wgpu::Sampler,
    _fallback_mask_texture: wgpu::Texture,
    fallback_mask_view: wgpu::TextureView,
    _fallback_liquid_texture: wgpu::Texture,
    fallback_liquid_view: wgpu::TextureView,

    vertex_buffer: wgpu::Buffer,
    vertex_buffer_size: u64,
    quad_instance_buffer: wgpu::Buffer,
    quad_instance_buffer_size: u64,
    sprite_instance_buffer: wgpu::Buffer,
    sprite_instance_buffer_size: u64,
    object_sprite_instance_buffer: wgpu::Buffer,
    object_sprite_instance_buffer_size: u64,
    landscape_instance_buffer: wgpu::Buffer,
    landscape_instance_buffer_size: u64,
    solid_rect_instance_buffer: wgpu::Buffer,
    solid_rect_instance_buffer_size: u64,
    quad_index_buffer: wgpu::Buffer,
    vertex_scratch: Vec<PackedVertex>,
    quad_instance_scratch: Vec<PackedQuadInstance>,
    sprite_instance_scratch: Vec<PackedSpriteInstance>,
    object_sprite_instance_scratch: Vec<PackedObjectSpriteInstance>,
    landscape_instance_scratch: Vec<PackedLandscapeInstance>,
    solid_rect_instance_scratch: Vec<PackedSolidRectInstance>,
    draw_call_scratch: Vec<DrawCall>,
    composition: Option<CompositionTarget>,
    last_presented_monitor_gamma: Option<bool>,
    /// Built the first time a caller asks for a reduced presentation, so a
    /// session that never saves a thumbnail owns no reduction pipeline.
    presentation_reducer: Option<PresentationReducer>,
    timestamp_profiler: Option<GpuTimestampProfiler>,
    timestamp_history: GpuTimestampHistory,
    last_stats: GpuRendererStats,
    /// Once a scene needs a source texture larger than this device supports,
    /// presentation stays on the CPU reference path until the device is
    /// recreated. Repeating GPU capture would only rediscover the same limit
    /// and spam the operator log.
    cpu_presentation_required: bool,
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
        let object_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("lc_gpu_object_sprite_layout"),
                entries: &[
                    texture_layout_entry(0, wgpu::TextureSampleType::Float { filterable: true }),
                    sampler_layout_entry(1),
                    texture_layout_entry(2, wgpu::TextureSampleType::Float { filterable: true }),
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
        let sprite_shader = shader(device, "lc_gpu_sprite_shader", SPRITE_SHADER);
        let object_sprite_shader =
            shader(device, "lc_gpu_object_sprite_shader", OBJECT_SPRITE_SHADER);
        let landscape_shader = shader(device, "lc_gpu_landscape_shader", LANDSCAPE_SHADER);
        let solid_shader = shader(device, "lc_gpu_solid_shader", SOLID_SHADER);
        let solid_rect_shader = shader(device, "lc_gpu_solid_rect_shader", SOLID_RECT_SHADER);
        let present_shader = shader(device, "lc_gpu_present_shader", PRESENT_SHADER);

        let quad_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lc_gpu_quad_pipeline_layout"),
            bind_group_layouts: &[
                Some(&gamma_bind_group_layout),
                Some(&quad_bind_group_layout),
            ],
            immediate_size: 0,
        });
        let object_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("lc_gpu_object_sprite_pipeline_layout"),
                bind_group_layouts: &[
                    Some(&gamma_bind_group_layout),
                    Some(&object_bind_group_layout),
                ],
                immediate_size: 0,
            });
        let landscape_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("lc_gpu_landscape_pipeline_layout"),
                bind_group_layouts: &[
                    Some(&gamma_bind_group_layout),
                    Some(&landscape_bind_group_layout),
                ],
                immediate_size: 0,
            });
        let solid_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("lc_gpu_solid_pipeline_layout"),
                bind_group_layouts: &[Some(&gamma_bind_group_layout)],
                immediate_size: 0,
            });
        let present_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("lc_gpu_present_pipeline_layout"),
                bind_group_layouts: &[Some(&present_bind_group_layout)],
                immediate_size: 0,
            });
        let monitor_gamma_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("lc_gpu_monitor_gamma_pipeline_layout"),
                bind_group_layouts: &[
                    Some(&present_bind_group_layout),
                    Some(&gamma_bind_group_layout),
                ],
                immediate_size: 0,
            });

        let quad_replace_pipeline = quad_scene_pipeline(
            device,
            "lc_gpu_quad_replace",
            &quad_pipeline_layout,
            &quad_shader,
            GpuBlend::Replace,
            GpuSolidAlphaMode::SourceOver,
        );
        let quad_normal_pipeline = quad_scene_pipeline(
            device,
            "lc_gpu_quad_normal",
            &quad_pipeline_layout,
            &quad_shader,
            GpuBlend::Normal,
            GpuSolidAlphaMode::SourceOver,
        );
        let quad_additive_pipeline = quad_scene_pipeline(
            device,
            "lc_gpu_quad_additive",
            &quad_pipeline_layout,
            &quad_shader,
            GpuBlend::Additive,
            GpuSolidAlphaMode::SourceOver,
        );
        let sprite_replace_pipeline = sprite_scene_pipeline(
            device,
            "lc_gpu_sprite_replace",
            &quad_pipeline_layout,
            &sprite_shader,
            GpuBlend::Replace,
        );
        let sprite_normal_pipeline = sprite_scene_pipeline(
            device,
            "lc_gpu_sprite_normal",
            &quad_pipeline_layout,
            &sprite_shader,
            GpuBlend::Normal,
        );
        let sprite_additive_pipeline = sprite_scene_pipeline(
            device,
            "lc_gpu_sprite_additive",
            &quad_pipeline_layout,
            &sprite_shader,
            GpuBlend::Additive,
        );
        let object_sprite_replace_pipeline = object_sprite_scene_pipeline(
            device,
            "lc_gpu_object_sprite_replace",
            &object_pipeline_layout,
            &object_sprite_shader,
            GpuBlend::Replace,
        );
        let object_sprite_normal_pipeline = object_sprite_scene_pipeline(
            device,
            "lc_gpu_object_sprite_normal",
            &object_pipeline_layout,
            &object_sprite_shader,
            GpuBlend::Normal,
        );
        let object_sprite_additive_pipeline = object_sprite_scene_pipeline(
            device,
            "lc_gpu_object_sprite_additive",
            &object_pipeline_layout,
            &object_sprite_shader,
            GpuBlend::Additive,
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
        let landscape_instance_pipeline = scene_pipeline_with_vertex_layout(
            device,
            "lc_gpu_landscape_instances",
            &landscape_pipeline_layout,
            &landscape_shader,
            wgpu::PrimitiveTopology::TriangleList,
            GpuBlend::Normal,
            GpuSolidAlphaMode::SourceOver,
            packed_landscape_instance_layout(),
            "vs_compact",
        );
        // Triangle-list solids interpolate across a whole primitive, so they
        // keep the generic vertex stream. Point and line commands resolve to
        // whole physical pixels and ride the instanced rectangle pipeline.
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
        let solid_rect_replace_pipeline = solid_rect_scene_pipeline(
            device,
            "lc_gpu_solid_rect_replace",
            &solid_pipeline_layout,
            &solid_rect_shader,
            GpuBlend::Replace,
            GpuSolidAlphaMode::SourceOver,
        );
        let solid_rect_over_normal_pipeline = solid_rect_scene_pipeline(
            device,
            "lc_gpu_solid_rect_over_normal",
            &solid_pipeline_layout,
            &solid_rect_shader,
            GpuBlend::Normal,
            GpuSolidAlphaMode::SourceOver,
        );
        let solid_rect_non_separate_normal_pipeline = solid_rect_scene_pipeline(
            device,
            "lc_gpu_solid_rect_non_separate_normal",
            &solid_pipeline_layout,
            &solid_rect_shader,
            GpuBlend::Normal,
            GpuSolidAlphaMode::NonSeparate,
        );
        let solid_rect_additive_pipeline = solid_rect_scene_pipeline(
            device,
            "lc_gpu_solid_rect_additive",
            &solid_pipeline_layout,
            &solid_rect_shader,
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
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lc_gpu_linear_clamp"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let linear_mip_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lc_gpu_linear_clamp_mip"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });
        let repeat_nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lc_gpu_nearest_repeat"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let present_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lc_gpu_present_nearest"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
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
        let quad_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lc_gpu_quad_instances"),
            size: INITIAL_VERTEX_BUFFER_SIZE,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sprite_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lc_gpu_sprite_instances"),
            size: INITIAL_VERTEX_BUFFER_SIZE,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let object_sprite_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lc_gpu_object_sprite_instances"),
            size: INITIAL_VERTEX_BUFFER_SIZE,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let landscape_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lc_gpu_landscape_instances"),
            size: INITIAL_VERTEX_BUFFER_SIZE,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let solid_rect_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lc_gpu_solid_rect_instances"),
            size: INITIAL_VERTEX_BUFFER_SIZE,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let quad_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lc_gpu_quad_indices"),
            contents: &[0, 0, 1, 0, 2, 0, 2, 0, 1, 0, 3, 0],
            usage: wgpu::BufferUsages::INDEX,
        });

        Self {
            surface_format,
            generation,
            health,
            texture_epoch: 0,
            textures: HashMap::new(),
            quad_bind_groups: HashMap::new(),
            object_bind_groups: HashMap::new(),
            landscape_bind_groups: HashMap::new(),
            gamma_texture,
            _gamma_view: gamma_view,
            gamma_bind_group,
            gamma_revision: None,
            quad_bind_group_layout,
            object_bind_group_layout,
            landscape_bind_group_layout,
            present_bind_group_layout,
            quad_replace_pipeline,
            quad_normal_pipeline,
            quad_additive_pipeline,
            sprite_replace_pipeline,
            sprite_normal_pipeline,
            sprite_additive_pipeline,
            object_sprite_replace_pipeline,
            object_sprite_normal_pipeline,
            object_sprite_additive_pipeline,
            landscape_pipeline,
            landscape_instance_pipeline,
            solid_replace_pipeline,
            solid_over_normal_pipeline,
            solid_non_separate_normal_pipeline,
            solid_additive_pipeline,
            solid_rect_replace_pipeline,
            solid_rect_over_normal_pipeline,
            solid_rect_non_separate_normal_pipeline,
            solid_rect_additive_pipeline,
            monitor_gamma_pipeline,
            present_pipeline,
            nearest_sampler,
            linear_sampler,
            linear_mip_sampler,
            mipmaps: false,
            smooth_landscape: false,
            shader_landscape: false,
            landscape_detail: 1,
            pending_shader_landscape: None,
            landscape_composer: None,
            repeat_nearest_sampler,
            present_sampler,
            _fallback_mask_texture: fallback_mask_texture,
            fallback_mask_view,
            _fallback_liquid_texture: fallback_liquid_texture,
            fallback_liquid_view,
            vertex_buffer,
            vertex_buffer_size: INITIAL_VERTEX_BUFFER_SIZE,
            quad_instance_buffer,
            quad_instance_buffer_size: INITIAL_VERTEX_BUFFER_SIZE,
            sprite_instance_buffer,
            sprite_instance_buffer_size: INITIAL_VERTEX_BUFFER_SIZE,
            object_sprite_instance_buffer,
            object_sprite_instance_buffer_size: INITIAL_VERTEX_BUFFER_SIZE,
            landscape_instance_buffer,
            landscape_instance_buffer_size: INITIAL_VERTEX_BUFFER_SIZE,
            solid_rect_instance_buffer,
            solid_rect_instance_buffer_size: INITIAL_VERTEX_BUFFER_SIZE,
            quad_index_buffer,
            vertex_scratch: Vec::new(),
            quad_instance_scratch: Vec::new(),
            sprite_instance_scratch: Vec::new(),
            object_sprite_instance_scratch: Vec::new(),
            landscape_instance_scratch: Vec::new(),
            solid_rect_instance_scratch: Vec::new(),
            draw_call_scratch: Vec::new(),
            composition: None,
            last_presented_monitor_gamma: None,
            presentation_reducer: None,
            timestamp_profiler: GpuTimestampProfiler::new(device),
            timestamp_history: GpuTimestampHistory::default(),
            last_stats: GpuRendererStats::default(),
            cpu_presentation_required: false,
        }
    }

    /// Rebuild every device-owned object after the application has replaced
    /// the `Pixels` device and queue.
    ///
    /// The local `pixels` patch returns Lost to the application and bounds
    /// Outdated/Suboptimal retries. Call this after constructing replacement
    /// `Pixels` for a lost surface or device; the next validated scene carries
    /// complete CPU backing for every referenced texture and therefore
    /// repopulates this empty cache without a CPU-frame fallback.
    pub fn recreate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
    ) -> u64 {
        let generation = self.generation.wrapping_add(1).max(1);
        if let Some(profiler) = self.timestamp_profiler.as_mut() {
            // A device-idle callback may already have mapped its old-device
            // buffer. Harvest that sample without polling the replacement
            // device before the old query objects are dropped.
            profiler.collect_mapped(&mut self.timestamp_history);
            self.timestamp_history.telemetry.dropped_frames = self
                .timestamp_history
                .telemetry
                .dropped_frames
                .saturating_add(profiler.pending_frames() as u64);
        }
        let mut timestamp_history = std::mem::take(&mut self.timestamp_history);
        // `build` starts every presentation flag at its C++-exact default, so
        // the configured opt-ins have to be carried over explicitly. The
        // renderer is a local in `main` that GameApp never holds, so nothing
        // downstream would re-apply them after a device loss.
        let carried = (
            self.mipmaps,
            self.smooth_landscape,
            self.shader_landscape,
            self.landscape_detail,
        );
        *self = Self::build(device, queue, surface_format, generation);
        timestamp_history.telemetry.device_discontinuities = timestamp_history
            .telemetry
            .device_discontinuities
            .saturating_add(1);
        self.timestamp_history = timestamp_history;
        self.mipmaps = carried.0;
        self.smooth_landscape = carried.1;
        self.shader_landscape = carried.2;
        self.landscape_detail = carried.3;
        // The composer owns a pipeline built against the OLD device, so it is
        // deliberately not carried; the next frame rebuilds it lazily.
        generation
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn health(&self) -> RetainedGpuRendererHealth {
        self.health.current()
    }

    /// Refuse further work after an observed device fault. Device loss is
    /// recoverable by rebuilding `Pixels` and calling [`Self::recreate`];
    /// validation, internal, and OOM failures remain fatal.
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

    pub fn timestamp_queries_enabled(&self) -> bool {
        self.timestamp_profiler.is_some()
    }

    pub fn timestamp_telemetry(&self) -> GpuTimestampTelemetry {
        self.timestamp_history.telemetry
    }

    pub fn take_completed_timestamp_frames(
        &mut self,
        device: &wgpu::Device,
    ) -> Vec<GpuTimestampFrame> {
        if let Some(profiler) = self.timestamp_profiler.as_mut() {
            profiler.collect_completed(device, &mut self.timestamp_history);
        }
        std::mem::take(&mut self.timestamp_history.completed)
    }

    /// Wait for all submitted timestamp readbacks and return every completed
    /// raw frame. This is an explicit benchmark-boundary operation; normal
    /// frame rendering and [`Self::take_completed_timestamp_frames`] only poll.
    pub fn drain_timestamp_frames(
        &mut self,
        device: &wgpu::Device,
    ) -> Result<Vec<GpuTimestampFrame>, GpuRendererError> {
        if let Some(profiler) = self.timestamp_profiler.as_mut() {
            profiler.drain(device, &mut self.timestamp_history)?;
        }
        Ok(std::mem::take(&mut self.timestamp_history.completed))
    }

    fn commit_timestamp_frame(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        active: Option<ActiveGpuTimestampFrame>,
    ) -> Result<(), GpuRendererError> {
        // The asynchronous health callbacks may fire while passes are being
        // encoded. Do the final fallible health check before marking a slot as
        // in flight: an error tells the caller to discard this encoder, and the
        // uncommitted slot can then be reused safely on a healthy device.
        self.check_health()?;
        if let Some(active) = active {
            self.timestamp_profiler
                .as_mut()
                .expect("active timestamps require a profiler")
                .finish_frame(encoder, active);
        }
        Ok(())
    }

    /// True after this device has rejected a retained source or shader texture
    /// by dimension. Callers should use their CPU presentation path directly
    /// instead of retrying retained GPU capture every frame.
    pub fn requires_cpu_presentation(&self) -> bool {
        self.cpu_presentation_required
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

    /// Encodes an area-reduced copy of the most recently presented composition
    /// before the next render pass overwrites its retained target.
    ///
    /// The result is byte-identical to reducing a full readback with
    /// `clonk_graphics::surface::downsample_rgba_box`, but only `dest_extent`
    /// pixels are copied out: a 200x150 save thumbnail of a 4K frame maps
    /// about 117 KiB instead of about 31.6 MiB.
    ///
    /// Returns `Ok(None)` when nothing has been presented yet, and when the
    /// reduction is not provably exact for this source. Both cases leave
    /// [`Self::readback_last_presentation`] and a CPU reduction as the
    /// caller's fallback.
    pub fn readback_last_presentation_reduced(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        dest_extent: [u32; 2],
    ) -> Result<Option<GpuReadbackTicket>, GpuRendererError> {
        let monitor_gamma = self.last_presented_monitor_gamma;
        let (Some(composition), Some(monitor_gamma)) = (self.composition.as_ref(), monitor_gamma)
        else {
            return Ok(None);
        };
        if !reduction_accumulator_fits(composition.extent, dest_extent) {
            return Ok(None);
        }
        let source = if monitor_gamma {
            &composition.gamma_resolved_view
        } else {
            &composition.view
        };
        let reducer = self
            .presentation_reducer
            .get_or_insert_with(|| PresentationReducer::new(device));
        let reduced = reducer.reduce(device, encoder, source, composition.extent, dest_extent);
        encode_readback(device, encoder, reduced, dest_extent).map(Some)
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
        self.last_stats = GpuRendererStats::default();
        let validation_started = Instant::now();
        self.check_health()?;
        let base = layers.first().ok_or(GpuRendererError::NoSceneLayers)?;
        let resources = validate_layers(layers)?;
        let shader_landscape = self
            .pending_shader_landscape
            .as_ref()
            .filter(|_| self.shader_landscape)
            .map(|(_, plan)| plan);
        let limit = device.limits().max_texture_dimension_2d;
        if let Err(error) = validate_retained_texture_limits(
            &resources,
            base.presentation.physical_extent,
            shader_landscape,
            self.landscape_detail,
            limit,
        ) {
            self.cpu_presentation_required |= matches!(
                &error,
                GpuRendererError::TextureDimensionExceeded { kind, .. }
                    if kind.supports_cpu_fallback()
            );
            return Err(error);
        }
        let scene = base.scene;
        let texture_sync_started = Instant::now();
        self.last_stats.cpu_stages.validation =
            texture_sync_started.duration_since(validation_started);
        self.texture_epoch = self.texture_epoch.wrapping_add(1).max(1);
        self.sync_gamma(queue, scene);
        self.sync_textures(device, queue, &resources)?;
        let texture_sync_finished = Instant::now();
        self.last_stats.cpu_stages.texture_synchronization =
            texture_sync_finished.duration_since(texture_sync_started);
        let shader_landscape_draw_calls =
            usize::from(self.shader_landscape && self.pending_shader_landscape.is_some());
        let timestamp_queries_enabled = self.timestamp_profiler.is_some();
        let mut timestamp_frame = match self.timestamp_profiler.as_mut() {
            Some(profiler) => profiler.begin_frame(
                device,
                self.generation,
                queue.get_timestamp_period(),
                &mut self.timestamp_history,
            ),
            None => None,
        };
        self.last_stats.timestamp_frame_id =
            timestamp_frame.as_ref().map(|timestamp| timestamp.frame_id);
        let shader_timestamp_pair = (shader_landscape_draw_calls != 0)
            .then(|| {
                timestamp_frame
                    .as_mut()
                    .map(|timestamp| timestamp.reserve(GpuTimestampPass::ShaderLandscape))
            })
            .flatten();
        let shader_timestamp_writes = shader_timestamp_pair.map(|pair| {
            timestamp_frame
                .as_ref()
                .expect("timestamp pair has an active frame")
                .timestamp_writes(pair)
        });
        let command_encoding_started = if timestamp_queries_enabled {
            Instant::now()
        } else {
            texture_sync_finished
        };
        self.compose_shader_landscape(device, queue, encoder, &resources, shader_timestamp_writes)?;
        self.last_stats.shader_landscape_draw_calls = shader_landscape_draw_calls;

        let stream_started = Instant::now();
        let shader_encoding = stream_started.duration_since(command_encoding_started);
        let draw_stream = self.build_layered_draw_stream(layers)?;
        self.last_stats.record_draw_stream(&draw_stream);
        let BuiltDrawStream {
            vertices,
            quad_instances,
            sprite_instances,
            object_sprite_instances,
            landscape_instances,
            solid_rect_instances,
            calls,
        } = draw_stream;
        let vertex_bytes = packed_vertex_bytes(&vertices);
        let quad_instance_bytes = packed_quad_instance_bytes(&quad_instances);
        let sprite_instance_bytes = packed_sprite_instance_bytes(&sprite_instances);
        let object_sprite_instance_bytes =
            packed_object_sprite_instance_bytes(&object_sprite_instances);
        let landscape_instance_bytes = packed_landscape_instance_bytes(&landscape_instances);
        let solid_rect_instance_bytes = packed_solid_rect_instance_bytes(&solid_rect_instances);
        self.ensure_bind_groups(device, &calls)?;
        let mut used_quad_bindings = HashSet::new();
        let mut used_object_bindings = HashSet::new();
        let mut used_landscape_bindings = HashSet::new();
        for call in &calls {
            match call.kind {
                DrawKind::Quad(key) | DrawKind::Sprite(key) => {
                    used_quad_bindings.insert(key);
                }
                DrawKind::ObjectSprite(key) => {
                    used_object_bindings.insert(key.binding);
                }
                DrawKind::Landscape(key) | DrawKind::LandscapeInstance(key) => {
                    used_landscape_bindings.insert(key);
                }
                DrawKind::Solid { .. } | DrawKind::SolidRect { .. } => {}
            }
        }
        // Bind groups are cheap to recreate and can otherwise grow with every
        // historical combination of retained textures. Keep only bindings
        // reachable by this frame; source textures themselves follow the
        // larger bounded LRU below and survive temporary invisibility.
        self.quad_bind_groups
            .retain(|key, _| used_quad_bindings.contains(key));
        self.object_bind_groups
            .retain(|key, _| used_object_bindings.contains(key));
        self.landscape_bind_groups
            .retain(|key, _| used_landscape_bindings.contains(key));
        self.ensure_vertex_buffer(device, vertex_bytes.len())?;
        if !vertex_bytes.is_empty() {
            queue.write_buffer(&self.vertex_buffer, 0, vertex_bytes);
        }
        self.ensure_quad_instance_buffer(device, quad_instance_bytes.len())?;
        if !quad_instance_bytes.is_empty() {
            queue.write_buffer(&self.quad_instance_buffer, 0, quad_instance_bytes);
        }
        self.ensure_sprite_instance_buffer(device, sprite_instance_bytes.len())?;
        if !sprite_instance_bytes.is_empty() {
            queue.write_buffer(&self.sprite_instance_buffer, 0, sprite_instance_bytes);
        }
        self.ensure_object_sprite_instance_buffer(device, object_sprite_instance_bytes.len())?;
        if !object_sprite_instance_bytes.is_empty() {
            queue.write_buffer(
                &self.object_sprite_instance_buffer,
                0,
                object_sprite_instance_bytes,
            );
        }
        self.ensure_landscape_instance_buffer(device, landscape_instance_bytes.len())?;
        if !landscape_instance_bytes.is_empty() {
            queue.write_buffer(&self.landscape_instance_buffer, 0, landscape_instance_bytes);
        }
        self.ensure_solid_rect_instance_buffer(device, solid_rect_instance_bytes.len())?;
        if !solid_rect_instance_bytes.is_empty() {
            queue.write_buffer(
                &self.solid_rect_instance_buffer,
                0,
                solid_rect_instance_bytes,
            );
        }
        self.last_stats.monitor_gamma_draw_calls = usize::from(scene.gamma_mode.monitor_postpass());
        self.last_stats.presentation_draw_calls = 1;
        self.last_stats.total_draw_calls = calls.len()
            + shader_landscape_draw_calls
            + self.last_stats.monitor_gamma_draw_calls
            + self.last_stats.presentation_draw_calls;
        self.last_stats.resident_source_textures = self.textures.len();
        let encoding_resumed = Instant::now();
        self.last_stats.cpu_stages.stream_packing_upload =
            encoding_resumed.duration_since(stream_started);

        self.ensure_composition(device, base.presentation.physical_extent);
        let composition = self.composition.as_ref().expect("composition was created");
        let clear = scene.clear;
        let scene_timestamp_pair = timestamp_frame
            .as_mut()
            .map(|timestamp| timestamp.reserve(GpuTimestampPass::Scene));
        let scene_timestamp_writes = scene_timestamp_pair.map(|pair| {
            timestamp_frame
                .as_ref()
                .expect("timestamp pair has an active frame")
                .timestamp_writes(pair)
        });
        {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &composition.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(clear.r) / 255.0,
                        g: f64::from(clear.g) / 255.0,
                        b: f64::from(clear.b) / 255.0,
                        a: f64::from(clear.a) / 255.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lc_gpu_scene_pass"),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
                timestamp_writes: scene_timestamp_writes,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !calls.is_empty() {
                pass.set_bind_group(0, &self.gamma_bind_group, &[]);
            }
            self.encode_draw_calls(&mut pass, &calls);
        }

        if scene.gamma_mode.monitor_postpass() {
            let gamma_timestamp_pair = timestamp_frame
                .as_mut()
                .map(|timestamp| timestamp.reserve(GpuTimestampPass::MonitorGamma));
            let gamma_timestamp_writes = gamma_timestamp_pair.map(|pair| {
                timestamp_frame
                    .as_ref()
                    .expect("timestamp pair has an active frame")
                    .timestamp_writes(pair)
            });
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &composition.gamma_resolved_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lc_gpu_monitor_gamma_pass"),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
                timestamp_writes: gamma_timestamp_writes,
                occlusion_query_set: None,
                multiview_mask: None,
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

        let present_timestamp_pair = timestamp_frame
            .as_mut()
            .map(|timestamp| timestamp.reserve(GpuTimestampPass::Presentation));
        let present_timestamp_writes = present_timestamp_pair.map(|pair| {
            timestamp_frame
                .as_ref()
                .expect("timestamp pair has an active frame")
                .timestamp_writes(pair)
        });
        {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: surface_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("lc_gpu_present_pass"),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
                timestamp_writes: present_timestamp_writes,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.present_pipeline);
            pass.set_bind_group(0, presented_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        self.commit_timestamp_frame(encoder, timestamp_frame)?;

        let encoding_finished = Instant::now();
        self.last_stats.cpu_stages.command_encoding =
            shader_encoding.saturating_add(encoding_finished.duration_since(encoding_resumed));

        self.vertex_scratch = vertices;
        self.quad_instance_scratch = quad_instances;
        self.sprite_instance_scratch = sprite_instances;
        self.object_sprite_instance_scratch = object_sprite_instances;
        self.landscape_instance_scratch = landscape_instances;
        self.solid_rect_instance_scratch = solid_rect_instances;
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
            wgpu::TexelCopyTextureInfo {
                texture: &self.gamma_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
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

    /// Opt in to trilinear + anisotropic minification for retained art. C++
    /// binds GL_LINEAR with no mip chain, so this is off unless configured.
    pub fn set_mipmaps(&mut self, mipmaps: bool) {
        if self.mipmaps == mipmaps {
            return;
        }
        self.mipmaps = mipmaps;
        // Level counts are fixed at creation and bind groups cache the
        // sampler, so both have to be rebuilt against the new policy.
        self.textures.clear();
        self.quad_bind_groups.clear();
        self.object_bind_groups.clear();
    }

    /// Opt in to alpha-weighted magnification of the landscape. C++ blits the
    /// landscape surface with GL_NEAREST, so a magnified terrain is hard
    /// blocks; this reconstructs it without pulling the fully transparent sky
    /// into the silhouette.
    pub fn set_smooth_landscape(&mut self, smooth: bool) {
        self.smooth_landscape = smooth;
    }

    pub fn mipmaps(&self) -> bool {
        self.mipmaps
    }

    pub fn smooth_landscape(&self) -> bool {
        self.smooth_landscape
    }

    /// Opt in to composing the landscape in the fragment shader instead of on
    /// the CPU. The CPU composer walks integer landscape coordinates, so one
    /// pattern texel per landscape pixel is its ceiling; this evaluates the
    /// identical arithmetic per fragment, which is what lets `landscape_detail`
    /// resolve finer material art (`ShaderLandscapeComposer`).
    pub fn set_shader_landscape(&mut self, shader: bool) {
        if self.shader_landscape == shader {
            return;
        }
        self.shader_landscape = shader;
        self.pending_shader_landscape = None;
        self.invalidate_shader_landscape_outputs();
    }

    pub fn shader_landscape(&self) -> bool {
        self.shader_landscape
    }

    /// Landscape supersampling factor for the shader composer. 1 reproduces the
    /// CPU composer byte for byte; N evaluates the pattern at 1/N of a
    /// landscape pixel, so N-times-larger material art keeps its world-space
    /// tiling period instead of stretching it. Clamped to the range the
    /// composer accepts — 0 is a validation error there.
    pub fn set_landscape_detail(&mut self, detail: u32) {
        let detail = detail.clamp(1, MAX_LANDSCAPE_DETAIL);
        if self.landscape_detail == detail {
            return;
        }
        self.landscape_detail = detail;
        self.invalidate_shader_landscape_outputs();
    }

    pub fn landscape_detail(&self) -> u32 {
        self.landscape_detail
    }

    fn invalidate_shader_landscape_outputs(&mut self) {
        self.textures
            .retain(|_, texture| matches!(texture.contents, CachedTextureContents::Source));
        self.quad_bind_groups.clear();
        self.object_bind_groups.clear();
        self.landscape_bind_groups.clear();
    }

    /// Hand the next frame's landscape composition inputs to the renderer.
    ///
    /// Kept off `GpuScene` on purpose: the plan is frame state, not retained
    /// scene content, and threading it through the recorder would put a
    /// multi-megabyte index plane and atlas into every scene literal.
    pub fn set_pending_shader_landscape(
        &mut self,
        plan: Option<(GpuTextureId, clonk_graphics::ShaderLandscapePlan)>,
    ) {
        self.pending_shader_landscape = plan;
    }

    /// Replace a CPU-composed landscape texture with a shader-composed one.
    ///
    /// Runs after `sync_textures`, so the texture the plan names already exists
    /// with the CPU composition uploaded. Composing over it keeps every
    /// downstream lookup — bind groups, `base_extent`, the liquid scale —
    /// working unchanged, and the landscape quad's UVs are normalized, so a
    /// `detail > 1` plane simply samples finer.
    fn compose_shader_landscape(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        resources: &[GpuTextureResource],
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) -> Result<(), GpuRendererError> {
        // Always take the plan: a frame that does not compose must not leave a
        // stale landscape queued for the next one.
        let Some((id, plan)) = self.pending_shader_landscape.take() else {
            return Ok(());
        };
        if !self.shader_landscape {
            return Ok(());
        }
        let source = resources
            .iter()
            .find(|resource| resource.id == id)
            .ok_or(GpuRendererError::MissingTexture(id))?;
        if source.format != GpuTextureFormat::Rgba8 {
            return Err(GpuRendererError::TextureFormatMismatch {
                id,
                expected: GpuTextureFormat::Rgba8,
                actual: source.format,
            });
        }
        let slots: Vec<ShaderLandscapeSlot> = plan
            .slots
            .iter()
            .map(|words| ShaderLandscapeSlot {
                colors: [words[0], words[1], words[2], words[3]],
                params: [words[4], words[5], words[6], words[7]],
                primary: [words[8], words[9], words[10], words[11]],
                overlay: [words[12], words[13], words[14], words[15]],
            })
            .collect();
        let inputs = ShaderLandscapeInputs {
            extent: plan.extent,
            index_plane: &plan.index_plane,
            shading_plane: plan.shading_plane.as_deref(),
            atlas: &plan.atlas,
            atlas_extent: plan.atlas_extent,
            slots: &slots,
            detail: self.landscape_detail,
        };
        validate_shader_landscape_texture_limits(
            &inputs,
            device.limits().max_texture_dimension_2d,
        )?;
        let extent = inputs.composed_extent();
        // The composition pass clears and rewrites every texel of its target,
        // so an output of the right extent can be composed into again. Keeping
        // it also keeps the bind groups that name it valid.
        let retained_output = self
            .textures
            .remove(&id)
            .filter(|cached| {
                cached.contents == CachedTextureContents::ShaderLandscape
                    && cached.extent == extent
                    && cached.format == GpuTextureFormat::Rgba8
            })
            .map(|cached| (cached._texture, cached.view));
        let recreated = retained_output.is_none();
        let composer = self
            .landscape_composer
            .get_or_insert_with(|| ShaderLandscapeComposer::new(device));
        let (texture, view) = retained_output.unwrap_or_else(|| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("lc_gpu_shader_landscape_composed"),
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
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            (texture, view)
        });
        composer.compose_into_profiled(
            device,
            queue,
            encoder,
            &view,
            inputs,
            !recreated,
            timestamp_writes,
        )?;
        let uploads = composer.last_uploads();
        let composed_texels = composer.last_composed_texels();

        let byte_len = u64::from(extent[0]) * u64::from(extent[1]) * 4;
        self.textures.insert(
            id,
            CachedTexture {
                revision: source.revision,
                source_extent: source.extent,
                source_format: source.format,
                contents: CachedTextureContents::ShaderLandscape,
                extent,
                format: GpuTextureFormat::Rgba8,
                byte_len,
                last_used_epoch: self.texture_epoch,
                _texture: texture,
                view,
            },
        );
        self.last_stats.created_shader_landscape_outputs += usize::from(recreated);
        self.last_stats.shader_landscape_upload_calls += uploads.calls;
        self.last_stats.shader_landscape_upload_bytes += uploads.bytes;
        self.last_stats.shader_landscape_composed_texels += composed_texels;
        if recreated {
            self.quad_bind_groups.clear();
            self.object_bind_groups.clear();
            self.landscape_bind_groups.clear();
        }
        Ok(())
    }

    fn sync_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        resources: &[GpuTextureResource],
    ) -> Result<(), GpuRendererError> {
        let mut live = HashSet::with_capacity(resources.len());
        let mut replaced = HashSet::new();
        let pending_shader_landscape = self
            .shader_landscape
            .then(|| self.pending_shader_landscape.as_ref().map(|(id, _)| *id))
            .flatten();
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

            let (preserve_shader_output, recreate) =
                self.textures
                    .get(&resource.id)
                    .map_or((false, true), |cached| {
                        let preserve = cached.preserves_shader_output(
                            self.shader_landscape,
                            pending_shader_landscape,
                            resource,
                        );
                        let recreate = !preserve
                            && (cached.contents == CachedTextureContents::ShaderLandscape
                                || cached.source_extent != resource.extent
                                || cached.source_format != resource.format);
                        (preserve, recreate)
                    });
            if preserve_shader_output {
                self.textures
                    .get_mut(&resource.id)
                    .expect("preserved shader landscape exists")
                    .last_used_epoch = self.texture_epoch;
                continue;
            }
            if recreate {
                let texture = create_source_texture(device, resource, self.mipmaps);
                let upload = upload_full(queue, &texture, resource);
                self.last_stats.created_source_textures += 1;
                self.last_stats.record_full_texture_upload(upload);
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                self.textures.insert(
                    resource.id,
                    CachedTexture {
                        revision: resource.revision,
                        source_extent: resource.extent,
                        source_format: resource.format,
                        contents: CachedTextureContents::Source,
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
                    let upload = upload_full(queue, &cached._texture, resource);
                    self.last_stats.record_full_texture_upload(upload);
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
                        self.last_stats.record_dirty_texture_upload(bytes);
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
        self.object_bind_groups.retain(|key, _| {
            [Some(key.texture), key.owner_texture]
                .into_iter()
                .flatten()
                .all(|id| retained.contains(&id) && !replaced.contains(&id))
        });
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
    ) -> Result<BuiltDrawStream, GpuRendererError> {
        let mut vertices = std::mem::take(&mut self.vertex_scratch);
        let mut quad_instances = std::mem::take(&mut self.quad_instance_scratch);
        let mut sprite_instances = std::mem::take(&mut self.sprite_instance_scratch);
        let mut object_sprite_instances = std::mem::take(&mut self.object_sprite_instance_scratch);
        let mut landscape_instances = std::mem::take(&mut self.landscape_instance_scratch);
        let mut solid_rect_instances = std::mem::take(&mut self.solid_rect_instance_scratch);
        let mut calls = std::mem::take(&mut self.draw_call_scratch);
        vertices.clear();
        quad_instances.clear();
        sprite_instances.clear();
        object_sprite_instances.clear();
        landscape_instances.clear();
        solid_rect_instances.clear();
        calls.clear();
        calls.reserve(layers.iter().map(|layer| layer.scene.commands.len()).sum());
        for layer in layers {
            let layer_call_start = calls.len();
            self.append_draw_stream(
                layer.scene,
                &layer.presentation,
                &mut vertices,
                &mut quad_instances,
                &mut sprite_instances,
                &mut object_sprite_instances,
                &mut landscape_instances,
                &mut solid_rect_instances,
                &mut calls,
                layer_call_start,
            )?;
        }
        Ok(BuiltDrawStream {
            vertices,
            quad_instances,
            sprite_instances,
            object_sprite_instances,
            landscape_instances,
            solid_rect_instances,
            calls,
        })
    }

    // Keep the independently typed upload streams explicit at this packing boundary.
    #[allow(clippy::too_many_arguments)]
    fn append_draw_stream(
        &self,
        scene: &GpuScene,
        presentation: &GpuPresentation,
        vertices: &mut Vec<PackedVertex>,
        quad_instances: &mut Vec<PackedQuadInstance>,
        sprite_instances: &mut Vec<PackedSpriteInstance>,
        object_sprite_instances: &mut Vec<PackedObjectSpriteInstance>,
        landscape_instances: &mut Vec<PackedLandscapeInstance>,
        solid_rect_instances: &mut Vec<PackedSolidRectInstance>,
        calls: &mut Vec<DrawCall>,
        layer_call_start: usize,
    ) -> Result<(), GpuRendererError> {
        let mut commands = scene.commands.iter().peekable();
        while let Some(command) = commands.next() {
            match command {
                GpuCommand::Quad { owner_mask, .. } => {
                    if owner_mask.is_some() {
                        return Err(GpuRendererError::OwnerMaskNotLowered);
                    }
                    let Some(run) = quad_run_key(command) else {
                        return Err(GpuRendererError::OwnerMaskNotLowered);
                    };
                    self.require_format(run.binding.texture, GpuTextureFormat::Rgba8)?;
                    let Some(projection) =
                        draw_projection(run.clip, scene.logical_extent, presentation)?
                    else {
                        while commands.peek().is_some_and(|next| {
                            matches!(next, GpuCommand::Quad { .. })
                                && quad_run_key(next) == Some(run)
                        }) {
                            let _ = commands.next();
                        }
                        continue;
                    };
                    append_prepared_quad_command(
                        quad_instances,
                        calls,
                        layer_call_start,
                        command,
                        scene.gamma_mode,
                        &projection,
                        run,
                    )?;
                    while commands.peek().is_some_and(|next| {
                        matches!(next, GpuCommand::Quad { .. }) && quad_run_key(next) == Some(run)
                    }) {
                        if let Some(next) = commands.next() {
                            append_prepared_quad_command(
                                quad_instances,
                                calls,
                                layer_call_start,
                                next,
                                scene.gamma_mode,
                                &projection,
                                run,
                            )?;
                        }
                    }
                }
                GpuCommand::SpriteBatch {
                    quads, mod2, gamma, ..
                } => {
                    if quads.is_empty() {
                        continue;
                    }
                    let run = quad_run_key(command)
                        .expect("sprite batches always have a textured run key");
                    self.require_format(run.binding.texture, GpuTextureFormat::Rgba8)?;
                    let Some(projection) =
                        draw_projection(run.clip, scene.logical_extent, presentation)?
                    else {
                        continue;
                    };
                    let start = sprite_instance_count(sprite_instances)?;
                    let gamma = fragment_gamma_flag(scene.gamma_mode, *gamma);
                    let sprite_projection = SpriteProjection::new(&projection);
                    for quad in quads {
                        sprite_instances.push(packed_sprite_instance(
                            *quad,
                            *mod2,
                            gamma,
                            sprite_projection,
                        )?);
                    }
                    DrawCall::push_compatible_quad(
                        calls,
                        layer_call_start,
                        DrawCall {
                            vertices: start..sprite_instance_count(sprite_instances)?,
                            scissor: projection.scissor,
                            blend: run.blend,
                            kind: DrawKind::Sprite(run.binding),
                        },
                    );
                }
                GpuCommand::ObjectBatch { sprites, gamma, .. } => {
                    if sprites.is_empty() {
                        continue;
                    }
                    let run = object_run_key(command)
                        .expect("object batches always have a textured run key");
                    self.require_format(run.binding.texture, GpuTextureFormat::Rgba8)?;
                    if let Some(owner_texture) = run.binding.owner_texture {
                        self.require_format(owner_texture, GpuTextureFormat::Rgba8)?;
                    }
                    let Some(projection) =
                        draw_projection(run.clip, scene.logical_extent, presentation)?
                    else {
                        continue;
                    };
                    let start = object_sprite_instance_count(object_sprite_instances)?;
                    let gamma = fragment_gamma_flag(scene.gamma_mode, *gamma);
                    for sprite in sprites {
                        object_sprite_instances.push(packed_object_sprite_instance(
                            *sprite,
                            gamma,
                            &projection,
                        )?);
                    }
                    DrawCall::push_compatible_quad(
                        calls,
                        layer_call_start,
                        DrawCall {
                            vertices: start..object_sprite_instance_count(object_sprite_instances)?,
                            scissor: projection.scissor,
                            blend: run.blend,
                            kind: DrawKind::ObjectSprite(run),
                        },
                    );
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
                    let gamma = fragment_gamma_flag(scene.gamma_mode, *gamma);
                    let packed = [
                        packed_landscape_vertex(
                            quad[0],
                            liquid_scale,
                            *phase,
                            gamma,
                            self.smooth_landscape,
                            &projection,
                        )?,
                        packed_landscape_vertex(
                            quad[1],
                            liquid_scale,
                            *phase,
                            gamma,
                            self.smooth_landscape,
                            &projection,
                        )?,
                        packed_landscape_vertex(
                            quad[2],
                            liquid_scale,
                            *phase,
                            gamma,
                            self.smooth_landscape,
                            &projection,
                        )?,
                        packed_landscape_vertex(
                            quad[3],
                            liquid_scale,
                            *phase,
                            gamma,
                            self.smooth_landscape,
                            &projection,
                        )?,
                    ];
                    let (start, end, kind) = match try_packed_landscape_instance(
                        packed,
                        liquid_scale,
                        *phase,
                        gamma,
                        self.smooth_landscape,
                    ) {
                        Some(instance) => {
                            let start = landscape_instance_count(landscape_instances)?;
                            landscape_instances.push(instance);
                            (
                                start,
                                landscape_instance_count(landscape_instances)?,
                                DrawKind::LandscapeInstance(LandscapeBindingKey {
                                    base: *base,
                                    mask: *liquid_mask,
                                    liquid: *liquid,
                                }),
                            )
                        }
                        None => {
                            let start = vertex_count(vertices)?;
                            for index in [0, 1, 2, 2, 1, 3] {
                                append_vertex(vertices, packed[index]);
                            }
                            (
                                start,
                                vertex_count(vertices)?,
                                DrawKind::Landscape(LandscapeBindingKey {
                                    base: *base,
                                    mask: *liquid_mask,
                                    liquid: *liquid,
                                }),
                            )
                        }
                    };
                    DrawCall::push_compatible_quad(
                        calls,
                        layer_call_start,
                        DrawCall {
                            vertices: start..end,
                            scissor: projection.scissor,
                            blend: GpuBlend::Normal,
                            kind,
                        },
                    );
                }
                GpuCommand::Solid {
                    vertices: solid,
                    topology,
                    alpha_mode,
                    clip,
                    blend,
                    style,
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
                    if !solid
                        .iter()
                        .flat_map(|vertex| vertex.color)
                        .all(f32::is_finite)
                    {
                        return Err(GpuRendererError::NonFiniteCoordinate);
                    }
                    let gamma = fragment_gamma_flag(scene.gamma_mode, style.gamma);
                    // Points and line fragments resolve to whole physical
                    // rectangles, so they share one compact instance stream and
                    // coalesce across commands; only interpolated triangles
                    // still need the generic vertex stream.
                    let (start, end, kind) = match topology {
                        GpuPrimitiveTopology::PointList => {
                            let start = solid_rect_instance_count(solid_rect_instances)?;
                            for vertex in solid {
                                if let Some(point) = packed_point_rect(*vertex, gamma, &projection)?
                                {
                                    solid_rect_instances.push(point);
                                }
                            }
                            (
                                start,
                                solid_rect_instance_count(solid_rect_instances)?,
                                DrawKind::SolidRect {
                                    alpha_mode: *alpha_mode,
                                },
                            )
                        }
                        GpuPrimitiveTopology::LineList => {
                            let start = solid_rect_instance_count(solid_rect_instances)?;
                            for pair in solid.chunks_exact(2) {
                                append_line_fragment_instances(
                                    solid_rect_instances,
                                    pair[0],
                                    pair[1],
                                    gamma,
                                    &projection,
                                )?;
                            }
                            (
                                start,
                                solid_rect_instance_count(solid_rect_instances)?,
                                DrawKind::SolidRect {
                                    alpha_mode: *alpha_mode,
                                },
                            )
                        }
                        GpuPrimitiveTopology::TriangleList => {
                            let start = vertex_count(vertices)?;
                            for vertex in solid {
                                append_vertex(
                                    vertices,
                                    packed_solid_vertex(
                                        vertex.position,
                                        vertex.color,
                                        gamma,
                                        style.dither,
                                        &projection,
                                    )?,
                                );
                            }
                            (
                                start,
                                vertex_count(vertices)?,
                                DrawKind::Solid {
                                    alpha_mode: *alpha_mode,
                                },
                            )
                        }
                    };
                    if start != end {
                        DrawCall::push_compatible_quad(
                            calls,
                            layer_call_start,
                            DrawCall {
                                vertices: start..end,
                                scissor: projection.scissor,
                                blend: *blend,
                                kind,
                            },
                        );
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
                DrawKind::Quad(key) | DrawKind::Sprite(key)
                    if !self.quad_bind_groups.contains_key(&key) =>
                {
                    let texture = self
                        .textures
                        .get(&key.texture)
                        .ok_or(GpuRendererError::MissingTexture(key.texture))?;
                    let sampler = if key.sampler == sampler_key(GpuSampler::Nearest) {
                        &self.nearest_sampler
                    } else if self.mipmaps {
                        &self.linear_mip_sampler
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
                DrawKind::ObjectSprite(run)
                    if !self.object_bind_groups.contains_key(&run.binding) =>
                {
                    let key = run.binding;
                    let texture = self
                        .textures
                        .get(&key.texture)
                        .ok_or(GpuRendererError::MissingTexture(key.texture))?;
                    let owner = match key.owner_texture {
                        Some(id) => self
                            .textures
                            .get(&id)
                            .ok_or(GpuRendererError::MissingTexture(id))?,
                        None => texture,
                    };
                    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("lc_gpu_object_sprite_bind_group"),
                        layout: &self.object_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&texture.view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&self.nearest_sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::TextureView(&owner.view),
                            },
                        ],
                    });
                    self.object_bind_groups.insert(key, bind_group);
                }
                DrawKind::Landscape(key) | DrawKind::LandscapeInstance(key)
                    if !self.landscape_bind_groups.contains_key(&key) =>
                {
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

    fn ensure_quad_instance_buffer(
        &mut self,
        device: &wgpu::Device,
        required: usize,
    ) -> Result<(), GpuRendererError> {
        let required =
            u64::try_from(required).map_err(|_| GpuRendererError::VertexRangeOverflow)?;
        if required <= self.quad_instance_buffer_size {
            return Ok(());
        }
        let size = required
            .checked_next_power_of_two()
            .ok_or(GpuRendererError::VertexRangeOverflow)?
            .max(INITIAL_VERTEX_BUFFER_SIZE);
        self.quad_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lc_gpu_quad_instances"),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.quad_instance_buffer_size = size;
        Ok(())
    }

    fn ensure_sprite_instance_buffer(
        &mut self,
        device: &wgpu::Device,
        required: usize,
    ) -> Result<(), GpuRendererError> {
        let required =
            u64::try_from(required).map_err(|_| GpuRendererError::VertexRangeOverflow)?;
        if required <= self.sprite_instance_buffer_size {
            return Ok(());
        }
        let size = required
            .checked_next_power_of_two()
            .ok_or(GpuRendererError::VertexRangeOverflow)?
            .max(INITIAL_VERTEX_BUFFER_SIZE);
        self.sprite_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lc_gpu_sprite_instances"),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.sprite_instance_buffer_size = size;
        Ok(())
    }

    fn ensure_object_sprite_instance_buffer(
        &mut self,
        device: &wgpu::Device,
        required: usize,
    ) -> Result<(), GpuRendererError> {
        let required =
            u64::try_from(required).map_err(|_| GpuRendererError::VertexRangeOverflow)?;
        if required <= self.object_sprite_instance_buffer_size {
            return Ok(());
        }
        let size = required
            .checked_next_power_of_two()
            .ok_or(GpuRendererError::VertexRangeOverflow)?
            .max(INITIAL_VERTEX_BUFFER_SIZE);
        self.object_sprite_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lc_gpu_object_sprite_instances"),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.object_sprite_instance_buffer_size = size;
        Ok(())
    }

    fn ensure_landscape_instance_buffer(
        &mut self,
        device: &wgpu::Device,
        required: usize,
    ) -> Result<(), GpuRendererError> {
        let required =
            u64::try_from(required).map_err(|_| GpuRendererError::VertexRangeOverflow)?;
        if required <= self.landscape_instance_buffer_size {
            return Ok(());
        }
        let size = required
            .checked_next_power_of_two()
            .ok_or(GpuRendererError::VertexRangeOverflow)?
            .max(INITIAL_VERTEX_BUFFER_SIZE);
        self.landscape_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lc_gpu_landscape_instances"),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.landscape_instance_buffer_size = size;
        Ok(())
    }

    fn ensure_solid_rect_instance_buffer(
        &mut self,
        device: &wgpu::Device,
        required: usize,
    ) -> Result<(), GpuRendererError> {
        let required =
            u64::try_from(required).map_err(|_| GpuRendererError::VertexRangeOverflow)?;
        if required <= self.solid_rect_instance_buffer_size {
            return Ok(());
        }
        let size = required
            .checked_next_power_of_two()
            .ok_or(GpuRendererError::VertexRangeOverflow)?
            .max(INITIAL_VERTEX_BUFFER_SIZE);
        self.solid_rect_instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lc_gpu_solid_rect_instances"),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.solid_rect_instance_buffer_size = size;
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

    fn solid_rect_pipeline(
        &self,
        blend: GpuBlend,
        alpha_mode: GpuSolidAlphaMode,
    ) -> &wgpu::RenderPipeline {
        match blend {
            GpuBlend::Replace => &self.solid_rect_replace_pipeline,
            GpuBlend::Normal => match alpha_mode {
                GpuSolidAlphaMode::SourceOver => &self.solid_rect_over_normal_pipeline,
                GpuSolidAlphaMode::NonSeparate => &self.solid_rect_non_separate_normal_pipeline,
            },
            GpuBlend::Additive => &self.solid_rect_additive_pipeline,
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
                    pass.set_vertex_buffer(0, self.quad_instance_buffer.slice(..));
                    pass.set_index_buffer(
                        self.quad_index_buffer.slice(..),
                        wgpu::IndexFormat::Uint16,
                    );
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
                    pass.draw_indexed(0..6, 0, call.vertices.clone());
                }
                DrawKind::Sprite(key) => {
                    pass.set_vertex_buffer(0, self.sprite_instance_buffer.slice(..));
                    pass.set_index_buffer(
                        self.quad_index_buffer.slice(..),
                        wgpu::IndexFormat::Uint16,
                    );
                    pass.set_pipeline(match call.blend {
                        GpuBlend::Replace => &self.sprite_replace_pipeline,
                        GpuBlend::Normal => &self.sprite_normal_pipeline,
                        GpuBlend::Additive => &self.sprite_additive_pipeline,
                    });
                    pass.set_bind_group(
                        1,
                        self.quad_bind_groups
                            .get(&key)
                            .expect("sprite binding was prepared"),
                        &[],
                    );
                    pass.draw_indexed(0..6, 0, call.vertices.clone());
                }
                DrawKind::ObjectSprite(key) => {
                    pass.set_vertex_buffer(0, self.object_sprite_instance_buffer.slice(..));
                    pass.set_index_buffer(
                        self.quad_index_buffer.slice(..),
                        wgpu::IndexFormat::Uint16,
                    );
                    pass.set_pipeline(match call.blend {
                        GpuBlend::Replace => &self.object_sprite_replace_pipeline,
                        GpuBlend::Normal => &self.object_sprite_normal_pipeline,
                        GpuBlend::Additive => &self.object_sprite_additive_pipeline,
                    });
                    pass.set_bind_group(
                        1,
                        self.object_bind_groups
                            .get(&key.binding)
                            .expect("object sprite binding was prepared"),
                        &[],
                    );
                    pass.draw_indexed(0..6, 0, call.vertices.clone());
                }
                DrawKind::Landscape(key) => {
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.set_pipeline(&self.landscape_pipeline);
                    pass.set_bind_group(
                        1,
                        self.landscape_bind_groups
                            .get(&key)
                            .expect("landscape binding was prepared"),
                        &[],
                    );
                    pass.draw(call.vertices.clone(), 0..1);
                }
                DrawKind::LandscapeInstance(key) => {
                    pass.set_vertex_buffer(0, self.landscape_instance_buffer.slice(..));
                    pass.set_index_buffer(
                        self.quad_index_buffer.slice(..),
                        wgpu::IndexFormat::Uint16,
                    );
                    pass.set_pipeline(&self.landscape_instance_pipeline);
                    pass.set_bind_group(
                        1,
                        self.landscape_bind_groups
                            .get(&key)
                            .expect("landscape binding was prepared"),
                        &[],
                    );
                    pass.draw_indexed(0..6, 0, call.vertices.clone());
                }
                DrawKind::Solid { alpha_mode } => {
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.set_pipeline(self.solid_pipeline(call.blend, alpha_mode));
                    pass.draw(call.vertices.clone(), 0..1);
                }
                DrawKind::SolidRect { alpha_mode } => {
                    pass.set_vertex_buffer(0, self.solid_rect_instance_buffer.slice(..));
                    pass.set_index_buffer(
                        self.quad_index_buffer.slice(..),
                        wgpu::IndexFormat::Uint16,
                    );
                    pass.set_pipeline(self.solid_rect_pipeline(call.blend, alpha_mode));
                    pass.draw_indexed(0..6, 0, call.vertices.clone());
                }
            }
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

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct PackedQuadInstance {
    clip: [[f32; 4]; 4],
    uv: [[f32; 4]; 2],
    modulation: [[f32; 4]; 4],
    sample_tile: [[f32; 4]; 4],
    flags: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct PackedSpriteInstance {
    clip_rect: [f32; 4],
    uv_rect: [f32; 4],
    modulation: u32,
    flags: u32,
}

/// One byte-exact, axis-aligned retained landscape command.
///
/// Full fog chunks and both canonical NoBoxFades triangles share this layout.
/// More general projective geometry or non-C4 modulation stays on the generic
/// vertex stream.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct PackedLandscapeInstance {
    clip_rect: [f32; 4],
    uv_rect: [f32; 4],
    modulation: [u32; 4],
    liquid_scale: [f32; 2],
    phase: [f32; 3],
    flags: u32,
}

const _: () = {
    assert!(std::mem::size_of::<PackedLandscapeInstance>() == 72);
    assert!(std::mem::align_of::<PackedLandscapeInstance>() == 4);
    assert!(PACKED_LANDSCAPE_INSTANCE_STRIDE <= LANDSCAPE_INSTANCE_BYTE_BUDGET);
};

/// One axis-aligned physical rectangle of flat color.
///
/// Aliased points and line fragments are whole physical pixels, so a rectangle
/// and its color say everything the rasterizer needs. The corners are stored
/// already projected to clip space, which keeps the exact `f32` values the
/// triangle-pair lowering fed to the vertex stage.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
struct PackedSolidRectInstance {
    clip_rect: [f32; 4],
    color: [f32; 4],
    flags: u32,
}

const _: () = {
    assert!(PACKED_SOLID_RECT_INSTANCE_STRIDE <= SOLID_RECT_INSTANCE_BYTE_BUDGET);
};

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct PackedObjectSpriteInstance {
    clip: [[f32; 3]; 4],
    uv_rect: [f32; 4],
    modulation: [u32; 4],
    sample_tile_size: f32,
    flags: u32,
}

fn packed_quad_instance(
    quad: [GpuVertex; 4],
    mod2: bool,
    gamma: bool,
    projection: &DrawProjection,
) -> Result<PackedQuadInstance, GpuRendererError> {
    let packed = [
        packed_quad_vertex(quad[0], mod2, gamma, projection)?,
        packed_quad_vertex(quad[1], mod2, gamma, projection)?,
        packed_quad_vertex(quad[2], mod2, gamma, projection)?,
        packed_quad_vertex(quad[3], mod2, gamma, projection)?,
    ];
    Ok(PackedQuadInstance {
        clip: packed.map(|vertex| vertex.clip),
        uv: [
            [
                packed[0].uv[0],
                packed[0].uv[1],
                packed[1].uv[0],
                packed[1].uv[1],
            ],
            [
                packed[2].uv[0],
                packed[2].uv[1],
                packed[3].uv[0],
                packed[3].uv[1],
            ],
        ],
        modulation: packed.map(|vertex| vertex.data0),
        sample_tile: packed.map(|vertex| vertex.data2),
        flags: [flag(mod2), flag(gamma)],
    })
}

fn packed_sprite_instance(
    quad: GpuSpriteQuad,
    mod2: bool,
    gamma: bool,
    projection: SpriteProjection,
) -> Result<PackedSpriteInstance, GpuRendererError> {
    Ok(PackedSpriteInstance {
        clip_rect: projection.clip_rect(quad.rect)?,
        uv_rect: quad.uv,
        modulation: quad.modulation,
        flags: u32::from(mod2) | (u32::from(gamma) << 1),
    })
}

fn packed_object_sprite_instance(
    sprite: GpuObjectSprite,
    gamma: bool,
    projection: &DrawProjection,
) -> Result<PackedObjectSpriteInstance, GpuRendererError> {
    if !sprite
        .uv
        .iter()
        .chain(std::iter::once(&sprite.sample_tile_size))
        .all(|value| value.is_finite())
    {
        return Err(GpuRendererError::NonFiniteCoordinate);
    }
    validate_object_sprite_flags(sprite)?;
    validate_object_sprite_sample_tile(sprite)?;
    let mut clip = [[0.0; 3]; 4];
    for (destination, position) in clip.iter_mut().zip(sprite.positions) {
        let [x, y, _, w] = clip_position(position, projection)?;
        *destination = [x, y, w];
    }
    Ok(PackedObjectSpriteInstance {
        clip,
        uv_rect: sprite.uv,
        modulation: sprite.modulation,
        sample_tile_size: sprite.sample_tile_size,
        flags: sprite.packed_flags() | (u32::from(gamma) << 4),
    })
}

fn validate_object_sprite_flags(sprite: GpuObjectSprite) -> Result<(), GpuRendererError> {
    sprite.has_valid_packed_flags().then_some(()).ok_or(
        GpuRendererError::InvalidObjectSpriteFlags {
            flags: sprite.packed_flags(),
        },
    )
}

fn validate_object_sprite_sample_tile(sprite: GpuObjectSprite) -> Result<(), GpuRendererError> {
    let valid = match sprite.sampler() {
        GpuSampler::Nearest => sprite.sample_tile_size == 0.0,
        GpuSampler::Linear => {
            // C4Surface::CreateTextures produces integral power-of-two tiles
            // from 2 through its 4096 maximum (C4Surface.cpp:166-189).
            let size = sprite.sample_tile_size as u32;
            (2..=4096).contains(&size)
                && size.is_power_of_two()
                && sprite.sample_tile_size == size as f32
        }
    };
    valid
        .then_some(())
        .ok_or(GpuRendererError::InvalidObjectSpriteSampleTile {
            sampler: sprite.sampler(),
            sample_tile_size: sprite.sample_tile_size,
        })
}

#[allow(clippy::too_many_arguments)]
fn append_prepared_quad_command(
    quad_instances: &mut Vec<PackedQuadInstance>,
    calls: &mut Vec<DrawCall>,
    batch_start: usize,
    command: &GpuCommand,
    gamma_mode: GpuGammaMode,
    projection: &DrawProjection,
    run: QuadRunKey,
) -> Result<(), GpuRendererError> {
    let mut append = |instance| -> Result<(), GpuRendererError> {
        let start = quad_instance_count(quad_instances)?;
        quad_instances.push(instance);
        DrawCall::push_compatible_quad(
            calls,
            batch_start,
            DrawCall {
                vertices: start..quad_instance_count(quad_instances)?,
                scissor: projection.scissor,
                blend: run.blend,
                kind: DrawKind::Quad(run.binding),
            },
        );
        Ok(())
    };
    if let GpuCommand::Quad {
        vertices,
        base_mod2,
        gamma,
        ..
    } = command
    {
        append(packed_quad_instance(
            *vertices,
            *base_mod2,
            fragment_gamma_flag(gamma_mode, *gamma),
            projection,
        )?)?;
    }
    Ok(())
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
    smooth: bool,
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
        data1: [liquid_scale[0], liquid_scale[1], flag(smooth), 0.0],
        data2: [phase[0], phase[1], phase[2], flag(gamma)],
    })
}

fn exact_normalized_byte(value: f32) -> Option<u8> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return None;
    }
    let byte = (value * 255.0).round() as u8;
    ((f32::from(byte) / 255.0).to_bits() == value.to_bits()).then_some(byte)
}

fn packed_c4_modulation(modulation: [f32; 4]) -> Option<u32> {
    let [red, green, blue, transparency] = modulation.map(exact_normalized_byte);
    Some(
        (u32::from(transparency?) << 24)
            | (u32::from(red?) << 16)
            | (u32::from(green?) << 8)
            | u32::from(blue?),
    )
}

fn same_float(left: f32, right: f32) -> bool {
    left.to_bits() == right.to_bits()
}

fn same_xy(left: [f32; 2], right: [f32; 2]) -> bool {
    same_float(left[0], right[0]) && same_float(left[1], right[1])
}

fn try_packed_landscape_instance(
    vertices: [PackedVertex; 4],
    liquid_scale: [f32; 2],
    phase: [f32; 3],
    gamma: bool,
    smooth: bool,
) -> Option<PackedLandscapeInstance> {
    let homogeneous_rect = vertices
        .iter()
        .all(|vertex| same_float(vertex.clip[2], 0.0) && same_float(vertex.clip[3], 1.0));
    if !homogeneous_rect {
        return None;
    }
    let clip = vertices.map(|vertex| [vertex.clip[0], vertex.clip[1]]);
    let uv = vertices.map(|vertex| vertex.uv);
    let full = same_float(clip[0][0], clip[2][0])
        && same_float(clip[1][0], clip[3][0])
        && same_float(clip[0][1], clip[1][1])
        && same_float(clip[2][1], clip[3][1])
        && same_float(uv[0][0], uv[2][0])
        && same_float(uv[1][0], uv[3][0])
        && same_float(uv[0][1], uv[1][1])
        && same_float(uv[2][1], uv[3][1]);
    let first_triangle = same_xy(clip[2], clip[3])
        && same_xy(uv[2], uv[3])
        && same_float(clip[0][0], clip[2][0])
        && same_float(clip[0][1], clip[1][1])
        && same_float(uv[0][0], uv[2][0])
        && same_float(uv[0][1], uv[1][1]);
    let second_triangle = same_xy(clip[2], clip[3])
        && same_xy(uv[2], uv[3])
        && same_float(clip[1][0], clip[2][0])
        && same_float(clip[0][1], clip[2][1])
        && same_float(uv[1][0], uv[2][0])
        && same_float(uv[0][1], uv[2][1]);
    let (clip_rect, uv_rect, shape) = if full {
        (
            [clip[0][0], clip[0][1], clip[3][0], clip[3][1]],
            [uv[0][0], uv[0][1], uv[3][0], uv[3][1]],
            0,
        )
    } else if first_triangle {
        (
            [clip[0][0], clip[0][1], clip[1][0], clip[2][1]],
            [uv[0][0], uv[0][1], uv[1][0], uv[2][1]],
            1,
        )
    } else if second_triangle {
        (
            [clip[0][0], clip[1][1], clip[2][0], clip[0][1]],
            [uv[0][0], uv[1][1], uv[2][0], uv[0][1]],
            2,
        )
    } else {
        return None;
    };
    let modulation = [
        packed_c4_modulation(vertices[0].data0)?,
        packed_c4_modulation(vertices[1].data0)?,
        packed_c4_modulation(vertices[2].data0)?,
        packed_c4_modulation(vertices[3].data0)?,
    ];
    Some(PackedLandscapeInstance {
        clip_rect,
        uv_rect,
        modulation,
        liquid_scale,
        phase,
        flags: (u32::from(gamma) * LANDSCAPE_FLAG_GAMMA)
            | (u32::from(smooth) * LANDSCAPE_FLAG_SMOOTH)
            | shape << LANDSCAPE_SHAPE_SHIFT,
    })
}

fn packed_solid_vertex(
    position: [f32; 3],
    color: [f32; 4],
    gamma: bool,
    dither: bool,
    projection: &DrawProjection,
) -> Result<PackedVertex, GpuRendererError> {
    Ok(PackedVertex {
        clip: clip_position(position, projection)?,
        uv: [0.0, 0.0],
        data0: color,
        data1: [flag(gamma), flag(dither), 0.0, 0.0],
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
) -> Result<Option<PackedSolidRectInstance>, GpuRendererError> {
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
    Ok(Some(packed_solid_rect_instance(
        [left as f64, top as f64, right as f64, bottom as f64],
        point.color,
        gamma,
        projection,
    )?))
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

/// Project one physical rectangle into a compact clip-space instance.
///
/// The corners route through [`packed_solid_physical_vertex`] so the clip
/// coordinates stay the exact `f32` values a triangle pair would have carried.
fn packed_solid_rect_instance(
    physical: [f64; 4],
    color: [f32; 4],
    gamma: bool,
    projection: &DrawProjection,
) -> Result<PackedSolidRectInstance, GpuRendererError> {
    let [left, top, right, bottom] = physical;
    let top_left = packed_solid_physical_vertex([left, top], color, gamma, projection)?;
    let bottom_right = packed_solid_physical_vertex([right, bottom], color, gamma, projection)?;
    Ok(PackedSolidRectInstance {
        clip_rect: [
            top_left.clip[0],
            top_left.clip[1],
            bottom_right.clip[0],
            bottom_right.clip[1],
        ],
        color,
        // Point and line fragments never carry the gradient dither; only the
        // interpolated triangle path asks for it.
        flags: if gamma { SOLID_RECT_FLAG_GAMMA } else { 0 },
    })
}

fn append_line_fragment_instances(
    instances: &mut Vec<PackedSolidRectInstance>,
    start: GpuSolidVertex,
    end: GpuSolidVertex,
    gamma: bool,
    projection: &DrawProjection,
) -> Result<u64, GpuRendererError> {
    // OpenGL 2.1 section 3.4 rasterizes an aliased x-major line into at
    // most one fragment per physical column (one per row for y-major), omits
    // the directed final fragment, and implements a wide line by replicating
    // that base fragment in the minor direction. An oriented rectangle is
    // observably wrong: it is direction-invariant and can cover two pixels in
    // one major column on a diagonal. Generate the exact half-open fragment
    // stream, then emit one one-pixel rectangle per selected fragment.
    walk_aliased_line_fragments(start, end, projection, |x, y, t| {
        let color = line_color_at_parameter(start, end, t)?;
        let left = x as f64;
        let top = y as f64;
        instances.push(packed_solid_rect_instance(
            [left, top, left + 1.0, top + 1.0],
            color,
            gamma,
            projection,
        )?);
        Ok(())
    })
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
        // A raster width is not a position, so the projection's scale does not
        // carry the zoom for it: multiply it in, or a magnified world keeps
        // unzoomed rain, spray and debug lines.
        line_width: presentation.scale * presentation.world_zoom.max(0.0),
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
    let mut solid_rect_instances = 0_u64;
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
                // Validate the worst-case generic fallback without repeating
                // compact classification on the hot path. This may reject an
                // unreachable multi-billion-command scene early, but it can
                // never admit a range that the six-vertex fallback overflows.
                packed_vertices = packed_vertices.saturating_add(6);
            }
            GpuCommand::SpriteBatch {
                texture,
                quads,
                clip,
                ..
            } => {
                if quads.is_empty() {
                    continue;
                }
                require_declared_format(&resources, *texture, GpuTextureFormat::Rgba8)?;
                let projection = draw_projection(*clip, scene.logical_extent, presentation)?;
                let mut minimum = [f32::INFINITY; 2];
                let mut maximum = [f32::NEG_INFINITY; 2];
                for quad in quads {
                    if !quad
                        .rect
                        .iter()
                        .chain(quad.uv.iter())
                        .all(|value| value.is_finite())
                    {
                        return Err(GpuRendererError::NonFiniteCoordinate);
                    }
                    let [left, top, right, bottom] = quad.rect;
                    minimum[0] = minimum[0].min(left).min(right);
                    minimum[1] = minimum[1].min(top).min(bottom);
                    maximum[0] = maximum[0].max(left).max(right);
                    maximum[1] = maximum[1].max(top).max(bottom);
                }
                if let Some(projection) = projection.as_ref() {
                    let _ = SpriteProjection::new(projection)
                        .clip_rect([minimum[0], minimum[1], maximum[0], maximum[1]])?;
                }
                packed_vertices = packed_vertices.saturating_add(
                    u64::try_from(quads.len())
                        .unwrap_or(u64::MAX)
                        .saturating_mul(6),
                );
            }
            GpuCommand::ObjectBatch {
                texture,
                owner_texture,
                sprites,
                clip,
                blend,
                ..
            } => {
                if sprites.is_empty() {
                    continue;
                }
                require_declared_format(&resources, *texture, GpuTextureFormat::Rgba8)?;
                if let Some(owner_texture) = owner_texture {
                    require_declared_format(&resources, *owner_texture, GpuTextureFormat::Rgba8)?;
                    let texture_extent = resources
                        .get(texture)
                        .expect("declared base texture was just validated")
                        .extent;
                    let owner_extent = resources
                        .get(owner_texture)
                        .expect("declared owner texture was just validated")
                        .extent;
                    if texture_extent != owner_extent {
                        return Err(GpuRendererError::ObjectTextureExtentMismatch {
                            texture: *texture,
                            owner_texture: *owner_texture,
                            texture_extent,
                            owner_extent,
                        });
                    }
                }
                if *blend == GpuBlend::Replace {
                    let outer_applies = sprites.first().is_some_and(|sprite| {
                        sprite.outer_modulation() != GpuOuterModulation::Ignore
                    });
                    if let Some(sprite) = sprites.iter().position(|sprite| {
                        (sprite.outer_modulation() != GpuOuterModulation::Ignore) != outer_applies
                    }) {
                        return Err(GpuRendererError::MixedReplaceObjectOuterModulation { sprite });
                    }
                }
                let projection = draw_projection(*clip, scene.logical_extent, presentation)?;
                for sprite in sprites {
                    if !sprite
                        .positions
                        .iter()
                        .flatten()
                        .chain(sprite.uv.iter())
                        .chain(std::iter::once(&sprite.sample_tile_size))
                        .all(|value| value.is_finite())
                    {
                        return Err(GpuRendererError::NonFiniteCoordinate);
                    }
                    validate_object_sprite_flags(*sprite)?;
                    if sprite.owner_layer() && owner_texture.is_none() {
                        return Err(GpuRendererError::ObjectOwnerLayerWithoutTexture);
                    }
                    validate_object_sprite_sample_tile(*sprite)?;
                    if let Some(projection) = projection.as_ref() {
                        for position in sprite.positions {
                            let _ = clip_position(position, projection)?;
                        }
                    }
                }
                packed_vertices = packed_vertices.saturating_add(
                    u64::try_from(sprites.len())
                        .unwrap_or(u64::MAX)
                        .saturating_mul(6),
                );
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
                let mut line_fragments = 0_u64;
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
                            line_fragments = line_fragments
                                .checked_add(fragment_count)
                                .ok_or(GpuRendererError::VertexRangeOverflow)?;
                        }
                    }
                }
                let count = u64::try_from(vertices.len())
                    .map_err(|_| GpuRendererError::VertexRangeOverflow)?;
                // Points and line fragments each cost one instance; only
                // triangles consume the shared vertex range.
                match topology {
                    GpuPrimitiveTopology::PointList => {
                        solid_rect_instances = solid_rect_instances.saturating_add(count);
                    }
                    GpuPrimitiveTopology::LineList => {
                        solid_rect_instances = solid_rect_instances.saturating_add(line_fragments);
                    }
                    GpuPrimitiveTopology::TriangleList => {
                        packed_vertices = packed_vertices.saturating_add(count);
                    }
                }
            }
        }
        if packed_vertices > u64::from(u32::MAX) || solid_rect_instances > u64::from(u32::MAX) {
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

fn validate_retained_texture_limits(
    resources: &[GpuTextureResource],
    composition_extent: [u32; 2],
    shader_landscape: Option<&clonk_graphics::ShaderLandscapePlan>,
    landscape_detail: u32,
    max_texture_dimension_2d: u32,
) -> Result<(), GpuRendererError> {
    // The CPU presentation buffer must use this same physical extent, so
    // reject it before promising a source-texture fallback that cannot be
    // presented by the device either.
    validate_texture_extent(
        RetainedGpuTextureKind::Composition,
        None,
        composition_extent,
        max_texture_dimension_2d,
    )?;
    validate_source_texture_limits(resources, max_texture_dimension_2d)?;
    if let Some(plan) = shader_landscape {
        validate_shader_landscape_texture_extents(
            plan.extent,
            plan.shading_plane.is_some(),
            plan.atlas_extent,
            landscape_detail,
            max_texture_dimension_2d,
        )?;
    }
    Ok(())
}

fn validate_source_texture_limits(
    resources: &[GpuTextureResource],
    max_texture_dimension_2d: u32,
) -> Result<(), GpuRendererError> {
    resources.iter().try_for_each(|resource| {
        validate_texture_extent(
            RetainedGpuTextureKind::Source,
            Some(resource.id),
            resource.extent,
            max_texture_dimension_2d,
        )
    })
}

fn validate_shader_landscape_texture_limits(
    inputs: &ShaderLandscapeInputs<'_>,
    max_texture_dimension_2d: u32,
) -> Result<(), GpuRendererError> {
    validate_shader_landscape_texture_extents(
        inputs.extent,
        inputs.shading_plane.is_some(),
        inputs.atlas_extent,
        inputs.detail,
        max_texture_dimension_2d,
    )
}

fn validate_shader_landscape_texture_extents(
    extent: [u32; 2],
    has_shading: bool,
    atlas_extent: [u32; 2],
    detail: u32,
    max_texture_dimension_2d: u32,
) -> Result<(), GpuRendererError> {
    validate_texture_extent(
        RetainedGpuTextureKind::ShaderLandscapeIndex,
        None,
        extent,
        max_texture_dimension_2d,
    )?;
    if has_shading {
        validate_texture_extent(
            RetainedGpuTextureKind::ShaderLandscapeShading,
            None,
            extent,
            max_texture_dimension_2d,
        )?;
    }
    validate_texture_extent(
        RetainedGpuTextureKind::ShaderLandscapeAtlas,
        None,
        atlas_extent,
        max_texture_dimension_2d,
    )?;
    validate_texture_extent(
        RetainedGpuTextureKind::ShaderLandscapeOutput,
        None,
        [
            extent[0].saturating_mul(detail.max(1)),
            extent[1].saturating_mul(detail.max(1)),
        ],
        max_texture_dimension_2d,
    )
}

fn validate_texture_extent(
    kind: RetainedGpuTextureKind,
    id: Option<GpuTextureId>,
    extent: [u32; 2],
    max_texture_dimension_2d: u32,
) -> Result<(), GpuRendererError> {
    if extent[0] > max_texture_dimension_2d || extent[1] > max_texture_dimension_2d {
        return Err(GpuRendererError::TextureDimensionExceeded {
            kind,
            id,
            extent,
            max_texture_dimension_2d,
        });
    }
    Ok(())
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

fn quad_instance_count(instances: &[PackedQuadInstance]) -> Result<u32, GpuRendererError> {
    u32::try_from(instances.len()).map_err(|_| GpuRendererError::VertexRangeOverflow)
}

fn sprite_instance_count(instances: &[PackedSpriteInstance]) -> Result<u32, GpuRendererError> {
    u32::try_from(instances.len()).map_err(|_| GpuRendererError::VertexRangeOverflow)
}

fn object_sprite_instance_count(
    instances: &[PackedObjectSpriteInstance],
) -> Result<u32, GpuRendererError> {
    u32::try_from(instances.len()).map_err(|_| GpuRendererError::VertexRangeOverflow)
}

fn landscape_instance_count(
    instances: &[PackedLandscapeInstance],
) -> Result<u32, GpuRendererError> {
    u32::try_from(instances.len()).map_err(|_| GpuRendererError::VertexRangeOverflow)
}

fn solid_rect_instance_count(
    instances: &[PackedSolidRectInstance],
) -> Result<u32, GpuRendererError> {
    u32::try_from(instances.len()).map_err(|_| GpuRendererError::VertexRangeOverflow)
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

fn packed_quad_instance_bytes(instances: &[PackedQuadInstance]) -> &[u8] {
    const {
        assert!(std::mem::size_of::<PackedQuadInstance>() == PACKED_QUAD_INSTANCE_STRIDE as usize);
    }
    // SAFETY: `PackedQuadInstance` is `repr(C)`, contains only contiguous
    // `f32` arrays, and the size assertion above excludes padding.
    unsafe {
        std::slice::from_raw_parts(
            instances.as_ptr().cast::<u8>(),
            std::mem::size_of_val(instances),
        )
    }
}

fn packed_sprite_instance_bytes(instances: &[PackedSpriteInstance]) -> &[u8] {
    const {
        assert!(
            std::mem::size_of::<PackedSpriteInstance>() == PACKED_SPRITE_INSTANCE_STRIDE as usize
        );
    }
    // SAFETY: `PackedSpriteInstance` is `repr(C)`, contains only contiguous
    // `f32` and `u32` fields, and the size assertion above excludes padding.
    unsafe {
        std::slice::from_raw_parts(
            instances.as_ptr().cast::<u8>(),
            std::mem::size_of_val(instances),
        )
    }
}

fn packed_landscape_instance_bytes(instances: &[PackedLandscapeInstance]) -> &[u8] {
    const {
        assert!(
            std::mem::size_of::<PackedLandscapeInstance>()
                == PACKED_LANDSCAPE_INSTANCE_STRIDE as usize
        );
    }
    // SAFETY: `PackedLandscapeInstance` is `repr(C)`, contains only
    // contiguous `f32` and `u32` fields, and the size assertion excludes
    // padding. Reading initialized object representations as bytes is valid.
    unsafe {
        std::slice::from_raw_parts(
            instances.as_ptr().cast::<u8>(),
            std::mem::size_of_val(instances),
        )
    }
}

fn packed_solid_rect_instance_bytes(instances: &[PackedSolidRectInstance]) -> &[u8] {
    const {
        assert!(
            std::mem::size_of::<PackedSolidRectInstance>()
                == PACKED_SOLID_RECT_INSTANCE_STRIDE as usize
        );
    }
    // SAFETY: `PackedSolidRectInstance` is `repr(C)`, contains only contiguous
    // `f32` and `u32` fields, and the size assertion above excludes padding.
    unsafe {
        std::slice::from_raw_parts(
            instances.as_ptr().cast::<u8>(),
            std::mem::size_of_val(instances),
        )
    }
}

fn packed_object_sprite_instance_bytes(instances: &[PackedObjectSpriteInstance]) -> &[u8] {
    const {
        assert!(
            std::mem::size_of::<PackedObjectSpriteInstance>()
                == PACKED_OBJECT_SPRITE_INSTANCE_STRIDE as usize
        );
        assert!(std::mem::size_of::<PackedObjectSpriteInstance>() <= 96);
    }
    // SAFETY: `PackedObjectSpriteInstance` is `repr(C)`, contains only
    // contiguous `f32` and `u32` arrays, and the size assertion above excludes
    // padding. Reading an initialized object representation as bytes is valid.
    unsafe {
        std::slice::from_raw_parts(
            instances.as_ptr().cast::<u8>(),
            std::mem::size_of_val(instances),
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

/// Mips are built once, on the CPU, from the complete backing a resource
/// always carries — and only for resources that never change. A revisioned
/// surface (the landscape cache, the liquid animation) would have to rebuild
/// its whole chain on every dirty rect, and it binds the nearest sampler
/// anyway, so it never samples one.
fn wants_mipmaps(resource: &GpuTextureResource) -> bool {
    resource.base_revision.is_none()
        && resource.dirty.is_empty()
        && mip_level_count(resource.extent) > 1
}

fn create_source_texture(
    device: &wgpu::Device,
    resource: &GpuTextureResource,
    mipmaps: bool,
) -> wgpu::Texture {
    let levels = if mipmaps && wants_mipmaps(resource) {
        mip_level_count(resource.extent)
    } else {
        1
    };
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("lc_gpu_retained_source"),
        size: wgpu::Extent3d {
            width: resource.extent[0],
            height: resource.extent[1],
            depth_or_array_layers: 1,
        },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: texture_format(resource.format),
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

/// Number of mip levels a source of this extent can hold, base included.
fn mip_level_count(extent: [u32; 2]) -> u32 {
    let longest = extent[0].max(extent[1]).max(1);
    1 + longest.ilog2()
}

/// Box-filter the complete CPU backing down to 1x1, returning every level
/// below the base.
///
/// RGBA is averaged in premultiplied space: sprite sheets are surrounded by
/// fully transparent texels whose colour bytes are arbitrary, and averaging
/// those straight would bleed them into every minified edge.
fn generate_mip_chain(
    pixels: &[u8],
    extent: [u32; 2],
    format: GpuTextureFormat,
) -> Vec<([u32; 2], Vec<u8>)> {
    let bytes = format.bytes_per_pixel();
    let mut levels = Vec::new();
    let mut source = pixels.to_vec();
    let [mut width, mut height] = [extent[0].max(1), extent[1].max(1)];
    while width > 1 || height > 1 {
        let next_width = (width / 2).max(1);
        let next_height = (height / 2).max(1);
        // An axis that has already bottomed out samples the same row/column
        // twice, which averages to itself.
        let step_x = if width > 1 { 1 } else { 0 };
        let step_y = if height > 1 { 1 } else { 0 };
        let mut level = vec![0_u8; next_width as usize * next_height as usize * bytes];
        for y in 0..next_height as usize {
            for x in 0..next_width as usize {
                let source_x = x * (1 + step_x as usize);
                let source_y = y * (1 + step_y as usize);
                let texel = |dx: usize, dy: usize| {
                    let sx = (source_x + dx).min(width as usize - 1);
                    let sy = (source_y + dy).min(height as usize - 1);
                    let offset = (sy * width as usize + sx) * bytes;
                    &source[offset..offset + bytes]
                };
                let quad = [
                    texel(0, 0),
                    texel(step_x as usize, 0),
                    texel(0, step_y as usize),
                    texel(step_x as usize, step_y as usize),
                ];
                let destination = (y * next_width as usize + x) * bytes;
                match format {
                    GpuTextureFormat::R8 => {
                        let sum: u32 = quad.iter().map(|texel| u32::from(texel[0])).sum();
                        level[destination] = ((sum + 2) / 4) as u8;
                    }
                    GpuTextureFormat::Rgba8 => {
                        let alpha: u32 = quad.iter().map(|texel| u32::from(texel[3])).sum();
                        let channel = |index: usize| {
                            if alpha == 0 {
                                return 0;
                            }
                            let weighted: u32 = quad
                                .iter()
                                .map(|texel| u32::from(texel[index]) * u32::from(texel[3]))
                                .sum();
                            ((weighted + alpha / 2) / alpha) as u8
                        };
                        level[destination] = channel(0);
                        level[destination + 1] = channel(1);
                        level[destination + 2] = channel(2);
                        level[destination + 3] = ((alpha + 2) / 4) as u8;
                    }
                }
            }
        }
        levels.push(([next_width, next_height], level.clone()));
        source = level;
        width = next_width;
        height = next_height;
    }
    levels
}

fn texture_format(format: GpuTextureFormat) -> wgpu::TextureFormat {
    match format {
        GpuTextureFormat::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
        GpuTextureFormat::R8 => wgpu::TextureFormat::R8Unorm,
    }
}

fn upload_full(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    resource: &GpuTextureResource,
) -> TextureUploadStats {
    let mut stats = TextureUploadStats::default();
    let bytes_per_row = resource.extent[0] * resource.format.bytes_per_pixel() as u32;
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &resource.pixels,
        wgpu::TexelCopyBufferLayout {
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
    stats.record(resource.pixels.len());
    if texture.mip_level_count() > 1 {
        stats.add(upload_mip_chain(queue, texture, resource));
    }
    stats
}

fn upload_mip_chain(
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    resource: &GpuTextureResource,
) -> TextureUploadStats {
    let mut stats = TextureUploadStats::default();
    let bytes = resource.format.bytes_per_pixel() as u32;
    for (level, (extent, pixels)) in
        generate_mip_chain(&resource.pixels, resource.extent, resource.format)
            .into_iter()
            .enumerate()
    {
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: level as u32 + 1,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(extent[0] * bytes),
                rows_per_image: Some(extent[1]),
            },
            wgpu::Extent3d {
                width: extent[0],
                height: extent[1],
                depth_or_array_layers: 1,
            },
        );
        stats.record(pixels.len());
    }
    stats
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
        wgpu::TexelCopyTextureInfo {
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
        wgpu::TexelCopyBufferLayout {
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
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
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
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
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

fn packed_quad_instance_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: PACKED_QUAD_INSTANCE_STRIDE,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &PACKED_QUAD_INSTANCE_ATTRIBUTES,
    }
}

fn packed_sprite_instance_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: PACKED_SPRITE_INSTANCE_STRIDE,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &PACKED_SPRITE_INSTANCE_ATTRIBUTES,
    }
}

fn packed_landscape_instance_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: PACKED_LANDSCAPE_INSTANCE_STRIDE,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &PACKED_LANDSCAPE_INSTANCE_ATTRIBUTES,
    }
}

fn packed_solid_rect_instance_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: PACKED_SOLID_RECT_INSTANCE_STRIDE,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &PACKED_SOLID_RECT_INSTANCE_ATTRIBUTES,
    }
}

fn packed_object_sprite_instance_layout() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: PACKED_OBJECT_SPRITE_INSTANCE_STRIDE,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &PACKED_OBJECT_SPRITE_INSTANCE_ATTRIBUTES,
    }
}

fn quad_scene_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    blend: GpuBlend,
    alpha_mode: GpuSolidAlphaMode,
) -> wgpu::RenderPipeline {
    scene_pipeline_with_vertex_layout(
        device,
        label,
        layout,
        shader,
        wgpu::PrimitiveTopology::TriangleList,
        blend,
        alpha_mode,
        packed_quad_instance_layout(),
        "vs_main",
    )
}

fn sprite_scene_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    blend: GpuBlend,
) -> wgpu::RenderPipeline {
    scene_pipeline_with_vertex_layout(
        device,
        label,
        layout,
        shader,
        wgpu::PrimitiveTopology::TriangleList,
        blend,
        GpuSolidAlphaMode::SourceOver,
        packed_sprite_instance_layout(),
        "vs_main",
    )
}

fn solid_rect_scene_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    blend: GpuBlend,
    alpha_mode: GpuSolidAlphaMode,
) -> wgpu::RenderPipeline {
    scene_pipeline_with_vertex_layout(
        device,
        label,
        layout,
        shader,
        wgpu::PrimitiveTopology::TriangleList,
        blend,
        alpha_mode,
        packed_solid_rect_instance_layout(),
        "vs_main",
    )
}

fn object_sprite_scene_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    blend: GpuBlend,
) -> wgpu::RenderPipeline {
    scene_pipeline_with_vertex_layout(
        device,
        label,
        layout,
        shader,
        wgpu::PrimitiveTopology::TriangleList,
        blend,
        GpuSolidAlphaMode::SourceOver,
        packed_object_sprite_instance_layout(),
        "vs_main",
    )
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
    scene_pipeline_with_vertex_layout(
        device,
        label,
        layout,
        shader,
        topology,
        blend,
        alpha_mode,
        packed_vertex_layout(),
        "vs_main",
    )
}

#[allow(clippy::too_many_arguments)]
fn scene_pipeline_with_vertex_layout(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    topology: wgpu::PrimitiveTopology,
    blend: GpuBlend,
    alpha_mode: GpuSolidAlphaMode,
    vertex_layout: wgpu::VertexBufferLayout<'static>,
    vertex_entry_point: &'static str,
) -> wgpu::RenderPipeline {
    let vertex_layouts = [Some(vertex_layout)];
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
            entry_point: Some(vertex_entry_point),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
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
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &targets,
        }),
        multiview_mask: None,
        cache: None,
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
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(match (monitor_gamma, surface_format.is_srgb()) {
                (false, false) => "fs_linear",
                (false, true) => "fs_srgb",
                (true, false) => "fs_monitor_linear",
                (true, true) => "fs_monitor_srgb",
            }),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &targets,
        }),
        multiview_mask: None,
        cache: None,
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

// ---------------------------------------------------------------------------
// Shader landscape composition (`Graphics.ShaderLandscape`)
// ---------------------------------------------------------------------------
//
// The retained CPU composer walks INTEGER landscape-map coordinates and hands
// them to `compose_material_surface_pixel`, so its finest possible sampling
// rate is one pattern texel per landscape pixel — shipping higher-resolution
// material art changes only the tiling period, never the detail. The pipeline
// below evaluates the identical arithmetic per fragment from an index plane, a
// precomputed placement-shading plane and a shared pattern atlas, which is what
// removes that cap.
//
// `detail` is the only knob that diverges from C++: at 1 the composed plane is
// byte-identical to the CPU composer, and at N the plane is N times larger in
// each axis with the pattern evaluated at 1/N landscape pixel. Because the
// pattern coordinate is the fine output coordinate, an N-times-larger pattern
// keeps its world-space tiling period exactly.

/// Slot is populated; an absent slot composes nothing (`Slot::Empty`).
pub const SHADER_LANDSCAPE_PRESENT: u32 = 1;
/// Take all three pattern modifiers from blue (`MATERIAL_OVERLAY_MONOCHROME`).
pub const SHADER_LANDSCAPE_MONOCHROME: u32 = 2;
/// A secondary (overlay) pattern follows the primary one.
pub const SHADER_LANDSCAPE_HAS_OVERLAY: u32 = 4;
/// The primary pattern is a `Surface8`; its atlas texels carry raw indices.
pub const SHADER_LANDSCAPE_PRIMARY_INDEXED: u32 = 8;
/// The overlay pattern is a `Surface8`.
pub const SHADER_LANDSCAPE_OVERLAY_INDEXED: u32 = 16;

/// A placement-shading texel whose darken channel holds this value is
/// suppressed entirely, mirroring the `own_density == 0` `continue` in the CPU
/// composer. Real darken amounts never exceed 60.
pub const SHADER_LANDSCAPE_SUPPRESSED: u8 = 255;

/// The uniform slot table is sized for the 128 texmap entries C4TexMap can
/// hold. At 64 bytes each that is 8 KiB, inside the 16 KiB downlevel limit.
pub const SHADER_LANDSCAPE_SLOTS: usize = 128;

/// Upper bound for `landscape_detail`. Each step squares the composed plane's
/// memory, and 4 already covers a 400% presentation scale.
pub const MAX_LANDSCAPE_DETAIL: u32 = 4;

/// One texmap slot in the layout `MATERIAL_LANDSCAPE_SHADER` binds.
///
/// This mirrors `clonk_frontend`'s `MaterialGpuSlot` field for field; that type
/// is crate-private, so the layout is restated rather than shared. The frontend
/// test `packed_material_slot_matches_the_cpu_composer` proves the packing
/// equals `compose_material_surface_pixel`, and
/// `shader_landscape_composition_matches_the_cpu_reference` proves this shader
/// equals the same arithmetic.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShaderLandscapeSlot {
    /// Material colour triplets 0/1/2 packed as `r | g<<8 | b<<16`, then
    /// `alpha[0..3]` packed the same way.
    pub colors: [u32; 4],
    /// `alpha[3..6]` packed, the primary CPattern zoom, the overlay zoom, and
    /// the `SHADER_LANDSCAPE_*` flag bits.
    pub params: [u32; 4],
    /// Atlas rect of the primary pattern: origin x, origin y, width, height.
    pub primary: [u32; 4],
    /// Atlas rect of the secondary (overlay) pattern.
    pub overlay: [u32; 4],
}

/// Everything one composition pass reads.
#[derive(Clone, Copy, Debug)]
pub struct ShaderLandscapeInputs<'a> {
    /// Landscape-map extent, i.e. `PixelGrid::width()`/`height()`.
    pub extent: [u32; 2],
    /// `PixelGrid::bytes()`, one landscape byte per map pixel.
    pub index_plane: &'a [u8],
    /// Interleaved `(lighten, darken)` amounts from `ApplyLighting`, two bytes
    /// per map pixel. `None` when `shade_materials` is off. Keeping this on the
    /// CPU is deliberate: the +-8-row placement loop is a C++ mirror and must
    /// not be re-derived in WGSL.
    pub shading_plane: Option<&'a [u8]>,
    /// RGBA pattern atlas. `Surface8` patterns store their index in red.
    pub atlas: &'a [u8],
    pub atlas_extent: [u32; 2],
    pub slots: &'a [ShaderLandscapeSlot],
    /// 1 reproduces the CPU composer byte for byte; N supersamples the plane.
    pub detail: u32,
}

impl ShaderLandscapeInputs<'_> {
    /// Extent of the composed plane this input set produces.
    pub fn composed_extent(&self) -> [u32; 2] {
        [
            self.extent[0].saturating_mul(self.detail.max(1)),
            self.extent[1].saturating_mul(self.detail.max(1)),
        ]
    }

    fn validate(&self) -> Result<(), GpuRendererError> {
        let pixels = (self.extent[0] as usize).saturating_mul(self.extent[1] as usize);
        if self.extent[0] == 0 || self.extent[1] == 0 || self.detail == 0 {
            return Err(GpuRendererError::ShaderLandscapeInputs("empty extent"));
        }
        if self.index_plane.len() < pixels {
            return Err(GpuRendererError::ShaderLandscapeInputs("short index plane"));
        }
        if self
            .shading_plane
            .is_some_and(|plane| plane.len() < pixels * 2)
        {
            return Err(GpuRendererError::ShaderLandscapeInputs(
                "short shading plane",
            ));
        }
        let atlas_pixels =
            (self.atlas_extent[0] as usize).saturating_mul(self.atlas_extent[1] as usize);
        if atlas_pixels == 0 || self.atlas.len() < atlas_pixels * 4 {
            return Err(GpuRendererError::ShaderLandscapeInputs("short atlas"));
        }
        if self.slots.len() > SHADER_LANDSCAPE_SLOTS {
            return Err(GpuRendererError::ShaderLandscapeInputs("too many slots"));
        }
        Ok(())
    }
}

const MATERIAL_LANDSCAPE_SHADER: &str = r#"
struct Slot {
    colors: vec4<u32>,
    params: vec4<u32>,
    primary: vec4<u32>,
    overlay: vec4<u32>,
};

struct SlotTable {
    slots: array<Slot, 128>,
};

struct ComposeParams {
    // x/y: landscape extent. z: detail factor. w: 1 when a shading plane is
    // bound, otherwise the neutral 1x1 fallback is.
    config: vec4<u32>,
};

@group(0) @binding(0) var index_plane: texture_2d<u32>;
@group(0) @binding(1) var shading_plane: texture_2d<u32>;
@group(0) @binding(2) var atlas: texture_2d<u32>;
@group(0) @binding(3) var<uniform> params: ComposeParams;
@group(0) @binding(4) var<uniform> slot_table: SlotTable;

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    // One oversized triangle covering the whole composed plane.
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(corners[index], 0.0, 1.0);
}

fn triplet(packed: u32, channel: u32) -> u32 {
    return (packed >> (channel * 8u)) & 0xffu;
}

// `lighten_material_channel` (materials.rs:163-169).
fn lighten_channel(channel: u32) -> u32 {
    if (channel & 0x80u) != 0u {
        return 255u;
    }
    return (channel << 1u) & 0xffu;
}

struct Pixel {
    rgb: vec3<u32>,
    transparency: u32,
};

fn atlas_texel(rect: vec4<u32>, coordinate: vec2<i32>, zoom: u32) -> vec4<u32> {
    var sample = coordinate;
    if zoom != 0u {
        sample = coordinate / vec2<i32>(i32(zoom));
    }
    let tiled = vec2<u32>(sample) % rect.zw;
    return textureLoad(atlas, vec2<i32>(rect.xy + tiled), 0);
}

fn apply_pattern(
    pixel: Pixel,
    slot: Slot,
    landscape_pixel: u32,
    rect: vec4<u32>,
    zoom: u32,
    indexed: bool,
    monochrome: bool,
    coordinate: vec2<i32>,
) -> Pixel {
    var result = pixel;
    if rect.z == 0u || rect.w == 0u {
        return result;
    }
    let texel = atlas_texel(rect, coordinate, zoom);
    if indexed {
        // `apply_indexed_material_pattern` (materials.rs:252-260).
        let shift = texel.r % 3u;
        var packed = slot.colors.x;
        if shift == 1u {
            packed = slot.colors.y;
        } else if shift == 2u {
            packed = slot.colors.z;
        }
        result.rgb = vec3<u32>(triplet(packed, 0u), triplet(packed, 1u), triplet(packed, 2u));
        var alpha = slot.colors.w;
        if (landscape_pixel & 0xf0u) != 0u {
            alpha = slot.params.x;
        }
        result.transparency = triplet(alpha, shift);
        return result;
    }
    // `apply_material_pattern` (materials.rs:192-204).
    var modifiers = texel.rgb;
    if monochrome {
        modifiers = vec3<u32>(texel.b);
    }
    result.rgb = vec3<u32>(
        lighten_channel((result.rgb.r * modifiers.r) >> 8u),
        lighten_channel((result.rgb.g * modifiers.g) >> 8u),
        lighten_channel((result.rgb.b * modifiers.b) >> 8u),
    );
    result.transparency = min(result.transparency + (255u - texel.a), 255u);
    return result;
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let fine = vec2<i32>(floor(position.xy));
    let map = fine / vec2<i32>(i32(params.config.z));
    if map.x < 0 || map.y < 0
        || u32(map.x) >= params.config.x || u32(map.y) >= params.config.y {
        return vec4<f32>(0.0);
    }
    let landscape_pixel = textureLoad(index_plane, map, 0).r;
    // Pixel zero is sky (C4Landscape.cpp:2622-2632).
    if landscape_pixel == 0u {
        return vec4<f32>(0.0);
    }
    let slot = slot_table.slots[landscape_pixel & 0x7fu];
    let flags = slot.params.w;
    if (flags & 1u) == 0u {
        return vec4<f32>(0.0);
    }

    var shading = vec2<u32>(0u, 0u);
    if params.config.w != 0u {
        let sample = textureLoad(shading_plane, map, 0);
        if sample.g == 255u {
            // `own_density == 0` leaves the pixel fully transparent.
            return vec4<f32>(0.0);
        }
        shading = vec2<u32>(sample.r, sample.g);
    }

    var alpha = slot.colors.w;
    if (landscape_pixel & 0x80u) != 0u {
        alpha = slot.params.x;
    }
    var pixel: Pixel;
    pixel.rgb = vec3<u32>(
        triplet(slot.colors.x, 0u),
        triplet(slot.colors.x, 1u),
        triplet(slot.colors.x, 2u),
    );
    pixel.transparency = triplet(alpha, 0u);

    let monochrome = (flags & 2u) != 0u;
    pixel = apply_pattern(
        pixel,
        slot,
        landscape_pixel,
        slot.primary,
        slot.params.y,
        (flags & 8u) != 0u,
        monochrome,
        fine,
    );
    if (flags & 4u) != 0u {
        pixel = apply_pattern(
            pixel,
            slot,
            landscape_pixel,
            slot.overlay,
            slot.params.z,
            (flags & 16u) != 0u,
            monochrome,
            fine,
        );
    }

    // `lighten_material_color` then `darken_material_color`, both saturating
    // (materials.rs:402-414). They are stored separately because the lighten
    // clamp at 255 is not recoverable from a single signed amount.
    var rgb = min(pixel.rgb + vec3<u32>(shading.x), vec3<u32>(255u));
    rgb = select(vec3<u32>(0u), rgb - vec3<u32>(shading.y), rgb >= vec3<u32>(shading.y));
    return vec4<f32>(
        f32(rgb.r) / 255.0,
        f32(rgb.g) / 255.0,
        f32(rgb.b) / 255.0,
        f32(255u - pixel.transparency) / 255.0,
    );
}
"#;

/// The rows a retained plane must re-upload, as a half-open row range.
///
/// `None` is an unchanged plane, which uploads nothing. A plane whose length
/// no longer matches its predecessor is uploaded whole: it belongs to a
/// different extent, and the caller has already recreated the texture.
fn changed_rows(previous: &[u8], next: &[u8], row_bytes: usize) -> Option<std::ops::Range<usize>> {
    if row_bytes == 0 {
        return None;
    }
    let rows = next.len() / row_bytes;
    if previous.len() != next.len() {
        return (rows > 0).then_some(0..rows);
    }
    let row = |index: usize| index * row_bytes..(index + 1) * row_bytes;
    let differs = |index: &usize| previous[row(*index)] != next[row(*index)];
    let first = (0..rows).find(differs)?;
    let last = (first..rows).rev().find(differs).unwrap_or(first);
    Some(first..last + 1)
}

/// The changed rows narrowed to the columns that actually differ.
///
/// A landscape edit is a rectangle, not a set of whole rows: digging one texel
/// out of a 4096-wide map changes one byte, and uploading its row would carry
/// 4095 unchanged ones with it. `Queue::write_texture` takes an origin and a
/// source row stride, so the columns are boundable exactly as the rows are.
///
/// Columns are in **texels**; the caller scales by the format's texel size.
fn changed_rect(
    previous: &[u8],
    next: &[u8],
    row_bytes: usize,
    bytes_per_texel: usize,
) -> Option<(std::ops::Range<usize>, std::ops::Range<usize>)> {
    let rows = changed_rows(previous, next, row_bytes)?;
    let texels = row_bytes.checked_div(bytes_per_texel).filter(|w| *w > 0)?;
    if previous.len() != next.len() {
        return Some((rows, 0..texels));
    }
    let texel = |row: usize, column: usize| {
        let start = row * row_bytes + column * bytes_per_texel;
        start..start + bytes_per_texel
    };
    let column_differs = |column: &usize| {
        rows.clone()
            .any(|row| previous[texel(row, *column)] != next[texel(row, *column)])
    };
    // `changed_rows` already found a difference, so both ends exist.
    let first = (0..texels).find(column_differs)?;
    let last = (first..texels).rev().find(column_differs).unwrap_or(first);
    Some((rows, first..last + 1))
}

/// What a retained composition's resources are shaped by. The planes are one
/// texel per map pixel, so they follow the map extent and — because an absent
/// shading plane composes from a 1x1 neutral texel — whether shading is on.
/// The atlas and its slot table come from the material catalogue instead, and
/// change only when it is reloaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ShaderLandscapeResourceKey {
    extent: [u32; 2],
    shading: bool,
    atlas_extent: [u32; 2],
}

/// Which retained resources the next composition can keep.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ShaderLandscapeReuse {
    planes: bool,
    atlas: bool,
    /// The bind group names every view, so it survives only when they all do.
    bind_group: bool,
}

impl ShaderLandscapeReuse {
    fn between(
        previous: Option<ShaderLandscapeResourceKey>,
        next: ShaderLandscapeResourceKey,
    ) -> Self {
        previous.map_or_else(Self::default, |previous| {
            let planes = previous.extent == next.extent && previous.shading == next.shading;
            let atlas = previous.atlas_extent == next.atlas_extent;
            Self {
                planes,
                atlas,
                bind_group: planes && atlas,
            }
        })
    }
}

/// Renders the landscape material composition per fragment.
///
/// Deliberate divergence from C++, opt in through `Graphics.ShaderLandscape`.
/// With `detail == 1` the composed plane is byte-identical to the retained CPU
/// composer, so enabling the flag alone changes nothing a player can see; the
/// detail factor is what lifts the cap.
#[derive(Debug)]
pub struct ShaderLandscapeComposer {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    retained: Option<RetainedShaderLandscape>,
    /// What the last composition wrote, for `GpuRendererStats`.
    last_uploads: ShaderLandscapeUploads,
    /// Output texels the last composition pass rewrote.
    last_composed_texels: u64,
}

/// One composition's GPU resources, kept across compositions.
///
/// The plane bytes are kept beside their textures so the next composition can
/// upload the rows that changed rather than the whole map, and so an unchanged
/// landscape uploads nothing at all.
#[derive(Debug)]
struct RetainedShaderLandscape {
    key: ShaderLandscapeResourceKey,
    index: wgpu::TextureView,
    index_plane: Vec<u8>,
    shading: wgpu::TextureView,
    shading_plane: Vec<u8>,
    atlas: wgpu::TextureView,
    atlas_bytes: Vec<u8>,
    params: wgpu::Buffer,
    config: [u32; 4],
    slots: wgpu::Buffer,
    slot_table: [ShaderLandscapeSlot; SHADER_LANDSCAPE_SLOTS],
    bind_group: wgpu::BindGroup,
    /// Held so the views above stay valid: a wgpu view does not keep its
    /// texture alive.
    _textures: [wgpu::Texture; 3],
}

impl RetainedShaderLandscape {
    /// Re-uploads the map planes by the rows that changed. An unchanged
    /// landscape uploads nothing.
    fn upload_planes(
        &mut self,
        queue: &wgpu::Queue,
        index_plane: &[u8],
        shading_plane: &[u8],
        shading_extent: [u32; 2],
    ) -> ShaderLandscapeUploads {
        let mut uploads = upload_changed_rows(
            queue,
            &self.index,
            &mut self.index_plane,
            index_plane,
            self.key.extent,
            1,
        );
        let shading = upload_changed_rows(
            queue,
            &self.shading,
            &mut self.shading_plane,
            shading_plane,
            shading_extent,
            2,
        );
        // With shading off the plane is a 1x1 neutral texel, so its rectangle
        // is not in map space and must not widen the map's.
        uploads.add(match self.key.shading {
            true => shading,
            false => ShaderLandscapeUploads {
                dirty: Some([0, 0, 0, 0]),
                ..shading
            },
        });
        uploads
    }

    /// Swaps in a reshaped atlas while keeping the map planes.
    ///
    /// A catalogue reload changes what the atlas *is*, not the map it is
    /// sampled for, so the index and shading textures and their byte caches
    /// move across untouched — which is the difference between uploading the
    /// atlas and re-uploading the whole map. The bind group names every view,
    /// so it is rebuilt from the two that survive plus the new one.
    fn with_reloaded_atlas(
        self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        layout: &wgpu::BindGroupLayout,
        atlas_bytes: &[u8],
        atlas_extent: [u32; 2],
    ) -> (Self, ShaderLandscapeUploads) {
        let Self {
            key,
            index,
            index_plane,
            shading,
            shading_plane,
            params,
            config,
            slots,
            slot_table,
            _textures: [index_texture, shading_texture, _old_atlas],
            ..
        } = self;
        let (atlas_texture, atlas) = uint_plane(
            device,
            queue,
            "lc_gpu_shader_landscape_atlas",
            wgpu::TextureFormat::Rgba8Uint,
            atlas_extent,
            atlas_bytes,
            4,
        );
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lc_gpu_shader_landscape_bind_group"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&index),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&shading),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&atlas),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: slots.as_entire_binding(),
                },
            ],
        });
        let uploads = ShaderLandscapeUploads {
            calls: 1,
            bytes: atlas_bytes.len() as u64,
            // A different catalogue re-colours every texel.
            dirty: None,
        };
        (
            Self {
                key: ShaderLandscapeResourceKey {
                    atlas_extent,
                    ..key
                },
                index,
                index_plane,
                shading,
                shading_plane,
                atlas,
                atlas_bytes: atlas_bytes.to_vec(),
                params,
                config,
                slots,
                slot_table,
                bind_group,
                _textures: [index_texture, shading_texture, atlas_texture],
            },
            uploads,
        )
    }

    /// The atlas comes from the material catalogue, so it survives every
    /// composition until a reload changes it.
    fn upload_atlas(
        &mut self,
        queue: &wgpu::Queue,
        atlas: &[u8],
        extent: [u32; 2],
    ) -> ShaderLandscapeUploads {
        let uploaded =
            upload_changed_rows(queue, &self.atlas, &mut self.atlas_bytes, atlas, extent, 4);
        // The atlas is sampled by every texel, so a changed one is unbounded
        // in output space however few of its own rows moved.
        match uploaded.calls {
            0 => uploaded,
            _ => ShaderLandscapeUploads {
                dirty: None,
                ..uploaded
            },
        }
    }

    /// The uniforms are small enough to rewrite whole, but only when the
    /// detail factor, extent or texmap actually moved.
    fn upload_uniforms(
        &mut self,
        queue: &wgpu::Queue,
        config: [u32; 4],
        table: &[ShaderLandscapeSlot; SHADER_LANDSCAPE_SLOTS],
    ) -> ShaderLandscapeUploads {
        let mut uploads = ShaderLandscapeUploads::clean();
        // Neither of these is confined to a rectangle: a changed detail
        // factor or slot table re-colours every texel of the output.
        if self.config != config {
            self.config = config;
            let bytes = u32_bytes(&config);
            uploads.add(ShaderLandscapeUploads {
                calls: 1,
                bytes: bytes.len() as u64,
                dirty: None,
            });
            queue.write_buffer(&self.params, 0, bytes);
        }
        if self.slot_table != *table {
            self.slot_table = *table;
            let bytes = shader_landscape_slot_bytes(table);
            uploads.add(ShaderLandscapeUploads {
                calls: 1,
                bytes: bytes.len() as u64,
                dirty: None,
            });
            queue.write_buffer(&self.slots, 0, bytes);
        }
        uploads
    }
}

/// Writes the rows of `next` that differ from `previous` into `view`'s
/// texture, and adopts them.
///
/// Reports the bytes written, so an unchanged plane is observably free rather
/// than only silently cheap.
fn upload_changed_rows(
    queue: &wgpu::Queue,
    view: &wgpu::TextureView,
    previous: &mut Vec<u8>,
    next: &[u8],
    extent: [u32; 2],
    bytes_per_texel: u32,
) -> ShaderLandscapeUploads {
    let texel_bytes = bytes_per_texel as usize;
    let row_bytes = extent[0] as usize * texel_bytes;
    let Some((rows, columns)) = changed_rect(previous, next, row_bytes, texel_bytes) else {
        return ShaderLandscapeUploads::clean();
    };
    // The source keeps the plane's own stride and starts at the rectangle's
    // first texel, so wgpu reads `width` texels out of each row rather than
    // the whole one. `write_texture` has no 256-byte row alignment rule —
    // that applies to buffer-to-texture copies.
    let offset = rows.start * row_bytes + columns.start * texel_bytes;
    let height = (rows.end - rows.start) as u32;
    let width = (columns.end - columns.start) as u32;
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: view.texture(),
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: columns.start as u32,
                y: rows.start as u32,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        &next[offset..],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(extent[0] * bytes_per_texel),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    previous.clear();
    previous.extend_from_slice(next);
    ShaderLandscapeUploads {
        calls: 1,
        bytes: u64::from(height) * u64::from(width) * texel_bytes as u64,
        dirty: Some([columns.start as u32, rows.start as u32, width, height]),
    }
}

/// What one composition wrote to its retained resources.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ShaderLandscapeUploads {
    calls: usize,
    bytes: u64,
    /// The map texels the uploads covered, as `(x, y, width, height)`.
    ///
    /// `None` means "not known to be bounded" — a fresh composition, or a
    /// change whose effect is not confined to a rectangle — and the pass then
    /// composes everything.
    dirty: Option<[u32; 4]>,
}

impl ShaderLandscapeUploads {
    /// Accumulates another upload's cost and widens the dirty rectangle to
    /// cover both. An unbounded contribution makes the union unbounded.
    fn add(&mut self, other: Self) {
        self.calls += other.calls;
        self.bytes += other.bytes;
        self.dirty = match (self.dirty, other.dirty) {
            (Some(left), Some(right)) => Some(union_rect(left, right)),
            (Some(only), None) if other.calls == 0 => Some(only),
            (left, None) if other.calls == 0 => left,
            _ => None,
        };
    }

    /// An upload that wrote nothing, and so dirtied nothing.
    fn clean() -> Self {
        Self {
            calls: 0,
            bytes: 0,
            dirty: Some([0, 0, 0, 0]),
        }
    }
}

/// The smallest rectangle covering both, ignoring empty ones.
fn union_rect(left: [u32; 4], right: [u32; 4]) -> [u32; 4] {
    if left[2] == 0 || left[3] == 0 {
        return right;
    }
    if right[2] == 0 || right[3] == 0 {
        return left;
    }
    let x = left[0].min(right[0]);
    let y = left[1].min(right[1]);
    let right_edge = (left[0] + left[2]).max(right[0] + right[2]);
    let bottom_edge = (left[1] + left[3]).max(right[1] + right[3]);
    [x, y, right_edge - x, bottom_edge - y]
}

impl ShaderLandscapeComposer {
    pub fn new(device: &wgpu::Device) -> Self {
        let uint_texture = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Uint,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let uniform = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lc_gpu_shader_landscape_layout"),
            entries: &[
                uint_texture(0),
                uint_texture(1),
                uint_texture(2),
                uniform(3),
                uniform(4),
            ],
        });
        let module = shader(
            device,
            "lc_gpu_shader_landscape_shader",
            MATERIAL_LANDSCAPE_SHADER,
        );
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lc_gpu_shader_landscape_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let targets = [Some(wgpu::ColorTargetState {
            format: wgpu::TextureFormat::Rgba8Unorm,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lc_gpu_shader_landscape"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            pipeline,
            bind_group_layout,
            retained: None,
            last_uploads: ShaderLandscapeUploads::default(),
            last_composed_texels: 0,
        }
    }

    /// What the last composition wrote to its retained resources.
    fn last_uploads(&self) -> ShaderLandscapeUploads {
        self.last_uploads
    }

    /// Output texels the last composition pass rewrote.
    fn last_composed_texels(&self) -> u64 {
        self.last_composed_texels
    }

    /// Composes into `target`, which must be an `Rgba8Unorm` view of exactly
    /// `inputs.composed_extent()`.
    pub fn compose_into(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        inputs: ShaderLandscapeInputs<'_>,
    ) -> Result<(), GpuRendererError> {
        // A caller that does not say whether the target kept its contents gets
        // the whole composition, which is always correct.
        self.compose_into_profiled(device, queue, encoder, target, inputs, false, None)
    }

    /// `output_reused` is whether `target` still holds the previous
    /// composition. Only then may the pass preserve what it does not redraw.
    #[allow(clippy::too_many_arguments)]
    fn compose_into_profiled(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        inputs: ShaderLandscapeInputs<'_>,
        output_reused: bool,
        timestamp_writes: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) -> Result<(), GpuRendererError> {
        inputs.validate()?;
        let pixels = inputs.extent[0] as usize * inputs.extent[1] as usize;
        let index_plane = &inputs.index_plane[..pixels];
        let neutral = [0_u8, 0];
        let (shading_extent, shading_plane) = inputs
            .shading_plane
            .map_or(([1, 1], &neutral[..]), |plane| {
                (inputs.extent, &plane[..pixels * 2])
            });
        let atlas_pixels = inputs.atlas_extent[0] as usize * inputs.atlas_extent[1] as usize;
        let atlas_bytes = &inputs.atlas[..atlas_pixels * 4];
        let key = ShaderLandscapeResourceKey {
            extent: inputs.extent,
            shading: inputs.shading_plane.is_some(),
            atlas_extent: inputs.atlas_extent,
        };
        let config: [u32; 4] = [
            inputs.extent[0],
            inputs.extent[1],
            inputs.detail,
            u32::from(inputs.shading_plane.is_some()),
        ];
        let mut table = [ShaderLandscapeSlot::default(); SHADER_LANDSCAPE_SLOTS];
        table[..inputs.slots.len()].copy_from_slice(inputs.slots);

        let reuse =
            ShaderLandscapeReuse::between(self.retained.as_ref().map(|retained| retained.key), key);
        let mut retained = self.retained.take();
        // A resource the next composition cannot keep is dropped before its
        // replacement is created, so a resize does not hold both.
        if !reuse.planes {
            retained = None;
        }

        // Starts clean rather than `default()`: an empty rectangle unions with
        // the first real one, where an unbounded `None` would swallow it.
        let mut uploads = ShaderLandscapeUploads::clean();
        // A catalogue reload reshapes the atlas and nothing else. Swapping it
        // in place keeps the map planes, whose textures and byte caches the
        // reload did not touch — the criterion is to invalidate exactly what
        // the change owns.
        if reuse.planes && !reuse.atlas {
            if let Some(previous) = retained.take() {
                let (replaced, atlas_uploads) = previous.with_reloaded_atlas(
                    device,
                    queue,
                    &self.bind_group_layout,
                    atlas_bytes,
                    inputs.atlas_extent,
                );
                uploads.add(atlas_uploads);
                retained = Some(replaced);
            }
        }
        let retained = match retained {
            Some(mut retained) => {
                uploads.add(retained.upload_planes(
                    queue,
                    index_plane,
                    shading_plane,
                    shading_extent,
                ));
                uploads.add(retained.upload_atlas(queue, atlas_bytes, inputs.atlas_extent));
                uploads.add(retained.upload_uniforms(queue, config, &table));
                retained
            }
            None => {
                // Creating a resource uploads its whole contents, so the fresh
                // path reports what it wrote for the same reason the retained
                // one does: the first composition is the warmup a caller
                // measures against.
                for written in [
                    index_plane.len(),
                    shading_plane.len(),
                    atlas_bytes.len(),
                    u32_bytes(&config).len(),
                    shader_landscape_slot_bytes(&table).len(),
                ] {
                    uploads.add(ShaderLandscapeUploads {
                        calls: 1,
                        bytes: written as u64,
                        // A fresh output holds nothing to preserve, so this
                        // composition writes all of it.
                        dirty: None,
                    });
                }
                let (index_texture, index) = uint_plane(
                    device,
                    queue,
                    "lc_gpu_shader_landscape_index",
                    wgpu::TextureFormat::R8Uint,
                    inputs.extent,
                    index_plane,
                    1,
                );
                let (shading_texture, shading) = uint_plane(
                    device,
                    queue,
                    "lc_gpu_shader_landscape_shading",
                    wgpu::TextureFormat::Rg8Uint,
                    shading_extent,
                    shading_plane,
                    2,
                );
                let (atlas_texture, atlas) = uint_plane(
                    device,
                    queue,
                    "lc_gpu_shader_landscape_atlas",
                    wgpu::TextureFormat::Rgba8Uint,
                    inputs.atlas_extent,
                    atlas_bytes,
                    4,
                );
                let params = uniform_buffer(
                    device,
                    queue,
                    "lc_gpu_shader_landscape_params",
                    u32_bytes(&config),
                );
                let slots = uniform_buffer(
                    device,
                    queue,
                    "lc_gpu_shader_landscape_slots",
                    shader_landscape_slot_bytes(&table),
                );
                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("lc_gpu_shader_landscape_bind_group"),
                    layout: &self.bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&index),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(&shading),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&atlas),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: params.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 4,
                            resource: slots.as_entire_binding(),
                        },
                    ],
                });
                RetainedShaderLandscape {
                    key,
                    index,
                    index_plane: index_plane.to_vec(),
                    shading,
                    shading_plane: shading_plane.to_vec(),
                    atlas,
                    atlas_bytes: atlas_bytes.to_vec(),
                    params,
                    config,
                    slots,
                    slot_table: table,
                    bind_group,
                    _textures: [index_texture, shading_texture, atlas_texture],
                }
            }
        };
        self.last_uploads = uploads;
        let retained = self.retained.insert(retained);

        // The fragment shader reads only the map texel under it, so a bounded
        // dirty rectangle scales straight to an output scissor. Preserving
        // what lies outside it means loading the attachment instead of
        // clearing it, which is sound exactly when the caller reused the
        // output it composed last time.
        let scissor =
            output_reused
                .then_some(uploads.dirty)
                .flatten()
                .map(|[x, y, width, height]| {
                    let detail = inputs.detail.max(1);
                    [x * detail, y * detail, width * detail, height * detail]
                });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("lc_gpu_shader_landscape_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: match scissor {
                        Some(_) => wgpu::LoadOp::Load,
                        None => wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    },
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &retained.bind_group, &[]);
        if let Some([x, y, width, height]) = scissor {
            pass.set_scissor_rect(x, y, width, height);
        }
        pass.draw(0..3, 0..1);
        drop(pass);
        self.last_composed_texels = match scissor {
            Some([_, _, width, height]) => u64::from(width) * u64::from(height),
            None => {
                let detail = u64::from(inputs.detail.max(1));
                u64::from(inputs.extent[0]) * u64::from(inputs.extent[1]) * detail * detail
            }
        };
        Ok(())
    }
}

fn uint_plane(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    format: wgpu::TextureFormat,
    extent: [u32; 2],
    bytes: &[u8],
    bytes_per_texel: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: extent[0],
            height: extent[1],
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
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(extent[0] * bytes_per_texel),
            rows_per_image: Some(extent[1]),
        },
        wgpu::Extent3d {
            width: extent[0],
            height: extent[1],
            depth_or_array_layers: 1,
        },
    );
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

fn uniform_buffer(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    bytes: &[u8],
) -> wgpu::Buffer {
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: bytes.len() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&buffer, 0, bytes);
    buffer
}

fn u32_bytes(values: &[u32]) -> &[u8] {
    // SAFETY: `u32` has no padding and any bit pattern is a valid `u8`.
    unsafe {
        std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), std::mem::size_of_val(values))
    }
}

fn shader_landscape_slot_bytes(slots: &[ShaderLandscapeSlot]) -> &[u8] {
    const {
        assert!(std::mem::size_of::<ShaderLandscapeSlot>() == 64);
    }
    // SAFETY: `ShaderLandscapeSlot` is `repr(C)` over `[u32; 4]` arrays and the
    // size assertion above excludes padding.
    unsafe { std::slice::from_raw_parts(slots.as_ptr().cast::<u8>(), std::mem::size_of_val(slots)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clonk_graphics::{
        Color, GammaRamp, GpuGammaLut, GpuObjectSprite, GpuOuterModulation, GpuSolidStyle,
        GpuSolidVertex, GpuTextureResource, PixelFormat, Surface,
    };
    use clonk_gui::{ImageData, Rect as GuiRect};
    use std::sync::Arc;

    #[test]
    fn renderer_cpu_stage_total_reconciles_named_intervals() {
        let stages = GpuRendererCpuStages {
            validation: std::time::Duration::from_nanos(1),
            texture_synchronization: std::time::Duration::from_nanos(2),
            stream_packing_upload: std::time::Duration::from_nanos(3),
            command_encoding: std::time::Duration::from_nanos(4),
        };

        assert_eq!(stages.total(), std::time::Duration::from_nanos(10));
    }

    #[test]
    fn timestamp_ticks_are_labeled_and_scaled_by_queue_period() {
        let pairs = [
            GpuTimestampQueryPair::new(GpuTimestampPass::Scene, 0, 1),
            GpuTimestampQueryPair::new(GpuTimestampPass::Presentation, 2, 3),
        ];

        let samples = decode_timestamp_frame(2.5, &pairs, &[10, 14, 20, 23])
            .expect("valid timestamp fixture");

        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].pass, GpuTimestampPass::Scene);
        assert_eq!(samples[0].begin_tick, 10);
        assert_eq!(samples[0].end_tick, 14);
        assert_eq!(samples[0].validity, GpuTimestampSampleValidity::Valid);
        assert_eq!(samples[0].duration_ns, Some(10.0));
        assert_eq!(samples[1].pass, GpuTimestampPass::Presentation);
        assert_eq!(samples[1].duration_ns, Some(7.5));
    }

    #[test]
    fn timestamp_decoder_preserves_absent_optional_passes() {
        let pairs = [
            GpuTimestampQueryPair::new(GpuTimestampPass::Scene, 0, 1),
            GpuTimestampQueryPair::new(GpuTimestampPass::Presentation, 2, 3),
        ];

        let samples =
            decode_timestamp_frame(1.0, &pairs, &[1, 2, 3, 4]).expect("valid timestamp fixture");

        assert_eq!(
            samples.iter().map(|sample| sample.pass).collect::<Vec<_>>(),
            vec![GpuTimestampPass::Scene, GpuTimestampPass::Presentation]
        );
    }

    #[test]
    fn timestamp_decoder_preserves_raw_ticks_and_marks_invalid_samples() {
        let pairs = [GpuTimestampQueryPair::new(GpuTimestampPass::Scene, 0, 1)];

        let invalid_period = decode_timestamp_frame(0.0, &pairs, &[1, 2])
            .expect("raw queries remain structurally decodable");
        assert_eq!(
            invalid_period[0].validity,
            GpuTimestampSampleValidity::InvalidPeriod
        );
        assert_eq!(invalid_period[0].duration_ns, None);
        assert_eq!(
            (invalid_period[0].begin_tick, invalid_period[0].end_tick),
            (1, 2)
        );

        let rollover = decode_timestamp_frame(1.0, &pairs, &[2, 1])
            .expect("timestamp rollover preserves the raw query pair");
        assert_eq!(
            rollover[0].validity,
            GpuTimestampSampleValidity::CounterRollover
        );
        assert_eq!(rollover[0].duration_ns, None);
        assert_eq!((rollover[0].begin_tick, rollover[0].end_tick), (2, 1));
    }

    #[test]
    fn timestamp_history_drops_oldest_frames_at_its_bound() {
        let mut history = GpuTimestampHistory::default();
        for frame_id in 1..=(GPU_TIMESTAMP_COMPLETED_HISTORY_LIMIT as u64 + 1) {
            history.push_completed(GpuTimestampFrame {
                frame_id,
                renderer_generation: 1,
                timestamp_period_ns: 1.0,
                passes: Vec::new(),
            });
        }

        assert_eq!(
            history.completed.len(),
            GPU_TIMESTAMP_COMPLETED_HISTORY_LIMIT
        );
        assert_eq!(history.completed[0].frame_id, 2);
        assert_eq!(history.telemetry.dropped_frames, 1);
    }

    #[test]
    fn timestamp_boundary_drain_has_a_finite_wait_budget() {
        assert!(matches!(
            timestamp_drain_poll_type(),
            wgpu::PollType::Wait {
                submission_index: None,
                timeout: Some(timeout),
            } if timeout == GPU_TIMESTAMP_DRAIN_TIMEOUT
        ));
    }

    #[test]
    fn timestamp_query_profiles_optional_passes_in_encoding_order_when_supported() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build Tokio runtime for timestamp adapter discovery");
        let instance = test_wgpu_instance();
        let Some((adapter, device, queue)) = request_test_device_with_features(
            &runtime,
            &instance,
            "lc_gpu_timestamp_test_device",
            true,
            wgpu::Features::TIMESTAMP_QUERY,
        ) else {
            eprintln!("no timestamp-capable wgpu adapter; skipping timestamp query smoke");
            return;
        };
        if !adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            eprintln!("adapter lacks timestamp queries; skipping timestamp query smoke");
            return;
        }
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lc_gpu_timestamp_test_target"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
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
        let base = GpuTextureId::fresh();
        let mut scene = test_scene(
            [1, 1],
            Color::transparent(),
            vec![GpuTextureResource::immutable_rgba(
                base,
                1,
                1,
                Arc::from([0_u8; 4]),
            )],
            Vec::new(),
        );
        scene.gamma_mode = GpuGammaMode::Monitor;
        let mut renderer = test_renderer(&device, &queue);
        renderer.set_shader_landscape(true);
        renderer.set_pending_shader_landscape(Some((
            base,
            clonk_graphics::ShaderLandscapePlan {
                extent: [1, 1],
                index_plane: vec![0],
                shading_plane: None,
                atlas: vec![0; 4],
                atlas_extent: [1, 1],
                slots: Vec::new(),
            },
        )));
        assert!(renderer.timestamp_queries_enabled());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("lc_gpu_timestamp_test_encoder"),
        });

        renderer
            .render(
                &device,
                &queue,
                &mut encoder,
                &target_view,
                &scene,
                &GpuPresentation::identity(1, 1),
                false,
            )
            .expect("encode timestamped retained frame");
        queue.submit(Some(encoder.finish()));
        let frames = renderer
            .drain_timestamp_frames(&device)
            .expect("drain timestamp query readback");

        assert_eq!(frames.len(), 1);
        assert_eq!(
            frames[0]
                .passes
                .iter()
                .map(|sample| sample.pass)
                .collect::<Vec<_>>(),
            vec![
                GpuTimestampPass::ShaderLandscape,
                GpuTimestampPass::Scene,
                GpuTimestampPass::MonitorGamma,
                GpuTimestampPass::Presentation,
            ]
        );
        assert!(frames[0].passes.iter().all(|sample| match sample.validity {
            GpuTimestampSampleValidity::Valid => sample.duration_ns.is_some(),
            GpuTimestampSampleValidity::InvalidPeriod
            | GpuTimestampSampleValidity::CounterRollover
            | GpuTimestampSampleValidity::InvalidDuration => sample.duration_ns.is_none(),
        }));
        assert_eq!(renderer.last_stats().shader_landscape_draw_calls, 1);
        assert_eq!(renderer.last_stats().monitor_gamma_draw_calls, 1);
        assert_eq!(renderer.last_stats().presentation_draw_calls, 1);
        assert!(renderer.last_stats().has_exact_draw_call_counts());
        let invalid_frame = frames[0]
            .passes
            .iter()
            .any(|sample| sample.validity != GpuTimestampSampleValidity::Valid);
        assert_eq!(
            renderer.timestamp_telemetry(),
            GpuTimestampTelemetry {
                readback_errors: u64::from(invalid_frame),
                ..GpuTimestampTelemetry::default()
            }
        );
    }

    #[test]
    fn timestamp_samples_survive_recreation_and_boundary_drain() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build Tokio runtime for timestamp adapter discovery");
        let instance = test_wgpu_instance();
        let Some((adapter, device, queue)) = request_test_device_with_features(
            &runtime,
            &instance,
            "lc_gpu_timestamp_recreate_test_device",
            true,
            wgpu::Features::TIMESTAMP_QUERY,
        ) else {
            eprintln!("no timestamp-capable wgpu adapter; skipping timestamp recreation smoke");
            return;
        };
        if !adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            eprintln!("adapter lacks timestamp queries; skipping timestamp recreation smoke");
            return;
        }
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lc_gpu_timestamp_recreate_test_target"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
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
        let scene = test_scene([1, 1], Color::transparent(), Vec::new(), Vec::new());
        let mut renderer = test_renderer(&device, &queue);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("lc_gpu_timestamp_before_recreate_encoder"),
        });
        renderer
            .render(
                &device,
                &queue,
                &mut encoder,
                &target_view,
                &scene,
                &GpuPresentation::identity(1, 1),
                false,
            )
            .expect("encode timestamped frame before recreation");
        queue.submit(Some(encoder.finish()));
        renderer
            .timestamp_profiler
            .as_mut()
            .expect("timestamp profiler")
            .drain(&device, &mut renderer.timestamp_history)
            .expect("drain old-generation timestamp query readback");
        assert_eq!(renderer.timestamp_history.completed.len(), 1);

        assert_eq!(
            renderer.recreate(&device, &queue, wgpu::TextureFormat::Rgba8Unorm),
            2
        );
        let carried = renderer.take_completed_timestamp_frames(&device);
        assert_eq!(carried.len(), 1);
        assert_eq!(carried[0].frame_id, 1);
        assert_eq!(carried[0].renderer_generation, 1);
        assert_eq!(renderer.timestamp_telemetry().device_discontinuities, 1);

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("lc_gpu_timestamp_after_recreate_encoder"),
        });
        renderer
            .render(
                &device,
                &queue,
                &mut encoder,
                &target_view,
                &scene,
                &GpuPresentation::identity(1, 1),
                false,
            )
            .expect("encode timestamped frame after recreation");
        queue.submit(Some(encoder.finish()));

        let drained = renderer
            .drain_timestamp_frames(&device)
            .expect("drain timestamp frames at benchmark boundary");
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].frame_id, 2);
        assert_eq!(drained[0].renderer_generation, 2);
    }

    #[test]
    fn renderer_recreation_records_a_discontinuity_without_timestamp_queries() {
        let Some((_runtime, _instance, _adapter, device, queue)) =
            test_wgpu_device("lc_gpu_unprofiled_recreate_test_device", true)
        else {
            eprintln!("no wgpu adapter; skipping unprofiled recreation smoke");
            return;
        };
        let mut renderer = test_renderer(&device, &queue);
        assert!(!renderer.timestamp_queries_enabled());

        assert_eq!(
            renderer.recreate(&device, &queue, wgpu::TextureFormat::Rgba8Unorm),
            2
        );

        assert_eq!(renderer.timestamp_telemetry().device_discontinuities, 1);
    }

    #[test]
    fn timestamp_recreation_counts_unresolved_old_frames_as_dropped() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build Tokio runtime for timestamp adapter discovery");
        let instance = test_wgpu_instance();
        let Some((_adapter, device, queue)) = request_test_device_with_features(
            &runtime,
            &instance,
            "lc_gpu_timestamp_pending_recreate_test_device",
            true,
            wgpu::Features::TIMESTAMP_QUERY,
        ) else {
            eprintln!("no timestamp-capable wgpu adapter; skipping pending recreation smoke");
            return;
        };
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lc_gpu_timestamp_pending_recreate_target"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
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
        let scene = test_scene([1, 1], Color::transparent(), Vec::new(), Vec::new());
        let mut renderer = test_renderer(&device, &queue);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("lc_gpu_timestamp_pending_recreate_encoder"),
        });

        renderer
            .render(
                &device,
                &queue,
                &mut encoder,
                &target_view,
                &scene,
                &GpuPresentation::identity(1, 1),
                false,
            )
            .expect("encode timestamp frame without submitting it");
        drop(encoder);
        renderer.recreate(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);

        assert_eq!(renderer.timestamp_telemetry().dropped_frames, 1);
        assert_eq!(renderer.timestamp_telemetry().device_discontinuities, 1);
    }

    #[test]
    fn timestamp_frame_aborts_before_commit_when_health_turns_fatal() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build Tokio runtime for timestamp adapter discovery");
        let instance = test_wgpu_instance();
        let Some((_adapter, device, queue)) = request_test_device_with_features(
            &runtime,
            &instance,
            "lc_gpu_timestamp_abort_test_device",
            true,
            wgpu::Features::TIMESTAMP_QUERY,
        ) else {
            eprintln!("no timestamp-capable wgpu adapter; skipping timestamp abort smoke");
            return;
        };
        let mut renderer = test_renderer(&device, &queue);
        let mut active = renderer
            .timestamp_profiler
            .as_mut()
            .expect("timestamp profiler")
            .begin_frame(
                &device,
                renderer.generation,
                queue.get_timestamp_period(),
                &mut renderer.timestamp_history,
            )
            .expect("reserve timestamp slot");
        active.reserve(GpuTimestampPass::Scene);
        record_renderer_health(
            &renderer.health.state,
            RetainedGpuRendererHealth::Fatal {
                reason: RetainedGpuFatalReason::Internal,
                detail: "test fault after encoding".to_owned(),
            },
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("lc_gpu_timestamp_abort_test_encoder"),
        });

        assert!(matches!(
            renderer.commit_timestamp_frame(&mut encoder, Some(active)),
            Err(GpuRendererError::DeviceFatal {
                reason: RetainedGpuFatalReason::Internal,
                ..
            })
        ));
        assert_eq!(
            renderer
                .timestamp_profiler
                .as_ref()
                .expect("timestamp profiler")
                .pending_frames(),
            0
        );
    }

    #[test]
    fn source_texture_limit_rejects_oversized_landscape_before_gpu_work() {
        let id = GpuTextureId::fresh();
        let resource = GpuTextureResource::immutable_rgba(
            id,
            33_900,
            1,
            Arc::from(vec![0_u8; 33_900 * 4].into_boxed_slice()),
        );

        assert!(matches!(
            validate_source_texture_limits(std::slice::from_ref(&resource), 32_768),
            Err(GpuRendererError::TextureDimensionExceeded {
                kind: RetainedGpuTextureKind::Source,
                id: Some(found),
                extent: [33_900, 1],
                max_texture_dimension_2d: 32_768,
            }) if found == id
        ));
    }

    #[test]
    fn retained_texture_limit_covers_composition_and_shader_targets() {
        let resources = Vec::<GpuTextureResource>::new();
        assert!(matches!(
            validate_retained_texture_limits(&resources, [32_769, 1], None, 1, 32_768),
            Err(GpuRendererError::TextureDimensionExceeded {
                kind: RetainedGpuTextureKind::Composition,
                id: None,
                extent: [32_769, 1],
                max_texture_dimension_2d: 32_768,
            })
        ));

        let shader_plan = clonk_graphics::ShaderLandscapePlan {
            extent: [8, 8],
            index_plane: Vec::new(),
            shading_plane: None,
            atlas: Vec::new(),
            atlas_extent: [1, 1],
            slots: Vec::new(),
        };
        assert!(matches!(
            validate_retained_texture_limits(&resources, [1, 1], Some(&shader_plan), 4, 31,),
            Err(GpuRendererError::TextureDimensionExceeded {
                kind: RetainedGpuTextureKind::ShaderLandscapeOutput,
                id: None,
                extent: [32, 32],
                max_texture_dimension_2d: 31,
            })
        ));
    }

    #[test]
    fn source_limit_requires_cpu_presentation_without_poisoning_device() {
        let Some((runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping retained texture limit device check");
            return;
        };
        let max_texture_dimension_2d = device.limits().max_texture_dimension_2d;
        let width = max_texture_dimension_2d
            .checked_add(1)
            .expect("test device texture limit leaves one larger extent");
        let source = GpuTextureResource::immutable_rgba(
            GpuTextureId::fresh(),
            width,
            1,
            Arc::from(vec![0_u8; width as usize * 4].into_boxed_slice()),
        );
        let scene = GpuScene {
            logical_extent: [1, 1],
            clear: Color::transparent(),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
            gamma_mode: GpuGammaMode::Disabled,
            textures: vec![source],
            commands: Vec::new(),
        };
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lc_gpu_texture_limit_test_target"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
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
        let mut renderer = test_renderer(&device, &queue);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("lc_gpu_texture_limit_test_encoder"),
        });
        let validation_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

        assert!(matches!(
            renderer.render(
                &device,
                &queue,
                &mut encoder,
                &target_view,
                &scene,
                &GpuPresentation::identity(1, 1),
                false,
            ),
            Err(GpuRendererError::TextureDimensionExceeded {
                kind: RetainedGpuTextureKind::Source,
                extent: [found, 1],
                max_texture_dimension_2d: found_limit,
                ..
            }) if found == width && found_limit == max_texture_dimension_2d
        ));
        assert!(renderer.requires_cpu_presentation());
        assert_eq!(renderer.health(), RetainedGpuRendererHealth::Healthy);
        let validation = runtime.block_on(validation_scope.pop());
        assert!(
            validation.is_none(),
            "source-limit preflight must not poison the device: {validation:?}"
        );
    }

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
    fn compact_landscape_instance_preserves_all_canonical_shapes_in_72_bytes() {
        let color = [17.0 / 255.0, 34.0 / 255.0, 51.0 / 255.0, 68.0 / 255.0];
        let vertex = |clip: [f32; 2], uv: [f32; 2]| PackedVertex {
            clip: [clip[0], clip[1], 0.0, 1.0],
            uv,
            data0: color,
            data1: [0.0; 4],
            data2: [0.0; 4],
        };
        let top_left = vertex([-0.75, 0.5], [0.125, 0.25]);
        let top_right = vertex([0.25, 0.5], [0.875, 0.25]);
        let bottom_left = vertex([-0.75, -0.5], [0.125, 0.75]);
        let bottom_right = vertex([0.25, -0.5], [0.875, 0.75]);
        let pack = |vertices| {
            try_packed_landscape_instance(vertices, [2.0, 4.0], [0.25, -0.5, 0.75], true, true)
                .expect("canonical landscape geometry is compact")
        };

        let full = pack([top_left, top_right, bottom_left, bottom_right]);
        let first = pack([top_left, top_right, bottom_left, bottom_left]);
        let second = pack([bottom_left, top_right, bottom_right, bottom_right]);

        assert_eq!(std::mem::size_of::<PackedLandscapeInstance>(), 72);
        assert_eq!(std::mem::align_of::<PackedLandscapeInstance>(), 4);
        assert_eq!(full.clip_rect, [-0.75, 0.5, 0.25, -0.5]);
        assert_eq!(full.uv_rect, [0.125, 0.25, 0.875, 0.75]);
        assert_eq!(full.modulation, [0x4411_2233; 4]);
        assert_eq!(full.flags, LANDSCAPE_FLAG_GAMMA | LANDSCAPE_FLAG_SMOOTH);
        assert_eq!(first.flags >> LANDSCAPE_SHAPE_SHIFT, 1);
        assert_eq!(second.flags >> LANDSCAPE_SHAPE_SHIFT, 2);
        assert_eq!(packed_landscape_instance_bytes(&[full]).len(), 72);
    }

    #[test]
    fn compact_landscape_rejects_projective_and_non_c4_vertices() {
        let vertex = |x, y, w, modulation| PackedVertex {
            clip: [x, y, 0.0, w],
            uv: [(x + 1.0) / 2.0, (1.0 - y) / 2.0],
            data0: modulation,
            data1: [0.0; 4],
            data2: [0.0; 4],
        };
        let exact = [1.0, 1.0, 1.0, 0.0];
        let projective = [
            vertex(-1.0, 1.0, 2.0, exact),
            vertex(1.0, 1.0, 2.0, exact),
            vertex(-1.0, -1.0, 2.0, exact),
            vertex(1.0, -1.0, 2.0, exact),
        ];
        let non_c4 = projective.map(|mut vertex| {
            vertex.clip[3] = 1.0;
            vertex.data0 = [0.5, 0.75, 1.0, 0.0];
            vertex
        });

        assert!(
            try_packed_landscape_instance(projective, [1.0; 2], [0.0; 3], false, false).is_none()
        );
        assert!(try_packed_landscape_instance(non_c4, [1.0; 2], [0.0; 3], false, false).is_none());
    }

    #[test]
    fn generic_vertex_stream_stats_report_count_and_bytes() {
        let vertex = PackedVertex {
            clip: [0.0; 4],
            uv: [0.0; 2],
            data0: [0.0; 4],
            data1: [0.0; 4],
            data2: [0.0; 4],
        };
        let stream = BuiltDrawStream {
            vertices: vec![vertex; 2],
            quad_instances: Vec::new(),
            sprite_instances: Vec::new(),
            object_sprite_instances: Vec::new(),
            landscape_instances: Vec::new(),
            solid_rect_instances: Vec::new(),
            calls: Vec::new(),
        };
        let mut stats = GpuRendererStats::default();

        stats.record_draw_stream(&stream);

        assert_eq!(stats.generic_vertices, 2);
        assert_eq!(stats.generic_vertex_upload_bytes, 2 * 18 * 4);
    }

    #[test]
    fn draw_stream_stats_cover_every_instance_upload_stream() {
        let stream = BuiltDrawStream {
            vertices: Vec::new(),
            quad_instances: vec![PackedQuadInstance {
                clip: [[0.0; 4]; 4],
                uv: [[0.0; 4]; 2],
                modulation: [[0.0; 4]; 4],
                sample_tile: [[0.0; 4]; 4],
                flags: [0.0; 2],
            }],
            sprite_instances: vec![PackedSpriteInstance {
                clip_rect: [0.0; 4],
                uv_rect: [0.0; 4],
                modulation: 0,
                flags: 0,
            }],
            object_sprite_instances: vec![PackedObjectSpriteInstance {
                clip: [[0.0; 3]; 4],
                uv_rect: [0.0; 4],
                modulation: [0; 4],
                sample_tile_size: 0.0,
                flags: 0,
            }],
            landscape_instances: vec![PackedLandscapeInstance {
                clip_rect: [0.0; 4],
                uv_rect: [0.0; 4],
                modulation: [0; 4],
                liquid_scale: [0.0; 2],
                phase: [0.0; 3],
                flags: 0,
            }],
            solid_rect_instances: vec![PackedSolidRectInstance {
                clip_rect: [0.0; 4],
                color: [0.0; 4],
                flags: 0,
            }],
            calls: Vec::new(),
        };
        let mut stats = GpuRendererStats::default();

        stats.record_draw_stream(&stream);

        assert_eq!(stats.quad_instances, 1);
        assert_eq!(
            stats.quad_instance_upload_bytes,
            PACKED_QUAD_INSTANCE_STRIDE as usize
        );
        assert_eq!(stats.sprite_instances, 1);
        assert_eq!(
            stats.sprite_instance_upload_bytes,
            PACKED_SPRITE_INSTANCE_STRIDE as usize
        );
        assert_eq!(stats.object_sprite_instances, 1);
        assert_eq!(
            stats.object_sprite_upload_bytes,
            PACKED_OBJECT_SPRITE_INSTANCE_STRIDE as usize
        );
        assert_eq!(stats.landscape_instances, 1);
        assert_eq!(
            stats.landscape_instance_upload_bytes,
            PACKED_LANDSCAPE_INSTANCE_STRIDE as usize
        );
        assert_eq!(stats.solid_rect_instances, 1);
        assert_eq!(
            stats.solid_rect_upload_bytes,
            PACKED_SOLID_RECT_INSTANCE_STRIDE as usize
        );
    }

    #[test]
    fn four_k_landscape_instance_stream_stays_below_196_kib() {
        let instance = PackedLandscapeInstance {
            clip_rect: [0.0; 4],
            uv_rect: [0.0; 4],
            modulation: [0; 4],
            liquid_scale: [0.0; 2],
            phase: [0.0; 3],
            flags: 0,
        };
        let stream = BuiltDrawStream {
            vertices: Vec::new(),
            quad_instances: Vec::new(),
            sprite_instances: Vec::new(),
            object_sprite_instances: Vec::new(),
            landscape_instances: vec![instance; 2_040],
            solid_rect_instances: Vec::new(),
            calls: Vec::new(),
        };
        let mut stats = GpuRendererStats::default();

        stats.record_draw_stream(&stream);

        assert_eq!(stats.landscape_instances, 2_040);
        assert_eq!(stats.landscape_instance_upload_bytes, 146_880);
        assert!(stats.landscape_instance_upload_bytes <= 196 * 1024);
        assert_eq!(stats.generic_vertices, 0);
        assert_eq!(stats.generic_vertex_upload_bytes, 0);
    }

    #[test]
    fn draw_stream_stats_count_every_retained_draw_kind() {
        let texture = GpuTextureId::fresh();
        let quad = QuadBindingKey {
            texture,
            sampler: sampler_key(GpuSampler::Nearest),
        };
        let landscape = LandscapeBindingKey {
            base: texture,
            mask: None,
            liquid: None,
        };
        let object = ObjectRunKey {
            binding: ObjectBindingKey {
                texture,
                owner_texture: None,
            },
            clip: None,
            blend: GpuBlend::Normal,
            gamma: false,
            replace_outer_applies: None,
        };
        let call = |kind| DrawCall {
            vertices: 0..1,
            scissor: Scissor {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            },
            blend: GpuBlend::Normal,
            kind,
        };
        let stream = BuiltDrawStream {
            vertices: Vec::new(),
            quad_instances: Vec::new(),
            sprite_instances: Vec::new(),
            object_sprite_instances: Vec::new(),
            landscape_instances: Vec::new(),
            solid_rect_instances: Vec::new(),
            calls: vec![
                call(DrawKind::Quad(quad)),
                call(DrawKind::Sprite(quad)),
                call(DrawKind::ObjectSprite(object)),
                call(DrawKind::Landscape(landscape)),
                call(DrawKind::LandscapeInstance(landscape)),
                call(DrawKind::Solid {
                    alpha_mode: GpuSolidAlphaMode::SourceOver,
                }),
                call(DrawKind::SolidRect {
                    alpha_mode: GpuSolidAlphaMode::SourceOver,
                }),
            ],
        };
        let mut stats = GpuRendererStats::default();

        stats.record_draw_stream(&stream);

        assert_eq!(stats.draw_calls, 7);
        assert_eq!(stats.compatible_resource_runs, 7);
        assert_eq!(stats.quad_draw_calls, 1);
        assert_eq!(stats.sprite_draw_calls, 1);
        assert_eq!(stats.object_sprite_draw_calls, 1);
        assert_eq!(stats.landscape_draw_calls, 2);
        assert_eq!(stats.solid_draw_calls, 1);
        assert_eq!(stats.solid_rect_draw_calls, 1);
    }

    #[test]
    fn draw_call_stats_reconcile_every_scene_and_fixed_pass() {
        let stats = GpuRendererStats {
            draw_calls: 6,
            quad_draw_calls: 1,
            sprite_draw_calls: 1,
            object_sprite_draw_calls: 1,
            landscape_draw_calls: 1,
            shader_landscape_draw_calls: 1,
            solid_draw_calls: 1,
            solid_rect_draw_calls: 1,
            monitor_gamma_draw_calls: 1,
            presentation_draw_calls: 1,
            total_draw_calls: 9,
            compatible_resource_runs: 6,
            ..GpuRendererStats::default()
        };

        assert!(stats.has_exact_draw_call_counts());

        let mut missing_fixed_pass = stats;
        missing_fixed_pass.presentation_draw_calls = 0;
        assert!(!missing_fixed_pass.has_exact_draw_call_counts());
    }

    #[test]
    fn texture_upload_stats_count_every_written_region() {
        let mut stats = GpuRendererStats::default();

        stats.record_full_texture_upload(TextureUploadStats {
            calls: 1,
            bytes: 16,
        });
        stats.record_dirty_texture_upload(4);
        stats.record_dirty_texture_upload(8);

        assert_eq!(stats.full_upload_calls, 1);
        assert_eq!(stats.full_upload_bytes, 16);
        assert_eq!(stats.dirty_upload_calls, 2);
        assert_eq!(stats.dirty_upload_bytes, 12);
    }

    #[test]
    fn mipmapped_full_upload_stats_count_every_queue_write() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping mip upload stats test");
            return;
        };
        let resource = GpuTextureResource {
            id: GpuTextureId::fresh(),
            extent: [4, 4],
            revision: 0,
            base_revision: None,
            format: GpuTextureFormat::Rgba8,
            pixels: Arc::from(vec![0; 4 * 4 * 4].into_boxed_slice()),
            dirty: Vec::new(),
        };
        let texture = create_source_texture(&device, &resource, true);

        let upload = upload_full(&queue, &texture, &resource);

        assert_eq!(upload.calls, 3, "base, 2x2, and 1x1 are three writes");
        assert_eq!(upload.bytes, 64 + 16 + 4);
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
    fn device_health_distinguishes_recoverable_loss_from_fatal_errors() {
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

        let internal = classify_uncaptured_wgpu_error(&wgpu::Error::Internal {
            source: Box::new(std::io::Error::other("internal error source")),
            description: "unexpected backend failure".to_owned(),
        });
        assert!(matches!(
            internal,
            RetainedGpuRendererHealth::Fatal {
                reason: RetainedGpuFatalReason::Internal,
                ..
            }
        ));
    }

    #[test]
    fn destroying_the_device_marks_the_renderer_for_recreation() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping retained device-loss callback check");
            return;
        };
        let renderer = test_renderer(&device, &queue);

        device.destroy();
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll destroyed retained renderer device");

        assert!(matches!(
            renderer.health(),
            RetainedGpuRendererHealth::RecreateRequired {
                reason: RetainedGpuRecreateReason::DeviceLost,
                ref detail,
            } if detail == "Destroyed"
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
            world_zoom: 1.0,
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
            world_zoom: 1.0,
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
    fn mip_chain_averages_in_premultiplied_space_and_halves_to_one_texel() {
        // Straight-alpha averaging is the classic sprite-halo bug: three
        // transparent black texels would drag an opaque white one to grey and
        // the minified edge would darken. Weighting by coverage keeps the
        // colour and only lowers alpha.
        let transparent = [0, 0, 0, 0];
        let white = [255, 255, 255, 255];
        let pixels: Vec<u8> = [white, transparent, transparent, transparent].concat();
        let chain = generate_mip_chain(&pixels, [2, 2], GpuTextureFormat::Rgba8);
        assert_eq!(chain.len(), 1, "2x2 has exactly one level below the base");
        assert_eq!(chain[0].0, [1, 1]);
        assert_eq!(
            chain[0].1,
            vec![255, 255, 255, 64],
            "colour survives; only coverage drops"
        );

        // A plain opaque average stays the arithmetic mean.
        let opaque: Vec<u8> = [[0, 0, 0, 255], [255, 255, 255, 255]].concat();
        let chain = generate_mip_chain(&opaque, [2, 1], GpuTextureFormat::Rgba8);
        assert_eq!(chain[0].1, vec![128, 128, 128, 255]);

        // Level extents halve and clamp at one, so a non-square source keeps
        // reducing its long axis after the short one bottoms out.
        assert_eq!(mip_level_count([8, 2]), 4);
        let wide = vec![255_u8; 8 * 2 * 4];
        let chain = generate_mip_chain(&wide, [8, 2], GpuTextureFormat::Rgba8);
        assert_eq!(
            chain.iter().map(|(extent, _)| *extent).collect::<Vec<_>>(),
            vec![[4, 1], [2, 1], [1, 1]]
        );
        for (extent, level) in &chain {
            assert_eq!(level.len(), (extent[0] * extent[1] * 4) as usize);
        }

        // Single-channel masks average directly.
        let mask = vec![0_u8, 255, 255, 255];
        let chain = generate_mip_chain(&mask, [2, 2], GpuTextureFormat::R8);
        assert_eq!(chain[0].1, vec![191]);

        // A 1x1 source has no chain at all.
        assert_eq!(mip_level_count([1, 1]), 1);
        assert!(generate_mip_chain(&[1, 2, 3, 4], [1, 1], GpuTextureFormat::Rgba8).is_empty());
    }

    #[test]
    fn landscape_vertices_carry_the_smoothing_flag_in_a_free_channel() {
        // `liquid_scale` only ever uses xy, so the magnification policy rides
        // along in z rather than costing another vertex attribute.
        let presentation = GpuPresentation {
            physical_extent: [4, 4],
            scale: 1.0,
            crop_top: 0,
            world_zoom: 1.0,
        };
        let projection = draw_projection(None, [4, 4], &presentation)
            .expect("valid presentation")
            .expect("clip intersects the framebuffer");
        let vertex = GpuVertex {
            position: [0.0, 0.0, 1.0],
            uv: [0.0, 0.0],
            modulation: [1.0, 1.0, 1.0, 0.0],
            owner_modulation: [0.0; 4],
            outer_modulation: clonk_graphics::GpuOuterModulation::default(),
            owner_outer_modulation: clonk_graphics::GpuOuterModulation::default(),
            sample_tile: [0.0, 0.0, 1.0, 1.0],
        };
        let data1 = |smooth: bool| {
            packed_landscape_vertex(vertex, [2.0, 4.0], [0.0; 3], false, smooth, &projection)
                .expect("pack one landscape vertex")
                .data1
        };

        assert_eq!(data1(false), [2.0, 4.0, 0.0, 0.0]);
        assert_eq!(data1(true), [2.0, 4.0, 1.0, 0.0]);
    }

    #[test]
    fn only_unchanging_sources_get_a_mip_chain() {
        let pixels: Arc<[u8]> = vec![255_u8; 64 * 64 * 4].into();
        let art = GpuTextureResource::immutable_rgba(GpuTextureId::fresh(), 64, 64, pixels.clone());
        assert!(
            wants_mipmaps(&art),
            "retained art is uploaded once and is exactly what minifies"
        );

        // A revisioned surface would have to rebuild its whole chain per dirty
        // rect; the landscape cache is one of these and binds Nearest anyway.
        let revisioned = GpuTextureResource {
            base_revision: Some(3),
            revision: 4,
            ..art.clone()
        };
        assert!(!wants_mipmaps(&revisioned));

        let partial = GpuTextureResource {
            dirty: vec![Rect::new(0, 0, 4, 4)],
            ..art.clone()
        };
        assert!(!wants_mipmaps(&partial));

        let single =
            GpuTextureResource::immutable_rgba(GpuTextureId::fresh(), 1, 1, vec![0_u8; 4].into());
        assert!(
            !wants_mipmaps(&single),
            "a 1x1 source has no level below it"
        );
    }

    #[test]
    fn solid_triangle_vertices_carry_gamma_and_dither_in_separate_flag_channels() {
        // The solid shader reads its fragment options out of `data1`. Gamma
        // owns channel 0; the dither must land in its own channel so the two
        // stay independently switchable.
        let presentation = GpuPresentation {
            physical_extent: [6, 4],
            scale: 1.0,
            crop_top: 0,
            world_zoom: 1.0,
        };
        let projection = draw_projection(None, [6, 4], &presentation)
            .expect("valid presentation")
            .expect("clip intersects the framebuffer");
        let flags = |gamma: bool, dither: bool| {
            packed_solid_vertex(
                [1.0, 1.0, 1.0],
                [1.0, 0.0, 0.0, 1.0],
                gamma,
                dither,
                &projection,
            )
            .expect("pack one solid vertex")
            .data1
        };

        assert_eq!(flags(false, false), [0.0, 0.0, 0.0, 0.0]);
        assert_eq!(flags(true, false), [1.0, 0.0, 0.0, 0.0]);
        assert_eq!(flags(false, true), [0.0, 1.0, 0.0, 0.0]);
        assert_eq!(flags(true, true), [1.0, 1.0, 0.0, 0.0]);
    }

    fn physical_rect(instance: PackedSolidRectInstance, projection: &DrawProjection) -> [f64; 4] {
        let width = f64::from(projection.physical_extent[0]);
        let height = f64::from(projection.physical_extent[1]);
        let x = |clip: f32| ((f64::from(clip) + 1.0) * width / 2.0).round();
        let y = |clip: f32| ((1.0 - f64::from(clip)) * height / 2.0).round();
        [
            x(instance.clip_rect[0]),
            y(instance.clip_rect[1]),
            x(instance.clip_rect[2]),
            y(instance.clip_rect[3]),
        ]
    }

    #[test]
    fn logical_line_pair_expands_to_cpp_application_scale_in_physical_space() {
        // Every selected physical pixel is one whole rectangle, so it costs one
        // compact instance. Lowering it to a triangle pair spent six 72-byte
        // vertices restating the same rectangle and colour.
        let presentation = GpuPresentation {
            physical_extent: [12, 8],
            scale: 2.0,
            crop_top: 0,
            world_zoom: 1.0,
        };
        let projection = draw_projection(None, [6, 4], &presentation)
            .expect("valid line presentation")
            .expect("line clip intersects the framebuffer");
        let color = [1.0, 0.0, 0.0, 1.0];
        let mut instances = Vec::new();
        append_line_fragment_instances(
            &mut instances,
            solid_vertex(1.5, 1.5, color),
            solid_vertex(4.5, 1.5, color),
            false,
            &projection,
        )
        .expect("expand line pair");

        let mut origins = instances
            .iter()
            .map(|instance| {
                let [left, top, right, bottom] = physical_rect(*instance, &projection);
                assert_eq!(
                    [right - left, bottom - top],
                    [1.0, 1.0],
                    "a line fragment covers exactly one physical pixel"
                );
                [left, top]
            })
            .collect::<Vec<_>>();
        origins.sort_by(|left, right| left.partial_cmp(right).expect("finite physical origin"));
        let mut expected = (2..8)
            .flat_map(|x| (2..4).map(move |y| [f64::from(x), f64::from(y)]))
            .collect::<Vec<_>>();
        expected.sort_by(|left, right| left.partial_cmp(right).expect("finite expected origin"));
        assert_eq!(origins, expected);
        assert_eq!(instances.len(), 6 * 2);
        assert!(instances.iter().all(|instance| instance.color == color));
    }

    #[test]
    fn diagonal_line_color_uses_cpp_window_space_projection_parameter() {
        let presentation = GpuPresentation::identity(5, 4);
        let projection = draw_projection(None, [5, 4], &presentation)
            .expect("valid line presentation")
            .expect("line clip intersects the framebuffer");
        let mut instances = Vec::new();
        append_line_fragment_instances(
            &mut instances,
            solid_vertex(0.5, 0.5, [0.0, 0.0, 0.0, 1.0]),
            solid_vertex(4.5, 2.5, [1.0, 0.0, 0.0, 1.0]),
            false,
            &projection,
        )
        .expect("expand diagonal line");
        let fragment = instances
            .iter()
            .find(|instance| physical_rect(**instance, &projection)[..2] == [1.0, 1.0])
            .expect("slope-one-half line covers physical pixel (1,1)");
        assert!((fragment.color[0] - 0.3).abs() < 1.0e-6);
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
    fn compact_sprite_projection_matches_generic_quad_at_fractional_scale_and_crop() {
        let presentation = GpuPresentation {
            physical_extent: [17, 11],
            scale: 1.5,
            crop_top: 2,
            world_zoom: 1.0,
        };
        let projection = draw_projection(Some(Rect::new(1, 2, 6, 4)), [9, 8], &presentation)
            .expect("valid fractional presentation")
            .expect("clip intersects the framebuffer");
        let sprite = GpuSpriteQuad {
            rect: [1.25, 2.5, 6.75, 5.875],
            uv: [0.125, 0.25, 0.875, 0.75],
            modulation: 0x407f_3fc0,
        };
        let compact =
            packed_sprite_instance(sprite, true, true, SpriteProjection::new(&projection))
                .expect("pack compact sprite");
        let modulation = [127.0 / 255.0, 63.0 / 255.0, 192.0 / 255.0, 64.0 / 255.0];
        let generic = packed_quad_instance(
            [
                GpuVertex::new(
                    [sprite.rect[0], sprite.rect[1], 1.0],
                    [sprite.uv[0], sprite.uv[1]],
                    modulation,
                ),
                GpuVertex::new(
                    [sprite.rect[2], sprite.rect[1], 1.0],
                    [sprite.uv[2], sprite.uv[1]],
                    modulation,
                ),
                GpuVertex::new(
                    [sprite.rect[0], sprite.rect[3], 1.0],
                    [sprite.uv[0], sprite.uv[3]],
                    modulation,
                ),
                GpuVertex::new(
                    [sprite.rect[2], sprite.rect[3], 1.0],
                    [sprite.uv[2], sprite.uv[3]],
                    modulation,
                ),
            ],
            true,
            true,
            &projection,
        )
        .expect("pack generic quad");

        assert_eq!(
            compact.clip_rect,
            [
                generic.clip[0][0],
                generic.clip[0][1],
                generic.clip[3][0],
                generic.clip[3][1],
            ]
        );
        assert_eq!(
            generic.clip[1],
            [compact.clip_rect[2], compact.clip_rect[1], 0.0, 1.0,]
        );
        assert_eq!(
            generic.clip[2],
            [compact.clip_rect[0], compact.clip_rect[3], 0.0, 1.0,]
        );
        assert_eq!(compact.uv_rect, sprite.uv);
        assert_eq!(compact.modulation, sprite.modulation);
        assert_eq!(compact.flags, 3);
    }

    #[test]
    fn compact_object_shader_selects_exactly_one_sampling_path() {
        let fragment = OBJECT_SPRITE_SHADER
            .split_once("@fragment")
            .expect("object shader has a fragment stage")
            .1;
        assert!(fragment.contains(
            "var source: vec4<f32>;\n    if linear {\n        source = sample_native_tile(input.uv, input.sample_tile_size, owner_layer);\n    } else {\n        source = sample_nearest(input.uv, owner_layer);\n    }"
        ));
        assert_eq!(fragment.matches("sample_native_tile(").count(), 1);
        assert_eq!(fragment.matches("sample_nearest(").count(), 1);
        assert!(!fragment.contains("select("));
    }

    #[test]
    fn compact_object_shader_selects_companion_texture_from_bit_five() {
        assert!(OBJECT_SPRITE_SHADER
            .contains("@group(1) @binding(2) var owner_image: texture_2d<f32>;"));
        let fragment = OBJECT_SPRITE_SHADER
            .split_once("@fragment")
            .expect("object shader has a fragment stage")
            .1;
        assert!(fragment.contains("let owner_layer = (input.packed_flags & 32u) != 0u;"));
        assert!(
            fragment.contains("sample_native_tile(input.uv, input.sample_tile_size, owner_layer)")
        );
        assert!(fragment.contains("sample_nearest(input.uv, owner_layer)"));
    }

    #[test]
    fn compact_object_run_key_splits_pair_gamma_and_required_replace_outer_boundaries() {
        let texture = GpuTextureId::fresh();
        let owner_a = GpuTextureId::fresh();
        let owner_b = GpuTextureId::fresh();
        let command = |owner_texture, gamma, blend, outer_modulation| GpuCommand::ObjectBatch {
            texture,
            owner_texture,
            sprites: vec![GpuObjectSprite::new(
                [[0.0, 0.0, 1.0]; 4],
                [0.0, 0.0, 1.0, 1.0],
                [0x00ff_ffff; 4],
                GpuSampler::Nearest,
                0.0,
                false,
                outer_modulation,
            )],
            clip: None,
            blend,
            gamma,
        };
        let base = object_run_key(&command(
            Some(owner_a),
            false,
            GpuBlend::Replace,
            GpuOuterModulation::Combine,
        ))
        .expect("object command has a run key");

        assert_ne!(
            base,
            object_run_key(&command(
                Some(owner_b),
                false,
                GpuBlend::Replace,
                GpuOuterModulation::Combine,
            ))
            .expect("second resource pair has a run key")
        );
        assert_ne!(
            base,
            object_run_key(&command(
                Some(owner_a),
                true,
                GpuBlend::Replace,
                GpuOuterModulation::Combine,
            ))
            .expect("gamma boundary has a run key")
        );
        assert_ne!(
            base,
            object_run_key(&command(
                Some(owner_a),
                false,
                GpuBlend::Replace,
                GpuOuterModulation::Ignore,
            ))
            .expect("replace outer boundary has a run key")
        );
        assert_eq!(
            object_run_key(&command(
                Some(owner_a),
                false,
                GpuBlend::Normal,
                GpuOuterModulation::Combine,
            )),
            object_run_key(&command(
                Some(owner_a),
                false,
                GpuBlend::Normal,
                GpuOuterModulation::Ignore,
            )),
            "non-replacement outer policy is carried per instance"
        );
    }

    #[test]
    fn compact_object_validation_rejects_invalid_tile_sizes() {
        let texture = GpuTextureId::fresh();
        let scene = |sampler, sample_tile_size| GpuScene {
            logical_extent: [1, 1],
            clear: Color::transparent(),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::identity()),
            gamma_mode: GpuGammaMode::Disabled,
            textures: vec![rgba_resource(texture, [255; 4])],
            commands: vec![GpuCommand::ObjectBatch {
                texture,
                owner_texture: None,
                sprites: vec![GpuObjectSprite::new(
                    [[0.0, 0.0, 1.0]; 4],
                    [0.0, 0.0, 1.0, 1.0],
                    [0x00ff_ffff; 4],
                    sampler,
                    sample_tile_size,
                    false,
                    GpuOuterModulation::Inherit,
                )],
                clip: None,
                blend: GpuBlend::Normal,
                gamma: false,
            }],
        };
        let presentation = GpuPresentation::identity(1, 1);

        for invalid in [0.0, -1.0, 3.0, 8_192.0, f32::MAX] {
            assert!(
                matches!(
                    RetainedGpuRenderer::validate_scene(
                        &scene(GpuSampler::Linear, invalid),
                        &presentation
                    ),
                    Err(GpuRendererError::InvalidObjectSpriteSampleTile {
                        sampler: GpuSampler::Linear,
                        sample_tile_size,
                    }) if sample_tile_size == invalid
                ),
                "linear tile size {invalid} was accepted"
            );
        }
        for valid in [2.0, 4_096.0] {
            assert!(RetainedGpuRenderer::validate_scene(
                &scene(GpuSampler::Linear, valid),
                &presentation
            )
            .is_ok());
        }
        assert!(RetainedGpuRenderer::validate_scene(
            &scene(GpuSampler::Nearest, 0.0),
            &presentation
        )
        .is_ok());
        assert!(matches!(
            RetainedGpuRenderer::validate_scene(&scene(GpuSampler::Nearest, 2.0), &presentation),
            Err(GpuRendererError::InvalidObjectSpriteSampleTile {
                sampler: GpuSampler::Nearest,
                sample_tile_size: 2.0,
            })
        ));
    }

    #[test]
    fn compact_object_validation_rejects_owner_layer_without_companion() {
        let texture = GpuTextureId::fresh();
        let sprite = GpuObjectSprite::new(
            [[0.0, 0.0, 1.0]; 4],
            [0.0, 0.0, 1.0, 1.0],
            [0x00ff_ffff; 4],
            GpuSampler::Nearest,
            0.0,
            false,
            GpuOuterModulation::Inherit,
        )
        .with_owner_layer();
        let scene = test_scene(
            [1, 1],
            Color::transparent(),
            vec![rgba_resource(texture, [255; 4])],
            vec![GpuCommand::ObjectBatch {
                texture,
                owner_texture: None,
                sprites: vec![sprite],
                clip: None,
                blend: GpuBlend::Normal,
                gamma: false,
            }],
        );

        assert!(matches!(
            RetainedGpuRenderer::validate_scene(&scene, &GpuPresentation::identity(1, 1)),
            Err(GpuRendererError::ObjectOwnerLayerWithoutTexture)
        ));
    }

    #[test]
    fn compact_object_validation_requires_compatible_owner_texture() {
        let texture = GpuTextureId::fresh();
        let owner_texture = GpuTextureId::fresh();
        let sprite = GpuObjectSprite::new(
            [[0.0, 0.0, 1.0]; 4],
            [0.0, 0.0, 1.0, 1.0],
            [0x00ff_ffff; 4],
            GpuSampler::Nearest,
            0.0,
            false,
            GpuOuterModulation::Inherit,
        )
        .with_owner_layer();
        let command = GpuCommand::ObjectBatch {
            texture,
            owner_texture: Some(owner_texture),
            sprites: vec![sprite],
            clip: None,
            blend: GpuBlend::Normal,
            gamma: false,
        };
        let scene = |owner| {
            let mut textures = vec![rgba_resource(texture, [255; 4])];
            textures.extend(owner);
            test_scene(
                [1, 1],
                Color::transparent(),
                textures,
                vec![command.clone()],
            )
        };
        let presentation = GpuPresentation::identity(1, 1);

        assert!(matches!(
            RetainedGpuRenderer::validate_scene(&scene(None), &presentation),
            Err(GpuRendererError::MissingTexture(id)) if id == owner_texture
        ));
        assert!(matches!(
            RetainedGpuRenderer::validate_scene(
                &scene(Some(r8_resource(owner_texture, 255))),
                &presentation,
            ),
            Err(GpuRendererError::TextureFormatMismatch {
                id,
                expected: GpuTextureFormat::Rgba8,
                actual: GpuTextureFormat::R8,
            }) if id == owner_texture
        ));
        let mismatched = GpuTextureResource::immutable_rgba(
            owner_texture,
            2,
            1,
            Arc::from([255_u8; 8].as_slice()),
        );
        assert!(matches!(
            RetainedGpuRenderer::validate_scene(&scene(Some(mismatched)), &presentation),
            Err(GpuRendererError::ObjectTextureExtentMismatch {
                texture: actual_texture,
                owner_texture: actual_owner,
                texture_extent: [1, 1],
                owner_extent: [2, 1],
            }) if actual_texture == texture && actual_owner == owner_texture
        ));
    }

    #[test]
    fn compact_object_validation_rejects_mixed_replace_outer_policy() {
        let texture = GpuTextureId::fresh();
        let sprite = |outer_modulation| {
            GpuObjectSprite::new(
                [[0.0, 0.0, 1.0]; 4],
                [0.0, 0.0, 1.0, 1.0],
                [0x00ff_ffff; 4],
                GpuSampler::Nearest,
                0.0,
                false,
                outer_modulation,
            )
        };
        let scene = test_scene(
            [1, 1],
            Color::transparent(),
            vec![rgba_resource(texture, [255; 4])],
            vec![GpuCommand::ObjectBatch {
                texture,
                owner_texture: None,
                sprites: vec![
                    sprite(GpuOuterModulation::Combine),
                    sprite(GpuOuterModulation::Ignore),
                ],
                clip: None,
                blend: GpuBlend::Replace,
                gamma: false,
            }],
        );

        assert!(matches!(
            RetainedGpuRenderer::validate_scene(&scene, &GpuPresentation::identity(1, 1)),
            Err(GpuRendererError::MixedReplaceObjectOuterModulation { sprite: 1 })
        ));
    }

    #[test]
    fn compact_object_validation_rejects_reserved_packed_flags() {
        let texture = GpuTextureId::fresh();
        let sprite = GpuObjectSprite::new(
            [[0.0, 0.0, 1.0]; 4],
            [0.0, 0.0, 1.0, 1.0],
            [0x00ff_ffff; 4],
            GpuSampler::Nearest,
            0.0,
            false,
            GpuOuterModulation::Inherit,
        );
        #[repr(C)]
        struct RawObjectSprite {
            positions: [[f32; 3]; 4],
            uv: [f32; 4],
            modulation: [u32; 4],
            sample_tile_size: f32,
            flags: u32,
        }
        let raw = RawObjectSprite {
            positions: sprite.positions,
            uv: sprite.uv,
            modulation: sprite.modulation,
            sample_tile_size: sprite.sample_tile_size,
            flags: sprite.packed_flags() | (1 << 4),
        };
        // SAFETY: both `repr(C)` types have the same fields in the same order;
        // every `u32` flag bit pattern is valid memory even when semantically rejected.
        let sprite = unsafe { std::mem::transmute::<RawObjectSprite, GpuObjectSprite>(raw) };
        let scene = GpuScene {
            logical_extent: [1, 1],
            clear: Color::transparent(),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::identity()),
            gamma_mode: GpuGammaMode::Disabled,
            textures: vec![rgba_resource(texture, [255; 4])],
            commands: vec![GpuCommand::ObjectBatch {
                texture,
                owner_texture: None,
                sprites: vec![sprite],
                clip: None,
                blend: GpuBlend::Normal,
                gamma: false,
            }],
        };

        assert!(matches!(
            RetainedGpuRenderer::validate_scene(&scene, &GpuPresentation::identity(1, 1)),
            Err(GpuRendererError::InvalidObjectSpriteFlags { flags }) if flags == 1 << 4
        ));
    }

    #[test]
    fn compact_object_sprite_preserves_projective_corners_and_sampling_state() {
        let presentation = GpuPresentation {
            physical_extent: [17, 11],
            scale: 1.5,
            crop_top: 2,
            world_zoom: 1.0,
        };
        let projection = draw_projection(Some(Rect::new(1, 2, 6, 4)), [9, 8], &presentation)
            .expect("valid fractional presentation")
            .expect("clip intersects the framebuffer");
        let positions = [
            [1.25, 2.5, 1.0],
            [13.5, 5.0, 2.0],
            [3.75, 17.625, 3.0],
            [27.0, 23.5, 4.0],
        ];
        let modulation = [0x0011_2233, 0x4044_5566, 0x8077_8899, 0xc0aa_bbcc];
        let sprite = GpuObjectSprite::new(
            positions,
            [0.875, 0.25, 0.125, 0.75],
            modulation,
            GpuSampler::Linear,
            128.0,
            true,
            GpuOuterModulation::Combine,
        );

        let packed = packed_object_sprite_instance(sprite, true, &projection)
            .expect("pack compact object sprite");
        let expected_clip = positions.map(|position| {
            let clip = clip_position(position, &projection).expect("project object corner");
            [clip[0], clip[1], clip[3]]
        });

        assert_eq!(packed.clip, expected_clip);
        assert_eq!(packed.uv_rect, sprite.uv);
        assert_eq!(packed.modulation, modulation);
        assert_eq!(packed.sample_tile_size, 128.0);
        assert_eq!(packed.flags, sprite.packed_flags() | (1 << 4));
        assert!(std::mem::size_of::<PackedObjectSpriteInstance>() <= 96);
    }

    #[test]
    fn compact_owner_selector_survives_packing_without_colliding_with_gamma() {
        let projection = draw_projection(None, [1, 1], &GpuPresentation::identity(1, 1))
            .expect("valid presentation")
            .expect("object intersects presentation");
        let sprite = GpuObjectSprite::new(
            [
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
            ],
            [0.0, 0.0, 1.0, 1.0],
            [0x00ff_ffff; 4],
            GpuSampler::Nearest,
            0.0,
            false,
            GpuOuterModulation::Combine,
        )
        .with_owner_layer();

        let without_gamma = packed_object_sprite_instance(sprite, false, &projection)
            .expect("pack owner sprite without gamma");
        let with_gamma = packed_object_sprite_instance(sprite, true, &projection)
            .expect("pack owner sprite with gamma");

        assert_eq!(without_gamma.flags & (1 << 5), 1 << 5);
        assert_eq!(without_gamma.flags & (1 << 4), 0);
        assert_eq!(with_gamma.flags & (1 << 5), 1 << 5);
        assert_eq!(with_gamma.flags & (1 << 4), 1 << 4);
        assert_eq!(std::mem::size_of::<PackedObjectSpriteInstance>(), 88);
    }

    #[test]
    fn mixed_object_sampling_uses_one_ordered_draw_without_generic_instances() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping compact object sampling check");
            return;
        };
        let texture = GpuTextureId::fresh();
        let positions = [
            [0.0, 0.0, 1.0],
            [4.0, 0.0, 1.0],
            [0.0, 2.0, 1.0],
            [4.0, 2.0, 1.0],
        ];
        let nearest = GpuObjectSprite::new(
            positions,
            [0.0, 0.0, 1.0, 1.0],
            [0x00ff_ffff; 4],
            GpuSampler::Nearest,
            0.0,
            false,
            GpuOuterModulation::Combine,
        );
        let linear = GpuObjectSprite::new(
            positions,
            [1.0, 0.0, 0.0, 1.0],
            [0x4000_ffff, 0x40ff_00ff, 0x4000_ff00, 0x40ff_ffff],
            GpuSampler::Linear,
            2.0,
            false,
            GpuOuterModulation::Combine,
        );
        let generic_vertices = |sprite: GpuObjectSprite| {
            let uv = [
                [sprite.uv[0], sprite.uv[1]],
                [sprite.uv[2], sprite.uv[1]],
                [sprite.uv[0], sprite.uv[3]],
                [sprite.uv[2], sprite.uv[3]],
            ];
            std::array::from_fn(|index| {
                let packed = sprite.modulation[index];
                let modulation = [
                    ((packed >> 16) & 0xff) as f32 / 255.0,
                    ((packed >> 8) & 0xff) as f32 / 255.0,
                    (packed & 0xff) as f32 / 255.0,
                    (packed >> 24) as f32 / 255.0,
                ];
                let vertex = GpuVertex::new(sprite.positions[index], uv[index], modulation);
                if sprite.sampler() == GpuSampler::Linear {
                    vertex.with_sample_tile(0.0, 0.0, sprite.sample_tile_size)
                } else {
                    vertex
                }
            })
        };
        let resource = rgba_resource_2x1(texture, [255, 48, 16, 255], [16, 64, 255, 255]);
        let scene = |commands| {
            test_scene(
                [4, 2],
                Color::new(11, 19, 31, 255),
                vec![resource.clone()],
                commands,
            )
        };
        let compact = scene(vec![GpuCommand::ObjectBatch {
            texture,
            owner_texture: None,
            sprites: vec![nearest, linear],
            clip: None,
            blend: GpuBlend::Normal,
            gamma: false,
        }]);
        let expanded = scene(vec![
            GpuCommand::Quad {
                texture,
                owner_mask: None,
                vertices: generic_vertices(nearest),
                clip: None,
                blend: GpuBlend::Normal,
                base_mod2: nearest.mod2(),
                owner_mod2: false,
                sampler: nearest.sampler(),
                gamma: false,
            },
            GpuCommand::Quad {
                texture,
                owner_mask: None,
                vertices: generic_vertices(linear),
                clip: None,
                blend: GpuBlend::Normal,
                base_mod2: linear.mod2(),
                owner_mod2: false,
                sampler: linear.sampler(),
                gamma: false,
            },
        ]);
        let mut renderer = test_renderer(&device, &queue);

        let compact_frame = render_identity_readback(&mut renderer, &device, &queue, &compact);
        let compact_stats = renderer.last_stats();
        let expanded_frame = render_identity_readback(&mut renderer, &device, &queue, &expanded);

        assert_eq!(compact_frame, expanded_frame);
        assert_eq!(compact_stats.draw_calls, 1);
        assert_eq!(compact_stats.object_sprite_instances, 2);
        assert_eq!(compact_stats.quad_instances, 0);
        assert_eq!(renderer.last_stats().draw_calls, 2);
    }

    #[test]
    fn owner_texture_pair_matches_explicit_base_owner_painter_sequence() {
        // LegacyClonk StdDDraw2.cpp:759-778 submits the base pass before the
        // owner-color pass for each face.
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping compact owner-pair parity check");
            return;
        };
        let texture = GpuTextureId::fresh();
        let owner_texture = GpuTextureId::fresh();
        let base_resource = rgba_resource_2x1(texture, [220, 40, 20, 160], [20, 180, 60, 192]);
        let owner_resource =
            rgba_resource_2x1(owner_texture, [40, 80, 240, 144], [240, 200, 20, 112]);
        let gamma = GpuGammaLut::from_ramp(&GammaRamp::from_control_points([
            0x102030, 0x708090, 0xd0e0f0,
        ]));
        let positions = |left: f32, right: f32, w: [f32; 4]| {
            [
                [left * w[0], 0.0, w[0]],
                [right * w[1], 0.0, w[1]],
                [left * w[2], 2.0 * w[2], w[2]],
                [right * w[3], 2.0 * w[3], w[3]],
            ]
        };
        let normalized = |packed: u32| {
            [
                ((packed >> 16) & 0xff) as f32 / 255.0,
                ((packed >> 8) & 0xff) as f32 / 255.0,
                (packed & 0xff) as f32 / 255.0,
                (packed >> 24) as f32 / 255.0,
            ]
        };
        let generic_vertices = |sprite: GpuObjectSprite| {
            let uv = [
                [sprite.uv[0], sprite.uv[1]],
                [sprite.uv[2], sprite.uv[1]],
                [sprite.uv[0], sprite.uv[3]],
                [sprite.uv[2], sprite.uv[3]],
            ];
            std::array::from_fn(|index| {
                let vertex = GpuVertex::new(
                    sprite.positions[index],
                    uv[index],
                    normalized(sprite.modulation[index]),
                );
                if sprite.sampler() == GpuSampler::Linear {
                    vertex.with_sample_tile(0.0, 0.0, sprite.sample_tile_size)
                } else {
                    vertex
                }
            })
        };
        let mut renderer = test_renderer(&device, &queue);

        for sampler in [GpuSampler::Nearest, GpuSampler::Linear] {
            let sample_tile_size = if sampler == GpuSampler::Linear {
                2.0
            } else {
                0.0
            };
            let sprite = |positions, uv, modulation, mod2| {
                GpuObjectSprite::new(
                    positions,
                    uv,
                    modulation,
                    sampler,
                    sample_tile_size,
                    mod2,
                    GpuOuterModulation::Combine,
                )
            };
            let base_1 = sprite(
                positions(0.0, 2.25, [1.0, 1.2, 0.9, 1.1]),
                [0.0, 1.0, 0.5, 0.0],
                [0x0010_f0c0, 0x1020_e0b0, 0x2030_d0a0, 0x3040_c090],
                false,
            );
            let owner_1 = sprite(
                base_1.positions,
                base_1.uv,
                [0x0020_c0ff, 0x1030_b0ef, 0x2040_a0df, 0x3050_90cf],
                true,
            )
            .with_owner_layer();
            let base_2 = sprite(
                positions(0.75, 3.0, [1.1, 0.95, 1.25, 1.0]),
                [1.0, 0.0, 0.5, 1.0],
                [0x2040_ff80, 0x3050_ef70, 0x4060_df60, 0x5070_cf50],
                true,
            );
            let owner_2 = sprite(
                base_2.positions,
                base_2.uv,
                [0x1000_80ff, 0x2010_70ef, 0x3020_60df, 0x4030_50cf],
                false,
            )
            .with_owner_layer();
            for gamma_mode in [
                GpuGammaMode::Disabled,
                GpuGammaMode::Fragment,
                GpuGammaMode::Monitor,
            ] {
                for blend in [GpuBlend::Normal, GpuBlend::Additive, GpuBlend::Replace] {
                    let scene = |commands| GpuScene {
                        logical_extent: [3, 2],
                        clear: Color::new(11, 19, 31, 113),
                        gamma: gamma.clone(),
                        gamma_mode,
                        textures: vec![base_resource.clone(), owner_resource.clone()],
                        commands,
                    };
                    let compact = scene(vec![GpuCommand::ObjectBatch {
                        texture,
                        owner_texture: Some(owner_texture),
                        sprites: vec![base_1, owner_1, base_2, owner_2],
                        clip: None,
                        blend,
                        gamma: true,
                    }]);
                    let generic = |texture, sprite: GpuObjectSprite| GpuCommand::Quad {
                        texture,
                        owner_mask: None,
                        vertices: generic_vertices(sprite),
                        clip: None,
                        blend,
                        base_mod2: sprite.mod2(),
                        owner_mod2: false,
                        sampler: sprite.sampler(),
                        gamma: true,
                    };
                    let expanded = scene(vec![
                        generic(texture, base_1),
                        generic(owner_texture, owner_1),
                        generic(texture, base_2),
                        generic(owner_texture, owner_2),
                    ]);

                    let compact_frame =
                        render_identity_readback(&mut renderer, &device, &queue, &compact);
                    let compact_stats = renderer.last_stats();
                    let expanded_frame =
                        render_identity_readback(&mut renderer, &device, &queue, &expanded);

                    assert_eq!(
                        compact_frame, expanded_frame,
                        "{sampler:?}, {gamma_mode:?}, {blend:?} owner pair"
                    );
                    assert_eq!(compact_stats.object_sprite_instances, 4);
                    assert_eq!(compact_stats.object_sprite_upload_bytes, 4 * 88);
                    assert_eq!(compact_stats.quad_instances, 0);
                    assert_eq!(compact_stats.draw_calls, 1);
                    assert_eq!(renderer.last_stats().quad_instances, 4);
                    assert_eq!(renderer.last_stats().draw_calls, 4);
                }
            }
        }
    }

    #[test]
    fn fog_chunked_owner_pair_with_ownclr_matches_explicit_painter_passes() {
        // LegacyClonk StdDDraw2.cpp:759-778 paints base then owner, and
        // StdDDraw2.cpp:773-777 leaves OWNCLR owner modulation untouched.
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping fogged owner-pair parity check");
            return;
        };
        let texture = GpuTextureId::fresh();
        let owner_texture = GpuTextureId::fresh();
        let resource = |id, owner: bool| {
            let mut pixels = Vec::with_capacity(128 * 64 * 4);
            for y in 0..64_u32 {
                for x in 0..128_u32 {
                    let pixel = if owner {
                        [(x * 3) as u8, (y * 5) as u8, (x ^ y) as u8, 144]
                    } else {
                        [(x * 2) as u8, (y * 4) as u8, (x + y) as u8, 208]
                    };
                    pixels.extend_from_slice(&pixel);
                }
            }
            GpuTextureResource::immutable_rgba(id, 128, 64, Arc::from(pixels.into_boxed_slice()))
        };
        let normalized = |packed: u32| {
            [
                ((packed >> 16) & 0xff) as f32 / 255.0,
                ((packed >> 8) & 0xff) as f32 / 255.0,
                (packed & 0xff) as f32 / 255.0,
                (packed >> 24) as f32 / 255.0,
            ]
        };
        let generic = |texture, sprite: GpuObjectSprite| {
            let uv = [
                [sprite.uv[0], sprite.uv[1]],
                [sprite.uv[2], sprite.uv[1]],
                [sprite.uv[0], sprite.uv[3]],
                [sprite.uv[2], sprite.uv[3]],
            ];
            let vertices = std::array::from_fn(|index| {
                let vertex = GpuVertex::new(
                    sprite.positions[index],
                    uv[index],
                    normalized(sprite.modulation[index]),
                )
                .with_outer_modulation(sprite.outer_modulation());
                if sprite.sampler() == GpuSampler::Linear {
                    vertex.with_sample_tile(0.0, 0.0, sprite.sample_tile_size)
                } else {
                    vertex
                }
            });
            GpuCommand::Quad {
                texture,
                owner_mask: None,
                vertices,
                clip: None,
                blend: GpuBlend::Normal,
                base_mod2: sprite.mod2(),
                owner_mod2: false,
                sampler: sprite.sampler(),
                gamma: false,
            }
        };
        let positions = |left: f32, right: f32| {
            [
                [left, 0.0, 1.0],
                [right, 0.0, 1.0],
                [left, 2.0, 1.0],
                [right, 2.0, 1.0],
            ]
        };
        let mut renderer = test_renderer(&device, &queue);

        for sampler in [GpuSampler::Nearest, GpuSampler::Linear] {
            let sample_tile_size = if sampler == GpuSampler::Linear {
                64.0
            } else {
                0.0
            };
            let sprite = |positions, uv, modulation, outer_modulation| {
                GpuObjectSprite::new(
                    positions,
                    uv,
                    modulation,
                    sampler,
                    sample_tile_size,
                    false,
                    outer_modulation,
                )
            };
            let base = [
                sprite(
                    positions(0.0, 2.0),
                    [0.25, 0.0, 0.5, 1.0],
                    [0x0010_f0c0, 0x1020_e0b0, 0x2030_d0a0, 0x3040_c090],
                    GpuOuterModulation::Combine,
                ),
                sprite(
                    positions(2.0, 4.0),
                    [0.5, 0.0, 0.75, 1.0],
                    [0x1040_ff80, 0x2050_ef70, 0x3060_df60, 0x4070_cf50],
                    GpuOuterModulation::Combine,
                ),
            ];
            let owner = [
                sprite(
                    base[0].positions,
                    base[0].uv,
                    [0x0020_c0ff, 0x1030_b0ef, 0x2040_a0df, 0x3050_90cf],
                    GpuOuterModulation::Ignore,
                )
                .with_owner_layer(),
                sprite(
                    base[1].positions,
                    base[1].uv,
                    [0x1000_80ff, 0x2010_70ef, 0x3020_60df, 0x4030_50cf],
                    GpuOuterModulation::Ignore,
                )
                .with_owner_layer(),
            ];
            let original_owner_modulation = owner.map(|sprite| sprite.modulation);
            let mut compact_command = GpuCommand::ObjectBatch {
                texture,
                owner_texture: Some(owner_texture),
                sprites: vec![base[0], base[1], owner[0], owner[1]],
                clip: None,
                blend: GpuBlend::Normal,
                gamma: false,
            };
            compact_command
                .apply_packed_c4_modulation(0x4080_ff40)
                .expect("compact C4 colors accept enclosing modulation");
            let mut generic_commands = vec![
                generic(texture, base[0]),
                generic(texture, base[1]),
                generic(owner_texture, owner[0]),
                generic(owner_texture, owner[1]),
            ];
            for command in &mut generic_commands {
                command
                    .apply_packed_c4_modulation(0x4080_ff40)
                    .expect("generic C4 colors accept enclosing modulation");
            }
            let GpuCommand::ObjectBatch { sprites, .. } = &compact_command else {
                unreachable!("the compact command remains an object batch");
            };
            assert_ne!(sprites[0].modulation, base[0].modulation);
            assert_eq!(
                [sprites[2].modulation, sprites[3].modulation],
                original_owner_modulation,
                "CLRSFC_OWNCLR suppresses enclosing modulation on owner layers"
            );

            let scene = |commands| GpuScene {
                logical_extent: [4, 2],
                clear: Color::new(9, 17, 25, 101),
                gamma: GpuGammaLut::from_ramp(&GammaRamp::identity()),
                gamma_mode: GpuGammaMode::Disabled,
                textures: vec![resource(texture, false), resource(owner_texture, true)],
                commands,
            };
            let compact = scene(vec![compact_command]);
            let expanded = scene(generic_commands);
            let compact_frame = render_identity_readback(&mut renderer, &device, &queue, &compact);
            let compact_stats = renderer.last_stats();
            let expanded_frame =
                render_identity_readback(&mut renderer, &device, &queue, &expanded);

            assert_eq!(compact_frame, expanded_frame, "{sampler:?} fog chunks");
            assert_eq!(compact_stats.object_sprite_instances, 4);
            assert_eq!(compact_stats.draw_calls, 1);
            assert_eq!(renderer.last_stats().quad_instances, 4);
            assert_eq!(
                renderer.last_stats().draw_calls,
                2,
                "the explicit reference coalesces its adjacent base chunks and owner chunks"
            );
        }
    }

    #[test]
    fn owner_pair_draws_split_exactly_at_run_boundaries() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping compact owner-pair run check");
            return;
        };
        let texture = GpuTextureId::fresh();
        let owner_a = GpuTextureId::fresh();
        let owner_b = GpuTextureId::fresh();
        let sprite = |outer_modulation| {
            GpuObjectSprite::new(
                [
                    [0.0, 0.0, 1.0],
                    [1.0, 0.0, 1.0],
                    [0.0, 1.0, 1.0],
                    [1.0, 1.0, 1.0],
                ],
                [0.0, 0.0, 1.0, 1.0],
                [0x00ff_ffff; 4],
                GpuSampler::Nearest,
                0.0,
                false,
                outer_modulation,
            )
            .with_owner_layer()
        };
        let command =
            |owner_texture, clip, blend, gamma, outer_modulation| GpuCommand::ObjectBatch {
                texture,
                owner_texture: Some(owner_texture),
                sprites: vec![sprite(outer_modulation)],
                clip,
                blend,
                gamma,
            };
        let scene = GpuScene {
            logical_extent: [1, 1],
            clear: Color::transparent(),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
            gamma_mode: GpuGammaMode::Fragment,
            textures: vec![
                rgba_resource(texture, [0, 0, 0, 0]),
                rgba_resource(owner_a, [255, 64, 16, 96]),
                rgba_resource(owner_b, [16, 64, 255, 96]),
            ],
            commands: vec![
                command(
                    owner_a,
                    None,
                    GpuBlend::Normal,
                    false,
                    GpuOuterModulation::Combine,
                ),
                command(
                    owner_a,
                    None,
                    GpuBlend::Normal,
                    false,
                    GpuOuterModulation::Combine,
                ),
                command(
                    owner_b,
                    None,
                    GpuBlend::Normal,
                    false,
                    GpuOuterModulation::Combine,
                ),
                command(
                    owner_a,
                    None,
                    GpuBlend::Normal,
                    false,
                    GpuOuterModulation::Combine,
                ),
                command(
                    owner_a,
                    Some(Rect::new(0, 0, 1, 1)),
                    GpuBlend::Normal,
                    false,
                    GpuOuterModulation::Combine,
                ),
                command(
                    owner_a,
                    None,
                    GpuBlend::Additive,
                    false,
                    GpuOuterModulation::Combine,
                ),
                command(
                    owner_a,
                    None,
                    GpuBlend::Normal,
                    true,
                    GpuOuterModulation::Combine,
                ),
                command(
                    owner_a,
                    None,
                    GpuBlend::Replace,
                    false,
                    GpuOuterModulation::Combine,
                ),
                command(
                    owner_a,
                    None,
                    GpuBlend::Replace,
                    false,
                    GpuOuterModulation::Ignore,
                ),
            ],
        };
        let mut renderer = test_renderer(&device, &queue);

        let _ = render_identity_readback(&mut renderer, &device, &queue, &scene);
        let stats = renderer.last_stats();

        assert_eq!(stats.object_sprite_instances, 9);
        assert_eq!(stats.object_sprite_upload_bytes, 9 * 88);
        assert_eq!(stats.object_sprite_draw_calls, 8);
        assert_eq!(stats.draw_calls, 8);
    }

    #[test]
    fn owner_pair_bind_group_is_invalidated_when_owner_view_is_recreated() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping owner bind-group invalidation check");
            return;
        };
        let texture = GpuTextureId::fresh();
        let owner_texture = GpuTextureId::fresh();
        let sprite = GpuObjectSprite::new(
            [
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
            ],
            [0.0, 0.0, 1.0, 1.0],
            [0x00ff_ffff; 4],
            GpuSampler::Nearest,
            0.0,
            false,
            GpuOuterModulation::Combine,
        )
        .with_owner_layer();
        let scene = test_scene(
            [1, 1],
            Color::transparent(),
            vec![
                rgba_resource(texture, [0; 4]),
                rgba_resource(owner_texture, [255; 4]),
            ],
            vec![GpuCommand::ObjectBatch {
                texture,
                owner_texture: Some(owner_texture),
                sprites: vec![sprite],
                clip: None,
                blend: GpuBlend::Normal,
                gamma: false,
            }],
        );
        let mut renderer = test_renderer(&device, &queue);
        let _ = render_identity_readback(&mut renderer, &device, &queue, &scene);
        assert_eq!(renderer.object_bind_groups.len(), 1);

        let replacement = GpuTextureResource::immutable_rgba(
            owner_texture,
            2,
            1,
            Arc::from([255_u8; 8].as_slice()),
        );
        renderer
            .sync_textures(&device, &queue, &[replacement])
            .expect("replace owner texture view");

        assert!(renderer.object_bind_groups.is_empty());
    }

    #[test]
    fn compact_fog_chunks_match_generic_pixels_through_both_axis_flips() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping compact fog-chunk parity check");
            return;
        };
        let texture = GpuTextureId::fresh();
        let mut pixels = Vec::with_capacity(128 * 128 * 4);
        for y in 0..128_u8 {
            for x in 0..128_u8 {
                pixels.extend_from_slice(&[x.wrapping_mul(3), y.wrapping_mul(5), x ^ y, 223]);
            }
        }
        let resource = GpuTextureResource::immutable_rgba(texture, 128, 128, pixels.into());
        let normalized = |packed: u32| {
            [
                ((packed >> 16) & 0xff) as f32 / 255.0,
                ((packed >> 8) & 0xff) as f32 / 255.0,
                (packed & 0xff) as f32 / 255.0,
                (packed >> 24) as f32 / 255.0,
            ]
        };
        let gamma = GpuGammaLut::from_ramp(&GammaRamp::from_control_points([
            0x102030, 0x708090, 0xd0e0f0,
        ]));

        for flip_x in [false, true] {
            for flip_y in [false, true] {
                let map_x = |local: f32| {
                    if flip_x {
                        30.0 - local * 2.0
                    } else {
                        local * 2.0
                    }
                };
                let map_y = |local: f32| {
                    if flip_y {
                        30.0 - local * 2.0
                    } else {
                        local * 2.0
                    }
                };
                let mut sprites = Vec::new();
                let mut generic = Vec::new();
                for (chunk_index, ((left, right), (top, bottom))) in [
                    ((0.0, 4.0), (0.0, 7.0)),
                    ((4.0, 15.0), (0.0, 7.0)),
                    ((0.0, 4.0), (7.0, 15.0)),
                    ((4.0, 15.0), (7.0, 15.0)),
                ]
                .into_iter()
                .enumerate()
                {
                    let positions = [
                        [map_x(left), map_y(top), 1.0],
                        [map_x(right), map_y(top), 1.0],
                        [map_x(left), map_y(bottom), 1.0],
                        [map_x(right), map_y(bottom), 1.0],
                    ];
                    let uv = [
                        (60.0 + left) / 128.0,
                        (57.0 + top) / 128.0,
                        (60.0 + right) / 128.0,
                        (57.0 + bottom) / 128.0,
                    ];
                    let base = (chunk_index as u32 + 1) * 0x0011_0b07;
                    let modulation = [
                        base,
                        base.saturating_add(0x1017_130d),
                        base.saturating_add(0x2029_1d11),
                        base.saturating_add(0x303b_2715),
                    ];
                    let mod2 = chunk_index % 2 != 0;
                    sprites.push(GpuObjectSprite::new(
                        positions,
                        uv,
                        modulation,
                        GpuSampler::Linear,
                        128.0,
                        mod2,
                        GpuOuterModulation::Combine,
                    ));
                    let source_uv = [
                        [uv[0], uv[1]],
                        [uv[2], uv[1]],
                        [uv[0], uv[3]],
                        [uv[2], uv[3]],
                    ];
                    let vertices = std::array::from_fn(|index| {
                        GpuVertex::new(
                            positions[index],
                            source_uv[index],
                            normalized(modulation[index]),
                        )
                        .with_sample_tile(0.0, 0.0, 128.0)
                    });
                    generic.push(GpuCommand::Quad {
                        texture,
                        owner_mask: None,
                        vertices,
                        clip: None,
                        blend: GpuBlend::Normal,
                        base_mod2: mod2,
                        owner_mod2: false,
                        sampler: GpuSampler::Linear,
                        gamma: true,
                    });
                }
                let scene = |commands| GpuScene {
                    logical_extent: [30, 30],
                    clear: Color::new(17, 29, 43, 113),
                    gamma: gamma.clone(),
                    gamma_mode: GpuGammaMode::Fragment,
                    textures: vec![resource.clone()],
                    commands,
                };
                let compact = scene(vec![GpuCommand::ObjectBatch {
                    texture,
                    owner_texture: None,
                    sprites,
                    clip: None,
                    blend: GpuBlend::Normal,
                    gamma: true,
                }]);
                let expanded = scene(generic);
                let mut renderer = test_renderer(&device, &queue);

                let compact_frame =
                    render_identity_readback(&mut renderer, &device, &queue, &compact);
                let compact_stats = renderer.last_stats();
                let expanded_frame =
                    render_identity_readback(&mut renderer, &device, &queue, &expanded);

                assert_eq!(
                    compact_frame, expanded_frame,
                    "flip_x={flip_x}, flip_y={flip_y}"
                );
                assert_eq!(compact_stats.draw_calls, 1);
                assert_eq!(compact_stats.object_sprite_instances, 4);
                assert_eq!(compact_stats.object_sprite_upload_bytes, 4 * 88);
                assert_eq!(compact_stats.quad_instance_upload_bytes, 0);
                assert_eq!(renderer.last_stats().quad_instance_upload_bytes, 4 * 232);
            }
        }
    }

    #[test]
    fn compact_top_faces_and_generic_construction_fallback_match_expanded_pixels() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping compact TopFace/fallback parity check");
            return;
        };
        let object_texture = GpuTextureId::fresh();
        let construction_texture = GpuTextureId::fresh();
        let object_resource = GpuTextureResource::immutable_rgba(
            object_texture,
            4,
            1,
            Arc::from(
                [
                    220, 32, 48, 192, 220, 32, 48, 192, 24, 216, 72, 176, 24, 216, 72, 176,
                ]
                .as_slice(),
            ),
        );
        let construction_resource = GpuTextureResource::immutable_rgba(
            construction_texture,
            1,
            1,
            Arc::from([32, 72, 232, 255].as_slice()),
        );
        let positions = |left: f32, top: f32, right: f32, bottom: f32| {
            [
                [left, top, 1.0],
                [right, top, 1.0],
                [left, bottom, 1.0],
                [right, bottom, 1.0],
            ]
        };
        let sprite = |positions, uv, modulation| {
            GpuObjectSprite::new(
                positions,
                uv,
                [modulation; 4],
                GpuSampler::Nearest,
                0.0,
                false,
                GpuOuterModulation::Combine,
            )
        };
        // This is the native list-wide order: every object base precedes every
        // TopFace, and the global construction facet remains a generic barrier.
        let object_sprites = vec![
            sprite(
                positions(0.0, 0.0, 4.0, 4.0),
                [0.0, 0.0, 0.5, 1.0],
                0x00ff_ffff,
            ),
            sprite(
                positions(1.0, 0.0, 5.0, 4.0),
                [0.0, 0.0, 0.5, 1.0],
                0x00d0_ffff,
            ),
            sprite(
                positions(1.0, 1.0, 4.0, 4.0),
                [0.5, 0.0, 1.0, 1.0],
                0x00ff_d0ff,
            ),
            sprite(
                positions(2.0, 1.0, 5.0, 4.0),
                [0.5, 0.0, 1.0, 1.0],
                0x00ff_ffff,
            ),
        ];
        let construction = sprite(
            positions(0.0, 2.0, 3.0, 5.0),
            [0.0, 0.0, 1.0, 1.0],
            0x00ff_ffff,
        );
        let normalized_modulation = |packed: u32| {
            [
                ((packed >> 16) & 0xff) as f32 / 255.0,
                ((packed >> 8) & 0xff) as f32 / 255.0,
                (packed & 0xff) as f32 / 255.0,
                (packed >> 24) as f32 / 255.0,
            ]
        };
        let generic_vertices = |sprite: GpuObjectSprite| {
            let uv = [
                [sprite.uv[0], sprite.uv[1]],
                [sprite.uv[2], sprite.uv[1]],
                [sprite.uv[0], sprite.uv[3]],
                [sprite.uv[2], sprite.uv[3]],
            ];
            std::array::from_fn(|index| {
                GpuVertex::new(
                    sprite.positions[index],
                    uv[index],
                    normalized_modulation(sprite.modulation[index]),
                )
                .with_outer_modulation(GpuOuterModulation::Combine)
            })
        };
        let generic_quad = |texture, sprite: GpuObjectSprite| GpuCommand::Quad {
            texture,
            owner_mask: None,
            vertices: generic_vertices(sprite),
            clip: None,
            blend: GpuBlend::Normal,
            base_mod2: false,
            owner_mod2: false,
            sampler: GpuSampler::Nearest,
            gamma: false,
        };
        let scene = |commands| GpuScene {
            logical_extent: [5, 5],
            clear: Color::opaque(8, 12, 24),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::identity()),
            gamma_mode: GpuGammaMode::Disabled,
            textures: vec![object_resource.clone(), construction_resource.clone()],
            commands,
        };
        let compact = scene(vec![
            GpuCommand::ObjectBatch {
                texture: object_texture,
                owner_texture: None,
                sprites: object_sprites.clone(),
                clip: None,
                blend: GpuBlend::Normal,
                gamma: false,
            },
            generic_quad(construction_texture, construction),
        ]);
        let mut expanded_commands = object_sprites
            .iter()
            .copied()
            .map(|sprite| generic_quad(object_texture, sprite))
            .collect::<Vec<_>>();
        expanded_commands.push(generic_quad(construction_texture, construction));
        let expanded = scene(expanded_commands);
        let mut renderer = test_renderer(&device, &queue);

        let compact_frame = render_identity_readback(&mut renderer, &device, &queue, &compact);
        let expanded_frame = render_identity_readback(&mut renderer, &device, &queue, &expanded);

        assert_eq!(compact_frame, expanded_frame);
        assert!(compact_frame
            .rgba
            .chunks_exact(4)
            .any(|pixel| pixel == [32, 72, 232, 255]));
    }

    #[test]
    fn compatible_pxs_line_commands_share_one_draw_of_compact_instances() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping compact PXS fragment check");
            return;
        };
        const SOLID: [u8; 4] = [255, 64, 32, 255];
        const CLEAR: [u8; 4] = [0, 0, 0, 255];
        let line = |row: f32| GpuCommand::Solid {
            vertices: vec![
                solid_vertex(0.5, row, rgba_f32(SOLID)),
                solid_vertex(3.5, row, rgba_f32(SOLID)),
            ],
            topology: GpuPrimitiveTopology::LineList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: None,
            blend: GpuBlend::Replace,
            style: GpuSolidStyle::NONE,
        };
        let scene = test_scene(
            [6, 4],
            Color::new(CLEAR[0], CLEAR[1], CLEAR[2], CLEAR[3]),
            Vec::new(),
            vec![line(0.5), line(2.5)],
        );
        let presentation = GpuPresentation::identity(6, 4);
        let mut renderer = test_renderer(&device, &queue);

        let frame = render_readback(&mut renderer, &device, &queue, &scene, &presentation);
        let stats = renderer.last_stats();

        let projection = draw_projection(None, [6, 4], &presentation)
            .expect("valid fixture presentation")
            .expect("fixture clip intersects the framebuffer");
        let mut covered = HashSet::new();
        for row in [0.5_f32, 2.5] {
            walk_aliased_line_fragments(
                solid_vertex(0.5, row, rgba_f32(SOLID)),
                solid_vertex(3.5, row, rgba_f32(SOLID)),
                &projection,
                |x, y, _| {
                    covered.insert((x, y));
                    Ok(())
                },
            )
            .expect("walk fixture line");
        }

        assert_eq!(stats.solid_rect_instances, covered.len());
        assert_eq!(
            stats.solid_rect_upload_bytes,
            covered.len() * PACKED_SOLID_RECT_INSTANCE_STRIDE as usize
        );
        assert_eq!(
            stats.draw_calls, 1,
            "compatible fragment runs share one painter-order draw"
        );
        for y in 0..4 {
            for x in 0..6 {
                let expected = if covered.contains(&(i64::from(x), i64::from(y))) {
                    SOLID
                } else {
                    CLEAR
                };
                assert_eq!(
                    readback_pixel(&frame, x, y),
                    expected,
                    "PXS fragment ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn compact_sprite_matches_expanded_quad_modulation_modes_and_blends() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping compact sprite parity check");
            return;
        };
        let texture = GpuTextureId::fresh();
        let packed_modulation = 0x407f_3fc0;
        let normalized_modulation = [127.0 / 255.0, 63.0 / 255.0, 192.0 / 255.0, 64.0 / 255.0];
        let gamma = GpuGammaLut::from_ramp(&GammaRamp::from_control_points([
            0x102030, 0x708090, 0xd0e0f0,
        ]));
        let scene = |command| GpuScene {
            logical_extent: [2, 2],
            clear: Color::new(17, 29, 43, 113),
            gamma: gamma.clone(),
            gamma_mode: GpuGammaMode::Fragment,
            textures: vec![rgba_resource(texture, [96, 144, 208, 191])],
            commands: vec![command],
        };
        let mut renderer = test_renderer(&device, &queue);

        for blend in [GpuBlend::Normal, GpuBlend::Additive] {
            for mod2 in [false, true] {
                let expanded = scene(GpuCommand::Quad {
                    texture,
                    owner_mask: None,
                    vertices: quad(0.0, 0.0, 2.0, 2.0, 1.0, normalized_modulation),
                    clip: None,
                    blend,
                    base_mod2: mod2,
                    owner_mod2: false,
                    sampler: GpuSampler::Nearest,
                    gamma: true,
                });
                let compact = scene(GpuCommand::SpriteBatch {
                    texture,
                    quads: vec![GpuSpriteQuad {
                        rect: [0.0, 0.0, 2.0, 2.0],
                        uv: [0.0, 0.0, 1.0, 1.0],
                        modulation: packed_modulation,
                    }],
                    clip: None,
                    blend,
                    mod2,
                    gamma: true,
                    outer_modulation: clonk_graphics::GpuOuterModulation::Combine,
                });

                let expanded = render_identity_readback(&mut renderer, &device, &queue, &expanded);
                let compact = render_identity_readback(&mut renderer, &device, &queue, &compact);

                assert_eq!(
                    compact, expanded,
                    "compact sprite differs for {blend:?}, mod2={mod2}",
                );
            }
        }
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
    fn compatible_particle_quads_share_one_painter_order_draw_call() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping particle draw-call batching check");
            return;
        };
        let texture = GpuTextureId::fresh();
        let command = |modulation| GpuCommand::Quad {
            texture,
            owner_mask: None,
            vertices: quad(0.0, 0.0, 1.0, 1.0, 1.0, modulation),
            clip: None,
            blend: GpuBlend::Normal,
            base_mod2: false,
            owner_mod2: false,
            sampler: GpuSampler::Nearest,
            gamma: false,
        };
        let commands = vec![
            command([1.0, 0.0, 0.0, 127.0 / 255.0]),
            command([0.0, 1.0, 0.0, 127.0 / 255.0]),
        ];
        let scene = test_scene(
            [1, 1],
            Color::transparent(),
            vec![rgba_resource(texture, [255; 4])],
            commands,
        );
        let mut renderer = test_renderer(&device, &queue);

        let frame = render_identity_readback(&mut renderer, &device, &queue, &scene);

        assert_eq!(frame.rgba, vec![64, 128, 0, 192]);
        assert_eq!(renderer.last_stats().draw_calls, 1);
    }

    #[test]
    fn compatible_fogged_landscape_chunks_share_one_painter_order_draw_call() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping landscape draw-call coalescing check");
            return;
        };
        let shared_base = GpuTextureId::fresh();
        let split_first_base = GpuTextureId::fresh();
        let split_second_base = GpuTextureId::fresh();
        let clip = Some(Rect::new(1, 0, 1, 1));
        let chunk = |base, modulation| GpuCommand::Landscape {
            base,
            liquid_mask: None,
            liquid: None,
            vertices: quad(0.0, 0.0, 3.0, 1.0, 1.0, modulation),
            clip,
            phase: [0.0; 3],
            gamma: false,
        };
        let scene =
            |textures, commands| test_scene([3, 1], Color::transparent(), textures, commands);
        let coalesced = scene(
            vec![rgba_resource(shared_base, [255; 4])],
            vec![
                chunk(shared_base, [1.0, 0.0, 0.0, 127.0 / 255.0]),
                chunk(shared_base, [0.0, 1.0, 0.0, 127.0 / 255.0]),
            ],
        );
        let forced_split = scene(
            vec![
                rgba_resource(split_first_base, [255; 4]),
                rgba_resource(split_second_base, [255; 4]),
            ],
            vec![
                chunk(split_first_base, [1.0, 0.0, 0.0, 127.0 / 255.0]),
                chunk(split_second_base, [0.0, 1.0, 0.0, 127.0 / 255.0]),
            ],
        );
        let mut renderer = test_renderer(&device, &queue);

        let coalesced_frame = render_identity_readback(&mut renderer, &device, &queue, &coalesced);
        assert_eq!(renderer.last_stats().draw_calls, 1);
        assert_eq!(renderer.last_stats().generic_vertices, 0);
        assert_eq!(renderer.last_stats().generic_vertex_upload_bytes, 0);
        assert_eq!(renderer.last_stats().landscape_instances, 2);
        assert_eq!(
            renderer.last_stats().landscape_instance_upload_bytes,
            2 * PACKED_LANDSCAPE_INSTANCE_STRIDE as usize
        );
        let split_frame = render_identity_readback(&mut renderer, &device, &queue, &forced_split);

        assert_eq!(renderer.last_stats().draw_calls, 2);
        assert_eq!(coalesced_frame, split_frame);
        assert_eq!(
            coalesced_frame.rgba,
            vec![0, 0, 0, 0, 64, 128, 0, 192, 0, 0, 0, 0]
        );
    }

    #[test]
    fn compact_landscape_matches_the_forced_generic_path() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping compact landscape parity check");
            return;
        };
        let texture = GpuTextureId::fresh();
        let modulation = [
            [255.0 / 255.0, 31.0 / 255.0, 63.0 / 255.0, 0.0],
            [127.0 / 255.0, 255.0 / 255.0, 31.0 / 255.0, 0.0],
            [63.0 / 255.0, 31.0 / 255.0, 255.0 / 255.0, 0.0],
            [255.0 / 255.0, 127.0 / 255.0, 63.0 / 255.0, 0.0],
        ];
        let compact_vertices = modulated_quad(0.0, 0.0, 3.0, 2.0, modulation);
        let command = |vertices| GpuCommand::Landscape {
            base: texture,
            liquid_mask: None,
            liquid: None,
            vertices,
            clip: Some(Rect::new(1, 0, 2, 2)),
            phase: [0.25, -0.5, 0.75],
            gamma: true,
        };
        let scene = |vertices| {
            let mut scene = test_scene(
                [3, 2],
                Color::new(11, 19, 31, 47),
                vec![rgba_resource(texture, [196, 143, 89, 223])],
                vec![command(vertices)],
            );
            scene.gamma = GpuGammaLut::from_ramp(&GammaRamp::from_control_points([
                0x102030, 0x708090, 0xd0e0f0,
            ]));
            scene.gamma_mode = GpuGammaMode::Fragment;
            scene
        };
        let mut renderer = test_renderer(&device, &queue);
        let presentation = GpuPresentation {
            physical_extent: [5, 4],
            scale: 1.5,
            crop_top: 1,
            world_zoom: 1.0,
        };

        for smooth in [false, true] {
            renderer.set_smooth_landscape(smooth);
            for gamma_mode in [
                GpuGammaMode::Disabled,
                GpuGammaMode::Fragment,
                GpuGammaMode::Monitor,
            ] {
                let mut scene = scene(compact_vertices);
                scene.gamma_mode = gamma_mode;
                assert_compact_landscape_matches_generic(
                    &mut renderer,
                    &device,
                    &queue,
                    &scene,
                    &presentation,
                    1,
                    1,
                    &format!(
                        "full landscape, smooth={smooth}, gamma_mode={gamma_mode:?}, fractional crop",
                    ),
                );
            }
        }
    }

    #[test]
    fn generic_landscape_fallback_is_an_ordered_compact_stream_barrier() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping mixed landscape fallback check");
            return;
        };
        let texture = GpuTextureId::fresh();
        let command = |w, modulation| GpuCommand::Landscape {
            base: texture,
            liquid_mask: None,
            liquid: None,
            vertices: quad(0.0, 0.0, 1.0, 1.0, w, modulation),
            clip: None,
            phase: [0.0; 3],
            gamma: false,
        };
        let red = [1.0, 0.0, 0.0, 127.0 / 255.0];
        let green = [0.0, 1.0, 0.0, 127.0 / 255.0];
        let blue = [0.0, 0.0, 1.0, 127.0 / 255.0];
        let scene = |commands| {
            test_scene(
                [1, 1],
                Color::transparent(),
                vec![rgba_resource(texture, [255; 4])],
                commands,
            )
        };
        let mixed = scene(vec![
            command(1.0, red),
            command(2.0, green),
            command(1.0, blue),
        ]);
        let generic = scene(vec![
            command(2.0, red),
            command(2.0, green),
            command(2.0, blue),
        ]);
        let mut renderer = test_renderer(&device, &queue);

        let mixed_frame = render_identity_readback(&mut renderer, &device, &queue, &mixed);
        let mixed_stats = renderer.last_stats();
        let generic_frame = render_identity_readback(&mut renderer, &device, &queue, &generic);

        assert_eq!(mixed_frame, generic_frame);
        assert_eq!(mixed_stats.draw_calls, 3);
        assert_eq!(mixed_stats.landscape_instances, 2);
        assert_eq!(mixed_stats.generic_vertices, 6);
        assert_eq!(renderer.last_stats().draw_calls, 1);
        assert_eq!(renderer.last_stats().landscape_instances, 0);
        assert_eq!(renderer.last_stats().generic_vertices, 18);
    }

    #[test]
    fn compact_landscape_upload_updates_without_resizing_its_buffer() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping compact landscape upload check");
            return;
        };
        let texture = GpuTextureId::fresh();
        let scene = |modulation| {
            test_scene(
                [1, 1],
                Color::transparent(),
                vec![rgba_resource(texture, [255; 4])],
                vec![GpuCommand::Landscape {
                    base: texture,
                    liquid_mask: None,
                    liquid: None,
                    vertices: quad(0.0, 0.0, 1.0, 1.0, 1.0, modulation),
                    clip: None,
                    phase: [0.0; 3],
                    gamma: false,
                }],
            )
        };
        let mut renderer = test_renderer(&device, &queue);

        let red =
            render_identity_readback(&mut renderer, &device, &queue, &scene([1.0, 0.0, 0.0, 0.0]));
        let green =
            render_identity_readback(&mut renderer, &device, &queue, &scene([0.0, 1.0, 0.0, 0.0]));

        assert_eq!(red.rgba, vec![255, 0, 0, 255]);
        assert_eq!(green.rgba, vec![0, 255, 0, 255]);
        assert_eq!(renderer.last_stats().landscape_instances, 1);
        assert_eq!(renderer.last_stats().generic_vertices, 0);
    }

    #[test]
    fn landscape_coalescing_keeps_binding_clip_and_layer_boundaries() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping landscape boundary check");
            return;
        };
        let base_a = GpuTextureId::fresh();
        let base_b = GpuTextureId::fresh();
        let mask_a = GpuTextureId::fresh();
        let mask_b = GpuTextureId::fresh();
        let liquid_a = GpuTextureId::fresh();
        let liquid_b = GpuTextureId::fresh();
        let command = |base, liquid_mask, liquid, clip| GpuCommand::Landscape {
            base,
            liquid_mask,
            liquid,
            vertices: quad(0.0, 0.0, 2.0, 2.0, 1.0, [1.0, 1.0, 1.0, 0.0]),
            clip,
            phase: [0.0; 3],
            gamma: false,
        };
        let resources = vec![
            rgba_resource(base_a, [255, 0, 0, 255]),
            rgba_resource(base_b, [0, 255, 0, 255]),
            r8_resource(mask_a, 0),
            r8_resource(mask_b, 0),
            rgba_resource(liquid_a, [128, 128, 128, 255]),
            rgba_resource(liquid_b, [128, 128, 128, 255]),
        ];
        let scene = test_scene(
            [2, 2],
            Color::transparent(),
            resources.clone(),
            vec![
                command(base_a, None, None, None),
                command(base_a, None, None, None),
                command(base_b, None, None, None),
                command(base_a, None, None, None),
                command(base_a, Some(mask_a), Some(liquid_a), None),
                command(base_a, Some(mask_a), Some(liquid_a), None),
                command(base_a, Some(mask_b), Some(liquid_a), None),
                command(base_a, Some(mask_b), Some(liquid_b), None),
                command(
                    base_a,
                    Some(mask_b),
                    Some(liquid_b),
                    Some(Rect::new(0, 0, 1, 1)),
                ),
            ],
        );
        let mut renderer = test_renderer(&device, &queue);

        let _ = render_identity_readback(&mut renderer, &device, &queue, &scene);
        assert_eq!(renderer.last_stats().draw_calls, 7);

        let one_command_scene = GpuScene {
            commands: vec![command(base_a, None, None, None)],
            textures: resources,
            ..scene
        };
        let presentation = GpuPresentation::identity(2, 2);
        let layers = [
            GpuSceneLayer::new(&one_command_scene, presentation),
            GpuSceneLayer::new(&one_command_scene, presentation),
        ];
        let _ = render_layers_readback(&mut renderer, &device, &queue, &layers);
        assert_eq!(renderer.last_stats().draw_calls, 2);
    }

    #[test]
    fn no_box_fades_landscape_triangles_keep_flat_colors_when_coalesced() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping NoBoxFades landscape parity check");
            return;
        };
        let shared_base = GpuTextureId::fresh();
        let split_first_base = GpuTextureId::fresh();
        let split_second_base = GpuTextureId::fresh();
        // LegacyClonk 7d43b47b7d789b533f32d005e64596e0a07019cd uses GL_FLAT
        // and each strip triangle's provoking vertex colour for NoBoxFades
        // (src/StdGL.cpp:667,710-763).
        let corners = quad(0.0, 0.0, 4.0, 4.0, 1.0, [1.0; 4]);
        let triangle = |base, indices: [usize; 4], modulation| GpuCommand::Landscape {
            base,
            liquid_mask: None,
            liquid: None,
            vertices: std::array::from_fn(|slot| {
                let corner = corners[indices[slot]];
                GpuVertex::new(corner.position, corner.uv, modulation)
            }),
            clip: None,
            phase: [0.0; 3],
            gamma: false,
        };
        let scene =
            |textures, commands| test_scene([4, 4], Color::transparent(), textures, commands);
        let coalesced = scene(
            vec![rgba_resource(shared_base, [255; 4])],
            vec![
                triangle(shared_base, [0, 1, 2, 2], [1.0, 0.0, 0.0, 0.0]),
                triangle(shared_base, [2, 1, 3, 3], [0.0, 1.0, 0.0, 0.0]),
            ],
        );
        let forced_split = scene(
            vec![
                rgba_resource(split_first_base, [255; 4]),
                rgba_resource(split_second_base, [255; 4]),
            ],
            vec![
                triangle(split_first_base, [0, 1, 2, 2], [1.0, 0.0, 0.0, 0.0]),
                triangle(split_second_base, [2, 1, 3, 3], [0.0, 1.0, 0.0, 0.0]),
            ],
        );
        let mut renderer = test_renderer(&device, &queue);

        let coalesced_frame = render_identity_readback(&mut renderer, &device, &queue, &coalesced);
        assert_eq!(renderer.last_stats().draw_calls, 1);
        assert_eq!(renderer.last_stats().generic_vertices, 0);
        assert_eq!(renderer.last_stats().landscape_instances, 2);
        assert_eq!(renderer.last_stats().landscape_instance_upload_bytes, 144);
        let split_frame = render_identity_readback(&mut renderer, &device, &queue, &forced_split);

        assert_eq!(renderer.last_stats().draw_calls, 2);
        assert_eq!(coalesced_frame, split_frame);
        assert!(coalesced_frame
            .rgba
            .chunks_exact(4)
            .any(|pixel| pixel == [255, 0, 0, 255]));
        assert!(coalesced_frame
            .rgba
            .chunks_exact(4)
            .any(|pixel| pixel == [0, 255, 0, 255]));
        assert_compact_landscape_matches_generic(
            &mut renderer,
            &device,
            &queue,
            &coalesced,
            &GpuPresentation::identity(4, 4),
            2,
            1,
            "both NoBoxFades triangle encodings",
        );
    }

    #[test]
    fn liquid_phase_and_fragment_gamma_remain_per_chunk_when_coalesced() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping liquid landscape parity check");
            return;
        };
        let shared = [
            GpuTextureId::fresh(),
            GpuTextureId::fresh(),
            GpuTextureId::fresh(),
        ];
        let split_first = [
            GpuTextureId::fresh(),
            GpuTextureId::fresh(),
            GpuTextureId::fresh(),
        ];
        let split_second = [
            GpuTextureId::fresh(),
            GpuTextureId::fresh(),
            GpuTextureId::fresh(),
        ];
        let command =
            |ids: [GpuTextureId; 3], left, right, modulation, phase, gamma| GpuCommand::Landscape {
                base: ids[0],
                liquid_mask: Some(ids[1]),
                liquid: Some(ids[2]),
                vertices: quad(left, 0.0, right, 1.0, 1.0, modulation),
                clip: None,
                phase,
                gamma,
            };
        let resources = |ids: [GpuTextureId; 3]| {
            vec![
                GpuTextureResource::immutable_rgba(
                    ids[0],
                    4,
                    1,
                    Arc::from([
                        100, 140, 220, 255, 110, 150, 210, 255, 120, 160, 200, 255, 130, 170, 190,
                        255,
                    ]),
                ),
                GpuTextureResource {
                    id: ids[1],
                    extent: [4, 1],
                    revision: 0,
                    base_revision: None,
                    format: GpuTextureFormat::R8,
                    pixels: Arc::from([255_u8; 4]),
                    dirty: Vec::new(),
                },
                rgba_resource_2x1(ids[2], [220, 96, 48, 255], [48, 208, 160, 255]),
            ]
        };
        let first = |ids| {
            command(
                ids,
                0.0,
                1.0,
                [191.0 / 255.0, 1.0, 127.0 / 255.0, 31.0 / 255.0],
                [0.3, -0.2, 0.1],
                false,
            )
        };
        let second = |ids| {
            command(
                ids,
                1.0,
                2.0,
                [1.0, 127.0 / 255.0, 191.0 / 255.0, 63.0 / 255.0],
                [-0.1, 0.4, 0.2],
                true,
            )
        };
        let gamma = GpuGammaLut::from_ramp(&GammaRamp::from_control_points([
            0x102030, 0x708090, 0xd0e0f0,
        ]));
        let scene = |textures, commands| GpuScene {
            logical_extent: [2, 1],
            clear: Color::new(11, 19, 31, 47),
            gamma: gamma.clone(),
            gamma_mode: GpuGammaMode::Fragment,
            textures,
            commands,
        };
        let coalesced = scene(resources(shared), vec![first(shared), second(shared)]);
        let forced_split = scene(
            resources(split_first)
                .into_iter()
                .chain(resources(split_second))
                .collect(),
            vec![first(split_first), second(split_second)],
        );
        let mut renderer = test_renderer(&device, &queue);

        let coalesced_frame = render_identity_readback(&mut renderer, &device, &queue, &coalesced);
        assert_eq!(renderer.last_stats().draw_calls, 1);
        assert_eq!(renderer.last_stats().generic_vertices, 0);
        assert_eq!(renderer.last_stats().landscape_instances, 2);
        let split_frame = render_identity_readback(&mut renderer, &device, &queue, &forced_split);

        assert_eq!(renderer.last_stats().draw_calls, 2);
        assert_eq!(coalesced_frame, split_frame);
        assert_ne!(
            readback_pixel(&coalesced_frame, 0, 0),
            readback_pixel(&coalesced_frame, 1, 0)
        );
        assert_compact_landscape_matches_generic(
            &mut renderer,
            &device,
            &queue,
            &coalesced,
            &GpuPresentation::identity(2, 1),
            2,
            1,
            "liquid phase with 4x1 base and 2x1 liquid",
        );
    }

    #[test]
    fn adjacent_multi_tile_landscape_runs_keep_order_without_regrouping() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping multi-tile landscape parity check");
            return;
        };
        let tile_a = GpuTextureId::fresh();
        let tile_a_split = GpuTextureId::fresh();
        let tile_b = GpuTextureId::fresh();
        let chunk = |base, left, right| GpuCommand::Landscape {
            base,
            liquid_mask: None,
            liquid: None,
            vertices: quad(left, 0.0, right, 1.0, 1.0, [1.0, 1.0, 1.0, 0.0]),
            clip: None,
            phase: [0.0; 3],
            gamma: false,
        };
        let scene = |commands| {
            test_scene(
                [3, 1],
                Color::transparent(),
                vec![
                    rgba_resource(tile_a, [220, 32, 48, 255]),
                    rgba_resource(tile_a_split, [220, 32, 48, 255]),
                    rgba_resource(tile_b, [24, 216, 72, 255]),
                ],
                commands,
            )
        };
        let coalesced = scene(vec![
            chunk(tile_a, 0.0, 1.0),
            chunk(tile_a, 1.0, 2.0),
            chunk(tile_b, 2.0, 3.0),
        ]);
        let forced_split = scene(vec![
            chunk(tile_a, 0.0, 1.0),
            chunk(tile_a_split, 1.0, 2.0),
            chunk(tile_b, 2.0, 3.0),
        ]);
        let mut renderer = test_renderer(&device, &queue);

        let coalesced_frame = render_identity_readback(&mut renderer, &device, &queue, &coalesced);
        assert_eq!(renderer.last_stats().draw_calls, 2);
        let split_frame = render_identity_readback(&mut renderer, &device, &queue, &forced_split);

        assert_eq!(renderer.last_stats().draw_calls, 3);
        assert_eq!(coalesced_frame, split_frame);
        assert_eq!(
            coalesced_frame.rgba,
            vec![220, 32, 48, 255, 220, 32, 48, 255, 24, 216, 72, 255,]
        );
        assert_compact_landscape_matches_generic(
            &mut renderer,
            &device,
            &queue,
            &coalesced,
            &GpuPresentation::identity(3, 1),
            3,
            2,
            "adjacent native landscape texture tiles",
        );
    }

    #[test]
    fn clipped_landscape_noop_does_not_split_a_compatible_run() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping clipped landscape run check");
            return;
        };
        let visible_base = GpuTextureId::fresh();
        let clipped_base = GpuTextureId::fresh();
        let chunk = |base, modulation, clip| GpuCommand::Landscape {
            base,
            liquid_mask: None,
            liquid: None,
            vertices: quad(0.0, 0.0, 1.0, 1.0, 1.0, modulation),
            clip,
            phase: [0.0; 3],
            gamma: false,
        };
        let scene = test_scene(
            [1, 1],
            Color::transparent(),
            vec![
                rgba_resource(visible_base, [255; 4]),
                rgba_resource(clipped_base, [255; 4]),
            ],
            vec![
                chunk(visible_base, [1.0, 0.0, 0.0, 127.0 / 255.0], None),
                chunk(
                    clipped_base,
                    [0.0, 0.0, 1.0, 0.0],
                    Some(Rect::new(2, 0, 1, 1)),
                ),
                chunk(visible_base, [0.0, 1.0, 0.0, 127.0 / 255.0], None),
            ],
        );
        let mut renderer = test_renderer(&device, &queue);

        let frame = render_identity_readback(&mut renderer, &device, &queue, &scene);

        assert_eq!(frame.rgba, vec![64, 128, 0, 192]);
        assert_eq!(renderer.last_stats().draw_calls, 1);
    }

    #[test]
    fn compatible_quad_before_sprite_batch_keeps_every_painter_order_instance() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping mixed particle batch check");
            return;
        };
        let texture = GpuTextureId::fresh();
        let scene = test_scene(
            [1, 1],
            Color::transparent(),
            vec![rgba_resource(texture, [255; 4])],
            vec![
                GpuCommand::Quad {
                    texture,
                    owner_mask: None,
                    vertices: quad(0.0, 0.0, 1.0, 1.0, 1.0, [1.0, 0.0, 0.0, 127.0 / 255.0]),
                    clip: None,
                    blend: GpuBlend::Normal,
                    base_mod2: false,
                    owner_mod2: false,
                    sampler: GpuSampler::Nearest,
                    gamma: false,
                },
                GpuCommand::SpriteBatch {
                    texture,
                    quads: vec![GpuSpriteQuad {
                        rect: [0.0, 0.0, 1.0, 1.0],
                        uv: [0.0, 0.0, 1.0, 1.0],
                        modulation: 0x7f00_ff00,
                    }],
                    clip: None,
                    blend: GpuBlend::Normal,
                    mod2: false,
                    gamma: false,
                    outer_modulation: clonk_graphics::GpuOuterModulation::Combine,
                },
            ],
        );
        let mut renderer = test_renderer(&device, &queue);

        let frame = render_identity_readback(&mut renderer, &device, &queue, &scene);

        assert_eq!(frame.rgba, vec![64, 128, 0, 192]);
        assert_eq!(renderer.last_stats().draw_calls, 2);
    }

    #[test]
    fn clipped_quad_does_not_consume_following_visible_sprite_batch() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping clipped mixed particle batch check");
            return;
        };
        let texture = GpuTextureId::fresh();
        let scene = test_scene(
            [1, 1],
            Color::transparent(),
            vec![rgba_resource(texture, [255; 4])],
            vec![
                GpuCommand::Quad {
                    texture,
                    owner_mask: None,
                    vertices: quad(0.0, 0.0, 1.0, 1.0, 1.0, [1.0, 0.0, 0.0, 1.0]),
                    clip: Some(Rect::new(2, 0, 1, 1)),
                    blend: GpuBlend::Normal,
                    base_mod2: false,
                    owner_mod2: false,
                    sampler: GpuSampler::Nearest,
                    gamma: false,
                },
                GpuCommand::SpriteBatch {
                    texture,
                    quads: vec![GpuSpriteQuad {
                        rect: [0.0, 0.0, 1.0, 1.0],
                        uv: [0.0, 0.0, 1.0, 1.0],
                        modulation: 0x0000_ff00,
                    }],
                    clip: None,
                    blend: GpuBlend::Normal,
                    mod2: false,
                    gamma: false,
                    outer_modulation: clonk_graphics::GpuOuterModulation::Combine,
                },
            ],
        );
        let mut renderer = test_renderer(&device, &queue);

        let frame = render_identity_readback(&mut renderer, &device, &queue, &scene);

        assert_eq!(frame.rgba, vec![0, 255, 0, 255]);
        assert_eq!(renderer.last_stats().draw_calls, 1);
    }

    #[test]
    fn sprite_batch_before_compatible_quad_keeps_every_painter_order_instance() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping reverse mixed particle batch check");
            return;
        };
        let texture = GpuTextureId::fresh();
        let scene = test_scene(
            [1, 1],
            Color::transparent(),
            vec![rgba_resource(texture, [255; 4])],
            vec![
                GpuCommand::SpriteBatch {
                    texture,
                    quads: vec![GpuSpriteQuad {
                        rect: [0.0, 0.0, 1.0, 1.0],
                        uv: [0.0, 0.0, 1.0, 1.0],
                        modulation: 0x7fff_0000,
                    }],
                    clip: None,
                    blend: GpuBlend::Normal,
                    mod2: false,
                    gamma: false,
                    outer_modulation: clonk_graphics::GpuOuterModulation::Combine,
                },
                GpuCommand::Quad {
                    texture,
                    owner_mask: None,
                    vertices: quad(0.0, 0.0, 1.0, 1.0, 1.0, [0.0, 1.0, 0.0, 127.0 / 255.0]),
                    clip: None,
                    blend: GpuBlend::Normal,
                    base_mod2: false,
                    owner_mod2: false,
                    sampler: GpuSampler::Nearest,
                    gamma: false,
                },
            ],
        );
        let mut renderer = test_renderer(&device, &queue);

        let frame = render_identity_readback(&mut renderer, &device, &queue, &scene);

        assert_eq!(frame.rgba, vec![64, 128, 0, 192]);
        assert_eq!(renderer.last_stats().draw_calls, 2);
    }

    #[test]
    fn compatible_adjacent_sprite_batches_share_one_painter_order_draw_call() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping compact sprite coalescing check");
            return;
        };
        let texture = GpuTextureId::fresh();
        let batch = |modulation| GpuCommand::SpriteBatch {
            texture,
            quads: vec![GpuSpriteQuad {
                rect: [0.0, 0.0, 1.0, 1.0],
                uv: [0.0, 0.0, 1.0, 1.0],
                modulation,
            }],
            clip: None,
            blend: GpuBlend::Normal,
            mod2: false,
            gamma: false,
            outer_modulation: clonk_graphics::GpuOuterModulation::Combine,
        };
        let scene = test_scene(
            [1, 1],
            Color::transparent(),
            vec![rgba_resource(texture, [255; 4])],
            vec![batch(0x7fff_0000), batch(0x7f00_ff00)],
        );
        let mut renderer = test_renderer(&device, &queue);

        let frame = render_identity_readback(&mut renderer, &device, &queue, &scene);

        assert_eq!(frame.rgba, vec![64, 128, 0, 192]);
        assert_eq!(renderer.last_stats().draw_calls, 1);
    }

    #[test]
    fn fire_like_sprite_batches_keep_texture_and_blend_state_boundaries() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping Fire-like sprite state check");
            return;
        };
        let fire_texture = GpuTextureId::fresh();
        let fire2_texture = GpuTextureId::fresh();
        let batch = |texture, rect, modulation, blend| GpuCommand::SpriteBatch {
            texture,
            quads: vec![GpuSpriteQuad {
                rect,
                uv: [0.0, 0.0, 1.0, 1.0],
                modulation,
            }],
            clip: None,
            blend,
            mod2: false,
            gamma: false,
            outer_modulation: clonk_graphics::GpuOuterModulation::Combine,
        };
        let scene = test_scene(
            [2, 1],
            Color::opaque(0, 0, 0),
            vec![
                rgba_resource(fire_texture, [255; 4]),
                rgba_resource(fire2_texture, [255; 4]),
            ],
            vec![
                batch(
                    fire_texture,
                    [0.0, 0.0, 1.0, 1.0],
                    0x00ff_0000,
                    GpuBlend::Normal,
                ),
                batch(
                    fire2_texture,
                    [1.0, 0.0, 2.0, 1.0],
                    0x0000_ff00,
                    GpuBlend::Additive,
                ),
            ],
        );
        let mut renderer = test_renderer(&device, &queue);

        let frame = render_identity_readback(&mut renderer, &device, &queue, &scene);

        assert_eq!(frame.rgba, vec![255, 0, 0, 255, 0, 255, 0, 255]);
        assert_eq!(renderer.last_stats().draw_calls, 2);
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
                style: GpuSolidStyle::NONE,
            }],
        };
        assert!(matches!(
            RetainedGpuRenderer::validate_scene(
                &scene,
                &GpuPresentation {
                    physical_extent: [2, 2],
                    scale: 2.0,
                    crop_top: 0,
                    world_zoom: 1.0,
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
        let (runtime, _instance, _adapter, device, queue) =
            test_wgpu_device("lc_gpu_layered_test_device", true)
                .expect("layered renderer test requires a working wgpu adapter");
        let validation_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

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
                style: GpuSolidStyle::NONE,
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
                style: GpuSolidStyle::NONE,
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
                    world_zoom: 1.0,
                },
            ),
            GpuSceneLayer::new(
                &physical_text,
                GpuPresentation::identity(physical_extent[0], physical_extent[1]),
            ),
        ];
        let mut renderer = test_renderer(&device, &queue);
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
        let validation = runtime.block_on(validation_scope.pop());
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

    // -- Shader landscape composition -------------------------------------

    /// Vertical-strip atlas identical to `clonk_frontend::materials`'
    /// `build_material_atlas`: rects share x=0 and stack by height.
    fn shader_landscape_atlas() -> ([u32; 2], Vec<u8>, [[u32; 4]; 2]) {
        let (primary_w, primary_h) = (4_u32, 3_u32);
        let (indexed_w, indexed_h) = (3_u32, 5_u32);
        let width = primary_w.max(indexed_w);
        let height = primary_h + indexed_h;
        let mut pixels = vec![0_u8; (width * height * 4) as usize];
        for row in 0..primary_h {
            for column in 0..primary_w {
                let index = (row * primary_w + column) as u8;
                let offset = ((row * width + column) * 4) as usize;
                pixels[offset] = index.wrapping_mul(37).wrapping_add(3);
                pixels[offset + 1] = index.wrapping_mul(91).wrapping_add(17);
                pixels[offset + 2] = index.wrapping_mul(53).wrapping_add(200);
                pixels[offset + 3] = index.wrapping_mul(29).wrapping_add(11);
            }
        }
        for row in 0..indexed_h {
            for column in 0..indexed_w {
                let index = (row * indexed_w + column) as u8;
                let offset = (((row + primary_h) * width + column) * 4) as usize;
                pixels[offset] = index.wrapping_mul(7);
                pixels[offset + 3] = 255;
            }
        }
        (
            [width, height],
            pixels,
            [
                [0, 0, primary_w, primary_h],
                [0, primary_h, indexed_w, indexed_h],
            ],
        )
    }

    fn pack_triplet(values: [u8; 3]) -> u32 {
        u32::from(values[0]) | (u32::from(values[1]) << 8) | (u32::from(values[2]) << 16)
    }

    fn atlas_texel(
        atlas: &[u8],
        atlas_extent: [u32; 2],
        rect: [u32; 4],
        x: i32,
        y: i32,
        zoom: u32,
    ) -> [u8; 4] {
        let sample_x = if zoom == 0 { x } else { x / zoom as i32 };
        let sample_y = if zoom == 0 { y } else { y / zoom as i32 };
        let texel_x = rect[0] + (sample_x as u32 % rect[2]);
        let texel_y = rect[1] + (sample_y as u32 % rect[3]);
        let offset = ((texel_y * atlas_extent[0] + texel_x) * 4) as usize;
        [
            atlas[offset],
            atlas[offset + 1],
            atlas[offset + 2],
            atlas[offset + 3],
        ]
    }

    /// CPU mirror of `MATERIAL_LANDSCAPE_SHADER`, written from
    /// `clonk-frontend/src/materials.rs:163-400`. `clonk_frontend`'s `materials`
    /// module is crate-private, so this restates that arithmetic instead of
    /// calling it; the frontend test `packed_material_slot_matches_the_cpu_composer`
    /// pins the same packing against `compose_material_surface_pixel` itself.
    fn compose_shader_landscape_reference(
        slot: &ShaderLandscapeSlot,
        landscape_pixel: u8,
        x: i32,
        y: i32,
        atlas: &[u8],
        atlas_extent: [u32; 2],
    ) -> [u8; 4] {
        let triplet = |packed: u32, channel: u32| ((packed >> (channel * 8)) & 0xff) as u8;
        let lighten = |channel: u8| {
            if channel & 0x80 != 0 {
                255
            } else {
                channel << 1
            }
        };
        let flags = slot.params[3];
        if flags & SHADER_LANDSCAPE_PRESENT == 0 {
            return [0; 4];
        }
        let base_alpha = if landscape_pixel & 0x80 == 0 {
            slot.colors[3]
        } else {
            slot.params[0]
        };
        let mut rgb = [
            triplet(slot.colors[0], 0),
            triplet(slot.colors[0], 1),
            triplet(slot.colors[0], 2),
        ];
        let mut transparency = triplet(base_alpha, 0);
        let monochrome = flags & SHADER_LANDSCAPE_MONOCHROME != 0;
        let apply =
            |rect: [u32; 4], zoom: u32, indexed: bool, rgb: &mut [u8; 3], transparency: &mut u8| {
                if rect[2] == 0 || rect[3] == 0 {
                    return;
                }
                let texel = atlas_texel(atlas, atlas_extent, rect, x, y, zoom);
                if indexed {
                    let shift = u32::from(texel[0] % 3);
                    let packed = slot.colors[shift as usize];
                    *rgb = [triplet(packed, 0), triplet(packed, 1), triplet(packed, 2)];
                    let alpha = if landscape_pixel & 0xf0 == 0 {
                        slot.colors[3]
                    } else {
                        slot.params[0]
                    };
                    *transparency = triplet(alpha, shift);
                    return;
                }
                let modifiers = if monochrome {
                    [texel[2]; 3]
                } else {
                    [texel[0], texel[1], texel[2]]
                };
                for channel in 0..3 {
                    rgb[channel] = lighten(
                        ((u16::from(rgb[channel]) * u16::from(modifiers[channel])) >> 8) as u8,
                    );
                }
                *transparency = transparency.saturating_add(255u8.saturating_sub(texel[3]));
            };
        apply(
            slot.primary,
            slot.params[1],
            flags & SHADER_LANDSCAPE_PRIMARY_INDEXED != 0,
            &mut rgb,
            &mut transparency,
        );
        if flags & SHADER_LANDSCAPE_HAS_OVERLAY != 0 {
            apply(
                slot.overlay,
                slot.params[2],
                flags & SHADER_LANDSCAPE_OVERLAY_INDEXED != 0,
                &mut rgb,
                &mut transparency,
            );
        }
        [rgb[0], rgb[1], rgb[2], 255u8.saturating_sub(transparency)]
    }

    fn shader_landscape_slots() -> Vec<ShaderLandscapeSlot> {
        let (_, _, rects) = shader_landscape_atlas();
        let color = [10_u8, 90, 200, 40, 130, 250, 70, 20, 160];
        let alpha = [0_u8, 30, 60, 90, 120, 200];
        let colors = [
            pack_triplet([color[0], color[1], color[2]]),
            pack_triplet([color[3], color[4], color[5]]),
            pack_triplet([color[6], color[7], color[8]]),
            pack_triplet([alpha[0], alpha[1], alpha[2]]),
        ];
        let alpha_high = pack_triplet([alpha[3], alpha[4], alpha[5]]);
        let slot = |primary: [u32; 4], overlay: [u32; 4], primary_zoom, overlay_zoom, flags| {
            ShaderLandscapeSlot {
                colors,
                params: [alpha_high, primary_zoom, overlay_zoom, flags],
                primary,
                overlay,
            }
        };
        let mut slots = vec![ShaderLandscapeSlot::default(); SHADER_LANDSCAPE_SLOTS];
        // 1: plain Surface32, zoom 0, no overlay.
        slots[1] = slot(rects[0], [0; 4], 0, 2, SHADER_LANDSCAPE_PRESENT);
        // 2: Surface32 with a zoom-2 Surface32 overlay.
        slots[2] = slot(
            rects[0],
            rects[0],
            0,
            2,
            SHADER_LANDSCAPE_PRESENT | SHADER_LANDSCAPE_HAS_OVERLAY,
        );
        // 3: huge-zoom monochrome primary with a Surface32 overlay. The overlay
        // must NOT be indexed here: an indexed pattern overwrites the running
        // pixel outright and would hide every monochrome difference.
        slots[3] = slot(
            rects[0],
            rects[0],
            4,
            2,
            SHADER_LANDSCAPE_PRESENT | SHADER_LANDSCAPE_MONOCHROME | SHADER_LANDSCAPE_HAS_OVERLAY,
        );
        // 19: monochrome primary with an indexed overlay.
        slots[19] = slot(
            rects[0],
            rects[1],
            0,
            2,
            SHADER_LANDSCAPE_PRESENT
                | SHADER_LANDSCAPE_MONOCHROME
                | SHADER_LANDSCAPE_HAS_OVERLAY
                | SHADER_LANDSCAPE_OVERLAY_INDEXED,
        );
        // 4: exact-zoom indexed primary with a Surface32 overlay.
        slots[4] = slot(
            rects[1],
            rects[0],
            1,
            1,
            SHADER_LANDSCAPE_PRESENT
                | SHADER_LANDSCAPE_HAS_OVERLAY
                | SHADER_LANDSCAPE_PRIMARY_INDEXED,
        );
        // 17: indexed primary, no overlay — exercises the 0xf0 alpha branch.
        slots[17] = slot(
            rects[1],
            [0; 4],
            0,
            2,
            SHADER_LANDSCAPE_PRESENT | SHADER_LANDSCAPE_PRIMARY_INDEXED,
        );
        // Slot 5 stays absent so the PRESENT bit is exercised in both states.
        slots
    }

    fn shader_landscape_index_plane(extent: [u32; 2]) -> Vec<u8> {
        const BYTES: [u8; 9] = [0, 1, 2, 3, 4, 0x05, 0x11, 0x81, 0x13];
        (0..extent[0] * extent[1])
            .map(|index| BYTES[(index as usize * 5 + index as usize / 7) % BYTES.len()])
            .collect()
    }

    fn shader_landscape_plan_fixture(extent: [u32; 2]) -> clonk_graphics::ShaderLandscapePlan {
        let (atlas_extent, atlas, _) = shader_landscape_atlas();
        clonk_graphics::ShaderLandscapePlan {
            extent,
            index_plane: shader_landscape_index_plane(extent),
            shading_plane: None,
            atlas,
            atlas_extent,
            slots: shader_landscape_slots()
                .iter()
                .map(|slot| {
                    let mut words = [0_u32; 16];
                    words[0..4].copy_from_slice(&slot.colors);
                    words[4..8].copy_from_slice(&slot.params);
                    words[8..12].copy_from_slice(&slot.primary);
                    words[12..16].copy_from_slice(&slot.overlay);
                    words
                })
                .collect(),
        }
    }

    fn shader_landscape_scene_fixture(
        base: GpuTextureId,
        logical_extent: [u32; 2],
        source_extent: [u32; 2],
        revision: u64,
    ) -> GpuScene {
        let corner = |x: f32, y: f32, u: f32, v: f32| GpuVertex {
            position: [x, y, 1.0],
            uv: [u, v],
            modulation: [1.0, 1.0, 1.0, 0.0],
            owner_modulation: [0.0; 4],
            outer_modulation: clonk_graphics::GpuOuterModulation::default(),
            owner_outer_modulation: clonk_graphics::GpuOuterModulation::default(),
            sample_tile: [0.0; 4],
        };
        let mut resource = GpuTextureResource::immutable_rgba(
            base,
            source_extent[0],
            source_extent[1],
            vec![0_u8; (source_extent[0] * source_extent[1] * 4) as usize].into(),
        );
        resource.revision = revision;
        let vertices = [
            corner(0.0, 0.0, 0.0, 0.0),
            corner(logical_extent[0] as f32, 0.0, 1.0, 0.0),
            corner(0.0, logical_extent[1] as f32, 0.0, 1.0),
            corner(logical_extent[0] as f32, logical_extent[1] as f32, 1.0, 1.0),
        ];
        let transparent_vertices = vertices.map(|vertex| GpuVertex {
            modulation: [1.0; 4],
            outer_modulation: clonk_graphics::GpuOuterModulation::Ignore,
            ..vertex
        });
        GpuScene {
            logical_extent,
            clear: Color::transparent(),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
            gamma_mode: GpuGammaMode::Disabled,
            textures: vec![resource],
            commands: vec![
                GpuCommand::Landscape {
                    base,
                    liquid_mask: None,
                    liquid: None,
                    vertices,
                    clip: None,
                    phase: [0.0; 3],
                    gamma: false,
                },
                GpuCommand::Quad {
                    texture: base,
                    owner_mask: None,
                    vertices: transparent_vertices,
                    clip: None,
                    blend: GpuBlend::Normal,
                    base_mod2: false,
                    owner_mod2: false,
                    sampler: GpuSampler::Nearest,
                    gamma: false,
                },
            ],
        }
    }

    fn shader_landscape_shading_plane(extent: [u32; 2]) -> Vec<u8> {
        (0..extent[0] * extent[1])
            .flat_map(|index| {
                let index = index as usize;
                if index % 23 == 5 {
                    return [0, SHADER_LANDSCAPE_SUPPRESSED];
                }
                [(index % 31) as u8, (index % 17) as u8]
            })
            .collect()
    }

    fn test_wgpu_instance() -> wgpu::Instance {
        wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            backend_options: wgpu::BackendOptions {
                dx12: wgpu::Dx12BackendOptions {
                    shader_compiler: wgpu::Dx12Compiler::Fxc,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        })
    }

    fn request_test_device(
        runtime: &tokio::runtime::Runtime,
        instance: &wgpu::Instance,
        label: &'static str,
        allow_fallback: bool,
    ) -> Option<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
        request_test_device_with_features(
            runtime,
            instance,
            label,
            allow_fallback,
            wgpu::Features::empty(),
        )
    }

    fn request_test_device_with_features(
        runtime: &tokio::runtime::Runtime,
        instance: &wgpu::Instance,
        label: &'static str,
        allow_fallback: bool,
        required_features: wgpu::Features,
    ) -> Option<(wgpu::Adapter, wgpu::Device, wgpu::Queue)> {
        let adapter = runtime
            .block_on(async {
                let primary = instance
                    .request_adapter(&wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::HighPerformance,
                        compatible_surface: None,
                        force_fallback_adapter: false,
                        apply_limit_buckets: false,
                    })
                    .await;
                if primary.is_ok() || !allow_fallback {
                    primary
                } else {
                    instance
                        .request_adapter(&wgpu::RequestAdapterOptions {
                            power_preference: wgpu::PowerPreference::LowPower,
                            compatible_surface: None,
                            force_fallback_adapter: true,
                            apply_limit_buckets: false,
                        })
                        .await
                }
            })
            .ok()?;
        if !adapter.features().contains(required_features) {
            return None;
        }
        let descriptor = wgpu::DeviceDescriptor {
            label: Some(label),
            required_features,
            required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
            ..Default::default()
        };
        let (device, queue) = runtime
            .block_on(adapter.request_device(&descriptor))
            .unwrap_or_else(|error| panic!("request {label}: {error}"));
        Some((adapter, device, queue))
    }

    fn test_wgpu_device(
        label: &'static str,
        allow_fallback: bool,
    ) -> Option<(
        tokio::runtime::Runtime,
        wgpu::Instance,
        wgpu::Adapter,
        wgpu::Device,
        wgpu::Queue,
    )> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build Tokio runtime for wgpu adapter discovery");
        let instance = test_wgpu_instance();
        let (adapter, device, queue) =
            request_test_device(&runtime, &instance, label, allow_fallback)?;
        Some((runtime, instance, adapter, device, queue))
    }

    fn shader_landscape_test_device() -> Option<(tokio::runtime::Runtime, wgpu::Device, wgpu::Queue)>
    {
        let (runtime, _instance, _adapter, device, queue) =
            test_wgpu_device("lc_gpu_shader_landscape_test_device", false)?;
        Some((runtime, device, queue))
    }

    fn compose_shader_landscape(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        inputs: ShaderLandscapeInputs<'_>,
    ) -> Vec<u8> {
        let mut composer = ShaderLandscapeComposer::new(device);
        compose_shader_landscape_with(&mut composer, device, queue, inputs)
    }

    /// Composes through `composer`, so successive calls exercise the resources
    /// it retains rather than a fresh set each time.
    fn compose_shader_landscape_with(
        composer: &mut ShaderLandscapeComposer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        inputs: ShaderLandscapeInputs<'_>,
    ) -> Vec<u8> {
        let extent = inputs.composed_extent();
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("lc_gpu_shader_landscape_test_target"),
            size: wgpu::Extent3d {
                width: extent[0],
                height: extent[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("lc_gpu_shader_landscape_test_encoder"),
        });
        composer
            .compose_into(device, queue, &mut encoder, &view, inputs)
            .expect("compose landscape materials in the fragment shader");
        let padded = (extent[0] as usize * 4).div_ceil(256) * 256;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lc_gpu_shader_landscape_test_readback"),
            size: (padded * extent[1] as usize) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
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
        queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        let (sender, receiver) = mpsc::sync_channel(1);
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll shader landscape readback");
        receiver
            .recv()
            .expect("shader landscape readback callback")
            .expect("map shader landscape readback");
        let mapped = slice
            .get_mapped_range()
            .expect("map shader landscape readback range");
        let row_bytes = extent[0] as usize * 4;
        (0..extent[1] as usize)
            .flat_map(|row| mapped[row * padded..row * padded + row_bytes].to_vec())
            .collect()
    }

    /// ACCEPTANCE: at detail 1 the fragment composer must be byte-identical to
    /// the CPU material composition it replaces. Anything else would make
    /// `Graphics.ShaderLandscape` a silent divergence rather than an opt-in.
    #[test]
    fn shader_landscape_composition_matches_the_cpu_reference() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping shader landscape composition readback");
            return;
        };
        let extent = [16_u32, 12_u32];
        let index_plane = shader_landscape_index_plane(extent);
        let shading_plane = shader_landscape_shading_plane(extent);
        let (atlas_extent, atlas, _) = shader_landscape_atlas();
        let slots = shader_landscape_slots();

        for shading in [None, Some(shading_plane.as_slice())] {
            let composed = compose_shader_landscape(
                &device,
                &queue,
                ShaderLandscapeInputs {
                    extent,
                    index_plane: &index_plane,
                    shading_plane: shading,
                    atlas: &atlas,
                    atlas_extent,
                    slots: &slots,
                    detail: 1,
                },
            );
            let mut opaque = 0_usize;
            for y in 0..extent[1] {
                for x in 0..extent[0] {
                    let map_index = (y * extent[0] + x) as usize;
                    let byte = index_plane[map_index];
                    let mut expected = if byte == 0 {
                        [0; 4]
                    } else {
                        compose_shader_landscape_reference(
                            &slots[usize::from(byte & 0x7f)],
                            byte,
                            x as i32,
                            y as i32,
                            &atlas,
                            atlas_extent,
                        )
                    };
                    if let Some(plane) = shading {
                        let (lighten, darken) = (plane[map_index * 2], plane[map_index * 2 + 1]);
                        if darken == SHADER_LANDSCAPE_SUPPRESSED {
                            expected = [0; 4];
                        } else if expected[3] != 0 || expected[..3] != [0, 0, 0] {
                            for channel in expected.iter_mut().take(3) {
                                *channel = channel.saturating_add(lighten).saturating_sub(darken);
                            }
                        }
                    }
                    let offset = map_index * 4;
                    let actual = [
                        composed[offset],
                        composed[offset + 1],
                        composed[offset + 2],
                        composed[offset + 3],
                    ];
                    assert_eq!(
                        actual,
                        expected,
                        "landscape byte {byte:#04x} at ({x}, {y}), shading {}",
                        shading.is_some()
                    );
                    if actual[3] != 0 {
                        opaque += 1;
                    }
                }
            }
            assert!(
                opaque > 40,
                "the fixture must compose real material, not an empty plane"
            );
        }
    }

    /// The whole wiring: a plan handed to the renderer must REPLACE the
    /// CPU-composed landscape texture before the frame draws, at
    /// `composed_extent()` rather than the map extent. Without the
    /// substitution the composer runs and nothing samples its output; without
    /// taking the plan, a stale one would recompose over a later landscape.
    #[test]
    fn a_pending_plan_replaces_the_landscape_texture_at_its_composed_extent() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping shader landscape substitution");
            return;
        };
        let extent = [16_u32, 12_u32];
        let (atlas_extent, atlas, _) = shader_landscape_atlas();
        let plan = clonk_graphics::ShaderLandscapePlan {
            extent,
            index_plane: shader_landscape_index_plane(extent),
            shading_plane: None,
            atlas,
            atlas_extent,
            slots: shader_landscape_slots()
                .iter()
                .map(|slot| {
                    let mut words = [0_u32; 16];
                    words[0..4].copy_from_slice(&slot.colors);
                    words[4..8].copy_from_slice(&slot.params);
                    words[8..12].copy_from_slice(&slot.primary);
                    words[12..16].copy_from_slice(&slot.overlay);
                    words
                })
                .collect(),
        };

        let base = GpuTextureId::fresh();
        let corner = |x: f32, y: f32, u: f32, v: f32| GpuVertex {
            position: [x, y, 1.0],
            uv: [u, v],
            modulation: [1.0, 1.0, 1.0, 0.0],
            owner_modulation: [0.0; 4],
            outer_modulation: clonk_graphics::GpuOuterModulation::default(),
            owner_outer_modulation: clonk_graphics::GpuOuterModulation::default(),
            sample_tile: [0.0; 4],
        };
        // The CPU-composed upload the plan is expected to displace.
        let scene = GpuScene {
            logical_extent: extent,
            clear: Color::transparent(),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
            gamma_mode: GpuGammaMode::Disabled,
            textures: vec![GpuTextureResource::immutable_rgba(
                base,
                extent[0],
                extent[1],
                vec![0_u8; (extent[0] * extent[1] * 4) as usize].into(),
            )],
            commands: vec![GpuCommand::Landscape {
                base,
                liquid_mask: None,
                liquid: None,
                vertices: [
                    corner(0.0, 0.0, 0.0, 0.0),
                    corner(extent[0] as f32, 0.0, 1.0, 0.0),
                    corner(0.0, extent[1] as f32, 0.0, 1.0),
                    corner(extent[0] as f32, extent[1] as f32, 1.0, 1.0),
                ],
                clip: None,
                phase: [0.0; 3],
                gamma: false,
            }],
        };

        for (detail, expected) in [(1_u32, extent), (3, [extent[0] * 3, extent[1] * 3])] {
            let mut renderer = test_renderer(&device, &queue);
            renderer.set_shader_landscape(true);
            renderer.set_landscape_detail(detail);
            renderer.set_pending_shader_landscape(Some((base, plan.clone())));
            let _ = render_extent_readback(&mut renderer, &device, &queue, &scene, extent);
            assert_eq!(
                renderer
                    .textures
                    .get(&base)
                    .expect("landscape texture")
                    .extent,
                expected,
                "detail {detail} must compose a plane {detail}x the map extent"
            );
        }

        // With the opt-in off the CPU upload must survive untouched, and the
        // plan must still be consumed rather than queued for a later frame.
        // The plan may describe a map that this device could not compose at
        // the requested detail, but it is unused while the opt-in is off and
        // must not force the already-valid CPU upload onto the fallback path.
        let mut disabled_plan = plan;
        disabled_plan.extent = [
            device
                .limits()
                .max_texture_dimension_2d
                .checked_add(1)
                .expect("test device texture limit leaves one larger extent"),
            1,
        ];
        disabled_plan.index_plane.clear();
        disabled_plan.shading_plane = None;
        disabled_plan.atlas.clear();
        disabled_plan.atlas_extent = [1, 1];
        let mut renderer = test_renderer(&device, &queue);
        renderer.set_landscape_detail(3);
        renderer.set_pending_shader_landscape(Some((base, disabled_plan)));
        let _ = render_extent_readback(&mut renderer, &device, &queue, &scene, extent);
        assert_eq!(
            renderer
                .textures
                .get(&base)
                .expect("landscape texture")
                .extent,
            extent,
            "the CPU-composed upload must stand when the opt-in is off"
        );
        assert!(
            renderer.pending_shader_landscape.is_none(),
            "the plan must be taken even when it is not composed"
        );
    }

    #[test]
    fn shader_landscape_output_lifecycle_survives_revisioned_frames_and_recovery() {
        // This consumes immutable retained scenes and renderer configuration
        // only; no landscape simulation or relight state participates.
        let (runtime, _instance, _adapter, device, queue) =
            test_wgpu_device("lc_gpu_shader_landscape_retention_test_device", true)
                .expect("shader landscape retention requires a working wgpu adapter");
        for (detail, extent, source_extent) in [(1, [12, 12], [12, 12]), (3, [13, 7], [16, 16])] {
            let plan = shader_landscape_plan_fixture(extent);
            let base = GpuTextureId::fresh();
            let scene = shader_landscape_scene_fixture(base, extent, source_extent, 1);
            let mut renderer = test_renderer(&device, &queue);
            renderer.set_shader_landscape(true);
            renderer.set_landscape_detail(detail);
            renderer.set_pending_shader_landscape(Some((base, plan.clone())));
            let validation_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let composed = render_identity_readback(&mut renderer, &device, &queue, &scene);
            assert_eq!(renderer.last_stats().full_upload_calls, 1);
            let cached = renderer.textures.get(&base).expect("composed landscape");
            assert_eq!(cached.source_extent, source_extent);
            assert_eq!(
                cached.extent,
                [extent[0] * detail, extent[1] * detail],
                "detail {detail}: the composed view extent must remain distinct from its CPU \
                 source extent"
            );
            assert_eq!(renderer.quad_bind_groups.len(), 1);
            assert_eq!(renderer.landscape_bind_groups.len(), 1);

            renderer.set_pending_shader_landscape(None);
            let unchanged = render_identity_readback(&mut renderer, &device, &queue, &scene);
            assert_eq!(
                renderer.last_stats().full_upload_calls,
                0,
                "detail {detail}: an unchanged CPU resource must not overwrite the \
                 authoritative shader output"
            );
            assert_eq!(
                unchanged, composed,
                "detail {detail}: the shader-composed landscape must remain authoritative without \
                 a new plan"
            );

            let mut changed_scene = scene.clone();
            changed_scene.textures[0].revision = 2;
            changed_scene.textures[0].base_revision = Some(1);
            changed_scene.textures[0].dirty = vec![Rect::new(0, 0, 1, 1)];
            let mut changed_plan = plan.clone();
            changed_plan.atlas.fill(255);
            renderer.set_pending_shader_landscape(Some((base, changed_plan)));
            let changed = render_identity_readback(&mut renderer, &device, &queue, &changed_scene);
            assert_eq!(
                (
                    renderer.last_stats().full_upload_calls,
                    renderer.last_stats().dirty_upload_calls,
                ),
                (0, 0),
                "detail {detail}: a fresh plan supersedes the CPU delta without uploading it into \
                 the render-only shader output"
            );
            assert_ne!(
                changed, composed,
                "detail {detail}: the second plan must draw through its newly composed texture \
                 view"
            );

            renderer.set_pending_shader_landscape(None);
            let changed_unchanged =
                render_identity_readback(&mut renderer, &device, &queue, &changed_scene);
            assert_eq!(renderer.last_stats().full_upload_calls, 0);
            assert_eq!(renderer.last_stats().dirty_upload_calls, 0);
            assert_eq!(
                changed_unchanged, changed,
                "detail {detail}: the second composed output must survive its unchanged frame"
            );

            if detail == 1 {
                renderer.set_landscape_detail(2);
                assert!(!renderer.textures.contains_key(&base));
                assert!(renderer.quad_bind_groups.is_empty());
                assert!(renderer.landscape_bind_groups.is_empty());
                let after_detail_change =
                    render_identity_readback(&mut renderer, &device, &queue, &changed_scene);
                assert_eq!(
                    (
                        renderer.last_stats().created_source_textures,
                        renderer.last_stats().full_upload_calls,
                    ),
                    (1, 1),
                    "changing detail must retire the old shader output and restore the complete \
                     CPU resource"
                );
                assert!(matches!(
                    renderer
                        .textures
                        .get(&base)
                        .expect("restored CPU landscape")
                        .contents,
                    CachedTextureContents::Source
                ));
                assert_ne!(
                    after_detail_change, changed,
                    "the old detail-1 output cannot remain authoritative at detail 2"
                );

                let mut detail_plan = plan.clone();
                detail_plan.atlas.fill(255);
                renderer.set_pending_shader_landscape(Some((base, detail_plan.clone())));
                let detail_composed =
                    render_identity_readback(&mut renderer, &device, &queue, &changed_scene);
                assert_ne!(detail_composed, after_detail_change);
                assert_eq!(renderer.quad_bind_groups.len(), 1);
                assert_eq!(renderer.landscape_bind_groups.len(), 1);

                renderer.set_shader_landscape(false);
                assert!(!renderer.textures.contains_key(&base));
                assert!(renderer.quad_bind_groups.is_empty());
                assert!(renderer.landscape_bind_groups.is_empty());
                renderer.set_pending_shader_landscape(Some((base, detail_plan.clone())));
                let disabled =
                    render_identity_readback(&mut renderer, &device, &queue, &changed_scene);
                assert_eq!(disabled, after_detail_change);
                assert!(renderer.pending_shader_landscape.is_none());
                assert!(matches!(
                    renderer
                        .textures
                        .get(&base)
                        .expect("disabled CPU landscape")
                        .contents,
                    CachedTextureContents::Source
                ));
                assert_eq!(renderer.quad_bind_groups.len(), 1);
                assert_eq!(renderer.landscape_bind_groups.len(), 1);

                changed_scene.textures[0].revision = 4;
                changed_scene.textures[0].base_revision = Some(3);
                changed_scene.textures[0].dirty = vec![Rect::new(1, 1, 1, 1)];
                let disabled_after_skipped_revision =
                    render_identity_readback(&mut renderer, &device, &queue, &changed_scene);
                assert_eq!(disabled_after_skipped_revision, disabled);
                assert_eq!(renderer.last_stats().full_upload_calls, 1);
                assert_eq!(renderer.last_stats().dirty_upload_calls, 0);

                renderer.set_shader_landscape(true);
                assert!(renderer.textures.contains_key(&base));
                assert!(renderer.quad_bind_groups.is_empty());
                assert!(renderer.landscape_bind_groups.is_empty());
                let enabled_without_plan =
                    render_identity_readback(&mut renderer, &device, &queue, &changed_scene);
                assert_eq!(enabled_without_plan, disabled);
                assert_eq!(renderer.last_stats().full_upload_calls, 0);

                renderer.set_pending_shader_landscape(Some((base, detail_plan.clone())));
                let enabled =
                    render_identity_readback(&mut renderer, &device, &queue, &changed_scene);
                assert_eq!(enabled, detail_composed);

                let generation = renderer.generation();
                renderer.recreate(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
                assert_ne!(renderer.generation(), generation);
                assert!(renderer.textures.is_empty());
                assert!(renderer.quad_bind_groups.is_empty());
                assert!(renderer.landscape_bind_groups.is_empty());
                let recovered_cpu =
                    render_identity_readback(&mut renderer, &device, &queue, &changed_scene);
                assert_eq!(recovered_cpu, disabled);
                assert_eq!(renderer.last_stats().created_source_textures, 1);
                assert_eq!(renderer.last_stats().full_upload_calls, 1);

                renderer.set_pending_shader_landscape(Some((base, detail_plan)));
                let recovered_shader =
                    render_identity_readback(&mut renderer, &device, &queue, &changed_scene);
                assert_eq!(recovered_shader, detail_composed);
            } else {
                let mut invalidated_scene = changed_scene.clone();
                invalidated_scene.textures[0].revision = 3;
                invalidated_scene.textures[0].base_revision = Some(2);
                invalidated_scene.textures[0].dirty = vec![Rect::new(1, 1, 1, 1)];
                let invalidated =
                    render_identity_readback(&mut renderer, &device, &queue, &invalidated_scene);
                assert_ne!(invalidated, changed);
                assert_eq!(renderer.last_stats().created_source_textures, 1);
                assert_eq!(renderer.last_stats().full_upload_calls, 1);
                assert!(matches!(
                    renderer
                        .textures
                        .get(&base)
                        .expect("invalidated CPU landscape")
                        .contents,
                    CachedTextureContents::Source
                ));
            }
            let validation = runtime.block_on(validation_scope.pop());
            assert!(
                validation.is_none(),
                "detail {detail}: the unchanged frame reported a wgpu validation error: \
                 {validation:?}"
            );
        }
    }

    #[test]
    fn recreating_the_renderer_keeps_its_configured_presentation_flags() {
        // `recreate` replaces the whole renderer after a lost device, and
        // `build` hard-codes every presentation flag to false. Without carrying
        // them over, device-loss recovery silently reverts the player's
        // configured opt-ins mid-session with nothing to re-apply them: the
        // renderer is a local in `main` that GameApp never holds, so the
        // options dialog cannot push them back either.
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping renderer recreate flag carry-over");
            return;
        };
        let mut renderer = test_renderer(&device, &queue);
        renderer.set_mipmaps(true);
        renderer.set_smooth_landscape(true);
        renderer.set_shader_landscape(true);
        renderer.set_landscape_detail(3);

        let generation = renderer.generation();
        renderer.recreate(&device, &queue, wgpu::TextureFormat::Rgba8Unorm);
        assert_ne!(
            renderer.generation(),
            generation,
            "recreate advances the generation"
        );

        assert!(renderer.mipmaps(), "mipmaps must survive a device loss");
        assert!(
            renderer.smooth_landscape(),
            "smooth landscape must survive a device loss"
        );
        assert!(
            renderer.shader_landscape(),
            "shader landscape must survive a device loss"
        );
        assert_eq!(
            renderer.landscape_detail(),
            3,
            "the landscape detail level must survive a device loss"
        );
    }

    /// The detail factor supersamples the composed plane without changing which
    /// map pixels carry material: sky and absent slots stay empty in every
    /// sub-pixel, while the pattern itself now varies inside one landscape
    /// pixel — which is exactly the detail the CPU composer cannot express.
    #[test]
    fn shader_landscape_detail_supersamples_the_same_terrain() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping shader landscape detail readback");
            return;
        };
        let extent = [16_u32, 12_u32];
        let index_plane = shader_landscape_index_plane(extent);
        let (atlas_extent, atlas, _) = shader_landscape_atlas();
        let slots = shader_landscape_slots();
        let inputs = |detail| ShaderLandscapeInputs {
            extent,
            index_plane: &index_plane,
            shading_plane: None,
            atlas: &atlas,
            atlas_extent,
            slots: &slots,
            detail,
        };
        let base = compose_shader_landscape(&device, &queue, inputs(1));
        let detailed = compose_shader_landscape(&device, &queue, inputs(2));
        assert_eq!(detailed.len(), base.len() * 4);
        let mut varied = 0_usize;
        for y in 0..extent[1] {
            for x in 0..extent[0] {
                let map_index = (y * extent[0] + x) as usize;
                let byte = index_plane[map_index];
                let structural = byte != 0
                    && slots[usize::from(byte & 0x7f)].params[3] & SHADER_LANDSCAPE_PRESENT != 0;
                let sub = |sub_x: u32, sub_y: u32| {
                    let offset = (((y * 2 + sub_y) * extent[0] * 2 + x * 2 + sub_x) * 4) as usize;
                    [
                        detailed[offset],
                        detailed[offset + 1],
                        detailed[offset + 2],
                        detailed[offset + 3],
                    ]
                };
                if !structural {
                    assert_eq!(
                        base[map_index * 4..map_index * 4 + 4],
                        [0; 4],
                        "sky and absent slots stay empty at ({x}, {y})"
                    );
                    for sub_y in 0..2 {
                        for sub_x in 0..2 {
                            assert_eq!(
                                sub(sub_x, sub_y),
                                [0; 4],
                                "detail must not paint sky at ({x}, {y})"
                            );
                        }
                    }
                    continue;
                }
                if sub(0, 0) != sub(1, 0) || sub(0, 0) != sub(0, 1) {
                    varied += 1;
                }
            }
        }
        assert!(
            varied > 0,
            "detail 2 must sample the pattern inside a landscape pixel"
        );
    }

    /// A PXS point and a debug line are sized from the application scale
    /// *and* the viewport zoom.
    ///
    /// A vertex position picks the zoom up from the projection, but a raster
    /// width is not a position: sizing it from `scale` alone leaves rain,
    /// spray, dug-material sparks and every debug line at their unzoomed width
    /// the moment the world is magnified. Zoom is pinned at 1.0 today, so this
    /// is the term being in place rather than a visible change.
    #[test]
    fn point_and_line_rasters_scale_with_the_world_zoom() {
        let projection = |scale: f32, world_zoom: f32| {
            let presentation = GpuPresentation {
                physical_extent: [640, 480],
                scale,
                crop_top: 0,
                world_zoom,
            };
            draw_projection(None, [320, 240], &presentation)
                .expect("the projection resolves")
                .expect("the clip covers the drawable")
        };

        assert_eq!(
            rounded_raster_width(&projection(1.0, 1.0)),
            1,
            "an unzoomed world at scale 1 is one physical pixel"
        );
        assert_eq!(
            rounded_raster_width(&projection(1.0, 3.0)),
            3,
            "a 3x world magnifies the point with it"
        );
        assert_eq!(
            rounded_raster_width(&projection(2.0, 2.0)),
            4,
            "scale and zoom compose rather than replacing one another"
        );
        assert_eq!(
            rounded_raster_width(&projection(2.0, 1.0)),
            2,
            "an unzoomed world still follows the application scale alone"
        );
        assert_eq!(
            rounded_raster_width(&projection(1.0, 0.25)),
            1,
            "a zoomed-out world never shrinks a point out of existence"
        );
    }

    /// A retained plane is re-uploaded by the rows that actually changed, so
    /// an unchanged landscape uploads nothing and a local edit uploads its own
    /// band rather than the whole map.
    #[test]
    fn a_retained_plane_uploads_only_the_rows_that_changed() {
        let previous = vec![0_u8; 4 * 3];

        assert_eq!(
            changed_rows(&previous, &previous, 4),
            None,
            "an unchanged plane uploads nothing"
        );

        let mut edited = previous.clone();
        edited[5] = 1;
        assert_eq!(
            changed_rows(&previous, &edited, 4),
            Some(1..2),
            "one edited row uploads that row alone"
        );

        let mut spanning = previous.clone();
        spanning[1] = 1;
        spanning[9] = 1;
        assert_eq!(
            changed_rows(&previous, &spanning, 4),
            Some(0..3),
            "separate edits upload the band that covers them"
        );

        assert_eq!(
            changed_rows(&previous, &[0_u8; 4 * 5], 4),
            Some(0..5),
            "a plane of a different length is uploaded whole"
        );
    }

    /// clonk-org/clonk-rs#273's third criterion — *extent/detail change,
    /// material reload, resize and device recreation invalidate exactly the
    /// resources they own* — as the decision that implements it, with no
    /// device needed.
    ///
    /// The planes are one texel per map pixel and the atlas comes from the
    /// material catalogue, so they are invalidated by different things. The
    /// bind group names every view, so it survives only when they all do.
    #[test]
    fn retained_landscape_resources_are_invalidated_by_what_owns_them() {
        let key =
            |extent: [u32; 2], shading: bool, atlas_extent: [u32; 2]| ShaderLandscapeResourceKey {
                extent,
                shading,
                atlas_extent,
            };
        let base = key([64, 64], false, [16, 16]);

        assert_eq!(
            ShaderLandscapeReuse::between(None, base),
            ShaderLandscapeReuse::default(),
            "the first composition has nothing to keep"
        );
        assert_eq!(
            ShaderLandscapeReuse::between(Some(base), base),
            ShaderLandscapeReuse {
                planes: true,
                atlas: true,
                bind_group: true,
            },
            "an unchanged landscape keeps everything, bind group included"
        );

        // A resize moves the planes; the catalogue did not move, but the bind
        // group named the plane views, so it goes too.
        assert_eq!(
            ShaderLandscapeReuse::between(Some(base), key([32, 32], false, [16, 16])),
            ShaderLandscapeReuse {
                planes: false,
                atlas: true,
                bind_group: false,
            }
        );
        // Turning shading on adds a plane of a different format.
        assert_eq!(
            ShaderLandscapeReuse::between(Some(base), key([64, 64], true, [16, 16])),
            ShaderLandscapeReuse {
                planes: false,
                atlas: true,
                bind_group: false,
            }
        );
        // A material reload that resizes the atlas leaves the map planes alone.
        assert_eq!(
            ShaderLandscapeReuse::between(Some(base), key([64, 64], false, [32, 32])),
            ShaderLandscapeReuse {
                planes: true,
                atlas: false,
                bind_group: false,
            }
        );

        // The detail factor is deliberately absent from the key: it scales the
        // composed *output*, which the renderer owns, and none of the
        // composer's own resources are shaped by it.
        assert_eq!(
            ShaderLandscapeReuse::between(Some(base), base),
            ShaderLandscapeReuse::between(Some(base), base),
        );
    }

    /// The band is then narrowed to the columns that differ, because a
    /// landscape edit is a rectangle: one texel out of a wide map must not
    /// carry its whole row.
    #[test]
    fn a_retained_plane_narrows_its_band_to_the_columns_that_changed() {
        // 4 texels per row, 3 rows, one byte per texel.
        let previous = vec![0_u8; 4 * 3];

        assert_eq!(changed_rect(&previous, &previous, 4, 1), None);

        let mut single = previous.clone();
        single[5] = 1;
        assert_eq!(
            changed_rect(&previous, &single, 4, 1),
            Some((1..2, 1..2)),
            "one texel is one row and one column"
        );

        // Two edits on the same row bound the columns between them.
        let mut spread = previous.clone();
        spread[4] = 1;
        spread[7] = 1;
        assert_eq!(changed_rect(&previous, &spread, 4, 1), Some((1..2, 0..4)));

        // Edits on different rows and columns give the covering rectangle,
        // which is what a single `write_texture` can carry.
        let mut diagonal = previous.clone();
        diagonal[1] = 1;
        diagonal[10] = 1;
        assert_eq!(changed_rect(&previous, &diagonal, 4, 1), Some((0..3, 1..3)));

        // A multi-byte texel is compared whole, so a change in either byte
        // moves the column.
        let wide_previous = vec![0_u8; 2 * 2 * 2];
        let mut wide_next = wide_previous.clone();
        wide_next[3] = 1;
        assert_eq!(
            changed_rect(&wide_previous, &wide_next, 4, 2),
            Some((0..1, 1..2))
        );

        assert_eq!(
            changed_rect(&previous, &[0_u8; 4 * 5], 4, 1),
            Some((0..5, 0..4)),
            "a plane of a different length is uploaded whole"
        );
    }

    /// A landscape update composes into the output it already has. Creating a
    /// new one each update also invalidated every quad, object and landscape
    /// bind group that named it, so the whole scene's bindings were rebuilt
    /// for one landscape edit.
    /// clonk-org/clonk-rs#273's first acceptance criterion: *after warmup, an
    /// unchanged shader landscape creates no textures, buffers, bind groups, or
    /// uploads*.
    ///
    /// clonk-org/clonk-rs#669 retained the resources and made the uploads
    /// row-wise, but only output *creation* was observable, so "uploads
    /// nothing" rested on reading `upload_changed_rows`. A re-upload of an
    /// unchanged plane would have cost a staging write per frame with nothing
    /// to catch it.
    #[test]
    fn an_unchanged_shader_landscape_creates_and_uploads_nothing() {
        let Some((_runtime, _instance, _adapter, device, queue)) =
            test_wgpu_device("lc_gpu_shader_landscape_idle_device", true)
        else {
            return;
        };
        let extent = [12_u32, 12_u32];
        let base = GpuTextureId::fresh();
        let scene = shader_landscape_scene_fixture(base, extent, extent, 1);
        let mut renderer = test_renderer(&device, &queue);
        renderer.set_shader_landscape(true);
        renderer.set_landscape_detail(1);
        let plan = shader_landscape_plan_fixture(extent);

        // Warmup: the first composition necessarily creates and uploads.
        renderer.set_pending_shader_landscape(Some((base, plan.clone())));
        let first = render_identity_readback(&mut renderer, &device, &queue, &scene);
        assert!(
            renderer.last_stats().shader_landscape_upload_calls > 0,
            "the first composition has to upload its planes"
        );

        // The same plan again: every resource is reusable and every plane byte
        // is the one already on the GPU.
        renderer.set_pending_shader_landscape(Some((base, plan)));
        let repeated = render_identity_readback(&mut renderer, &device, &queue, &scene);
        let stats = renderer.last_stats();
        assert_eq!(
            stats.created_shader_landscape_outputs, 0,
            "an unchanged landscape composes into the output it already has"
        );
        assert_eq!(
            (
                stats.shader_landscape_upload_calls,
                stats.shader_landscape_upload_bytes
            ),
            (0, 0),
            "and uploads nothing at all"
        );
        assert_eq!(
            repeated, first,
            "and still presents the same pixels it did before"
        );
    }

    /// clonk-org/clonk-rs#273's second criterion, upload half: *a small edit
    /// uploads only its exact index region*.
    ///
    /// clonk-org/clonk-rs#669 uploads the changed **rows**, so one texel on a
    /// wide map still costs its whole row — 64 bytes here for a one-byte edit,
    /// and a 4096-wide map would pay 4096. The region is a rectangle, and
    /// `Queue::write_texture` takes an origin and a row stride, so the columns
    /// can be bounded the same way the rows already are.
    #[test]
    fn a_small_landscape_edit_uploads_only_its_own_rectangle() {
        let Some((_runtime, _instance, _adapter, device, queue)) =
            test_wgpu_device("lc_gpu_shader_landscape_rect_upload_device", true)
        else {
            return;
        };
        let extent = [64_u32, 64_u32];
        let base = GpuTextureId::fresh();
        let scene = shader_landscape_scene_fixture(base, extent, extent, 1);
        let mut renderer = test_renderer(&device, &queue);
        renderer.set_shader_landscape(true);
        renderer.set_landscape_detail(1);
        let plan = shader_landscape_plan_fixture(extent);

        renderer.set_pending_shader_landscape(Some((base, plan.clone())));
        let before = render_identity_readback(&mut renderer, &device, &queue, &scene);

        // One texel, in the middle of a 64-wide row.
        let mut edited = plan.clone();
        let texel = 10 * extent[0] as usize + 32;
        edited.index_plane[texel] = u8::from(edited.index_plane[texel] == 0);
        renderer.set_pending_shader_landscape(Some((base, edited)));
        let after = render_identity_readback(&mut renderer, &device, &queue, &scene);

        assert_ne!(after, before, "the edit has to reach the output");
        assert_eq!(
            renderer.last_stats().shader_landscape_upload_bytes,
            1,
            "a one-texel edit is one byte, not its whole row"
        );
    }

    /// clonk-org/clonk-rs#273's third criterion, the half the reuse decision
    /// already knew but the caller ignored: *invalidate exactly the resources
    /// they own*.
    ///
    /// `ShaderLandscapeReuse::between` reports `planes: true, atlas: false` for
    /// a catalogue reload that resizes the atlas, and the caller then dropped
    /// the retained set wholesale — so the index and shading planes were
    /// recreated and re-uploaded in full because the *catalogue* changed. On a
    /// large map that is megabytes for a change that touched none of it.
    #[test]
    fn a_catalogue_reload_keeps_the_map_planes_it_did_not_change() {
        let Some((_runtime, _instance, _adapter, device, queue)) =
            test_wgpu_device("lc_gpu_shader_landscape_catalogue_reload_device", true)
        else {
            return;
        };
        let extent = [64_u32, 64_u32];
        let base = GpuTextureId::fresh();
        let scene = shader_landscape_scene_fixture(base, extent, extent, 1);
        let mut renderer = test_renderer(&device, &queue);
        renderer.set_shader_landscape(true);
        renderer.set_landscape_detail(1);

        let plan = shader_landscape_plan_fixture(extent);
        renderer.set_pending_shader_landscape(Some((base, plan.clone())));
        let before = render_identity_readback(&mut renderer, &device, &queue, &scene);

        // A reloaded catalogue: a differently shaped atlas over the same map.
        let mut reloaded = plan.clone();
        let atlas_extent = [plan.atlas_extent[0] + 2, plan.atlas_extent[1] + 1];
        reloaded.atlas_extent = atlas_extent;
        reloaded.atlas = (0..(atlas_extent[0] * atlas_extent[1]) as usize * 4)
            .map(|byte| (byte % 251) as u8)
            .collect();
        renderer.set_pending_shader_landscape(Some((base, reloaded)));
        let after = render_identity_readback(&mut renderer, &device, &queue, &scene);

        let atlas_bytes = u64::from(atlas_extent[0]) * u64::from(atlas_extent[1]) * 4;
        assert_eq!(
            renderer.last_stats().shader_landscape_upload_bytes,
            atlas_bytes,
            "only the atlas moved, so only the atlas is uploaded"
        );
        assert_ne!(
            after, before,
            "the reloaded catalogue still has to reach the output"
        );
    }

    /// clonk-org/clonk-rs#273's second criterion, recompose half: *a small
    /// edit recomposes only its exact index region*.
    ///
    /// The fragment shader reads its own map texel and nothing else —
    /// `textureLoad(index_plane, map, 0)` with `map = fine / detail` — so the
    /// composition of an output texel depends only on the map texel under it,
    /// the slot table and the atlas. A dirty map rectangle therefore scales
    /// straight to an output scissor with no neighbourhood expansion.
    ///
    /// The risk a scissor carries is stale pixels outside it, which a
    /// from-scratch comparison is the only thing that catches: the existing
    /// parity tests compose on a fresh composer every time and would miss it.
    #[test]
    fn an_incremental_recompose_matches_composing_the_same_plan_from_scratch() {
        let Some((_runtime, _instance, _adapter, device, queue)) =
            test_wgpu_device("lc_gpu_shader_landscape_scissor_device", true)
        else {
            return;
        };
        let extent = [32_u32, 32_u32];
        let base = GpuTextureId::fresh();
        let scene = shader_landscape_scene_fixture(base, extent, extent, 1);
        let plan = shader_landscape_plan_fixture(extent);

        let mut edited = plan.clone();
        for row in 8..11_usize {
            for column in 5..9_usize {
                let texel = row * extent[0] as usize + column;
                edited.index_plane[texel] = u8::from(edited.index_plane[texel] == 0);
            }
        }

        // Incremental: the edit lands on a composer that already holds the
        // previous composition and its output.
        let mut incremental = test_renderer(&device, &queue);
        incremental.set_shader_landscape(true);
        incremental.set_landscape_detail(1);
        incremental.set_pending_shader_landscape(Some((base, plan)));
        let _ = render_identity_readback(&mut incremental, &device, &queue, &scene);
        incremental.set_pending_shader_landscape(Some((base, edited.clone())));
        let after_edit = render_identity_readback(&mut incremental, &device, &queue, &scene);
        let composed = incremental.last_stats().shader_landscape_composed_texels;

        // From scratch: the same plan with nothing retained.
        let mut fresh = test_renderer(&device, &queue);
        fresh.set_shader_landscape(true);
        fresh.set_landscape_detail(1);
        fresh.set_pending_shader_landscape(Some((base, edited)));
        let from_scratch = render_identity_readback(&mut fresh, &device, &queue, &scene);

        assert_eq!(
            after_edit, from_scratch,
            "an incrementally recomposed landscape must be the landscape"
        );
        let full = u64::from(extent[0]) * u64::from(extent[1]);
        assert!(
            composed < full,
            "a 4x3 edit recomposed {composed} of {full} texels"
        );
    }

    /// The same comparison at a detail factor above one, which is where the
    /// scissor's own arithmetic lives.
    ///
    /// clonk-org/clonk-rs#707 scales the dirty map rectangle by the detail
    /// factor to reach output space. At detail 1 that scaling is the identity,
    /// so the test that introduced it could not tell a correct `x * detail`
    /// from a missing one — and clonk-org/clonk-rs#273's fifth criterion asks
    /// for detail 2–4 to keep the same semantics.
    #[test]
    fn an_incremental_recompose_at_detail_matches_composing_from_scratch() {
        let Some((_runtime, _instance, _adapter, device, queue)) =
            test_wgpu_device("lc_gpu_shader_landscape_scissor_detail_device", true)
        else {
            return;
        };
        let extent = [16_u32, 16_u32];
        let detail = 3_u32;
        let base = GpuTextureId::fresh();
        let source_extent = [extent[0] * detail, extent[1] * detail];
        let scene = shader_landscape_scene_fixture(base, extent, source_extent, 1);
        let plan = shader_landscape_plan_fixture(extent);

        // An edit that does not start at the origin: an off-by-one in the
        // scaled origin shows up as a shifted patch rather than a missing one.
        let mut edited = plan.clone();
        for row in 6..9_usize {
            for column in 4..7_usize {
                let texel = row * extent[0] as usize + column;
                edited.index_plane[texel] = u8::from(edited.index_plane[texel] == 0);
            }
        }

        let mut incremental = test_renderer(&device, &queue);
        incremental.set_shader_landscape(true);
        incremental.set_landscape_detail(detail);
        incremental.set_pending_shader_landscape(Some((base, plan)));
        let _ = render_identity_readback(&mut incremental, &device, &queue, &scene);
        incremental.set_pending_shader_landscape(Some((base, edited.clone())));
        let after_edit = render_identity_readback(&mut incremental, &device, &queue, &scene);
        let composed = incremental.last_stats().shader_landscape_composed_texels;

        let mut fresh = test_renderer(&device, &queue);
        fresh.set_shader_landscape(true);
        fresh.set_landscape_detail(detail);
        fresh.set_pending_shader_landscape(Some((base, edited)));
        let from_scratch = render_identity_readback(&mut fresh, &device, &queue, &scene);

        assert_eq!(
            after_edit, from_scratch,
            "a supersampled landscape must recompose to the same pixels"
        );
        assert_eq!(
            composed,
            u64::from(3 * detail) * u64::from(3 * detail),
            "the scissor covers the edit scaled by the detail factor, and no more"
        );
    }

    #[test]
    fn a_landscape_update_composes_into_the_retained_output() {
        let Some((_runtime, _instance, _adapter, device, queue)) =
            test_wgpu_device("lc_gpu_shader_landscape_output_retention_device", true)
        else {
            return;
        };
        let extent = [12_u32, 12_u32];
        let base = GpuTextureId::fresh();
        let scene = shader_landscape_scene_fixture(base, extent, extent, 1);
        let mut renderer = test_renderer(&device, &queue);
        renderer.set_shader_landscape(true);
        renderer.set_landscape_detail(1);

        renderer.set_pending_shader_landscape(Some((base, shader_landscape_plan_fixture(extent))));
        let first = render_identity_readback(&mut renderer, &device, &queue, &scene);
        assert_eq!(
            renderer.last_stats().created_shader_landscape_outputs,
            1,
            "the first composition has no output to keep"
        );

        let mut edited = shader_landscape_plan_fixture(extent);
        let row = extent[0] as usize;
        for byte in &mut edited.index_plane[4 * row..6 * row] {
            *byte = u8::from(*byte == 0);
        }
        renderer.set_pending_shader_landscape(Some((base, edited.clone())));
        let updated = render_identity_readback(&mut renderer, &device, &queue, &scene);
        assert_eq!(
            renderer.last_stats().created_shader_landscape_outputs,
            0,
            "an update of the same extent composes into the output it already has"
        );
        assert_ne!(updated, first, "the edited plan must reach the output");
        assert!(
            !renderer.quad_bind_groups.is_empty(),
            "a retained output keeps the bind groups that name it"
        );

        // A different extent owns a different output, so that one is recreated.
        let larger = [16_u32, 16_u32];
        let larger_scene = shader_landscape_scene_fixture(base, larger, larger, 1);
        renderer.set_pending_shader_landscape(Some((base, shader_landscape_plan_fixture(larger))));
        let _ = render_identity_readback(&mut renderer, &device, &queue, &larger_scene);
        assert_eq!(
            renderer.last_stats().created_shader_landscape_outputs,
            1,
            "a resized landscape cannot reuse the smaller output"
        );
    }

    /// Composing twice through one composer must give the same pixels as
    /// composing each landscape on its own. The retained planes upload only
    /// the rows that changed, so a row left stale — or a partial write placed
    /// at the wrong origin — shows up here as a pixel difference and nowhere
    /// else.
    #[test]
    fn a_retained_composition_matches_composing_each_landscape_fresh() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            return;
        };
        let extent = [8_u32, 8_u32];
        let (atlas_extent, atlas, _) = shader_landscape_atlas();
        let slots = shader_landscape_slots();
        let first = shader_landscape_index_plane(extent);
        // A band in the middle changes material; the rows around it do not.
        let mut second = first.clone();
        let row = extent[0] as usize;
        for byte in &mut second[3 * row..5 * row] {
            *byte = u8::from(*byte == 0);
        }
        assert_ne!(first, second, "the fixture must actually differ");

        fn inputs<'a>(
            plane: &'a [u8],
            extent: [u32; 2],
            atlas: &'a [u8],
            atlas_extent: [u32; 2],
            slots: &'a [ShaderLandscapeSlot],
        ) -> ShaderLandscapeInputs<'a> {
            ShaderLandscapeInputs {
                extent,
                index_plane: plane,
                shading_plane: None,
                atlas,
                atlas_extent,
                slots,
                detail: 1,
            }
        }
        let first_inputs = || inputs(&first, extent, &atlas, atlas_extent, &slots);
        let second_inputs = || inputs(&second, extent, &atlas, atlas_extent, &slots);

        let mut composer = ShaderLandscapeComposer::new(&device);
        let retained_first =
            compose_shader_landscape_with(&mut composer, &device, &queue, first_inputs());
        let retained_second =
            compose_shader_landscape_with(&mut composer, &device, &queue, second_inputs());
        let retained_again =
            compose_shader_landscape_with(&mut composer, &device, &queue, second_inputs());

        assert_eq!(
            retained_first,
            compose_shader_landscape(&device, &queue, first_inputs()),
            "the first composition through a retained composer is unchanged"
        );
        assert_eq!(
            retained_second,
            compose_shader_landscape(&device, &queue, second_inputs()),
            "an edited band recomposes exactly as a fresh composition does"
        );
        assert_eq!(
            retained_again, retained_second,
            "recomposing an unchanged landscape repeats it"
        );
    }

    /// The composition resources are retained, so each set survives exactly
    /// the inputs that do not invalidate it: the planes follow the map extent
    /// and whether shading is present, the atlas follows the material
    /// catalogue, and the bind group — which names every view — survives only
    /// when both do.
    #[test]
    fn retained_composition_resources_survive_the_inputs_that_do_not_invalidate_them() {
        let key = ShaderLandscapeResourceKey {
            extent: [64, 32],
            shading: true,
            atlas_extent: [16, 16],
        };

        assert_eq!(
            ShaderLandscapeReuse::between(Some(key), key),
            ShaderLandscapeReuse {
                planes: true,
                atlas: true,
                bind_group: true,
            },
            "an unchanged composition recreates nothing"
        );

        assert_eq!(
            ShaderLandscapeReuse::between(None, key),
            ShaderLandscapeReuse::default(),
            "the first composition has nothing to keep"
        );

        let resized = ShaderLandscapeResourceKey {
            extent: [64, 64],
            ..key
        };
        assert_eq!(
            ShaderLandscapeReuse::between(Some(key), resized),
            ShaderLandscapeReuse {
                planes: false,
                atlas: true,
                bind_group: false,
            },
            "a resized map keeps the catalogue it did not touch"
        );

        let unshaded = ShaderLandscapeResourceKey {
            shading: false,
            ..key
        };
        assert_eq!(
            ShaderLandscapeReuse::between(Some(key), unshaded),
            ShaderLandscapeReuse {
                planes: false,
                atlas: true,
                bind_group: false,
            },
            "the shading plane changes extent with its presence"
        );

        let reloaded = ShaderLandscapeResourceKey {
            atlas_extent: [32, 16],
            ..key
        };
        assert_eq!(
            ShaderLandscapeReuse::between(Some(key), reloaded),
            ShaderLandscapeReuse {
                planes: true,
                atlas: false,
                bind_group: false,
            },
            "a material reload keeps the planes it did not touch"
        );
    }

    #[test]
    fn shader_landscape_rejects_a_short_index_plane() {
        let inputs = ShaderLandscapeInputs {
            extent: [4, 4],
            index_plane: &[0; 8],
            shading_plane: None,
            atlas: &[0; 4],
            atlas_extent: [1, 1],
            slots: &[],
            detail: 1,
        };
        assert!(matches!(
            inputs.validate(),
            Err(GpuRendererError::ShaderLandscapeInputs("short index plane"))
        ));
    }

    #[test]
    fn smooth_landscape_magnification_antialiases_without_a_sky_halo() {
        // The landscape cache stores sky as RGBA(0,0,0,0) against opaque
        // material, so plain bilinear would ring every silhouette with dark
        // grey. Magnify a one-texel-wide edge 8x and check the boundary both
        // ramps in coverage and keeps the material's colour.
        let Some((_runtime, _instance, _adapter, device, queue)) =
            test_wgpu_device("lc_gpu_landscape_smooth_test_device", false)
        else {
            eprintln!("no wgpu adapter; skipping landscape magnification readback");
            return;
        };

        // Two texels: opaque red material on the left, sky on the right.
        let base = GpuTextureId::fresh();
        let mask = GpuTextureId::fresh();
        let liquid = GpuTextureId::fresh();
        let textures = vec![
            GpuTextureResource::immutable_rgba(base, 2, 1, vec![200, 0, 0, 255, 0, 0, 0, 0].into()),
            GpuTextureResource {
                format: GpuTextureFormat::R8,
                pixels: vec![0_u8; 2].into(),
                ..GpuTextureResource::immutable_rgba(mask, 2, 1, vec![0_u8; 8].into())
            },
            GpuTextureResource::immutable_rgba(liquid, 1, 1, vec![128, 128, 128, 255].into()),
        ];
        let corner = |x: f32, y: f32, u: f32, v: f32| GpuVertex {
            position: [x, y, 1.0],
            uv: [u, v],
            modulation: [1.0, 1.0, 1.0, 0.0],
            owner_modulation: [0.0; 4],
            outer_modulation: clonk_graphics::GpuOuterModulation::default(),
            owner_outer_modulation: clonk_graphics::GpuOuterModulation::default(),
            sample_tile: [0.0; 4],
        };
        let scene = GpuScene {
            logical_extent: [16, 4],
            clear: Color::transparent(),
            gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
            gamma_mode: GpuGammaMode::Disabled,
            textures,
            commands: vec![GpuCommand::Landscape {
                base,
                liquid_mask: Some(mask),
                liquid: Some(liquid),
                vertices: [
                    corner(0.0, 0.0, 0.0, 0.0),
                    corner(16.0, 0.0, 1.0, 0.0),
                    corner(0.0, 4.0, 0.0, 1.0),
                    corner(16.0, 4.0, 1.0, 1.0),
                ],
                clip: None,
                phase: [0.0; 3],
                gamma: false,
            }],
        };

        let row = |renderer: &mut RetainedGpuRenderer| {
            let frame = render_identity_readback(renderer, &device, &queue, &scene);
            (0..16)
                .map(|x| readback_pixel(&frame, x, 1))
                .collect::<Vec<_>>()
        };

        let mut classic = test_renderer(&device, &queue);
        let classic_row = row(&mut classic);
        assert!(
            classic_row
                .iter()
                .all(|pixel| pixel[3] == 0 || pixel[3] == 255),
            "the C++ path is a hard nearest step: {classic_row:?}"
        );

        let mut smooth = test_renderer(&device, &queue);
        smooth.set_smooth_landscape(true);
        let smooth_row = row(&mut smooth);
        let partial: Vec<[u8; 4]> = smooth_row
            .iter()
            .copied()
            .filter(|pixel| pixel[3] > 0 && pixel[3] < 255)
            .collect();
        assert!(
            !partial.is_empty(),
            "magnifying a one-texel edge must produce coverage in between: {smooth_row:?}"
        );
        for pixel in &partial {
            // The readback is already composited over a transparent clear, so
            // an edge that kept the material's colour reads back as exactly
            // that colour scaled by its coverage. Straight-alpha filtering
            // would have darkened the colour first and landed well below this.
            let expected = (200.0 * f32::from(pixel[3]) / 255.0).round() as i32;
            assert!(
                (i32::from(pixel[0]) - expected).abs() <= 2 && pixel[1] == 0 && pixel[2] == 0,
                "a partially covered edge keeps the material colour, not a sky-blended halo: \
                 {pixel:?} (coverage implies red {expected})"
            );
        }
    }

    #[test]
    fn gpu_renderer_matches_cpu_reference_frame() {
        let (runtime, instance, adapter, device, queue) =
            test_wgpu_device("lc_gpu_parity_test_device", true).unwrap_or_else(|| {
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
        let validation_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);

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
        let mut renderer = test_renderer(&device, &queue);

        let initial = render_identity_readback(&mut renderer, &device, &queue, &scene);
        let validation = runtime.block_on(validation_scope.pop());
        assert!(
            validation.is_none(),
            "initial device frame reported wgpu validation error: {validation:?}"
        );
        let validation_scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        assert_eq!(
            initial.rgba,
            expected_frame(LOGICAL, initial_mutable, &scene.gamma),
            "initial retained GPU frame must match the local CPU oracle"
        );
        assert_eq!(renderer.last_stats().created_source_textures, 10);
        assert_eq!(renderer.last_stats().full_upload_calls, 10);
        assert_eq!(renderer.last_stats().full_upload_bytes, 46);
        assert_eq!(renderer.last_stats().dirty_upload_calls, 0);
        assert_eq!(renderer.last_stats().dirty_upload_bytes, 0);
        assert_eq!(renderer.last_stats().generic_vertices, 6);
        assert_eq!(
            renderer.last_stats().generic_vertex_upload_bytes,
            6 * PACKED_VERTEX_STRIDE as usize
        );
        assert_eq!(renderer.last_stats().landscape_instances, 1);
        assert_eq!(
            renderer.last_stats().landscape_instance_upload_bytes,
            PACKED_LANDSCAPE_INSTANCE_STRIDE as usize
        );
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
        let raw = render_identity_readback(&mut renderer, &device, &queue, &raw_scene);
        let mut expected_monitor = raw.rgba.clone();
        monitor_ramp.apply_to_rgba_bytes(&mut expected_monitor);
        let mut monitor_scene = raw_scene;
        monitor_scene.gamma_mode = GpuGammaMode::Monitor;
        let monitor = render_identity_readback(&mut renderer, &device, &queue, &monitor_scene);
        assert_ne!(monitor.rgba, raw.rgba);
        assert_eq!(
            monitor.rgba, expected_monitor,
            "monitor gamma must resolve the complete composition before readback",
        );
        assert_eq!(
            renderer.last_stats().total_draw_calls,
            renderer.last_stats().draw_calls + 2,
            "raw draw evidence includes monitor-gamma and presentation passes",
        );
        assert_eq!(renderer.last_stats().monitor_gamma_draw_calls, 1);
        assert_eq!(renderer.last_stats().presentation_draw_calls, 1);
        assert!(renderer.last_stats().has_exact_draw_call_counts());
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
        let _ = render_identity_readback(&mut renderer, &device, &queue, &hidden);
        assert_eq!(
            renderer.last_stats().resident_source_textures,
            10,
            "temporarily hidden C4Surface textures stay resident"
        );
        assert_eq!(renderer.last_stats().full_upload_calls, 0);
        assert_eq!(renderer.last_stats().dirty_upload_calls, 0);
        let visible_again = render_identity_readback(&mut renderer, &device, &queue, &scene);
        assert_eq!(visible_again.rgba, initial.rgba);
        assert_eq!(renderer.last_stats().created_source_textures, 0);
        assert_eq!(renderer.last_stats().full_upload_calls, 0);
        assert_eq!(renderer.last_stats().full_upload_bytes, 0);
        assert_eq!(renderer.last_stats().dirty_upload_calls, 0);

        let scaled = render_readback(
            &mut renderer,
            &device,
            &queue,
            &scene,
            &GpuPresentation {
                physical_extent: [LOGICAL[0] * 2, LOGICAL[1] * 2],
                scale: 2.0,
                crop_top: 0,
                world_zoom: 1.0,
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
                    world_zoom: 1.0,
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
                    world_zoom: 1.0,
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
                style: GpuSolidStyle::NONE,
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
                world_zoom: 1.0,
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
                world_zoom: 1.0,
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
                style: GpuSolidStyle::NONE,
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
                style: GpuSolidStyle::NONE,
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
            let diagonal = render_identity_readback(&mut renderer, &device, &queue, &scene);
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
                style: GpuSolidStyle::NONE,
            }],
        };
        let frame = render_identity_readback(&mut renderer, &device, &queue, &frame_scene);
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
                style: GpuSolidStyle::NONE,
            }],
        };
        let translucent_point =
            render_identity_readback(&mut renderer, &device, &queue, &translucent_point_scene);
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
                style: GpuSolidStyle::NONE,
            }],
        };
        let additive = render_identity_readback(&mut renderer, &device, &queue, &additive_scene);
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
                style: GpuSolidStyle::NONE,
            }],
        };
        let additive_filled =
            render_identity_readback(&mut renderer, &device, &queue, &additive_filled_scene);
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
                style: GpuSolidStyle::NONE,
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
                world_zoom: 1.0,
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
        let tiled_gpu = render_identity_readback(&mut renderer, &device, &queue, &tiled_scene);
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
        let resized =
            render_extent_readback(&mut renderer, &device, &queue, &scene, resized_extent);
        assert_eq!(
            resized.rgba,
            expected_frame(resized_extent, initial_mutable, &scene.gamma),
            "physical resize must preserve scene coordinates and content"
        );
        assert_eq!(renderer.last_stats().created_source_textures, 0);
        assert_eq!(renderer.last_stats().full_upload_calls, 0);
        assert_eq!(renderer.last_stats().full_upload_bytes, 0);
        assert_eq!(renderer.last_stats().dirty_upload_calls, 0);
        assert_eq!(renderer.last_stats().dirty_upload_bytes, 0);
        assert_eq!(renderer.last_stats().generic_vertices, 6);
        assert_eq!(renderer.last_stats().landscape_instances, 1);
        assert_eq!(
            renderer.last_stats().landscape_instance_upload_bytes,
            PACKED_LANDSCAPE_INSTANCE_STRIDE as usize
        );
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

        let dirty = render_extent_readback(&mut renderer, &device, &queue, &scene, resized_extent);
        assert_eq!(
            dirty.rgba,
            expected_frame(resized_extent, updated_mutable, &scene.gamma),
            "one dirty texel must update every use without a full upload"
        );
        assert_eq!(renderer.last_stats().created_source_textures, 0);
        assert_eq!(renderer.last_stats().full_upload_calls, 0);
        assert_eq!(renderer.last_stats().full_upload_bytes, 0);
        assert_eq!(renderer.last_stats().dirty_upload_calls, 1);
        assert_eq!(renderer.last_stats().dirty_upload_bytes, 4);
        assert!(!renderer.last_stats().composition_recreated);

        let validation = runtime.block_on(validation_scope.pop());
        assert!(
            validation.is_none(),
            "first device reported wgpu validation error: {validation:?}"
        );

        let (_replacement_adapter, replacement_device, replacement_queue) =
            request_test_device(&runtime, &instance, "lc_gpu_replacement_test_device", true)
                .expect("request a fresh adapter and replacement wgpu device");
        let replacement_validation_scope =
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
        let recreated = render_extent_readback(
            &mut renderer,
            &replacement_device,
            &replacement_queue,
            &scene,
            resized_extent,
        );
        assert_eq!(
            recreated.rgba,
            expected_frame(resized_extent, updated_mutable, &scene.gamma),
            "device recreation must regenerate every retained source from complete backing"
        );
        assert_eq!(renderer.last_stats().created_source_textures, 10);
        assert_eq!(renderer.last_stats().full_upload_calls, 10);
        assert_eq!(renderer.last_stats().full_upload_bytes, 46);
        assert_eq!(renderer.last_stats().dirty_upload_calls, 0);
        assert_eq!(renderer.last_stats().dirty_upload_bytes, 0);
        assert_eq!(renderer.last_stats().generic_vertices, 6);
        assert_eq!(renderer.last_stats().landscape_instances, 1);
        assert_eq!(
            renderer.last_stats().landscape_instance_upload_bytes,
            PACKED_LANDSCAPE_INSTANCE_STRIDE as usize
        );
        assert!(renderer.last_stats().composition_recreated);
        let validation = runtime.block_on(replacement_validation_scope.pop());
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
            style: GpuSolidStyle::NONE,
        });
        commands.push(GpuCommand::Solid {
            // Producers encode logical pixel centers.  Non-unit W proves the
            // point expansion preserves homogeneous coordinates.
            vertices: vec![solid_vertex_w(6.5, 4.5, 2.0, rgba_f32(POINT))],
            topology: GpuPrimitiveTopology::PointList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: None,
            blend: GpuBlend::Replace,
            style: GpuSolidStyle::NONE,
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

    fn r8_resource(id: GpuTextureId, pixel: u8) -> GpuTextureResource {
        GpuTextureResource {
            id,
            extent: [1, 1],
            revision: 0,
            base_revision: None,
            format: GpuTextureFormat::R8,
            pixels: Arc::from([pixel]),
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

    fn test_renderer(device: &wgpu::Device, queue: &wgpu::Queue) -> RetainedGpuRenderer {
        RetainedGpuRenderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm)
    }

    fn test_scene(
        logical_extent: [u32; 2],
        clear: Color,
        textures: Vec<GpuTextureResource>,
        commands: Vec<GpuCommand>,
    ) -> GpuScene {
        GpuScene {
            logical_extent,
            clear,
            gamma: GpuGammaLut::from_ramp(&GammaRamp::standard()),
            gamma_mode: GpuGammaMode::Disabled,
            textures,
            commands,
        }
    }

    fn render_identity_readback(
        renderer: &mut RetainedGpuRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &GpuScene,
    ) -> GpuReadbackFrame {
        render_extent_readback(renderer, device, queue, scene, scene.logical_extent)
    }

    fn render_extent_readback(
        renderer: &mut RetainedGpuRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &GpuScene,
        extent: [u32; 2],
    ) -> GpuReadbackFrame {
        render_readback(
            renderer,
            device,
            queue,
            scene,
            &GpuPresentation::identity(extent[0], extent[1]),
        )
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

    fn forced_generic_landscape_scene(scene: &GpuScene) -> GpuScene {
        let mut scene = scene.clone();
        for command in &mut scene.commands {
            let GpuCommand::Landscape { vertices, .. } = command else {
                continue;
            };
            for vertex in vertices {
                vertex.position = vertex.position.map(|coordinate| coordinate * 2.0);
            }
        }
        scene
    }

    #[allow(clippy::too_many_arguments)]
    fn assert_compact_landscape_matches_generic(
        renderer: &mut RetainedGpuRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        scene: &GpuScene,
        presentation: &GpuPresentation,
        expected_instances: usize,
        expected_draws: usize,
        label: &str,
    ) {
        let compact = render_readback(renderer, device, queue, scene, presentation);
        let compact_stats = renderer.last_stats();
        let generic_scene = forced_generic_landscape_scene(scene);
        let generic = render_readback(renderer, device, queue, &generic_scene, presentation);
        let generic_stats = renderer.last_stats();

        assert_eq!(compact, generic, "{label}: compact and generic pixels");
        assert_eq!(
            (
                compact_stats.landscape_instances,
                compact_stats.landscape_instance_upload_bytes,
                compact_stats.generic_vertices,
                compact_stats.generic_vertex_upload_bytes,
            ),
            (
                expected_instances,
                expected_instances * PACKED_LANDSCAPE_INSTANCE_STRIDE as usize,
                0,
                0,
            ),
            "{label}: compact stream",
        );
        assert_eq!(
            (
                generic_stats.landscape_instances,
                generic_stats.landscape_instance_upload_bytes,
                generic_stats.generic_vertices,
                generic_stats.generic_vertex_upload_bytes,
            ),
            (
                0,
                0,
                expected_instances * 6,
                expected_instances * 6 * PACKED_VERTEX_STRIDE as usize,
            ),
            "{label}: generic stream",
        );
        assert_eq!(
            (
                compact_stats.draw_calls,
                compact_stats.landscape_draw_calls,
                generic_stats.draw_calls,
                generic_stats.landscape_draw_calls,
            ),
            (
                expected_draws,
                expected_draws,
                expected_draws,
                expected_draws
            ),
            "{label}: landscape draw runs",
        );
    }

    #[test]
    fn reduced_presentation_readback_matches_the_cpu_box_reduction() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping reduced presentation readback test");
            return;
        };
        // Odd extents with a non-integer ratio on both axes: every destination
        // cell covers a different source span, so a reduction that assumed one
        // uniform box would disagree with the CPU oracle on most pixels.
        const SOURCE: [u32; 2] = [37, 23];
        const DEST: [u32; 2] = [7, 5];
        let mut renderer = test_renderer(&device, &queue);
        let scene = reduction_source_scene(GpuTextureId::fresh(), SOURCE);
        let full = render_extent_readback(&mut renderer, &device, &queue, &scene, SOURCE);
        let reduced = read_reduced_presentation(&mut renderer, &device, &queue, DEST);
        let expected = clonk_graphics::surface::downsample_rgba_box(
            &full.rgba, SOURCE[0], SOURCE[1], DEST[0], DEST[1],
        )
        .expect("CPU reduction of the presented frame");
        assert_eq!(reduced.extent, DEST);
        assert_eq!(
            reduced.rgba, expected,
            "GPU thumbnail reduction must match downsample_rgba_box byte for byte"
        );
    }

    #[test]
    fn reduced_presentation_readback_matches_the_cpu_box_reduction_at_edge_extents() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping reduced presentation edge extent test");
            return;
        };
        let mut renderer = test_renderer(&device, &queue);
        for (source, dest) in [
            // A single pixel, and a single-pixel axis on either side.
            ([1_u32, 1_u32], [1_u32, 1_u32]),
            ([1, 47], [1, 5]),
            ([47, 1], [5, 1]),
            // A source smaller than the thumbnail magnifies by repetition.
            ([64, 48], SAVE_THUMBNAIL_TEST_EXTENT),
            // An exact integer ratio, then one that divides on neither axis.
            ([400, 300], SAVE_THUMBNAIL_TEST_EXTENT),
            ([401, 301], SAVE_THUMBNAIL_TEST_EXTENT),
        ] {
            let scene = reduction_source_scene(GpuTextureId::fresh(), source);
            let full = render_extent_readback(&mut renderer, &device, &queue, &scene, source);
            let reduced = read_reduced_presentation(&mut renderer, &device, &queue, dest);
            let expected = clonk_graphics::surface::downsample_rgba_box(
                &full.rgba, source[0], source[1], dest[0], dest[1],
            )
            .expect("CPU reduction of the presented frame");
            assert_eq!(reduced.extent, dest, "{source:?} -> {dest:?}");
            assert_eq!(reduced.rgba, expected, "{source:?} -> {dest:?}");
        }
    }

    #[test]
    fn a_reduced_presentation_maps_only_the_thumbnail_and_its_row_padding() {
        let Some((_runtime, device, queue)) = shader_landscape_test_device() else {
            eprintln!("no wgpu adapter; skipping reduced presentation transfer size test");
            return;
        };
        const SOURCE: [u32; 2] = [1024, 768];
        let mut renderer = test_renderer(&device, &queue);
        let scene = reduction_source_scene(GpuTextureId::fresh(), SOURCE);
        let _ = render_extent_readback(&mut renderer, &device, &queue, &scene, SOURCE);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("lc_gpu_reduced_presentation_transfer_test_encoder"),
        });
        let reduced = renderer
            .readback_last_presentation_reduced(&device, &mut encoder, SAVE_THUMBNAIL_TEST_EXTENT)
            .expect("encode reduced retained GPU frame")
            .expect("reduced retained GPU frame exists");
        let full = renderer
            .readback_last_presentation(&device, &mut encoder)
            .expect("encode full retained GPU frame")
            .expect("full retained GPU frame exists");
        queue.submit(Some(encoder.finish()));

        let thumbnail_pixels =
            u64::from(SAVE_THUMBNAIL_TEST_EXTENT[0]) * u64::from(SAVE_THUMBNAIL_TEST_EXTENT[1]) * 4;
        let padding = u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * u64::from(SAVE_THUMBNAIL_TEST_EXTENT[1]);
        assert!(
            reduced.mapped_bytes() <= thumbnail_pixels + padding,
            "a thumbnail-only request maps {} bytes for {thumbnail_pixels} thumbnail bytes",
            reduced.mapped_bytes(),
        );
        assert_eq!(
            full.mapped_bytes(),
            u64::from(SOURCE[0]) * u64::from(SOURCE[1]) * 4,
            "the full readback this replaces transfers the complete frame",
        );
    }

    #[test]
    fn a_reduction_that_could_overflow_the_shader_accumulator_is_refused() {
        assert!(reduction_accumulator_fits([3840, 2160], [200, 150]));
        assert!(reduction_accumulator_fits([200, 150], [200, 150]));
        assert!(reduction_accumulator_fits([64, 48], [200, 150]));
        // The largest 2D source any device reports still reduces exactly.
        assert!(reduction_accumulator_fits([32768, 32768], [200, 150]));
        // One cell's premultiplied sum must stay inside the shader's u32.
        assert!(reduction_accumulator_fits(
            [MAX_REDUCTION_SAMPLES, 1],
            [1, 1]
        ));
        assert!(!reduction_accumulator_fits(
            [MAX_REDUCTION_SAMPLES + 1, 1],
            [1, 1]
        ));
        assert!(!reduction_accumulator_fits([0, 150], [200, 150]));
        assert!(!reduction_accumulator_fits([200, 150], [200, 0]));
    }

    /// The engine's save thumbnail size. `clonk-app` owns the constant; this
    /// crate only has to prove the reduction is exact at that extent.
    const SAVE_THUMBNAIL_TEST_EXTENT: [u32; 2] = [200, 150];

    /// A frame that spans the alpha and colour extremes the box reduction has
    /// to average, drawn 1:1 so the presented composition carries them intact.
    fn reduction_source_scene(id: GpuTextureId, extent: [u32; 2]) -> GpuScene {
        let pixels: Vec<u8> = (0..extent[0] as usize * extent[1] as usize)
            .flat_map(|index| {
                // Fully transparent texels must contribute nothing at all, and
                // opaque ones must not be dragged toward whatever they store.
                let alpha = [0_u8, 255, 17, 128, 254][index % 5];
                [
                    (index * 37 % 256) as u8,
                    (index * 91 % 256) as u8,
                    (index * 13 % 256) as u8,
                    alpha,
                ]
            })
            .collect();
        test_scene(
            extent,
            Color::new(0, 0, 0, 0),
            vec![GpuTextureResource {
                id,
                extent,
                revision: 0,
                base_revision: None,
                format: GpuTextureFormat::Rgba8,
                pixels: Arc::from(pixels.into_boxed_slice()),
                dirty: Vec::new(),
            }],
            vec![GpuCommand::Quad {
                texture: id,
                owner_mask: None,
                vertices: quad(
                    0.0,
                    0.0,
                    extent[0] as f32,
                    extent[1] as f32,
                    1.0,
                    [1.0, 1.0, 1.0, 0.0],
                ),
                clip: None,
                blend: GpuBlend::Replace,
                base_mod2: false,
                owner_mod2: false,
                sampler: GpuSampler::Nearest,
                gamma: false,
            }],
        )
    }

    fn read_reduced_presentation(
        renderer: &mut RetainedGpuRenderer,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        extent: [u32; 2],
    ) -> GpuReadbackFrame {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("lc_gpu_reduced_presentation_test_encoder"),
        });
        let ticket = renderer
            .readback_last_presentation_reduced(device, &mut encoder, extent)
            .expect("encode reduced retained GPU frame")
            .expect("reduced retained GPU frame exists");
        queue.submit(Some(encoder.finish()));
        ticket.read(device).expect("map reduced retained GPU frame")
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
