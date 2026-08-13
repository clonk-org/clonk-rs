// Blits the CPU frame buffer onto the drawable.
//
// The geometry is one oversized triangle covering the clip volume, which is
// cheaper than two triangles and has no seam down the diagonal. Its three
// corners are generated from the vertex index rather than read from a buffer;
// they are the same (-1,-1), (3,-1), (-1,3) that a vertex buffer would hold.
// See https://github.com/parasyte/pixels/issues/180.

struct VertexOutput {
    @location(0) tex_coord: vec2<f32>,
    @builtin(position) position: vec4<f32>,
}

struct Locals {
    transform: mat4x4<f32>,
    input_size: vec4<f32>,
}
@group(0) @binding(2) var<uniform> r_locals: Locals;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    let position = vec2<f32>(
        f32(i32(vertex_index) & 1) * 4.0 - 1.0,
        f32(i32(vertex_index) / 2) * 4.0 - 1.0,
    );

    var out: VertexOutput;
    out.tex_coord = fma(position, vec2<f32>(0.5, -0.5), vec2<f32>(0.5, 0.5));
    out.position = r_locals.transform * vec4<f32>(position, 0.0, 1.0);
    return out;
}

@group(0) @binding(0) var r_tex_color: texture_2d<f32>;
@group(0) @binding(1) var r_tex_sampler: sampler;

@fragment
fn fs_main(@location(0) tex_coord: vec2<f32>) -> @location(0) vec4<f32> {
    return textureSample(r_tex_color, r_tex_sampler, tex_coord);
}
