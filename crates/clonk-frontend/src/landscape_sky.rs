use super::*;

pub(crate) struct LandscapeRenderCache {
    pub(crate) grid: PixelGrid,
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) shade_materials: bool,
    pub(crate) border_state: (i32, i32, bool, bool, Option<u8>),
    pub(crate) pixels: Arc<[u8]>,
    pub(crate) liquid_mask: Arc<[u8]>,
    gpu_texture_id: GpuTextureId,
    gpu_liquid_mask_id: GpuTextureId,
    gpu_revision: u64,
    gpu_published_revision: u64,
    pub(crate) gpu_dirty: Vec<SurfaceRect>,
    pub(crate) composition_scratch: Vec<u8>,
}

impl LandscapeRenderCache {
    pub(crate) fn new(
        grid: PixelGrid,
        width: u32,
        height: u32,
        shade_materials: bool,
        border_state: (i32, i32, bool, bool, Option<u8>),
    ) -> Self {
        let pixel_count = width as usize * height as usize;
        Self {
            grid,
            width,
            height,
            shade_materials,
            border_state,
            pixels: Arc::from(vec![0; pixel_count.saturating_mul(4)].into_boxed_slice()),
            liquid_mask: Arc::from(vec![0; pixel_count].into_boxed_slice()),
            gpu_texture_id: GpuTextureId::fresh(),
            gpu_liquid_mask_id: GpuTextureId::fresh(),
            gpu_revision: 0,
            gpu_published_revision: 0,
            gpu_dirty: Vec::new(),
            composition_scratch: Vec::new(),
        }
    }

    pub(crate) fn record_gpu_update(&mut self, regions: &[(u32, u32, u32, u32)]) {
        if regions.is_empty() {
            return;
        }
        self.gpu_revision = self.gpu_revision.wrapping_add(1);
        self.gpu_dirty
            .extend(regions.iter().filter_map(|&(x, y, width, height)| {
                (width != 0 && height != 0)
                    .then_some(SurfaceRect::new(x as i32, y as i32, width, height))
            }));
        if self.gpu_dirty.len() > 128 {
            self.gpu_dirty.clear();
            self.gpu_dirty
                .push(SurfaceRect::new(0, 0, self.width, self.height));
        }
    }

    fn take_gpu_resources(&mut self) -> (GpuTextureResource, GpuTextureResource) {
        let dirty = std::mem::take(&mut self.gpu_dirty);
        let base_revision = (!dirty.is_empty()).then_some(self.gpu_published_revision);
        self.gpu_published_revision = self.gpu_revision;
        (
            GpuTextureResource {
                id: self.gpu_texture_id,
                extent: [self.width, self.height],
                revision: self.gpu_revision,
                base_revision,
                format: clonk_graphics::GpuTextureFormat::Rgba8,
                pixels: Arc::clone(&self.pixels),
                dirty: dirty.clone(),
            },
            GpuTextureResource {
                id: self.gpu_liquid_mask_id,
                extent: [self.width, self.height],
                revision: self.gpu_revision,
                base_revision,
                format: clonk_graphics::GpuTextureFormat::R8,
                pixels: Arc::clone(&self.liquid_mask),
                dirty,
            },
        )
    }
}

fn gpu_landscape_modulation(raw: u32) -> [f32; 4] {
    split_c4_color(raw).map(|channel| f32::from(channel) / 255.0)
}

pub(crate) fn gpu_rgba(color: Color) -> [f32; 4] {
    [
        f32::from(color.r) / 255.0,
        f32::from(color.g) / 255.0,
        f32::from(color.b) / 255.0,
        f32::from(color.a) / 255.0,
    ]
}

pub(crate) fn record_gpu_solid_quad(
    surface: &mut Surface,
    bounds: (f32, f32, f32, f32),
    colors: [Color; 4],
    blend: GpuBlend,
    style: GpuSolidStyle,
) {
    let (left, top, right, bottom) = bounds;
    let vertex = |x, y, color| GpuSolidVertex {
        position: [x, y, 1.0],
        color: gpu_rgba(color),
        outer_modulation: GpuSolidOuterModulation::PackedC4,
    };
    let corners = [
        vertex(left, top, colors[0]),
        vertex(right, top, colors[1]),
        vertex(left, bottom, colors[2]),
        vertex(right, bottom, colors[3]),
    ];
    surface.push_gpu_command(GpuCommand::Solid {
        vertices: vec![
            corners[0], corners[1], corners[2], corners[2], corners[1], corners[3],
        ],
        topology: GpuPrimitiveTopology::TriangleList,
        alpha_mode: GpuSolidAlphaMode::SourceOver,
        clip: surface.clip(),
        blend,
        style,
    });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_gpu_landscape(
    surface: &mut Surface,
    cache: &mut LandscapeRenderCache,
    surface_width: u32,
    surface_height: u32,
    viewport_x: f32,
    viewport_y: f32,
    zoom: f32,
    blit: SpriteBlitState,
    fog: Option<&FogDrawContext>,
    fog_sampler: Option<&FogSpriteSampler>,
    liquid_animation: Option<(&ImageData, [f32; 3])>,
    gamma: bool,
) -> bool {
    if !surface.is_gpu_scene_capture_active()
        || (fog.is_some() && fog_sampler.is_none())
        || surface_width == 0
        || surface_height == 0
        || !zoom.is_finite()
        || zoom <= 0.0
    {
        return false;
    }

    let base_id = cache.gpu_texture_id;
    let liquid_mask_id = cache.gpu_liquid_mask_id;
    let (base_resource, liquid_mask_resource) = cache.take_gpu_resources();
    surface.add_gpu_texture(base_resource);
    let (liquid_mask, liquid, phase) = if let Some((image, phase)) = liquid_animation {
        surface.add_gpu_texture(liquid_mask_resource);
        surface.add_gpu_texture(image.gpu_texture_resource());
        (Some(liquid_mask_id), Some(image.gpu_texture_id()), phase)
    } else {
        (None, None, [0.0; 3])
    };

    let clip = surface.clip();
    let base_modulation = blit.modulation.unwrap_or(0x00ff_ffff);
    let source_width = surface_width as f32 / zoom;
    let source_height = surface_height as f32 / zoom;
    let offset = blit.renderer_config.destination_offset();
    let indent = blit.renderer_config.texture_indent();
    let base_texture_size = cpp_tex_size(cache.width, cache.height) as f32;
    let mut emit = |x: (f32, f32), y: (f32, f32), fog_modulation: Option<[u32; 4]>| {
        let outer_modulation = if blit.modulation.is_some() || fog_modulation.is_some() {
            GpuOuterModulation::Combine
        } else {
            GpuOuterModulation::Inherit
        };
        let mut modulation = fog_modulation
            .map(|fog| fog.map(|value| modulate_c4_colors(base_modulation, value)))
            .unwrap_or([base_modulation; 4]);
        if !blit.renderer_config.shader && blit.renderer_config.no_alpha_add {
            for color in &mut modulation {
                *color &= 0x00ff_ffff;
            }
        }
        let modulation = modulation.map(gpu_landscape_modulation);
        let screen_left = offset + x.0 / source_width * surface_width as f32;
        let screen_right = offset + x.1 / source_width * surface_width as f32;
        let screen_top = offset + y.0 / source_height * surface_height as f32;
        let screen_bottom = offset + y.1 / source_height * surface_height as f32;
        let world_center_x = viewport_x + (x.0 + x.1) / 2.0;
        let world_center_y = viewport_y + (y.0 + y.1) / 2.0;
        let physical_tile = if indent != 0.0 {
            let Some(tile) = cpp_texture_tile_for_source(
                cache.width,
                cache.height,
                world_center_x,
                world_center_y,
                fog_sampler.is_some(),
            ) else {
                return;
            };
            Some(tile)
        } else {
            None
        };
        let adjust = |edge: f32, tile_origin: i32, physical_size: i32| {
            (edge + indent).clamp(
                tile_origin as f32,
                tile_origin.saturating_add(physical_size) as f32,
            )
        };
        let (world_left, world_right, world_top, world_bottom) = physical_tile.map_or(
            (
                viewport_x + x.0,
                viewport_x + x.1,
                viewport_y + y.0,
                viewport_y + y.1,
            ),
            |(tile_x, tile_y, physical_size)| {
                (
                    adjust(viewport_x + x.0, tile_x, physical_size),
                    adjust(viewport_x + x.1, tile_x, physical_size),
                    adjust(viewport_y + y.0, tile_y, physical_size),
                    adjust(viewport_y + y.1, tile_y, physical_size),
                )
            },
        );
        let positions = [
            [screen_left, screen_top, 1.0],
            [screen_right, screen_top, 1.0],
            [screen_left, screen_bottom, 1.0],
            [screen_right, screen_bottom, 1.0],
        ];
        let uv = [
            [
                world_left / cache.width as f32,
                world_top / cache.height as f32,
            ],
            [
                world_right / cache.width as f32,
                world_top / cache.height as f32,
            ],
            [
                world_left / cache.width as f32,
                world_bottom / cache.height as f32,
            ],
            [
                world_right / cache.width as f32,
                world_bottom / cache.height as f32,
            ],
        ];
        let command = |indices: [usize; 4], modulation: [[f32; 4]; 4]| GpuCommand::Landscape {
            base: base_id,
            liquid_mask,
            liquid,
            vertices: std::array::from_fn(|slot| {
                let index = indices[slot];
                GpuVertex::new(positions[index], uv[index], modulation[slot])
                    .with_outer_modulation(outer_modulation)
            }),
            clip,
            phase,
            gamma,
        };
        if blit.renderer_config.no_box_fades && fog_modulation.is_some() {
            surface.push_gpu_command(command([0, 1, 2, 2], [modulation[2]; 4]));
            surface.push_gpu_command(command([2, 1, 3, 3], [modulation[3]; 4]));
        } else {
            surface.push_gpu_command(command([0, 1, 2, 3], modulation));
        }
    };

    if let Some(sampler) = fog_sampler {
        for quad in &sampler.quads {
            emit(quad.x, quad.y, Some(quad.modulation));
        }
    } else if indent != 0.0 {
        let x_ranges =
            FogSpriteSampler::axis_ranges(viewport_x, source_width, base_texture_size, false);
        let y_ranges =
            FogSpriteSampler::axis_ranges(viewport_y, source_height, base_texture_size, false);
        if x_ranges
            .len()
            .checked_mul(y_ranges.len())
            .is_none_or(|chunks| chunks > 1_000_000)
        {
            return false;
        }
        for y in y_ranges {
            for &x in &x_ranges {
                emit(x, y, None);
            }
        }
    } else {
        emit((0.0, source_width), (0.0, source_height), None);
    }
    true
}

// Below this size Rayon scheduling costs more than the independent landscape
// row work it can distribute. Live viewports are substantially larger, while
// tiny loader/test surfaces stay on the same scalar row implementation.
pub(crate) const PARALLEL_LANDSCAPE_MIN_PIXELS: usize = 128 * 128;

/// Horizontal landscape coordinates precomputed once per frame. The normal
/// zero-indent renderer reuses integer source/liquid positions in every row;
/// nonzero TexIndent retains the raw coordinate for physical-tile sampling.
#[derive(Clone, Copy)]
pub(crate) struct LandscapeXSample {
    raw: f32,
    world: i32,
    liquid: i32,
}

impl LandscapeXSample {
    pub(crate) fn new(raw: f32, texture_size: i32) -> Self {
        let world = if raw.is_finite() {
            raw.floor() as i32
        } else {
            -1
        };
        Self {
            raw,
            world,
            liquid: world.rem_euclid(texture_size),
        }
    }

    #[inline]
    pub(crate) fn zero_indent_texel(
        self,
        world_width: i32,
        world_y: i32,
        liquid_y: i32,
    ) -> Option<(i32, i32, i32, i32)> {
        (self.world >= 0 && self.world < world_width).then_some((
            self.world,
            world_y,
            self.liquid,
            liquid_y,
        ))
    }
}

/// Immutable inputs for one visible landscape blit. Rows own disjoint RGBA
/// destination slices; all remaining state is read-only and therefore safe to
/// share through Rayon's persistent worker pool.
pub(crate) struct LandscapeRowRenderContext<'a> {
    pub(crate) grid: &'a PixelGrid,
    pub(crate) cache_pixels: &'a [u8],
    pub(crate) cache_width: i32,
    pub(crate) cache_height: i32,
    pub(crate) screen_width: u32,
    pub(crate) screen_height: u32,
    pub(crate) viewport_y: f32,
    pub(crate) zoom: f32,
    pub(crate) texture_size: i32,
    pub(crate) x_samples: &'a [LandscapeXSample],
    pub(crate) blit: SpriteBlitState,
    pub(crate) fog: Option<&'a FogDrawContext>,
    pub(crate) fog_sampler: Option<&'a FogSpriteSampler>,
    pub(crate) fog_axes: Option<(&'a [FogAxisSample], &'a [FogAxisSample])>,
    pub(crate) liquid_animation: Option<(&'a ImageData, [f32; 3])>,
    pub(crate) gamma: Option<&'a clonk_graphics::GammaRamp>,
    pub(crate) clip: Option<SurfaceRect>,
    #[cfg(test)]
    pub(crate) destination_samples: Arc<AtomicUsize>,
}

impl LandscapeRowRenderContext<'_> {
    fn visible_x_range(&self, screen_y: u32) -> Option<(usize, usize)> {
        let Some(clip) = self.clip else {
            return (self.screen_width != 0).then_some((0, self.screen_width as usize));
        };
        let y = i64::from(screen_y);
        let top = i64::from(clip.y);
        let bottom = top + i64::from(clip.height);
        if y < top || y >= bottom {
            return None;
        }
        let surface_right = i64::from(self.screen_width);
        let left = i64::from(clip.x).clamp(0, surface_right);
        let right = (i64::from(clip.x) + i64::from(clip.width)).clamp(0, surface_right);
        (left < right).then_some((left as usize, right as usize))
    }
}

/// Draws one landscape row. Both scalar and parallel dispatch use this exact
/// function so fog interpolation, liquid phase math, gamma, clipping and
/// alpha compositing cannot drift between paths.
fn draw_ground_textured_row(
    context: &LandscapeRowRenderContext<'_>,
    screen_y: u32,
    row: &mut [u8],
) {
    let row_bytes = context.screen_width as usize * 4;
    if row.len() < row_bytes {
        return;
    }
    let destination_y = screen_y as f32 + 0.5 - context.blit.renderer_config.destination_offset();
    if destination_y < 0.0 || destination_y >= context.screen_height as f32 {
        return;
    }
    let raw_world_y = context.viewport_y + destination_y / context.zoom;
    if raw_world_y < 0.0 || raw_world_y >= context.cache_height as f32 {
        return;
    }
    let Some((start_x, end_x)) = context.visible_x_range(screen_y) else {
        return;
    };
    let indent = context.blit.renderer_config.texture_indent();
    let zero_indent_y = (indent == 0.0).then(|| {
        let world_y = raw_world_y.floor() as i32;
        (world_y, world_y.rem_euclid(context.texture_size))
    });

    for screen_x in start_x..end_x {
        let x_sample = context.x_samples[screen_x];
        let source_texel = match zero_indent_y {
            Some((world_y, liquid_y)) => {
                x_sample.zero_indent_texel(context.cache_width, world_y, liquid_y)
            }
            None => cpp_landscape_source_texel(
                context.cache_width as u32,
                context.cache_height as u32,
                x_sample.raw,
                raw_world_y,
                indent,
            ),
        };
        let Some((world_x, world_y, liquid_x, liquid_y)) = source_texel else {
            continue;
        };
        let source_offset =
            (world_y as usize * context.cache_width as usize + world_x as usize) * 4;
        if context.cache_pixels[source_offset + 3] == 0 {
            continue;
        }
        let color = Color::new(
            context.cache_pixels[source_offset],
            context.cache_pixels[source_offset + 1],
            context.cache_pixels[source_offset + 2],
            context.cache_pixels[source_offset + 3],
        );
        let pixel_blit = match (context.fog_sampler, context.fog_axes) {
            (Some(sampler), Some((x_samples, y_samples))) => sampler.blit_at_axes(
                context.blit,
                x_samples[screen_x],
                y_samples[screen_y as usize],
            ),
            _ => fog_sprite_blit_at(
                None,
                context.fog,
                context.blit,
                (screen_x as f32 + 0.5) / context.screen_width as f32,
                (screen_y as f32 + 0.5) / context.screen_height as f32,
                screen_x as i32,
                screen_y as i32,
            ),
        };
        let source = context
            .liquid_animation
            .filter(|_| {
                context
                    .grid
                    .density_at(world_x, world_y)
                    .is_some_and(|density| (25..50).contains(&density))
            })
            .map_or_else(
                || prepare_sprite_fragment(color, None, None, pixel_blit),
                |(image, modulation)| {
                    let delta =
                        LiquidAnimationCycle::delta_at(image, liquid_x, liquid_y, modulation);
                    prepare_liquid_animation_fragment(color, delta, pixel_blit)
                },
            );
        if source.alpha() == 0.0 {
            continue;
        }

        let destination_offset = screen_x * 4;
        let destination = if source.alpha() == 255.0 {
            Color::transparent()
        } else {
            #[cfg(test)]
            if context.gamma.is_some() {
                context.destination_samples.fetch_add(1, Ordering::Relaxed);
            }
            Color::new(
                row[destination_offset],
                row[destination_offset + 1],
                row[destination_offset + 2],
                row[destination_offset + 3],
            )
        };
        let output = composite_sprite_fragment(source, destination, pixel_blit, context.gamma);
        row[destination_offset..destination_offset + 4]
            .copy_from_slice(&[output.r, output.g, output.b, output.a]);
    }
}

pub(crate) fn draw_ground_textured_rows(
    context: &LandscapeRowRenderContext<'_>,
    pixels: &mut [u8],
    parallel: bool,
) {
    let row_bytes = context.screen_width as usize * 4;
    if row_bytes == 0 || context.screen_height == 0 {
        return;
    }
    let row_count = (pixels.len() / row_bytes).min(context.screen_height as usize);
    let rows = &mut pixels[..row_count * row_bytes];
    if parallel && row_count > 1 {
        rows.par_chunks_mut(row_bytes)
            .enumerate()
            .for_each(|(screen_y, row)| draw_ground_textured_row(context, screen_y as u32, row));
    } else {
        for (screen_y, row) in rows.chunks_mut(row_bytes).enumerate() {
            draw_ground_textured_row(context, screen_y as u32, row);
        }
    }
}

pub(crate) const PARALLEL_SKY_MIN_PIXELS: usize = 128 * 128;

#[derive(Clone, Copy)]
pub(crate) struct SkyTileBounds {
    dest_x: i32,
    pub(crate) dest_y: i32,
    pub(crate) source_left: i32,
    pub(crate) source_top: i32,
    source_right: i32,
    pub(crate) source_bottom: i32,
}

impl SkyTileBounds {
    pub(crate) fn visible(
        surface_width: u32,
        surface_height: u32,
        image_width: u32,
        image_height: u32,
        dest_x: i32,
        dest_y: i32,
    ) -> Option<Self> {
        let width = i32::try_from(image_width).unwrap_or(i32::MAX);
        let height = i32::try_from(image_height).unwrap_or(i32::MAX);
        let source_left = (-dest_x).clamp(0, width);
        let source_top = (-dest_y).clamp(0, height);
        let source_right = (surface_width as i32 - dest_x).clamp(0, width);
        let source_bottom = (surface_height as i32 - dest_y).clamp(0, height);
        (source_left < source_right && source_top < source_bottom).then_some(Self {
            dest_x,
            dest_y,
            source_left,
            source_top,
            source_right,
            source_bottom,
        })
    }

    pub(crate) fn width(self) -> i32 {
        self.source_right - self.source_left
    }

    pub(crate) fn height(self) -> i32 {
        self.source_bottom - self.source_top
    }

    pub(crate) fn pixel_count(self) -> usize {
        self.width() as usize * self.height() as usize
    }

    pub(crate) fn target_left(self) -> i32 {
        self.dest_x + self.source_left
    }

    pub(crate) fn target_top(self) -> i32 {
        self.dest_y + self.source_top
    }
}

pub(crate) struct SkyTileRegion {
    pub(crate) bounds: SkyTileBounds,
    fog_sampler: Option<FogSpriteSampler>,
    fog_axes: Option<(Vec<FogAxisSample>, Vec<FogAxisSample>)>,
}

impl SkyTileRegion {
    pub(crate) fn new(
        bounds: SkyTileBounds,
        fog: Option<&FogDrawContext>,
        image_width: u32,
        image_height: u32,
    ) -> Self {
        let fog_sampler = fog.and_then(|fog| {
            FogSpriteSampler::new(
                fog,
                (
                    bounds.target_left() as f32,
                    bounds.target_top() as f32,
                    bounds.width() as f32,
                    bounds.height() as f32,
                ),
                (
                    bounds.source_left as f32,
                    bounds.source_top as f32,
                    bounds.width() as f32,
                    bounds.height() as f32,
                ),
                (image_width, image_height),
                false,
                |x, y| (x, y),
            )
        });
        let fog_axes = fog_sampler
            .as_ref()
            .map(|sampler| sampler.raster_axes(bounds.width() as u32, bounds.height() as u32));
        Self {
            bounds,
            fog_sampler,
            fog_axes,
        }
    }
}

pub(crate) struct SkyTileRowRenderContext<'a> {
    pub(crate) lit_texels: &'a [Color],
    pub(crate) image_width: usize,
    pub(crate) surface_width: u32,
    pub(crate) regions: &'a [SkyTileRegion],
    pub(crate) region_indices_by_row: &'a [Vec<usize>],
    pub(crate) base_blit: SpriteBlitState,
    pub(crate) uses_blit_modulation: bool,
    pub(crate) fog: Option<&'a FogDrawContext>,
    pub(crate) gamma: Option<&'a clonk_graphics::GammaRamp>,
    pub(crate) clip: Option<SurfaceRect>,
}

impl SkyTileRowRenderContext<'_> {
    fn visible_x_range(&self, screen_y: u32) -> Option<(usize, usize)> {
        let Some(clip) = self.clip else {
            return (self.surface_width != 0).then_some((0, self.surface_width as usize));
        };
        let y = i64::from(screen_y);
        let top = i64::from(clip.y);
        let bottom = top + i64::from(clip.height);
        if y < top || y >= bottom {
            return None;
        }
        let surface_right = i64::from(self.surface_width);
        let left = i64::from(clip.x).clamp(0, surface_right);
        let right = (i64::from(clip.x) + i64::from(clip.width)).clamp(0, surface_right);
        (left < right).then_some((left as usize, right as usize))
    }
}

pub(crate) fn lit_sky_texels(image: &ImageData, lighting: f32) -> Vec<Color> {
    let expected = (image.width() as usize)
        .checked_mul(image.height() as usize)
        .unwrap_or(0);
    image
        .pixels()
        .chunks_exact(4)
        .take(expected)
        .map(|pixel| Color::new(pixel[0], pixel[1], pixel[2], pixel[3]).modulate(lighting))
        .collect()
}

pub(crate) struct RetainedLitSkyTexture {
    pub(crate) source: GpuTextureId,
    pub(crate) lighting: u32,
    pub(crate) image: ImageData,
    pub(crate) texture: GpuTextureId,
    pub(crate) revision: u64,
}

fn draw_sky_tile_row(context: &SkyTileRowRenderContext<'_>, screen_y: u32, row: &mut [u8]) {
    let row_bytes = context.surface_width as usize * 4;
    if row.len() < row_bytes {
        return;
    }
    let Some((clip_left, clip_right)) = context.visible_x_range(screen_y) else {
        return;
    };
    let Some(region_indices) = context.region_indices_by_row.get(screen_y as usize) else {
        return;
    };
    for &region_index in region_indices {
        let Some(region) = context.regions.get(region_index) else {
            continue;
        };
        let bounds = region.bounds;
        let source_y = screen_y as i32 - bounds.dest_y;
        if source_y < bounds.source_top || source_y >= bounds.source_bottom {
            continue;
        }
        let target_left = bounds.target_left() as usize;
        let target_right = (bounds.dest_x + bounds.source_right) as usize;
        let draw_left = target_left.max(clip_left);
        let draw_right = target_right.min(clip_right);
        for target_x in draw_left..draw_right {
            let source_x = target_x as i32 - bounds.dest_x;
            let source_index = source_y as usize * context.image_width + source_x as usize;
            let Some(&color) = context.lit_texels.get(source_index) else {
                continue;
            };
            if color.a == 0 {
                continue;
            }
            let pixel_blit = if context.uses_blit_modulation {
                match (region.fog_sampler.as_ref(), region.fog_axes.as_ref()) {
                    (Some(sampler), Some((x_samples, y_samples))) => sampler.blit_at_axes(
                        context.base_blit,
                        x_samples[(source_x - bounds.source_left) as usize],
                        y_samples[(source_y - bounds.source_top) as usize],
                    ),
                    _ => fog_sprite_blit_at(
                        None,
                        context.fog,
                        context.base_blit,
                        (source_x as f32 + 0.5 - bounds.source_left as f32) / bounds.width() as f32,
                        (source_y as f32 + 0.5 - bounds.source_top as f32) / bounds.height() as f32,
                        target_x as i32,
                        screen_y as i32,
                    ),
                }
            } else {
                context.base_blit
            };
            let source = prepare_sprite_fragment(color, None, None, pixel_blit);
            if source.alpha() == 0.0 {
                continue;
            }
            let destination_offset = target_x * 4;
            let destination = if source.alpha() == 255.0 {
                Color::transparent()
            } else {
                Color::new(
                    row[destination_offset],
                    row[destination_offset + 1],
                    row[destination_offset + 2],
                    row[destination_offset + 3],
                )
            };
            let output = composite_sprite_fragment(source, destination, pixel_blit, context.gamma);
            row[destination_offset..destination_offset + 4]
                .copy_from_slice(&[output.r, output.g, output.b, output.a]);
        }
    }
}

pub(crate) fn draw_sky_tile_rows(
    context: &SkyTileRowRenderContext<'_>,
    pixels: &mut [u8],
    surface_height: u32,
    parallel: bool,
) {
    let row_bytes = context.surface_width as usize * 4;
    if row_bytes == 0 || surface_height == 0 {
        return;
    }
    let row_count = (pixels.len() / row_bytes).min(surface_height as usize);
    let rows = &mut pixels[..row_count * row_bytes];
    if parallel && row_count > 1 {
        rows.par_chunks_mut(row_bytes)
            .enumerate()
            .for_each(|(screen_y, row)| draw_sky_tile_row(context, screen_y as u32, row));
    } else {
        for (screen_y, row) in rows.chunks_mut(row_bytes).enumerate() {
            draw_sky_tile_row(context, screen_y as u32, row);
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct LiquidAnimationCycle {
    pub(crate) values: [f32; 3],
}

impl Default for LiquidAnimationCycle {
    fn default() -> Self {
        Self {
            values: [-0.2, 0.0, 0.2],
        }
    }
}

impl LiquidAnimationCycle {
    /// Advances the process-presentation cycle once per landscape blit, as
    /// StdGL::BlitLandscape does independently of the synchronized game tick.
    pub(crate) fn advance(&mut self) -> [f32; 3] {
        for value in &mut self.values {
            *value += 0.05;
            if *value > 0.9 {
                *value = -0.3;
            }
        }
        self.values
            .map(|value| (if value > 0.3 { 0.6 - value } else { value }) / 3.0)
    }

    fn delta_at(image: &ImageData, x: i32, y: i32, modulation: [f32; 3]) -> f32 {
        let width = image.width();
        let height = image.height();
        if width == 0 || height == 0 {
            return 0.0;
        }
        let sample_x = x.rem_euclid(width as i32) as u32;
        let sample_y = y.rem_euclid(height as i32) as u32;
        let offset = ((sample_y * width + sample_x) * 4) as usize;
        let pixels = image.pixels();
        if offset + 3 > pixels.len() {
            return 0.0;
        }
        (0..3)
            .map(|channel| {
                (f32::from(pixels[offset + channel]) / 255.0 - 0.5) * modulation[channel]
            })
            .sum()
    }
}

/// Precomputed `ApplyLighting` placement shading, two bytes per landscape map
/// pixel: the lighten amount then the total darken amount.
///
/// The +-8-row loop in `C4Landscape::ApplyLighting`
/// (src/C4Landscape.cpp:2816-2872) is a bit-exact C++ mirror, so a fragment
/// composer must consume it rather than re-derive it. Two channels are
/// required, not one: `LightenClrBy` saturates at 255 before `DarkenClrBy`
/// runs, and a channel that clamped high cannot be recovered from a single
/// signed amount. `SHADING_PLANE_SUPPRESSED` in the darken channel encodes the
/// `if (!iOwnDens) continue;` case, which leaves the pixel fully transparent.
pub(crate) const SHADING_PLANE_SUPPRESSED: u8 = 255;

pub(crate) fn placement_shading_plane(
    bytes: &[u8],
    width: u32,
    height: u32,
    placements: &[i32; 128],
    border_state: (i32, i32, bool, bool, Option<u8>),
) -> Vec<u8> {
    // Same border rules as the retained CPU composer's local `byte_with_border`
    // (C4Landscape::GetPix/GetPlacement).
    let byte_with_border = |x: i32, y: i32| {
        let (left_open, right_open, top_open, bottom_open, vehicle) = border_state;
        let border = |is_open: bool| is_open.then_some(0).or(vehicle);
        if x < 0 {
            return border(y < left_open);
        }
        if x as u32 >= width {
            return border(y < right_open);
        }
        if y < 0 {
            return border(top_open);
        }
        if y as u32 >= height {
            return border(bottom_open);
        }
        bytes.get(y as usize * width as usize + x as usize).copied()
    };
    let placement_at = |x: i32, y: i32| {
        byte_with_border(x, y).map_or(0, |byte| placements[usize::from(byte & 0x7f)])
    };
    let mut plane = vec![0_u8; (width as usize * height as usize).saturating_mul(2)];
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let offset = (y as usize * width as usize + x as usize) * 2;
            let Some(byte) = byte_with_border(x, y).filter(|byte| *byte != 0) else {
                continue;
            };
            let own_density = placements[usize::from(byte & 0x7f)];
            if own_density == 0 {
                plane[offset + 1] = SHADING_PLANE_SUPPRESSED;
                continue;
            }
            let own_density =
                (2 * own_density + placement_at(x - 1, y) + placement_at(x + 1, y)) / 4;
            let window = |step: i32| {
                (1..=8)
                    .map(|offset| placement_at(x, y + step * offset))
                    .sum::<i32>()
            };
            let mut lighten = 0;
            let mut darken = 0;
            let compare_density = window(-1) / 8;
            if own_density > compare_density {
                lighten = (2 * (own_density - compare_density)).min(30);
            } else if own_density < compare_density && own_density < 30 {
                darken = (2 * (compare_density - own_density)).min(30);
            }
            let compare_density = window(1) / 8;
            if own_density > compare_density {
                darken += (2 * (own_density - compare_density)).min(30);
            }
            plane[offset] = lighten.clamp(0, 254) as u8;
            plane[offset + 1] = darken.clamp(0, 254) as u8;
        }
    }
    plane
}

#[cfg(test)]
mod placement_shading_tests {
    use super::*;

    const WIDTH: u32 = 3;
    const HEIGHT: u32 = 20;
    /// Open on every side so the landscape border reads as sky.
    const OPEN: (i32, i32, bool, bool, Option<u8>) = (i32::MAX, i32::MAX, true, true, None);

    fn placements(entries: &[(usize, i32)]) -> [i32; 128] {
        let mut placements = [0; 128];
        for &(index, placement) in entries {
            placements[index] = placement;
        }
        placements
    }

    fn shading_at(plane: &[u8], x: u32, y: u32) -> (u8, u8) {
        let offset = (y as usize * WIDTH as usize + x as usize) * 2;
        (plane[offset], plane[offset + 1])
    }

    /// C4Landscape.cpp:2856-2871 — a pixel whose eight rows above and below
    /// hold the same placement is neither lightened nor darkened.
    #[test]
    fn uniform_material_receives_no_placement_shading() {
        let bytes = vec![1_u8; (WIDTH * HEIGHT) as usize];
        let plane = placement_shading_plane(
            &bytes,
            WIDTH,
            HEIGHT,
            &placements(&[(1, 70)]),
            (0, 0, false, false, Some(1)),
        );
        assert_eq!(shading_at(&plane, 1, 10), (0, 0));
    }

    /// A single dense row against sky lightens from above AND darkens from
    /// below in the same pixel — the case a single signed channel cannot carry,
    /// because `LightenClrBy` clamps at 255 before `DarkenClrBy` subtracts.
    #[test]
    fn an_isolated_row_both_lightens_and_darkens() {
        let mut bytes = vec![0_u8; (WIDTH * HEIGHT) as usize];
        for x in 0..WIDTH as usize {
            bytes[10 * WIDTH as usize + x] = 1;
        }
        let plane = placement_shading_plane(&bytes, WIDTH, HEIGHT, &placements(&[(1, 70)]), OPEN);
        assert_eq!(shading_at(&plane, 1, 10), (30, 30));
    }

    /// C4Landscape.cpp:2862-2865 — light material beneath heavy material is
    /// darkened, and only when its own density is below 30.
    #[test]
    fn light_material_under_heavy_material_darkens() {
        let mut bytes = vec![0_u8; (WIDTH * HEIGHT) as usize];
        for y in 0..HEIGHT as usize {
            for x in 0..WIDTH as usize {
                bytes[y * WIDTH as usize + x] = if y < 10 { 1 } else { 2 };
            }
        }
        let plane = placement_shading_plane(
            &bytes,
            WIDTH,
            HEIGHT,
            &placements(&[(1, 70), (2, 5)]),
            (0, 0, false, false, Some(2)),
        );
        assert_eq!(shading_at(&plane, 1, 10), (0, 30));
    }

    /// C4Landscape.cpp:2851 — `if (!iOwnDens) continue;` leaves the pixel
    /// untouched, i.e. fully transparent in the composed plane.
    #[test]
    fn zero_placement_material_is_suppressed() {
        let bytes = vec![1_u8; (WIDTH * HEIGHT) as usize];
        let plane = placement_shading_plane(&bytes, WIDTH, HEIGHT, &placements(&[]), OPEN);
        assert_eq!(shading_at(&plane, 1, 10), (0, SHADING_PLANE_SUPPRESSED));
    }

    /// Sky carries no shading; the composer never reaches the placement branch
    /// for it (C4Landscape.cpp:2841-2845).
    #[test]
    fn sky_carries_no_placement_shading() {
        let bytes = vec![0_u8; (WIDTH * HEIGHT) as usize];
        let plane = placement_shading_plane(&bytes, WIDTH, HEIGHT, &placements(&[(1, 70)]), OPEN);
        assert!(plane.iter().all(|value| *value == 0));
    }
}
