//! Blitting the CPU frame buffer onto the drawable: where it lands, and the
//! pipeline that puts it there.

use wgpu::util::DeviceExt;

/// Where the frame buffer lands on the drawable, and how much of it is covered.
///
/// `transform` is the column-major 4x4 the vertex shader applies to a
/// full-screen triangle; `clip_rect` is the scissor that keeps the blit inside
/// the letterboxed area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlitTransform {
    transform: [f32; 16],
    clip_rect: (u32, u32, u32, u32),
    buffer: (f32, f32),
}

impl BlitTransform {
    /// The whole-pixel-multiple fit that presentation has always used.
    ///
    /// The buffer is magnified by the largest integer factor that still fits
    /// the drawable, and is never minified: a buffer larger than the drawable
    /// keeps scale 1 and is clipped. Mirrors `pixels` 0.17.2
    /// `ScalingMatrix::new` under `ScalingMode::PixelPerfect`, which is the
    /// only mode the application ever selected.
    pub fn pixel_perfect(buffer: (u32, u32), drawable: (u32, u32)) -> Self {
        let (buffer_width, buffer_height) = (buffer.0.max(1) as f32, buffer.1.max(1) as f32);
        let (drawable_width, drawable_height) =
            (drawable.0.max(1) as f32, drawable.1.max(1) as f32);

        let scale = (drawable_width / buffer_width)
            .max(1.0)
            .min((drawable_height / buffer_height).max(1.0))
            .floor()
            .max(1.0);
        let (scaled_width, scaled_height) = (buffer_width * scale, buffer_height * scale);

        // A drawable with an odd extent has no whole-pixel centre, so the
        // half-pixel remainder is carried in the translation. Dropping it
        // shifts every presented frame by half a pixel.
        let sw = scaled_width / drawable_width;
        let sh = scaled_height / drawable_height;
        let tx = (drawable_width / 2.0).fract() / drawable_width;
        let ty = (drawable_height / 2.0).fract() / drawable_height;
        #[rustfmt::skip]
        let transform = [
            sw,  0.0, 0.0, 0.0,
            0.0, sh,  0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            tx,  ty,  0.0, 1.0,
        ];

        let clipped_width = scaled_width.min(drawable_width);
        let clipped_height = scaled_height.min(drawable_height);
        let clip_rect = (
            ((drawable_width - clipped_width) / 2.0) as u32,
            ((drawable_height - clipped_height) / 2.0) as u32,
            clipped_width as u32,
            clipped_height as u32,
        );

        Self {
            transform,
            clip_rect,
            buffer: (buffer_width, buffer_height),
        }
    }

    /// The scissor rectangle: the drawable's inner bounds, without the border.
    pub const fn clip_rect(&self) -> (u32, u32, u32, u32) {
        self.clip_rect
    }

    /// The shader's uniform block: the 4x4, then the buffer extent and its
    /// reciprocal.
    pub fn uniform_bytes(&self) -> [u8; UNIFORM_BYTES] {
        let mut uniform = [0_u8; UNIFORM_BYTES];
        let tail = [
            self.buffer.0,
            self.buffer.1,
            1.0 / self.buffer.0,
            1.0 / self.buffer.1,
        ];
        self.transform
            .iter()
            .chain(tail.iter())
            .zip(uniform.chunks_exact_mut(4))
            .for_each(|(value, slot)| slot.copy_from_slice(&value.to_le_bytes()));
        uniform
    }
}

/// A 4x4 of `f32` followed by the buffer extent and its reciprocal.
const UNIFORM_BYTES: usize = (16 + 4) * std::mem::size_of::<f32>();

/// The pipeline that puts the CPU frame buffer onto the drawable.
///
/// One full-screen triangle sampled with a nearest filter, scissored to the
/// blit's clip rectangle. Nearest is not a preference: magnifying the software
/// frame with any interpolation would blur every pixel the rasterizer placed.
#[derive(Debug)]
pub(crate) struct Blitter {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    clip_rect: (u32, u32, u32, u32),
}

impl Blitter {
    pub(crate) fn new(
        device: &wgpu::Device,
        texture: &wgpu::Texture,
        target_format: wgpu::TextureFormat,
        transform: BlitTransform,
    ) -> Self {
        let module = device.create_shader_module(wgpu::include_wgsl!("../shaders/blit.wgsl"));
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("clonk_surface_blit_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            lod_min_clamp: 0.0,
            lod_max_clamp: 1.0,
            compare: None,
            anisotropy_clamp: 1,
            border_color: None,
        });
        let uniform = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("clonk_surface_blit_uniform"),
            contents: &transform.uniform_bytes(),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("clonk_surface_blit_bind_group_layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("clonk_surface_blit_pipeline_layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("clonk_surface_blit_pipeline"),
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
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });

        let bind_group = bind(device, &bind_group_layout, texture, &sampler, &uniform);
        Self {
            pipeline,
            bind_group_layout,
            bind_group,
            sampler,
            uniform,
            clip_rect: transform.clip_rect(),
        }
    }

    /// Point the blit at a new frame-buffer texture after a buffer resize.
    pub(crate) fn rebind(&mut self, device: &wgpu::Device, texture: &wgpu::Texture) {
        self.bind_group = bind(
            device,
            &self.bind_group_layout,
            texture,
            &self.sampler,
            &self.uniform,
        );
    }

    /// Re-aim the blit after either extent changed.
    pub(crate) fn set_transform(&mut self, queue: &wgpu::Queue, transform: BlitTransform) {
        queue.write_buffer(&self.uniform, 0, &transform.uniform_bytes());
        self.clip_rect = transform.clip_rect();
    }

    pub(crate) fn blit(&self, encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("clonk_surface_blit_pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_scissor_rect(
            self.clip_rect.0,
            self.clip_rect.1,
            self.clip_rect.2,
            self.clip_rect.3,
        );
        pass.draw(0..3, 0..1);
    }
}

fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    texture: &wgpu::Texture,
    sampler: &wgpu::Sampler,
    uniform: &wgpu::Buffer,
) -> wgpu::BindGroup {
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("clonk_surface_blit_bind_group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: uniform.as_entire_binding(),
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every consumer sizes its frame buffer to the drawable's own physical
    // extent, so this is the case that actually ships: scale 1, no letterbox,
    // and a transform that must not nudge the image off the pixel grid.
    #[test]
    fn a_buffer_matching_an_even_drawable_blits_one_to_one() {
        let transform = BlitTransform::pixel_perfect((960, 640), (960, 640));

        assert_eq!(
            transform.transform,
            [
                1.0, 0.0, 0.0, 0.0, //
                0.0, 1.0, 0.0, 0.0, //
                0.0, 0.0, 1.0, 0.0, //
                0.0, 0.0, 0.0, 1.0,
            ]
        );
        assert_eq!(transform.clip_rect(), (0, 0, 960, 640));
    }

    // An odd extent has no whole-pixel centre. The remainder rides in the
    // translation column, and a port that drops it moves every presented frame
    // half a pixel — which no snapshot at an even extent would ever show.
    #[test]
    fn an_odd_drawable_carries_its_half_pixel_remainder_in_the_translation() {
        let transform = BlitTransform::pixel_perfect((961, 641), (961, 641));

        assert_eq!(transform.transform[12], 0.5 / 961.0);
        assert_eq!(transform.transform[13], 0.5 / 641.0);
        assert_eq!(transform.clip_rect(), (0, 0, 961, 641));
    }

    // Magnification is whole-pixel and takes the smaller axis, so a buffer that
    // would fit three times across but only twice down is magnified twice and
    // letterboxed on both axes.
    #[test]
    fn a_smaller_buffer_is_magnified_by_whole_pixels_and_letterboxed() {
        let transform = BlitTransform::pixel_perfect((320, 240), (960, 640));

        assert_eq!(transform.transform[0], 640.0 / 960.0);
        assert_eq!(transform.transform[5], 480.0 / 640.0);
        assert_eq!(transform.clip_rect(), (160, 80, 640, 480));
    }

    // A buffer larger than the drawable is never minified: it stays at scale 1
    // and the scissor clamps to the drawable rather than growing past it.
    #[test]
    fn a_buffer_larger_than_the_drawable_is_clipped_rather_than_shrunk() {
        let transform = BlitTransform::pixel_perfect((1920, 1080), (960, 640));

        assert_eq!(transform.transform[0], 2.0);
        assert_eq!(transform.transform[5], 1080.0 / 640.0);
        assert_eq!(transform.clip_rect(), (0, 0, 960, 640));
    }

    // The shader reads one uniform block: the 4x4 followed by the buffer extent
    // and its reciprocal. Getting the tail wrong samples the wrong texels
    // without failing anything else, so the layout is pinned byte for byte.
    #[test]
    fn the_uniform_block_carries_the_matrix_then_the_buffer_extent_and_its_reciprocal() {
        let transform = BlitTransform::pixel_perfect((320, 240), (960, 640));

        let uniform = transform.uniform_bytes();

        assert_eq!(uniform.len(), 80, "a 4x4 of f32 followed by four more");
        assert_eq!(uniform[0..4], (640.0_f32 / 960.0).to_le_bytes());
        assert_eq!(uniform[64..68], 320.0_f32.to_le_bytes());
        assert_eq!(uniform[68..72], 240.0_f32.to_le_bytes());
        assert_eq!(uniform[72..76], (1.0_f32 / 320.0).to_le_bytes());
        assert_eq!(uniform[76..80], (1.0_f32 / 240.0).to_le_bytes());
    }
}
