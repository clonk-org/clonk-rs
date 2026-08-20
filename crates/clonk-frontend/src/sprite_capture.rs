use super::*;

#[derive(Clone, Copy)]
struct CapturedGpuSpriteChunk {
    position: [[f32; 3]; 4],
    uv: [[f32; 2]; 4],
    fog_modulation: Option<[u32; 4]>,
    sample_tile: Option<[f32; 3]>,
    physical_tile: Option<(i32, i32, i32)>,
}

const CPP_MAX_TEXTURE_SIZE: u32 = 4_096;
const PHYSICAL_TEXTURE_TILE_CACHE_MAX_ENTRIES: usize = 4_096;
const PHYSICAL_TEXTURE_TILE_CACHE_MAX_BYTES: usize = 128 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PhysicalTextureTileKey {
    pub(crate) source: GpuTextureId,
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) size: u32,
}

struct CachedPhysicalTextureTile {
    resource: GpuTextureResource,
    last_used: u64,
}

pub(crate) struct PhysicalTextureTileCache {
    entries: HashMap<PhysicalTextureTileKey, CachedPhysicalTextureTile>,
    retained_bytes: usize,
    clock: u64,
    max_entries: usize,
    max_bytes: usize,
}

impl Default for PhysicalTextureTileCache {
    fn default() -> Self {
        Self::with_limits(
            PHYSICAL_TEXTURE_TILE_CACHE_MAX_ENTRIES,
            PHYSICAL_TEXTURE_TILE_CACHE_MAX_BYTES,
        )
    }
}

impl PhysicalTextureTileCache {
    pub(crate) fn with_limits(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            retained_bytes: 0,
            clock: 0,
            max_entries,
            max_bytes,
        }
    }

    pub(crate) fn get(&mut self, key: &PhysicalTextureTileKey) -> Option<GpuTextureResource> {
        self.clock = self.clock.wrapping_add(1).max(1);
        self.entries.get_mut(key).map(|entry| {
            entry.last_used = self.clock;
            entry.resource.clone()
        })
    }

    pub(crate) fn insert(&mut self, key: PhysicalTextureTileKey, resource: GpuTextureResource) {
        self.clock = self.clock.wrapping_add(1).max(1);
        let byte_len = resource.pixels.len();
        if let Some(replaced) = self.entries.insert(
            key,
            CachedPhysicalTextureTile {
                resource,
                last_used: self.clock,
            },
        ) {
            self.retained_bytes = self
                .retained_bytes
                .saturating_sub(replaced.resource.pixels.len());
        }
        self.retained_bytes = self.retained_bytes.saturating_add(byte_len);
        while self.entries.len() > self.max_entries || self.retained_bytes > self.max_bytes {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| *key);
            let Some(oldest) = oldest else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.retained_bytes = self
                    .retained_bytes
                    .saturating_sub(removed.resource.pixels.len());
            }
        }
    }

    #[cfg(test)]
    pub(crate) const fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }
}

fn needs_physical_texture_tiles(width: u32, height: u32) -> bool {
    width > CPP_MAX_TEXTURE_SIZE || height > CPP_MAX_TEXTURE_SIZE
}

fn physical_texture_tile_resource(
    source: &GpuTextureResource,
    tile: (i32, i32, i32),
) -> Option<GpuTextureResource> {
    // ImageData and derived owner layers are immutable. Mutable surfaces need
    // revision-aware dirty-tile publication before they can use this cache.
    if source.revision != 0 || source.base_revision.is_some() || !source.dirty.is_empty() {
        return None;
    }
    let (x, y, size) = tile;
    let (x, y, size) = (
        u32::try_from(x).ok()?,
        u32::try_from(y).ok()?,
        u32::try_from(size).ok()?.max(1),
    );
    if x >= source.extent[0] || y >= source.extent[1] || !source.is_valid() {
        return None;
    }
    let key = PhysicalTextureTileKey {
        source: source.id,
        x,
        y,
        size,
    };
    static CACHE: OnceLock<std::sync::Mutex<PhysicalTextureTileCache>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(|| std::sync::Mutex::new(PhysicalTextureTileCache::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(resource) = cache.get(&key) {
        return Some(resource);
    }

    let bytes_per_pixel = source.format.bytes_per_pixel();
    let pixel_count = usize::try_from(size).ok()?.checked_pow(2)?;
    let byte_len = pixel_count.checked_mul(bytes_per_pixel)?;
    let mut pixels = match source.format {
        clonk_graphics::GpuTextureFormat::Rgba8 => [255, 255, 255, 0].repeat(pixel_count),
        clonk_graphics::GpuTextureFormat::R8 => vec![0; byte_len],
    };
    let copy_width = size.min(source.extent[0].saturating_sub(x));
    let copy_height = size.min(source.extent[1].saturating_sub(y));
    let copy_bytes = usize::try_from(copy_width)
        .ok()?
        .checked_mul(bytes_per_pixel)?;
    let source_stride = usize::try_from(source.extent[0])
        .ok()?
        .checked_mul(bytes_per_pixel)?;
    let tile_stride = usize::try_from(size).ok()?.checked_mul(bytes_per_pixel)?;
    let source_x = usize::try_from(x).ok()?.checked_mul(bytes_per_pixel)?;
    for row in 0..usize::try_from(copy_height).ok()? {
        let source_start = usize::try_from(y)
            .ok()?
            .checked_add(row)?
            .checked_mul(source_stride)?
            .checked_add(source_x)?;
        let source_end = source_start.checked_add(copy_bytes)?;
        let target_start = row.checked_mul(tile_stride)?;
        let target_end = target_start.checked_add(copy_bytes)?;
        pixels
            .get_mut(target_start..target_end)?
            .copy_from_slice(source.pixels.get(source_start..source_end)?);
    }

    let resource = GpuTextureResource {
        id: GpuTextureId::fresh(),
        extent: [size, size],
        revision: 0,
        base_revision: None,
        format: source.format,
        pixels: Arc::from(pixels.into_boxed_slice()),
        dirty: Vec::new(),
    };
    cache.insert(key, resource.clone());
    Some(resource)
}

fn normalized_c4_modulation(modulation: u32) -> [f32; 4] {
    split_c4_color(modulation).map(|channel| f32::from(channel) / 255.0)
}

fn packed_c4_modulation(modulation: [f32; 4]) -> u32 {
    let [red, green, blue, transparency] =
        modulation.map(|channel| (channel * 255.0).round() as u8);
    (u32::from(transparency) << 24)
        | (u32::from(red) << 16)
        | (u32::from(green) << 8)
        | u32::from(blue)
}

fn captured_sprite_modulation(
    modulation: u32,
    fog_modulation: Option<[u32; 4]>,
    uses_mod2: bool,
    renderer_config: AdvancedRendererConfig,
) -> ([[f32; 4]; 4], bool) {
    let combined = if modulation == 0 {
        [0; 4]
    } else {
        fog_modulation.map_or([modulation; 4], |fog| {
            fog.map(|fog| modulate_c4_colors(modulation, fog))
        })
    };
    let mod2 = uses_mod2
        && modulation != 0
        && fog_modulation.is_none_or(|_| combined.iter().any(|modulation| *modulation != 0));
    let mut normalized = combined.map(normalized_c4_modulation);
    if !mod2 && !renderer_config.shader && renderer_config.no_alpha_add {
        for modulation in &mut normalized {
            modulation[3] = 0.0;
        }
    }
    (normalized, mod2)
}

fn captured_sprite_position(transform: &GraphicsTransform, x: f32, y: f32) -> Option<[f32; 3]> {
    let matrix = &transform.mat;
    let position = [
        matrix[0] * x + matrix[1] * y + matrix[2],
        matrix[3] * x + matrix[4] * y + matrix[5],
        matrix[6] * x + matrix[7] * y + matrix[8],
    ];
    if !position.iter().all(|value| value.is_finite()) || position[2] == 0.0 {
        return None;
    }
    let projected = [position[0] / position[2], position[1] / position[2]];
    projected
        .iter()
        .all(|value| value.is_finite())
        .then_some(position)
}

fn compact_fog_axis_ranges(
    origin: f32,
    extent: f32,
    chunk_size: f32,
    flipped: bool,
) -> Option<([(f32, f32); 2], usize)> {
    if !origin.is_finite()
        || !extent.is_finite()
        || !chunk_size.is_finite()
        || extent <= 0.0
        || extent > chunk_size
        || chunk_size <= 0.0
    {
        return None;
    }
    let end = origin + extent;
    if !end.is_finite() || end <= origin {
        return None;
    }
    let boundary = ((origin / chunk_size).floor() + 1.0) * chunk_size;
    let mut ranges = [(0.0, extent), (0.0, 0.0)];
    let count = if boundary > origin && boundary < end {
        ranges = [(0.0, boundary - origin), (boundary - origin, extent)];
        2
    } else {
        1
    };
    if flipped {
        for range in ranges.iter_mut().take(count) {
            *range = (extent - range.1, extent - range.0);
        }
        if count == 2 && ranges[0].0 > ranges[1].0 {
            ranges.swap(0, 1);
        }
    }
    Some((ranges, count))
}

#[allow(clippy::too_many_arguments)]
fn capture_compact_fogged_object_sprite(
    surface: &mut Surface,
    dest: (f32, f32, f32, f32),
    fog_dest: (f32, f32, f32, f32),
    transform: &GraphicsTransform,
    image: &ImageData,
    mask: Option<&ColorByOwnerMask>,
    source: FloatSourceRect,
    flip_x: bool,
    owner_color: Option<u32>,
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
    fog: &FogDrawContext,
    sampler: GpuSampler,
) -> bool {
    if needs_physical_texture_tiles(image.width(), image.height())
        || blit.fog_modulation.is_some()
        || blit.renderer_config.texture_indent() != 0.0
        || blit.renderer_config.no_box_fades
    {
        return false;
    }
    let chunk_size = cpp_tex_size(image.width(), image.height()).min(64) as f32;
    let Some((x_ranges, x_count)) =
        compact_fog_axis_ranges(source.x, source.width, chunk_size, flip_x)
    else {
        return false;
    };
    let Some((y_ranges, y_count)) =
        compact_fog_axis_ranges(source.y, source.height, chunk_size, false)
    else {
        return false;
    };
    let (dest_x, dest_y, dest_width, dest_height) = dest;
    let (fog_x, fog_y, fog_width, fog_height) = fog_dest;
    if ![
        fog_x,
        fog_y,
        fog_width,
        fog_height,
        dest_x,
        dest_y,
        dest_width,
        dest_height,
    ]
    .iter()
    .all(|value| value.is_finite())
        || fog_width <= 0.0
        || fog_height <= 0.0
    {
        return false;
    }

    let tile_size = cpp_tex_size(image.width(), image.height()) as f32;
    let owner_layers = match (mask, owner_color) {
        (Some(mask), Some(mut owner_modulation)) => {
            if let Some(global) = blit.modulation {
                if blit.mode & C4GFXBLIT_CLRSFC_OWNCLR == 0 {
                    owner_modulation = modulate_c4_colors(owner_modulation, global);
                }
            }
            let Some((base, owner)) = mask.gpu_layer_resources(image) else {
                return false;
            };
            Some((
                base,
                owner,
                owner_modulation,
                blit.mode & C4GFXBLIT_CLRSFC_MOD2 != 0,
                if blit.mode & C4GFXBLIT_CLRSFC_OWNCLR != 0 {
                    GpuOuterModulation::Ignore
                } else {
                    GpuOuterModulation::Combine
                },
            ))
        }
        _ => None,
    };
    let base_resource = owner_layers
        .as_ref()
        .map(|(base, ..)| base.clone())
        .unwrap_or_else(|| image.gpu_texture_resource());
    let mut base_captured = [None; 4];
    let mut owner_captured = [None; 4];
    let mut captured_count = 0;
    for &(top, bottom) in y_ranges.iter().take(y_count) {
        for &(left, right) in x_ranges.iter().take(x_count) {
            let local = [(left, top), (right, top), (left, bottom), (right, bottom)];
            let mut positions = [[0.0; 3]; 4];
            let mut uv = [[0.0; 2]; 4];
            let mut fog_modulation = [0; 4];
            for (index, (local_x, local_y)) in local.into_iter().enumerate() {
                let normalized_x = local_x / source.width;
                let normalized_y = local_y / source.height;
                let Some(position) = captured_sprite_position(
                    transform,
                    dest_x + normalized_x * dest_width,
                    dest_y + normalized_y * dest_height,
                ) else {
                    return false;
                };
                positions[index] = position;
                let sample_x = if flip_x {
                    source.x + (1.0 - normalized_x) * source.width
                } else {
                    source.x + normalized_x * source.width
                };
                let sample_y = source.y + normalized_y * source.height;
                uv[index] = [
                    sample_x / image.width() as f32,
                    sample_y / image.height() as f32,
                ];
                let (fog_sample_x, fog_sample_y) = transform.transform_point(
                    fog_x + normalized_x * fog_width,
                    fog_y + normalized_y * fog_height,
                );
                if !fog_sample_x.is_finite() || !fog_sample_y.is_finite() {
                    return false;
                }
                fog_modulation[index] = fog.modulation_at_point(fog_sample_x, fog_sample_y);
            }
            if positions
                .iter()
                .any(|position| position[2].is_sign_positive())
                && positions
                    .iter()
                    .any(|position| position[2].is_sign_negative())
            {
                return false;
            }
            let (base_modulation, base_mod2) = captured_sprite_modulation(
                blit.modulation.unwrap_or(0x00ff_ffff),
                Some(fog_modulation),
                blit.mode & C4GFXBLIT_MOD2 != 0,
                blit.renderer_config,
            );
            let uv_rect = [uv[0][0], uv[0][1], uv[3][0], uv[3][1]];
            let sample_tile_size = if sampler == GpuSampler::Linear {
                tile_size
            } else {
                0.0
            };
            base_captured[captured_count] = Some(GpuObjectSprite::new(
                positions,
                uv_rect,
                base_modulation.map(packed_c4_modulation),
                sampler,
                sample_tile_size,
                base_mod2,
                GpuOuterModulation::Combine,
            ));
            if let Some((_, _, owner_modulation, owner_mod2, owner_outer_modulation)) =
                owner_layers.as_ref()
            {
                let (owner_modulation, owner_mod2) = captured_sprite_modulation(
                    *owner_modulation,
                    Some(fog_modulation),
                    *owner_mod2,
                    blit.renderer_config,
                );
                owner_captured[captured_count] = Some(
                    GpuObjectSprite::new(
                        positions,
                        uv_rect,
                        owner_modulation.map(packed_c4_modulation),
                        sampler,
                        sample_tile_size,
                        owner_mod2,
                        *owner_outer_modulation,
                    )
                    .with_owner_layer(),
                );
            }
            captured_count += 1;
        }
    }

    let texture = base_resource.id;
    let clip = surface.clip();
    let blend = if blit.mode & C4GFXBLIT_ADDITIVE != 0 {
        GpuBlend::Additive
    } else {
        GpuBlend::Normal
    };
    let gamma = gamma.is_some_and(|gamma| !gamma.is_passthrough());
    let _ = surface.add_gpu_texture(base_resource);
    if let Some((_, owner_resource, ..)) = owner_layers {
        let owner_texture = owner_resource.id;
        let _ = surface.add_gpu_texture(owner_resource);
        for sprite in base_captured.into_iter().take(captured_count).flatten() {
            let _ = surface.push_gpu_owner_object_sprite(
                texture,
                owner_texture,
                sprite,
                clip,
                blend,
                gamma,
            );
        }
        for sprite in owner_captured.into_iter().take(captured_count).flatten() {
            let _ = surface.push_gpu_owner_object_sprite(
                texture,
                owner_texture,
                sprite,
                clip,
                blend,
                gamma,
            );
        }
    } else {
        for sprite in base_captured.into_iter().take(captured_count).flatten() {
            let _ = surface.push_gpu_object_sprite(texture, sprite, clip, blend, gamma);
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn capture_gpu_sprite(
    surface: &mut Surface,
    dest: (f32, f32, f32, f32),
    fog_dest: (f32, f32, f32, f32),
    transform: &GraphicsTransform,
    image: &ImageData,
    mask: Option<&ColorByOwnerMask>,
    source: FloatSourceRect,
    flip_x: bool,
    owner_color: Option<u32>,
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
    fog: Option<&FogDrawContext>,
    sampler: GpuSampler,
    inclusive_source_end: bool,
) -> bool {
    capture_gpu_sprite_with_resource(
        surface,
        dest,
        fog_dest,
        transform,
        image,
        mask,
        source,
        flip_x,
        owner_color,
        blit,
        gamma,
        fog,
        sampler,
        inclusive_source_end,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn capture_gpu_sprite_with_resource(
    surface: &mut Surface,
    dest: (f32, f32, f32, f32),
    fog_dest: (f32, f32, f32, f32),
    transform: &GraphicsTransform,
    image: &ImageData,
    mask: Option<&ColorByOwnerMask>,
    source: FloatSourceRect,
    flip_x: bool,
    owner_color: Option<u32>,
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
    fog: Option<&FogDrawContext>,
    sampler: GpuSampler,
    inclusive_source_end: bool,
    retained_resource: Option<GpuTextureResource>,
) -> bool {
    capture_gpu_sprite_impl(
        surface,
        dest,
        fog_dest,
        transform,
        image,
        mask,
        source,
        flip_x,
        owner_color,
        blit,
        gamma,
        fog,
        sampler,
        inclusive_source_end,
        retained_resource,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn capture_gpu_object_sprite(
    surface: &mut Surface,
    dest: (f32, f32, f32, f32),
    fog_dest: (f32, f32, f32, f32),
    transform: &GraphicsTransform,
    image: &ImageData,
    mask: Option<&ColorByOwnerMask>,
    source: FloatSourceRect,
    flip_x: bool,
    owner_color: Option<u32>,
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
    fog: Option<&FogDrawContext>,
    sampler: GpuSampler,
    inclusive_source_end: bool,
) -> bool {
    capture_gpu_sprite_impl(
        surface,
        dest,
        fog_dest,
        transform,
        image,
        mask,
        source,
        flip_x,
        owner_color,
        blit,
        gamma,
        fog,
        sampler,
        inclusive_source_end,
        None,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn capture_gpu_sprite_impl(
    surface: &mut Surface,
    dest: (f32, f32, f32, f32),
    fog_dest: (f32, f32, f32, f32),
    transform: &GraphicsTransform,
    image: &ImageData,
    mask: Option<&ColorByOwnerMask>,
    source: FloatSourceRect,
    flip_x: bool,
    owner_color: Option<u32>,
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
    fog: Option<&FogDrawContext>,
    sampler: GpuSampler,
    inclusive_source_end: bool,
    retained_resource: Option<GpuTextureResource>,
    compact_object: bool,
) -> bool {
    if !surface.is_gpu_scene_capture_active() {
        return false;
    }
    let (dest_x, dest_y, dest_width, dest_height) = dest;
    if ![
        dest_x,
        dest_y,
        dest_width,
        dest_height,
        source.x,
        source.y,
        source.width,
        source.height,
    ]
    .iter()
    .all(|value| value.is_finite())
        || dest_width <= 0.0
        || dest_height <= 0.0
        || !source.is_valid()
        || image.width() == 0
        || image.height() == 0
        || source.x < 0.0
        || source.y < 0.0
        || source.x + source.width > image.width() as f32
        || source.y + source.height > image.height() as f32
        || transform.inverse().is_none()
        || retained_resource.as_ref().is_some_and(|resource| {
            mask.is_some()
                || resource.extent != [image.width(), image.height()]
                || resource.format != clonk_graphics::GpuTextureFormat::Rgba8
                || !resource.is_valid()
        })
    {
        return false;
    }

    let physical_texture_tiles = needs_physical_texture_tiles(image.width(), image.height());
    if physical_texture_tiles && inclusive_source_end {
        // Inclusive blits scale their sample extent before C++ walks physical
        // C4TexRefs. Until retained splitting operates in that scaled space,
        // keep the exact CPU path instead of assigning a chunk to the wrong tile.
        return false;
    }

    if compact_object
        && retained_resource.is_none()
        && !inclusive_source_end
        && fog.is_some_and(|fog| {
            capture_compact_fogged_object_sprite(
                surface,
                dest,
                fog_dest,
                transform,
                image,
                mask,
                source,
                flip_x,
                owner_color,
                blit,
                gamma,
                fog,
                sampler,
            )
        })
    {
        return true;
    }

    let fog_sampler = match fog {
        Some(fog) => {
            let Some(sampler) = FogSpriteSampler::new(
                fog,
                fog_dest,
                (source.x, source.y, source.width, source.height),
                (image.width(), image.height()),
                flip_x,
                |x, y| transform.transform_point(x, y),
            ) else {
                // Sampling one point per fragment is a CPU-only recovery path.
                // Never silently drop active fog from a retained command.
                return false;
            };
            Some(sampler)
        }
        None => None,
    };

    let tile_size = cpp_tex_size(image.width(), image.height()) as f32;
    let texture_indent = blit.renderer_config.texture_indent();
    if fog_sampler.is_none()
        && blit.fog_modulation.is_none()
        && texture_indent == 0.0
        && (mask.is_none() || compact_object)
        && !physical_texture_tiles
    {
        let sample_width = if inclusive_source_end {
            (source.width - 1.0).max(0.0)
        } else {
            source.width
        };
        let sample_height = if inclusive_source_end {
            (source.height - 1.0).max(0.0)
        } else {
            source.height
        };
        let local = [
            (0.0, 0.0),
            (source.width, 0.0),
            (0.0, source.height),
            (source.width, source.height),
        ];
        let mut positions = [[0.0; 3]; 4];
        let mut uv = [[0.0; 2]; 4];
        for (index, (local_x, local_y)) in local.into_iter().enumerate() {
            let normalized_x = local_x / source.width;
            let normalized_y = local_y / source.height;
            let target_x = dest_x + normalized_x * dest_width;
            let target_y = dest_y + normalized_y * dest_height;
            let Some(position) = captured_sprite_position(transform, target_x, target_y) else {
                return false;
            };
            positions[index] = position;
            let sample_x = if flip_x {
                source.x + (1.0 - normalized_x) * sample_width
            } else {
                source.x + normalized_x * sample_width
            };
            let sample_y = source.y + normalized_y * sample_height;
            uv[index] = [
                sample_x / image.width() as f32,
                sample_y / image.height() as f32,
            ];
        }
        if positions
            .iter()
            .any(|position| position[2].is_sign_positive())
            && positions
                .iter()
                .any(|position| position[2].is_sign_negative())
        {
            return false;
        }

        let (base_modulation, base_mod2) = captured_sprite_modulation(
            blit.modulation.unwrap_or(0x00ff_ffff),
            None,
            blit.mode & C4GFXBLIT_MOD2 != 0,
            blit.renderer_config,
        );
        let base_outer_modulation = if blit.modulation.is_some() {
            GpuOuterModulation::Combine
        } else {
            GpuOuterModulation::Inherit
        };
        let owner_layers = match (mask, owner_color) {
            (Some(mask), Some(mut owner_modulation)) => {
                if let Some(global) = blit.modulation {
                    if blit.mode & C4GFXBLIT_CLRSFC_OWNCLR == 0 {
                        owner_modulation = modulate_c4_colors(owner_modulation, global);
                    }
                }
                let Some((base, owner)) = mask.gpu_layer_resources(image) else {
                    return false;
                };
                let (owner_modulation, owner_mod2) = captured_sprite_modulation(
                    owner_modulation,
                    None,
                    blit.mode & C4GFXBLIT_CLRSFC_MOD2 != 0,
                    blit.renderer_config,
                );
                let owner_outer_modulation = if blit.mode & C4GFXBLIT_CLRSFC_OWNCLR != 0 {
                    GpuOuterModulation::Ignore
                } else {
                    GpuOuterModulation::Combine
                };
                Some((
                    base,
                    owner,
                    owner_modulation,
                    owner_mod2,
                    owner_outer_modulation,
                ))
            }
            _ => None,
        };
        let base_resource = owner_layers
            .as_ref()
            .map(|(base, ..)| base.clone())
            .unwrap_or_else(|| retained_resource.unwrap_or_else(|| image.gpu_texture_resource()));
        let blend = if blit.mode & C4GFXBLIT_ADDITIVE != 0 {
            GpuBlend::Additive
        } else {
            GpuBlend::Normal
        };
        let gamma = gamma.is_some_and(|gamma| !gamma.is_passthrough());
        if compact_object {
            let texture = base_resource.id;
            let clip = surface.clip();
            let uv_rect = [uv[0][0], uv[0][1], uv[3][0], uv[3][1]];
            let sample_tile_size = if sampler == GpuSampler::Linear {
                tile_size
            } else {
                0.0
            };
            let base_sprite = GpuObjectSprite::new(
                positions,
                uv_rect,
                base_modulation.map(packed_c4_modulation),
                sampler,
                sample_tile_size,
                base_mod2,
                base_outer_modulation,
            );
            let owner_sprite = owner_layers.as_ref().map(
                |(_, _, owner_modulation, owner_mod2, owner_outer_modulation)| {
                    GpuObjectSprite::new(
                        positions,
                        uv_rect,
                        owner_modulation.map(packed_c4_modulation),
                        sampler,
                        sample_tile_size,
                        *owner_mod2,
                        *owner_outer_modulation,
                    )
                    .with_owner_layer()
                },
            );
            if let Some((_, owner_resource, ..)) = owner_layers {
                let Some(owner_sprite) = owner_sprite else {
                    return false;
                };
                let owner_texture = owner_resource.id;
                let _ = surface.add_gpu_texture(base_resource);
                let _ = surface.add_gpu_texture(owner_resource);
                let _ = surface.push_gpu_owner_object_sprite(
                    texture,
                    owner_texture,
                    base_sprite,
                    clip,
                    blend,
                    gamma,
                );
                let _ = surface.push_gpu_owner_object_sprite(
                    texture,
                    owner_texture,
                    owner_sprite,
                    clip,
                    blend,
                    gamma,
                );
            } else {
                let _ = surface.add_gpu_texture(base_resource);
                let _ = surface.push_gpu_object_sprite(texture, base_sprite, clip, blend, gamma);
            }
            return true;
        }
        let sample_tile = (sampler == GpuSampler::Linear).then_some([0.0, 0.0, tile_size]);
        let vertices = std::array::from_fn(|index| {
            let vertex = GpuVertex::new(positions[index], uv[index], base_modulation[index])
                .with_outer_modulation(base_outer_modulation)
                .with_owner_outer_modulation(base_outer_modulation);
            sample_tile.map_or(vertex, |[x, y, size]| vertex.with_sample_tile(x, y, size))
        });
        let command = GpuCommand::Quad {
            texture: base_resource.id,
            owner_mask: None,
            vertices,
            clip: surface.clip(),
            blend,
            base_mod2,
            owner_mod2: false,
            sampler,
            gamma,
        };
        let _ = surface.add_gpu_texture(base_resource);
        let _ = surface.push_gpu_command(command);
        return true;
    }
    let fallback_reasons = GpuSpriteFallbackReasons {
        spatial_fog: fog_sampler.is_some(),
        precomputed_fog_modulation: blit.fog_modulation.is_some(),
        texture_indent: texture_indent != 0.0,
        owner_mask: mask.is_some(),
        physical_texture_tiles,
    };
    let chunk_geometry = if let Some(sampler) = fog_sampler.as_ref() {
        // Fog chunks use min(native tile size, 64), so every chunk is
        // already contained within exactly one C4TexRef tile.
        sampler
            .quads
            .iter()
            .map(|quad| (quad.x, quad.y, Some(quad.modulation)))
            .collect::<Vec<_>>()
    } else if texture_indent != 0.0 || physical_texture_tiles {
        // TexIndent restarts at each physical C4TexRef. A single interpolated
        // UV quad cannot express that piecewise transform, so retain one GPU
        // command per native texture tile only while the switch is active.
        let x_ranges = FogSpriteSampler::axis_ranges(source.x, source.width, tile_size, flip_x);
        let y_ranges = FogSpriteSampler::axis_ranges(source.y, source.height, tile_size, false);
        if x_ranges
            .len()
            .checked_mul(y_ranges.len())
            .is_none_or(|chunks| chunks > 1_000_000)
        {
            return false;
        }
        y_ranges
            .iter()
            .flat_map(|&y| x_ranges.iter().copied().map(move |x| (x, y, None)))
            .collect()
    } else {
        vec![(
            (0.0, source.width),
            (0.0, source.height),
            blit.fog_modulation.map(|sample| sample.modulation),
        )]
    };

    let sample_width = if inclusive_source_end {
        (source.width - 1.0).max(0.0)
    } else {
        source.width
    };
    let sample_height = if inclusive_source_end {
        (source.height - 1.0).max(0.0)
    } else {
        source.height
    };
    let mut chunks = Vec::with_capacity(chunk_geometry.len());
    for (x_range, y_range, fog_modulation) in chunk_geometry {
        let normalized_center_x = (x_range.0 + x_range.1) / (2.0 * source.width);
        let normalized_center_y = (y_range.0 + y_range.1) / (2.0 * source.height);
        let center_x = if flip_x {
            source.x + (1.0 - normalized_center_x) * sample_width
        } else {
            source.x + normalized_center_x * sample_width
        };
        let center_y = source.y + normalized_center_y * sample_height;
        let physical_tile = if texture_indent != 0.0 || physical_texture_tiles {
            let Some(tile) = cpp_texture_tile_for_source(
                image.width(),
                image.height(),
                center_x,
                center_y,
                fog_sampler.is_some(),
            ) else {
                continue;
            };
            Some(tile)
        } else {
            None
        };
        let texture_transform = physical_tile.and_then(|(tile_x, tile_y, physical_size)| {
            let physical_size = physical_size as f32;
            let denominator = physical_size + 2.0 * texture_indent;
            if !denominator.is_finite() || denominator.abs() <= f32::EPSILON {
                return None;
            }
            let chunk_size = if fog_sampler.is_some() {
                physical_size.min(64.0)
            } else {
                physical_size
            };
            let chunk_start = |center: f32, tile_origin: i32, source_origin: f32| {
                let tile_origin = tile_origin as f32;
                source_origin
                    .max(tile_origin + ((center - tile_origin) / chunk_size).floor() * chunk_size)
            };
            Some((
                tile_x,
                tile_y,
                physical_size,
                denominator,
                chunk_start(center_x, tile_x, source.x),
                chunk_start(center_y, tile_y, source.y),
            ))
        });
        let local = [
            (x_range.0, y_range.0),
            (x_range.1, y_range.0),
            (x_range.0, y_range.1),
            (x_range.1, y_range.1),
        ];
        let mut positions = [[0.0; 3]; 4];
        let mut uv = [[0.0; 2]; 4];
        for (index, (local_x, local_y)) in local.into_iter().enumerate() {
            let normalized_x = local_x / source.width;
            let normalized_y = local_y / source.height;
            let target_x = dest_x + normalized_x * dest_width;
            let target_y = dest_y + normalized_y * dest_height;
            let Some(position) = captured_sprite_position(transform, target_x, target_y) else {
                return false;
            };
            positions[index] = position;
            let sample_x = if flip_x {
                source.x + (1.0 - normalized_x) * sample_width
            } else {
                source.x + normalized_x * sample_width
            };
            let sample_y = source.y + normalized_y * sample_height;
            let (sample_x, sample_y) = texture_transform.map_or(
                (sample_x, sample_y),
                |(tile_x, tile_y, physical_size, denominator, quad_start_x, quad_start_y)| {
                    let adjust = |edge: f32, tile_origin: i32, quad_start: f32| {
                        let tile_origin = tile_origin as f32;
                        (quad_start
                            + texture_indent
                            + (edge - quad_start) * physical_size / denominator)
                            .clamp(tile_origin, tile_origin + physical_size)
                    };
                    (
                        adjust(sample_x, tile_x, quad_start_x),
                        adjust(sample_y, tile_y, quad_start_y),
                    )
                },
            );
            uv[index] = [
                sample_x / image.width() as f32,
                sample_y / image.height() as f32,
            ];
        }
        if positions
            .iter()
            .any(|position| position[2].is_sign_positive())
            && positions
                .iter()
                .any(|position| position[2].is_sign_negative())
        {
            return false;
        }
        // The shader derives the native tile origin from each fragment's
        // interpolated source coordinate. This preserves C4TexRef seam and
        // padding behavior without expanding an unfogged image into one draw
        // command per tile.
        let sample_tile = (sampler == GpuSampler::Linear).then(|| {
            physical_tile.map_or([0.0, 0.0, tile_size], |(x, y, size)| {
                [x as f32, y as f32, size as f32]
            })
        });
        chunks.push(CapturedGpuSpriteChunk {
            position: positions,
            uv,
            fog_modulation,
            sample_tile,
            physical_tile,
        });
    }

    let main_modulation = blit.modulation.unwrap_or(0x00ff_ffff);
    let owner_modulation = owner_color.map(|mut owner| {
        if let Some(global) = blit.modulation {
            if blit.mode & C4GFXBLIT_CLRSFC_OWNCLR == 0 {
                owner = modulate_c4_colors(owner, global);
            }
        }
        owner
    });
    let blend = if blit.mode & C4GFXBLIT_ADDITIVE != 0 {
        GpuBlend::Additive
    } else {
        GpuBlend::Normal
    };
    let clip = surface.clip();
    let gamma = gamma.is_some_and(|gamma| !gamma.is_passthrough());
    let commands_for = |resource: &GpuTextureResource,
                        modulation,
                        uses_mod2,
                        outer_modulation|
     -> Option<(Vec<GpuTextureResource>, Vec<GpuCommand>)> {
        let mut resources = if physical_texture_tiles {
            Vec::new()
        } else {
            vec![resource.clone()]
        };
        let mut commands = Vec::with_capacity(chunks.len());
        for chunk in &chunks {
            let (texture, uv, sample_tile) = if physical_texture_tiles {
                let tile = chunk.physical_tile?;
                let tiled = physical_texture_tile_resource(resource, tile)?;
                let (tile_x, tile_y, tile_size) = tile;
                let mut uv = chunk.uv;
                for value in &mut uv {
                    value[0] = ((value[0] * image.width() as f32 - tile_x as f32)
                        / tile_size as f32)
                        .clamp(0.0, 1.0);
                    value[1] = ((value[1] * image.height() as f32 - tile_y as f32)
                        / tile_size as f32)
                        .clamp(0.0, 1.0);
                }
                let sample_tile =
                    (sampler == GpuSampler::Linear).then_some([0.0, 0.0, tile_size as f32]);
                let texture = tiled.id;
                resources.push(tiled);
                (texture, uv, sample_tile)
            } else {
                (resource.id, chunk.uv, chunk.sample_tile)
            };
            let outer_modulation = if outer_modulation == GpuOuterModulation::Ignore {
                GpuOuterModulation::Ignore
            } else if chunk.fog_modulation.is_some() {
                GpuOuterModulation::Combine
            } else {
                outer_modulation
            };
            let (modulation, mod2) = captured_sprite_modulation(
                modulation,
                chunk.fog_modulation,
                uses_mod2,
                blit.renderer_config,
            );
            let command = |indices: [usize; 4], modulation: [[f32; 4]; 4]| {
                let vertices = std::array::from_fn(|slot| {
                    let index = indices[slot];
                    let vertex = GpuVertex::new(chunk.position[index], uv[index], modulation[slot])
                        .with_outer_modulation(outer_modulation)
                        .with_owner_outer_modulation(outer_modulation);
                    sample_tile.map_or(vertex, |[x, y, size]| vertex.with_sample_tile(x, y, size))
                });
                GpuCommand::Quad {
                    texture,
                    owner_mask: None,
                    vertices,
                    clip,
                    blend,
                    base_mod2: mod2,
                    owner_mod2: false,
                    sampler,
                    gamma,
                }
            };
            if blit.renderer_config.no_box_fades && chunk.fog_modulation.is_some() {
                commands.extend([
                    command([0, 1, 2, 2], [modulation[2]; 4]),
                    command([2, 1, 3, 3], [modulation[3]; 4]),
                ]);
            } else {
                commands.push(command([0, 1, 2, 3], modulation));
            }
        }
        Some((resources, commands))
    };

    let (base_resource, overlay_resource) = match (mask, owner_modulation) {
        (Some(mask), Some(_)) => {
            let Some((base, overlay)) = mask.gpu_layer_resources(image) else {
                return false;
            };
            (base, Some(overlay))
        }
        _ => (
            retained_resource.unwrap_or_else(|| image.gpu_texture_resource()),
            None,
        ),
    };
    let Some((base_resources, base_commands)) = commands_for(
        &base_resource,
        main_modulation,
        blit.mode & C4GFXBLIT_MOD2 != 0,
        if blit.modulation.is_some() {
            GpuOuterModulation::Combine
        } else {
            GpuOuterModulation::Inherit
        },
    ) else {
        return false;
    };
    let overlay_commands =
        if let Some((overlay, modulation)) = overlay_resource.as_ref().zip(owner_modulation) {
            let Some(commands) = commands_for(
                overlay,
                modulation,
                blit.mode & C4GFXBLIT_CLRSFC_MOD2 != 0,
                if blit.mode & C4GFXBLIT_CLRSFC_OWNCLR != 0 {
                    GpuOuterModulation::Ignore
                } else {
                    GpuOuterModulation::Combine
                },
            ) else {
                return false;
            };
            Some(commands)
        } else {
            None
        };

    let _ = surface.record_gpu_sprite_fallback(
        fallback_reasons,
        usize::from(fallback_reasons.spatial_fog).saturating_mul(chunks.len()),
    );
    for resource in base_resources {
        let _ = surface.add_gpu_texture(resource);
    }
    if let Some((resources, _)) = overlay_commands.as_ref() {
        for resource in resources {
            let _ = surface.add_gpu_texture(resource.clone());
        }
    }
    // Native C4Surface owner bitmaps are two complete painter-order passes.
    // Keep every base chunk ahead of every owner chunk, rather than
    // interleaving the layers chunk by chunk.
    for command in base_commands {
        let _ = surface.push_gpu_command(command);
    }
    if let Some((_, commands)) = overlay_commands {
        for command in commands {
            let _ = surface.push_gpu_command(command);
        }
    }
    true
}

pub(crate) fn gpu_sampler_for_blit(sampling: BlitSampling) -> GpuSampler {
    match sampling {
        BlitSampling::Nearest => GpuSampler::Nearest,
        BlitSampling::Linear => GpuSampler::Linear,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_object_image_region_transformed_float_source(
    surface: &mut Surface,
    dest: (f32, f32, f32, f32),
    transform: &GraphicsTransform,
    image: &ImageData,
    mask: Option<&ColorByOwnerMask>,
    source: &FloatSourceRect,
    sampling: BlitSampling,
    flip_x: bool,
    owner_color: Option<u32>,
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
    fog: Option<&FogDrawContext>,
) {
    let offset = blit.renderer_config.destination_offset();
    let captured_dest = (dest.0 + offset, dest.1 + offset, dest.2, dest.3);
    if capture_gpu_object_sprite(
        surface,
        captured_dest,
        captured_dest,
        transform,
        image,
        mask,
        *source,
        flip_x,
        owner_color,
        blit,
        gamma,
        fog,
        gpu_sampler_for_blit(sampling),
        false,
    ) {
        return;
    }
    draw_image_region_transformed_float_source(
        surface,
        dest,
        transform,
        image,
        mask,
        source,
        sampling,
        flip_x,
        owner_color,
        blit,
        gamma,
        fog,
    );
}

/// Sprite blit through a full projective matrix. This is the CPU equivalent
/// of C++'s transformed GL/software blit and intentionally keeps the normal
/// owner-colour, modulation, gamma and framebuffer-composition pipeline.
/// This float-source form is private to C4Object faces; all other callers
/// retain the integer [`SourceRect`] entry points below.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_image_region_transformed_float_source(
    surface: &mut Surface,
    dest: (f32, f32, f32, f32),
    transform: &GraphicsTransform,
    image: &ImageData,
    mask: Option<&ColorByOwnerMask>,
    source: &FloatSourceRect,
    sampling: BlitSampling,
    flip_x: bool,
    owner_color: Option<u32>,
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
    fog: Option<&FogDrawContext>,
) {
    let offset = blit.renderer_config.destination_offset();
    let dest = (dest.0 + offset, dest.1 + offset, dest.2, dest.3);
    let (dest_x, dest_y, dest_width, dest_height) = dest;
    if dest_width <= 0.0 || dest_height <= 0.0 || !source.is_valid() {
        return;
    }
    if image.width() == 0 || image.height() == 0 {
        return;
    }
    let Some(inverse) = transform.inverse() else {
        return;
    };
    if capture_gpu_sprite(
        surface,
        dest,
        dest,
        transform,
        image,
        mask,
        *source,
        flip_x,
        owner_color,
        blit,
        gamma,
        fog,
        gpu_sampler_for_blit(sampling),
        false,
    ) {
        return;
    }
    let fog_sampler = fog.and_then(|fog| {
        FogSpriteSampler::new(
            fog,
            dest,
            (source.x, source.y, source.width, source.height),
            (image.width(), image.height()),
            flip_x,
            |x, y| transform.transform_point(x, y),
        )
    });

    let corners = [
        (dest_x, dest_y),
        (dest_x + dest_width, dest_y),
        (dest_x, dest_y + dest_height),
        (dest_x + dest_width, dest_y + dest_height),
    ];
    let mut min_x = f32::MAX;
    let mut min_y = f32::MAX;
    let mut max_x = f32::MIN;
    let mut max_y = f32::MIN;
    for (x, y) in corners {
        let (x, y) = transform.transform_point(x, y);
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    let bounds = surface.bounds();
    let min_x = (min_x.floor() as i32).max(bounds.x);
    let min_y = (min_y.floor() as i32).max(bounds.y);
    let max_x = (max_x.ceil() as i32).min(bounds.x + bounds.width as i32);
    let max_y = (max_y.ceil() as i32).min(bounds.y + bounds.height as i32);
    if min_x >= max_x || min_y >= max_y {
        return;
    }

    for target_y in min_y..max_y {
        for target_x in min_x..max_x {
            let (sample_x, sample_y) =
                inverse.transform_point(target_x as f32 + 0.5, target_y as f32 + 0.5);
            if !sample_x.is_finite() || !sample_y.is_finite() {
                continue;
            }
            let normalized_x = (sample_x - dest_x) / dest_width;
            let normalized_y = (sample_y - dest_y) / dest_height;
            if !(0.0..1.0).contains(&normalized_x) || !(0.0..1.0).contains(&normalized_y) {
                continue;
            }

            let (source_edge_x, source_edge_y) =
                source.source_edge(normalized_x, normalized_y, flip_x);
            let pixel_blit = fog_sprite_blit_at(
                fog_sampler.as_ref(),
                fog,
                blit,
                normalized_x,
                normalized_y,
                target_x,
                target_y,
            );
            let Some(source) = prepare_runtime_sprite_sample(
                image,
                mask,
                source,
                fog.is_some(),
                source_edge_x,
                source_edge_y,
                sampling,
                owner_color,
                pixel_blit,
            ) else {
                continue;
            };
            if source.alpha() == 0.0 {
                continue;
            }
            blend_prepared_sprite_fragment(
                surface,
                target_x as u32,
                target_y as u32,
                source,
                pixel_blit,
                gamma,
            );
        }
    }
}

/// Untransformed float-source counterpart used by straight C4Object faces.
/// Its normalized sampling is identical to [`draw_image_region`] whenever
/// the source coordinates are integral.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_image_region_float_source(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    mask: Option<&ColorByOwnerMask>,
    source: &FloatSourceRect,
    sampling: BlitSampling,
    flip_x: bool,
    owner_color: Option<u32>,
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
    fog: Option<&FogDrawContext>,
) {
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 || !source.is_valid() {
        return;
    }

    if blit.renderer_config.has_adjusted_quad_geometry() {
        return draw_image_region_float_source_adjusted(
            surface,
            rect,
            image,
            mask,
            source,
            sampling,
            flip_x,
            owner_color,
            blit,
            gamma,
            fog,
        );
    }

    let dest_width = rect.size.width.max(1.0).round() as u32;
    let dest_height = rect.size.height.max(1.0).round() as u32;
    if dest_width == 0 || dest_height == 0 || image.width() == 0 || image.height() == 0 {
        return;
    }

    let dest_x = rect.origin.x.round() as i32;
    let dest_y = rect.origin.y.round() as i32;
    if capture_gpu_sprite(
        surface,
        (
            dest_x as f32,
            dest_y as f32,
            dest_width as f32,
            dest_height as f32,
        ),
        (
            rect.origin.x,
            rect.origin.y,
            rect.size.width,
            rect.size.height,
        ),
        &GraphicsTransform::identity(),
        image,
        mask,
        *source,
        flip_x,
        owner_color,
        blit,
        gamma,
        fog,
        gpu_sampler_for_blit(sampling),
        false,
    ) {
        return;
    }
    let fog_sampler = fog.and_then(|fog| {
        FogSpriteSampler::new(
            fog,
            (
                rect.origin.x,
                rect.origin.y,
                rect.size.width,
                rect.size.height,
            ),
            (source.x, source.y, source.width, source.height),
            (image.width(), image.height()),
            flip_x,
            |x, y| (x, y),
        )
    });

    let bounds = surface.bounds();
    for dy in 0..dest_height {
        let target_y = dest_y + dy as i32;
        if target_y < bounds.y || target_y >= bounds.y + bounds.height as i32 {
            continue;
        }

        let normalized_y = (dy as f32 + 0.5) / dest_height as f32;

        for dx in 0..dest_width {
            let target_x = dest_x + dx as i32;
            if target_x < bounds.x || target_x >= bounds.x + bounds.width as i32 {
                continue;
            }

            let normalized_x = (dx as f32 + 0.5) / dest_width as f32;
            let (source_edge_x, source_edge_y) =
                source.source_edge(normalized_x, normalized_y, flip_x);
            let pixel_blit = fog_sprite_blit_at(
                fog_sampler.as_ref(),
                fog,
                blit,
                (target_x as f32 + 0.5 - rect.origin.x) / rect.size.width,
                (target_y as f32 + 0.5 - rect.origin.y) / rect.size.height,
                target_x,
                target_y,
            );
            let Some(source) = prepare_runtime_sprite_sample(
                image,
                mask,
                source,
                fog.is_some(),
                source_edge_x,
                source_edge_y,
                sampling,
                owner_color,
                pixel_blit,
            ) else {
                continue;
            };
            if source.alpha() == 0.0 {
                continue;
            }
            blend_prepared_sprite_fragment(
                surface,
                target_x as u32,
                target_y as u32,
                source,
                pixel_blit,
                gamma,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_object_image_region_float_source(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    mask: Option<&ColorByOwnerMask>,
    source: &FloatSourceRect,
    sampling: BlitSampling,
    flip_x: bool,
    owner_color: Option<u32>,
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
    fog: Option<&FogDrawContext>,
) {
    let adjusted = blit.renderer_config.has_adjusted_quad_geometry();
    let offset = blit.renderer_config.destination_offset();
    let dest_width = rect.size.width.max(1.0).round();
    let dest_height = rect.size.height.max(1.0).round();
    let dest = if adjusted {
        (
            rect.origin.x + offset,
            rect.origin.y + offset,
            rect.size.width,
            rect.size.height,
        )
    } else {
        (
            rect.origin.x.round(),
            rect.origin.y.round(),
            dest_width,
            dest_height,
        )
    };
    let fog_dest = if adjusted {
        dest
    } else {
        (
            rect.origin.x,
            rect.origin.y,
            rect.size.width,
            rect.size.height,
        )
    };
    if capture_gpu_object_sprite(
        surface,
        dest,
        fog_dest,
        &GraphicsTransform::identity(),
        image,
        mask,
        *source,
        flip_x,
        owner_color,
        blit,
        gamma,
        fog,
        gpu_sampler_for_blit(sampling),
        false,
    ) {
        return;
    }
    draw_image_region_float_source(
        surface,
        rect,
        image,
        mask,
        source,
        sampling,
        flip_x,
        owner_color,
        blit,
        gamma,
        fog,
    );
}

/// Rasterize the CStdGL quad when BlitOffset and/or TexIndent makes its
/// submitted geometry differ from Rust's legacy integer fast path. Pixel
/// centers are tested against the shifted float quad and sampling remains
/// relative to that shifted geometry, so BlitOffset cancels out of texture
/// coordinates exactly as it does in StdDDraw2.cpp.
#[allow(clippy::too_many_arguments)]
fn draw_image_region_float_source_adjusted(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    mask: Option<&ColorByOwnerMask>,
    source: &FloatSourceRect,
    sampling: BlitSampling,
    flip_x: bool,
    owner_color: Option<u32>,
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
    fog: Option<&FogDrawContext>,
) {
    if rect.size.width <= 0.0
        || rect.size.height <= 0.0
        || !source.is_valid()
        || image.width() == 0
        || image.height() == 0
    {
        return;
    }
    let offset = blit.renderer_config.destination_offset();
    let dest = (
        rect.origin.x + offset,
        rect.origin.y + offset,
        rect.size.width,
        rect.size.height,
    );
    if !dest.0.is_finite() || !dest.1.is_finite() {
        return;
    }
    if capture_gpu_sprite(
        surface,
        dest,
        dest,
        &GraphicsTransform::identity(),
        image,
        mask,
        *source,
        flip_x,
        owner_color,
        blit,
        gamma,
        fog,
        gpu_sampler_for_blit(sampling),
        false,
    ) {
        return;
    }
    let fog_sampler = fog.and_then(|fog| {
        FogSpriteSampler::new(
            fog,
            dest,
            (source.x, source.y, source.width, source.height),
            (image.width(), image.height()),
            flip_x,
            |x, y| (x, y),
        )
    });
    let bounds = surface.bounds();
    let min_x = ((dest.0 - 0.5).ceil() as i32).max(bounds.x);
    let min_y = ((dest.1 - 0.5).ceil() as i32).max(bounds.y);
    let max_x = ((dest.0 + dest.2 - 0.5).ceil() as i32).min(bounds.x + bounds.width as i32);
    let max_y = ((dest.1 + dest.3 - 0.5).ceil() as i32).min(bounds.y + bounds.height as i32);

    for target_y in min_y..max_y {
        let normalized_y = (target_y as f32 + 0.5 - dest.1) / dest.3;
        if !(0.0..1.0).contains(&normalized_y) {
            continue;
        }
        for target_x in min_x..max_x {
            let normalized_x = (target_x as f32 + 0.5 - dest.0) / dest.2;
            if !(0.0..1.0).contains(&normalized_x) {
                continue;
            }
            let (source_edge_x, source_edge_y) =
                source.source_edge(normalized_x, normalized_y, flip_x);
            let pixel_blit = fog_sprite_blit_at(
                fog_sampler.as_ref(),
                fog,
                blit,
                normalized_x,
                normalized_y,
                target_x,
                target_y,
            );
            let Some(source) = prepare_runtime_sprite_sample(
                image,
                mask,
                source,
                fog.is_some(),
                source_edge_x,
                source_edge_y,
                sampling,
                owner_color,
                pixel_blit,
            ) else {
                continue;
            };
            if source.alpha() == 0.0 {
                continue;
            }
            blend_prepared_sprite_fragment(
                surface,
                target_x as u32,
                target_y as u32,
                source,
                pixel_blit,
                gamma,
            );
        }
    }
}

pub(crate) fn draw_image_region(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    mask: Option<&ColorByOwnerMask>,
    source: &SourceRect,
    flip_x: bool,
    owner_color: Option<u32>,
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
    fog: Option<&FogDrawContext>,
) {
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return;
    }

    if source.width <= 0 || source.height <= 0 {
        return;
    }

    if blit.renderer_config.has_adjusted_quad_geometry() {
        return draw_image_region_float_source(
            surface,
            rect,
            image,
            mask,
            &FloatSourceRect::scaled(*source, 1.0),
            BlitSampling::Nearest,
            flip_x,
            owner_color,
            blit,
            gamma,
            fog,
        );
    }

    let dest_width = rect.size.width.max(1.0).round() as u32;
    let dest_height = rect.size.height.max(1.0).round() as u32;
    if dest_width == 0 || dest_height == 0 || image.width() == 0 || image.height() == 0 {
        return;
    }

    let dest_x = rect.origin.x.round() as i32;
    let dest_y = rect.origin.y.round() as i32;
    if capture_gpu_sprite(
        surface,
        (
            dest_x as f32,
            dest_y as f32,
            dest_width as f32,
            dest_height as f32,
        ),
        (
            rect.origin.x,
            rect.origin.y,
            rect.size.width,
            rect.size.height,
        ),
        &GraphicsTransform::identity(),
        image,
        mask,
        FloatSourceRect {
            x: source.x as f32,
            y: source.y as f32,
            width: source.width as f32,
            height: source.height as f32,
        },
        flip_x,
        owner_color,
        blit,
        gamma,
        fog,
        GpuSampler::Nearest,
        false,
    ) {
        return;
    }
    let fog_sampler = fog.and_then(|fog| {
        FogSpriteSampler::new(
            fog,
            (
                rect.origin.x,
                rect.origin.y,
                rect.size.width,
                rect.size.height,
            ),
            (
                source.x as f32,
                source.y as f32,
                source.width as f32,
                source.height as f32,
            ),
            (image.width(), image.height()),
            flip_x,
            |x, y| (x, y),
        )
    });

    let bounds = surface.bounds();
    let image_width = image.width() as i32;
    let image_height = image.height() as i32;
    let pixels = image.pixels();

    for dy in 0..dest_height {
        let target_y = dest_y + dy as i32;
        if target_y < bounds.y || target_y >= bounds.y + bounds.height as i32 {
            continue;
        }

        let normalized_y = dy as f32 / dest_height as f32;
        let src_y = (normalized_y * source.height as f32)
            .floor()
            .clamp(0.0, (source.height - 1) as f32) as i32
            + source.y;
        if src_y < 0 || src_y >= image_height {
            continue;
        }

        for dx in 0..dest_width {
            let target_x = dest_x + dx as i32;
            if target_x < bounds.x || target_x >= bounds.x + bounds.width as i32 {
                continue;
            }

            let rel = ((dx as f32 / dest_width as f32) * source.width as f32)
                .floor()
                .clamp(0.0, (source.width - 1) as f32) as i32;
            let sample_x_rel = if flip_x {
                (source.width - 1).saturating_sub(rel)
            } else {
                rel
            };
            let src_x = source.x + sample_x_rel;
            if src_x < 0 || src_x >= image_width {
                continue;
            }

            let idx = (src_y as usize * image.width() as usize + src_x as usize) * 4;
            if idx + 3 >= pixels.len() {
                continue;
            }

            let color = Color::new(
                pixels[idx],
                pixels[idx + 1],
                pixels[idx + 2],
                pixels[idx + 3],
            );
            let owner_mask = mask.map(|mask_map| mask_map.value_at(src_x as u32, src_y as u32));
            let pixel_blit = fog_sprite_blit_at(
                fog_sampler.as_ref(),
                fog,
                blit,
                (target_x as f32 + 0.5 - rect.origin.x) / rect.size.width,
                (target_y as f32 + 0.5 - rect.origin.y) / rect.size.height,
                target_x,
                target_y,
            );
            let source = prepare_sprite_fragment(color, owner_mask, owner_color, pixel_blit);
            if source.alpha() == 0.0 {
                continue;
            }
            blend_prepared_sprite_fragment(
                surface,
                target_x as u32,
                target_y as u32,
                source,
                pixel_blit,
                gamma,
            );
        }
    }
}

pub(crate) fn draw_image_region_rotated(
    surface: &mut Surface,
    center_x: f32,
    center_y: f32,
    dest_width: f32,
    dest_height: f32,
    image: &ImageData,
    mask: Option<&ColorByOwnerMask>,
    source: &SourceRect,
    flip_x: bool,
    owner_color: Option<u32>,
    rotation_degrees: f32,
    blit: SpriteBlitState,
    gamma: Option<&clonk_graphics::GammaRamp>,
    fog: Option<&FogDrawContext>,
) {
    if dest_width <= 0.0 || dest_height <= 0.0 {
        return;
    }
    if source.width <= 0 || source.height <= 0 {
        return;
    }
    if image.width() == 0 || image.height() == 0 {
        return;
    }

    if blit.renderer_config.has_adjusted_quad_geometry() {
        let angle_rad = rotation_degrees.to_radians();
        let cos_theta = angle_rad.cos();
        let sin_theta = angle_rad.sin();
        let transform = GraphicsTransform::set(
            cos_theta,
            -sin_theta,
            center_x - cos_theta * center_x + sin_theta * center_y,
            sin_theta,
            cos_theta,
            center_y - sin_theta * center_x - cos_theta * center_y,
            0.0,
            0.0,
            1.0,
        );
        return draw_image_region_transformed_float_source(
            surface,
            (
                center_x - dest_width / 2.0,
                center_y - dest_height / 2.0,
                dest_width,
                dest_height,
            ),
            &transform,
            image,
            mask,
            &FloatSourceRect::scaled(*source, 1.0),
            BlitSampling::Nearest,
            flip_x,
            owner_color,
            blit,
            gamma,
            fog,
        );
    }

    let bounds = surface.bounds();
    if bounds.width == 0 || bounds.height == 0 {
        return;
    }

    let half_w = dest_width / 2.0;
    let half_h = dest_height / 2.0;
    let angle_rad = rotation_degrees.to_radians();
    let cos_theta = angle_rad.cos();
    let sin_theta = angle_rad.sin();
    let transform = GraphicsTransform::set(
        cos_theta,
        -sin_theta,
        center_x - cos_theta * center_x + sin_theta * center_y,
        sin_theta,
        cos_theta,
        center_y - sin_theta * center_x - cos_theta * center_y,
        0.0,
        0.0,
        1.0,
    );
    // A rotated sprite is a transformed sprite with no owner mask, which the
    // compact record already represents: projective positions, reversed UV
    // edges, per-corner packed modulation, MOD2 and per-instance sampling. The
    // only caller is the rotated definition-particle path, whose 40-byte
    // axis-aligned batch cannot express rotation and so fell back to a
    // 232-byte generic quad — for 59 of the 130 particle definitions in the
    // current content snapshot (clonk-org/clonk-rs#271).
    if capture_gpu_object_sprite(
        surface,
        (
            center_x - half_w,
            center_y - half_h,
            dest_width,
            dest_height,
        ),
        (
            center_x - half_w,
            center_y - half_h,
            dest_width,
            dest_height,
        ),
        &transform,
        image,
        mask,
        FloatSourceRect {
            x: source.x as f32,
            y: source.y as f32,
            width: source.width as f32,
            height: source.height as f32,
        },
        flip_x,
        owner_color,
        blit,
        gamma,
        fog,
        GpuSampler::Nearest,
        true,
    ) {
        return;
    }
    let fog_sampler = fog.and_then(|fog| {
        FogSpriteSampler::new(
            fog,
            (
                center_x - half_w,
                center_y - half_h,
                dest_width,
                dest_height,
            ),
            (
                source.x as f32,
                source.y as f32,
                source.width as f32,
                source.height as f32,
            ),
            (image.width(), image.height()),
            flip_x,
            |x, y| {
                let dx = x - center_x;
                let dy = y - center_y;
                (
                    center_x + dx * cos_theta - dy * sin_theta,
                    center_y + dx * sin_theta + dy * cos_theta,
                )
            },
        )
    });

    let corners = [
        (-half_w, -half_h),
        (half_w, -half_h),
        (-half_w, half_h),
        (half_w, half_h),
    ];

    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for (x, y) in corners {
        let rotated_x = x * cos_theta - y * sin_theta;
        let rotated_y = x * sin_theta + y * cos_theta;
        min_x = min_x.min(rotated_x);
        max_x = max_x.max(rotated_x);
        min_y = min_y.min(rotated_y);
        max_y = max_y.max(rotated_y);
    }

    let surface_min_x = bounds.x;
    let surface_min_y = bounds.y;
    let surface_max_x = bounds.x + bounds.width as i32 - 1;
    let surface_max_y = bounds.y + bounds.height as i32 - 1;

    let min_px = ((center_x + min_x).floor() as i32).clamp(surface_min_x, surface_max_x);
    let max_px = ((center_x + max_x).ceil() as i32).clamp(surface_min_x, surface_max_x);
    let min_py = ((center_y + min_y).floor() as i32).clamp(surface_min_y, surface_max_y);
    let max_py = ((center_y + max_y).ceil() as i32).clamp(surface_min_y, surface_max_y);

    if min_px > max_px || min_py > max_py {
        return;
    }

    let src_width = source.width as f32;
    let src_height = source.height as f32;
    let pixels = image.pixels();

    for y in min_py..=max_py {
        for x in min_px..=max_px {
            let dx = (x as f32) - center_x;
            let dy = (y as f32) - center_y;

            let local_x = dx * cos_theta + dy * sin_theta;
            let local_y = -dx * sin_theta + dy * cos_theta;

            if local_x < -half_w || local_x > half_w || local_y < -half_h || local_y > half_h {
                continue;
            }

            let normalized_x = (local_x + half_w) / dest_width;
            let normalized_y = (local_y + half_h) / dest_height;

            if !(0.0..=1.0).contains(&normalized_x) || !(0.0..=1.0).contains(&normalized_y) {
                continue;
            }

            let src_x_float = (normalized_x * (src_width - 1.0)).clamp(0.0, src_width - 1.0);
            let src_y_float = (normalized_y * (src_height - 1.0)).clamp(0.0, src_height - 1.0);

            let src_x_local = src_x_float.floor() as i32;
            let src_y_local = src_y_float.floor() as i32;

            let src_x_offset = if flip_x {
                (source.width - 1).saturating_sub(src_x_local)
            } else {
                src_x_local
            };
            let sample_x = source.x + src_x_offset;
            let sample_y = source.y + src_y_local;

            if sample_x < 0
                || sample_y < 0
                || sample_x >= image.width() as i32
                || sample_y >= image.height() as i32
            {
                continue;
            }

            let idx = (sample_y as usize * image.width() as usize + sample_x as usize) * 4;
            if idx + 3 >= pixels.len() {
                continue;
            }

            let color = Color::new(
                pixels[idx],
                pixels[idx + 1],
                pixels[idx + 2],
                pixels[idx + 3],
            );
            let owner_mask =
                mask.map(|mask_map| mask_map.value_at(sample_x as u32, sample_y as u32));
            let fog_dx = x as f32 + 0.5 - center_x;
            let fog_dy = y as f32 + 0.5 - center_y;
            let fog_local_x = fog_dx * cos_theta + fog_dy * sin_theta;
            let fog_local_y = -fog_dx * sin_theta + fog_dy * cos_theta;
            let pixel_blit = fog_sprite_blit_at(
                fog_sampler.as_ref(),
                fog,
                blit,
                (fog_local_x + half_w) / dest_width,
                (fog_local_y + half_h) / dest_height,
                x,
                y,
            );
            let source = prepare_sprite_fragment(color, owner_mask, owner_color, pixel_blit);
            if source.alpha() == 0.0 {
                continue;
            }
            blend_prepared_sprite_fragment(surface, x as u32, y as u32, source, pixel_blit, gamma);
        }
    }
}

pub fn draw_image(surface: &mut Surface, rect: &GuiRect, image: &ImageData) {
    draw_image_with_gamma(surface, rect, image, None);
}

/// Nearest-neighbour GUI image draw with the active C++ fragment gamma ramp.
pub fn draw_image_with_gamma(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    if let Some(renderer_config) = active_advanced_renderer_config()
        .filter(|config| config.changes_generic_textured_blit(0, false))
    {
        return draw_image_source_configured_on_surface(
            surface,
            rect,
            image,
            FloatSourceRect {
                x: 0.0,
                y: 0.0,
                width: image.width() as f32,
                height: image.height() as f32,
            },
            BlitSampling::Nearest,
            gamma,
            BilinearBlend::AlphaOver,
            None,
            renderer_config,
        );
    }
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return;
    }

    let dest_width = rect.size.width.max(1.0).round() as u32;
    let dest_height = rect.size.height.max(1.0).round() as u32;
    if dest_width == 0 || dest_height == 0 || image.width() == 0 || image.height() == 0 {
        return;
    }

    let dest_x = rect.origin.x.round() as i32;
    let dest_y = rect.origin.y.round() as i32;

    if capture_gpu_gui_image(
        surface,
        (
            dest_x as f32,
            dest_y as f32,
            dest_width as f32,
            dest_height as f32,
        ),
        image,
        FloatSourceRect {
            x: 0.0,
            y: 0.0,
            width: image.width() as f32,
            height: image.height() as f32,
        },
        GpuSampler::Nearest,
        BilinearBlend::AlphaOver,
        None,
        gamma,
    ) {
        return;
    }

    if gamma.is_none() && dest_width == image.width() && dest_height == image.height() {
        if let Ok(src_surface) = Surface::from_bytes(
            image.width(),
            image.height(),
            PixelFormat::Rgba8888,
            image.pixels().to_vec(),
        ) {
            let _ = surface.blit(&src_surface, SurfacePoint::new(dest_x, dest_y));
        }
        return;
    }

    let bounds = surface.bounds();
    let src_width = image.width();
    let src_height = image.height();
    let pixels = image.pixels();

    for dy in 0..dest_height {
        let target_y = dest_y + dy as i32;
        if target_y < bounds.y || target_y >= bounds.y + bounds.height as i32 {
            continue;
        }

        let src_y = ((dy as f32 / dest_height as f32) * src_height as f32)
            .floor()
            .clamp(0.0, (src_height - 1) as f32) as u32;

        for dx in 0..dest_width {
            let target_x = dest_x + dx as i32;
            if target_x < bounds.x || target_x >= bounds.x + bounds.width as i32 {
                continue;
            }

            let src_x = ((dx as f32 / dest_width as f32) * src_width as f32)
                .floor()
                .clamp(0.0, (src_width - 1) as f32) as u32;
            let idx = ((src_y * src_width + src_x) * 4) as usize;
            if idx + 3 >= pixels.len() {
                continue;
            }

            let color = Color::new(
                pixels[idx],
                pixels[idx + 1],
                pixels[idx + 2],
                pixels[idx + 3],
            );

            if color.a == 0 {
                continue;
            }

            blend_prepared_sprite_fragment(
                surface,
                target_x as u32,
                target_y as u32,
                PreparedSpriteFragment::Legacy(color),
                SpriteBlitState::normal(),
                gamma,
            );
        }
    }
}

/// 1:1 alpha-over blit of an `image` subregion to `(dest_x, dest_y)`.
///
/// Mirrors a C++ facet blit at native size (no stretch), as used by
/// `C4GUI::Element::DrawBar`'s begin/middle/end slices (C4Gui.cpp:281-311).
/// With `gamma`, source fragments are encoded through the blit shader's
/// gamma lookup and blended in float like the GL blend stage.
#[allow(clippy::too_many_arguments)]
pub fn draw_image_strip(
    surface: &mut Surface,
    dest_x: i32,
    dest_y: i32,
    image: &ImageData,
    src_x: u32,
    src_y: u32,
    src_w: u32,
    src_h: u32,
    gamma: Option<&clonk_graphics::GammaRamp>,
) {
    let (iw, ih) = (image.width(), image.height());
    let src_w = src_w.min(iw.saturating_sub(src_x));
    let src_h = src_h.min(ih.saturating_sub(src_y));
    if let Some(renderer_config) = active_advanced_renderer_config()
        .filter(|config| config.changes_generic_textured_blit(0, false))
    {
        return draw_image_source_configured_on_surface(
            surface,
            &GuiRect::new(dest_x as f32, dest_y as f32, src_w as f32, src_h as f32),
            image,
            FloatSourceRect {
                x: src_x as f32,
                y: src_y as f32,
                width: src_w as f32,
                height: src_h as f32,
            },
            BlitSampling::Nearest,
            gamma,
            BilinearBlend::AlphaOver,
            None,
            renderer_config,
        );
    }
    if capture_gpu_gui_image(
        surface,
        (dest_x as f32, dest_y as f32, src_w as f32, src_h as f32),
        image,
        FloatSourceRect {
            x: src_x as f32,
            y: src_y as f32,
            width: src_w as f32,
            height: src_h as f32,
        },
        GpuSampler::Nearest,
        BilinearBlend::AlphaOver,
        None,
        gamma,
    ) {
        return;
    }
    let pixels = image.pixels();
    for sy in 0..src_h {
        let ty = dest_y + sy as i32;
        if ty < 0 || ty >= surface.height() as i32 {
            continue;
        }
        for sx in 0..src_w {
            let tx = dest_x + sx as i32;
            if tx < 0 || tx >= surface.width() as i32 {
                continue;
            }
            let idx = (((src_y + sy) * iw + (src_x + sx)) * 4) as usize;
            let Some(rgba) = pixels.get(idx..idx + 4) else {
                continue;
            };
            if rgba[3] == 0 {
                continue;
            }
            blend_prepared_sprite_fragment(
                surface,
                tx as u32,
                ty as u32,
                PreparedSpriteFragment::Legacy(Color::new(rgba[0], rgba[1], rgba[2], rgba[3])),
                SpriteBlitState::normal(),
                gamma,
            );
        }
    }
}

/// The GL texture tile size the C++ engine picks for an image: the next
/// power of two of min(W, H), capped at the 4096 max texture size
/// (C4Surface::CreateTextures, C4Surface.cpp:166-189). Images larger than
/// the tile in either dimension are split across multiple textures. Linear
/// taps clamp at each physical texture edge and never cross into a neighbor.
pub(crate) fn cpp_tex_size(width: u32, height: u32) -> u32 {
    let need = width.min(height).max(1);
    let mut n = 1u32;
    while (1 << n) < need {
        n += 1;
    }
    (1u32 << n).min(4096)
}

fn cpp_last_tex_size(width: u32, height: u32, base: i32) -> i32 {
    let base = base as u32;
    if width.is_multiple_of(base) || height.is_multiple_of(base) {
        return base as i32;
    }
    let needed = (width % base).max(height % base).max(1);
    let mut size = 2u32;
    while size < needed {
        size = size.saturating_mul(2);
    }
    size as i32
}

fn cpp_tile_dimensions(width: u32, height: u32, base: i32) -> (i32, i32) {
    (
        (width as i32 - 1) / base + 1,
        (height as i32 - 1) / base + 1,
    )
}

fn cpp_tile_size_at(
    width: u32,
    height: u32,
    base: i32,
    tile_x_index: i32,
    tile_y_index: i32,
) -> i32 {
    let (tiles_x, tiles_y) = cpp_tile_dimensions(width, height, base);
    if tile_x_index == tiles_x - 1 && tile_y_index == tiles_y - 1 {
        cpp_last_tex_size(width, height, base)
    } else {
        base
    }
}

/// CStdDDraw retains its chunk step after selecting a smaller final
/// bottom-right C4TexRef. The ordinary step is the base texture size; FoW
/// lowers it to 64, so a smaller final texture can still emit fog chunks.
fn cpp_tile_has_blit_chunks(
    width: u32,
    height: u32,
    base: i32,
    tile_x_index: i32,
    tile_y_index: i32,
    fog_chunked: bool,
) -> bool {
    let chunk_size = if fog_chunked { base.min(64) } else { base };
    cpp_tile_size_at(width, height, base, tile_x_index, tile_y_index) >= chunk_size
}

pub(crate) fn cpp_texture_tile_for_source(
    width: u32,
    height: u32,
    source_edge_x: f32,
    source_edge_y: f32,
    fog_chunked: bool,
) -> Option<(i32, i32, i32)> {
    if width == 0
        || height == 0
        || !source_edge_x.is_finite()
        || !source_edge_y.is_finite()
        || source_edge_x < 0.0
        || source_edge_y < 0.0
    {
        return None;
    }
    let base = cpp_tex_size(width, height) as i32;
    let tile_x_index = (source_edge_x.floor() as i32).div_euclid(base);
    let tile_y_index = (source_edge_y.floor() as i32).div_euclid(base);
    let (tiles_x, tiles_y) = cpp_tile_dimensions(width, height, base);
    if tile_x_index < 0
        || tile_y_index < 0
        || tile_x_index >= tiles_x
        || tile_y_index >= tiles_y
        || !cpp_tile_has_blit_chunks(width, height, base, tile_x_index, tile_y_index, fog_chunked)
    {
        return None;
    }
    let tile_size = cpp_tile_size_at(width, height, base, tile_x_index, tile_y_index);
    let tile_x = tile_x_index * base;
    let tile_y = tile_y_index * base;
    if source_edge_x >= tile_x.saturating_add(tile_size) as f32
        || source_edge_y >= tile_y.saturating_add(tile_size) as f32
    {
        return None;
    }
    Some((tile_x, tile_y, tile_size))
}

/// Selects the landscape texture from the unadjusted source coordinate, then
/// adds `TexIndent` inside that physical texture and lets GL_CLAMP_TO_EDGE
/// retain the selected tile. `BlitLandscape` submits `(q + I) / T` directly;
/// unlike ordinary `Blit`, it has no `T / (T + 2I)` rescale
/// (StdGL.cpp:713-760). The final pair retains the unclamped, tile-local
/// coordinate used by repeating Liquid.png texture unit 2.
pub(crate) fn cpp_landscape_source_texel(
    width: u32,
    height: u32,
    raw_x: f32,
    raw_y: f32,
    indent: f32,
) -> Option<(i32, i32, i32, i32)> {
    if width == 0
        || height == 0
        || !raw_x.is_finite()
        || !raw_y.is_finite()
        || raw_x < 0.0
        || raw_y < 0.0
        || raw_x >= width as f32
        || raw_y >= height as f32
    {
        return None;
    }
    let base = cpp_tex_size(width, height) as i32;
    let tile_x_index = (raw_x.floor() as i32).div_euclid(base);
    let tile_y_index = (raw_y.floor() as i32).div_euclid(base);
    let tile_size = cpp_tile_size_at(width, height, base, tile_x_index, tile_y_index);
    let tile_x = tile_x_index * base;
    let tile_y = tile_y_index * base;
    let liquid_x = (raw_x - tile_x as f32 + indent).floor() as i32;
    let liquid_y = (raw_y - tile_y as f32 + indent).floor() as i32;
    let source_x = ((raw_x + indent).floor() as i32)
        .clamp(tile_x, tile_x.saturating_add(tile_size).saturating_sub(1));
    let source_y = ((raw_y + indent).floor() as i32)
        .clamp(tile_y, tile_y.saturating_add(tile_size).saturating_sub(1));
    (source_x >= 0 && source_y >= 0 && source_x < width as i32 && source_y < height as i32)
        .then_some((source_x, source_y, liquid_x, liquid_y))
}

fn image_tile_texel(
    image: &ImageData,
    tile_x: i32,
    tile_y: i32,
    tile_size: i32,
    x_rel: i32,
    y_rel: i32,
) -> [f32; 4] {
    let x = tile_x + x_rel.clamp(0, tile_size - 1);
    let y = tile_y + y_rel.clamp(0, tile_size - 1);
    if x < 0 || y < 0 || x >= image.width() as i32 || y >= image.height() as i32 {
        // C4TexRef initializes untouched physical padding to 0xffffffff,
        // which is transparent white in C4's inverted-alpha convention.
        return [255.0, 255.0, 255.0, 0.0];
    }
    let idx = ((y as u32 * image.width() + x as u32) * 4) as usize;
    image
        .pixels()
        .get(idx..idx + 4)
        .map(|pixel| {
            if pixel[3] == 0 {
                // C4Surface::SetPixDw normalizes loaded transparent texels to
                // transparent black; padding above remains distinguishable.
                [0.0; 4]
            } else {
                [
                    f32::from(pixel[0]),
                    f32::from(pixel[1]),
                    f32::from(pixel[2]),
                    f32::from(pixel[3]),
                ]
            }
        })
        .unwrap_or([255.0, 255.0, 255.0, 0.0])
}

/// Bilinearly samples one `tile_size` texture tile of `image` at GL_LINEAR
/// coordinates `(u_rel, v_rel)` relative to the tile origin
/// `(tile_x, tile_y)` in image texels.
///
/// The engine sets GL_CLAMP_TO_EDGE at texture creation
/// (C4Surface.cpp:1102-1103), so the two taps clamp to the tile's edge
/// texels; texels inside the tile but outside the image are padding.
pub(crate) fn bilinear_sample_tile(
    image: &ImageData,
    tile_x: i32,
    tile_y: i32,
    tile_size: i32,
    u_rel: f32,
    v_rel: f32,
) -> [f32; 4] {
    let pixels = image.pixels();
    let texel = |x_rel: i32, y_rel: i32| -> [f32; 4] {
        let x = tile_x + x_rel.clamp(0, tile_size - 1);
        let y = tile_y + y_rel.clamp(0, tile_size - 1);
        if x < 0 || y < 0 || x >= image.width() as i32 || y >= image.height() as i32 {
            // C4Surface::CreateTextures clears unused texture storage with
            // 0xffffffff. Converted from C4 transparency-alpha to ordinary
            // opacity-alpha, that padding is transparent white.
            return [255.0, 255.0, 255.0, 0.0];
        }
        let idx = ((y as u32 * image.width() + x as u32) * 4) as usize;
        pixels
            .get(idx..idx + 4)
            .map(|pixel| {
                [
                    f32::from(pixel[0]),
                    f32::from(pixel[1]),
                    f32::from(pixel[2]),
                    f32::from(pixel[3]),
                ]
            })
            .unwrap_or([0.0; 4])
    };
    let (x0, y0) = (u_rel.floor() as i32, v_rel.floor() as i32);
    let (fx, fy) = (u_rel - x0 as f32, v_rel - y0 as f32);
    let (p00, p10) = (texel(x0, y0), texel(x0 + 1, y0));
    let (p01, p11) = (texel(x0, y0 + 1), texel(x0 + 1, y0 + 1));
    std::array::from_fn(|channel| {
        let top = p00[channel] * (1.0 - fx) + p10[channel] * fx;
        let bottom = p01[channel] * (1.0 - fx) + p11[channel] * fx;
        top * (1.0 - fy) + bottom * fy
    })
}

fn bilinear_sample_runtime_tile(
    image: &ImageData,
    tile_x: i32,
    tile_y: i32,
    tile_size: i32,
    u_rel: f32,
    v_rel: f32,
) -> [f32; 4] {
    let texel = |x, y| image_tile_texel(image, tile_x, tile_y, tile_size, x, y);
    let (x0, y0) = (u_rel.floor() as i32, v_rel.floor() as i32);
    let (fx, fy) = (u_rel - x0 as f32, v_rel - y0 as f32);
    let (p00, p10) = (texel(x0, y0), texel(x0 + 1, y0));
    let (p01, p11) = (texel(x0, y0 + 1), texel(x0 + 1, y0 + 1));
    std::array::from_fn(|c| {
        let top = p00[c] * (1.0 - fx) + p10[c] * fx;
        let bottom = p01[c] * (1.0 - fx) + p11[c] * fx;
        top * (1.0 - fy) + bottom * fy
    })
}

fn bilinear_sample_owner_tile(
    mask: &ColorByOwnerMask,
    tile_x: i32,
    tile_y: i32,
    tile_size: i32,
    u_rel: f32,
    v_rel: f32,
) -> FilteredColorByOwnerSample {
    let (x0, y0) = (u_rel.floor() as i32, v_rel.floor() as i32);
    let (fx, fy) = (u_rel - x0 as f32, v_rel - y0 as f32);
    let interpolate = |p00: f32, p10: f32, p01: f32, p11: f32| {
        let top = p00 * (1.0 - fx) + p10 * fx;
        let bottom = p01 * (1.0 - fx) + p11 * fx;
        top * (1.0 - fy) + bottom * fy
    };
    let pixel_count = u64::from(mask.width) * u64::from(mask.height);
    if pixel_count.checked_mul(4) == Some(mask.pixels.len() as u64) {
        let texel = |x_rel: i32, y_rel: i32| {
            let x = tile_x + x_rel.clamp(0, tile_size - 1);
            let y = tile_y + y_rel.clamp(0, tile_size - 1);
            if x < 0 || y < 0 || x >= mask.width as i32 || y >= mask.height as i32 {
                return [255.0, 255.0, 255.0, 0.0];
            }
            match mask.value_at(x as u32, y as u32) {
                ColorByOwnerSample::Overlay(color) if color.a != 0 => [
                    f32::from(color.r),
                    f32::from(color.g),
                    f32::from(color.b),
                    f32::from(color.a),
                ],
                _ => [0.0; 4],
            }
        };
        let (p00, p10) = (texel(x0, y0), texel(x0 + 1, y0));
        let (p01, p11) = (texel(x0, y0 + 1), texel(x0 + 1, y0 + 1));
        return FilteredColorByOwnerSample::Overlay(std::array::from_fn(|channel| {
            interpolate(p00[channel], p10[channel], p01[channel], p11[channel])
        }));
    }

    let texel = |x_rel: i32, y_rel: i32| {
        let x = tile_x + x_rel.clamp(0, tile_size - 1);
        let y = tile_y + y_rel.clamp(0, tile_size - 1);
        if x < 0 || y < 0 || x >= mask.width as i32 || y >= mask.height as i32 {
            return 0.0;
        }
        match mask.value_at(x as u32, y as u32) {
            ColorByOwnerSample::Scalar(value) => f32::from(value),
            ColorByOwnerSample::Overlay(_) => 0.0,
        }
    };
    FilteredColorByOwnerSample::Scalar(interpolate(
        texel(x0, y0),
        texel(x0 + 1, y0),
        texel(x0, y0 + 1),
        texel(x0 + 1, y0 + 1),
    ))
}

pub(crate) fn prepare_runtime_sprite_sample(
    image: &ImageData,
    mask: Option<&ColorByOwnerMask>,
    source: &FloatSourceRect,
    fog_chunked: bool,
    source_edge_x: f32,
    source_edge_y: f32,
    sampling: BlitSampling,
    owner_color: Option<u32>,
    blit: SpriteBlitState,
) -> Option<PreparedSpriteFragment> {
    prepare_runtime_sprite_sample_with_texture_size(
        image,
        mask,
        source,
        fog_chunked,
        source_edge_x,
        source_edge_y,
        sampling,
        owner_color,
        blit,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn prepare_runtime_sprite_sample_with_texture_size(
    image: &ImageData,
    mask: Option<&ColorByOwnerMask>,
    source: &FloatSourceRect,
    fog_chunked: bool,
    source_edge_x: f32,
    source_edge_y: f32,
    sampling: BlitSampling,
    owner_color: Option<u32>,
    blit: SpriteBlitState,
    physical_texture_size: Option<i32>,
) -> Option<PreparedSpriteFragment> {
    let (tile_x, tile_y, tile_size) = if let Some(tile_size) = physical_texture_size {
        if tile_size <= 0 || !source_edge_x.is_finite() || !source_edge_y.is_finite() {
            return None;
        }
        (0, 0, tile_size)
    } else {
        cpp_texture_tile_for_source(
            image.width(),
            image.height(),
            source_edge_x,
            source_edge_y,
            fog_chunked,
        )?
    };
    let indent = blit.renderer_config.texture_indent();
    let adjust_edge = |edge: f32, tile_origin: i32, source_origin: f32| {
        if indent == 0.0 {
            return edge;
        }
        let tile_size = tile_size as f32;
        let denominator = tile_size + 2.0 * indent;
        if !denominator.is_finite() || denominator.abs() <= f32::EPSILON {
            return edge;
        }
        let chunk_size = if fog_chunked {
            tile_size.min(64.0)
        } else {
            tile_size
        };
        let chunk_origin =
            tile_origin as f32 + ((edge - tile_origin as f32) / chunk_size).floor() * chunk_size;
        let quad_start = source_origin.max(chunk_origin);
        let adjusted = quad_start + indent + (edge - quad_start) * tile_size / denominator;
        let physical_end = tile_origin as f32 + tile_size;
        adjusted.clamp(tile_origin as f32, physical_end)
    };
    let source_edge_x = adjust_edge(source_edge_x, tile_x, source.x);
    let source_edge_y = adjust_edge(source_edge_y, tile_y, source.y);
    match sampling {
        BlitSampling::Nearest => {
            let source_x = (source_edge_x.floor() as i32)
                .clamp(tile_x, tile_x.saturating_add(tile_size).saturating_sub(1));
            let source_y = (source_edge_y.floor() as i32)
                .clamp(tile_y, tile_y.saturating_add(tile_size).saturating_sub(1));
            let pixel = image_tile_texel(
                image,
                tile_x,
                tile_y,
                tile_size,
                source_x - tile_x,
                source_y - tile_y,
            );
            let color = Color::new(
                pixel[0] as u8,
                pixel[1] as u8,
                pixel[2] as u8,
                pixel[3] as u8,
            );
            let owner_mask = mask.and_then(|mask| {
                (source_x >= 0
                    && source_y >= 0
                    && source_x < mask.width as i32
                    && source_y < mask.height as i32)
                    .then(|| mask.value_at(source_x as u32, source_y as u32))
            });
            Some(prepare_sprite_fragment(
                color,
                owner_mask,
                owner_color,
                blit,
            ))
        }
        BlitSampling::Linear => {
            let u_rel = source_edge_x - 0.5 - tile_x as f32;
            let v_rel = source_edge_y - 0.5 - tile_y as f32;
            let source =
                bilinear_sample_runtime_tile(image, tile_x, tile_y, tile_size, u_rel, v_rel);
            let owner_mask = mask.map(|mask| {
                bilinear_sample_owner_tile(mask, tile_x, tile_y, tile_size, u_rel, v_rel)
            });
            Some(prepare_filtered_sprite_fragment(
                source,
                owner_mask,
                owner_color,
                blit,
            ))
        }
    }
}

/// How a stretched blit combines with the framebuffer.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BilinearBlend {
    /// `glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA)`.
    AlphaOver,
    /// `glBlendFunc(GL_SRC_ALPHA, GL_ONE)` (StdGL.cpp:908, additive blits).
    Additive,
}
