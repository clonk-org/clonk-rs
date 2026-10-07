//! CPU execution of the retained painter-ordered scene.

use crate::{
    Color, GpuBlend, GpuCommand, GpuPrimitiveTopology, GpuSampler, GpuScene, GpuSoftwareBlend,
    GpuSolidAlphaMode, GpuSolidVertex, GpuTextureFormat, GpuTextureResource, GpuVertex, Rect,
};
use rayon::prelude::*;
use std::hash::{Hash, Hasher};
#[path = "cpu_scene_spans.rs"]
mod spans;
#[path = "cpu_scene_textures.rs"]
mod textures;
#[path = "cpu_scene_tiles.rs"]
mod tiles;

#[cfg(test)]
std::thread_local! {
    static FORCE_GENERIC_SPANS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static SHADER_GAMMA_DIVISIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Debug, thiserror::Error)]
pub enum CpuSceneError {
    #[error("retained CPU frame has an invalid extent or output length")]
    InvalidFrame,
    #[error("retained CPU command {0} is not supported")]
    UnsupportedCommand(usize),
}

#[derive(Default)]
pub struct CpuSceneRenderer {
    stats: CpuSceneStats,
    cached_pixels: Vec<u8>,
    extent: [u32; 2],
    tiles: Vec<tiles::Tile>,
    tables: SpanTables,
    texture_regions: textures::TextureRegions,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CpuSceneStats {
    pub rasterized_tiles: usize,
    pub reused_tiles: usize,
}

impl CpuSceneRenderer {
    pub fn stats(&self) -> CpuSceneStats {
        self.stats
    }
    pub fn rendered_pixels(&self) -> &[u8] {
        &self.cached_pixels
    }
    /// Execute scale-native commands directly over the accumulated output.
    pub fn render_loaded(
        &mut self,
        scene: &GpuScene,
        frame: &mut [u8],
    ) -> Result<(), CpuSceneError> {
        let [width, height] = scene.logical_extent;
        if width == 0
            || height == 0
            || (width as usize)
                .checked_mul(height as usize)
                .and_then(|area| area.checked_mul(4))
                != Some(frame.len())
        {
            return Err(CpuSceneError::InvalidFrame);
        }
        validate_scene(scene)?;
        self.tables.update(scene);
        let mut target = RasterTarget {
            pixels: frame,
            bounds: Rect::new(0, 0, width, height),
            tables: &self.tables,
            row_origin: 0,
            gamma_raw: None,
            gamma_override: None,
        };
        for (index, command) in scene.commands.iter().enumerate() {
            for atom in 0..tiles::atom_count(command) {
                execute_atom(scene, &mut target, command, index, atom)?;
            }
        }
        Ok(())
    }
    pub fn render(&mut self, scene: &GpuScene, frame: &mut [u8]) -> Result<(), CpuSceneError> {
        self.render_cached(scene, frame, true)
    }
    /// Rasterize an isolated logical layer; resolve monitor gamma after scale
    /// and composition of all logical and physical layers.
    pub fn render_without_monitor(
        &mut self,
        scene: &GpuScene,
        frame: &mut [u8],
    ) -> Result<(), CpuSceneError> {
        self.render_cached(scene, frame, false)
    }
    pub fn invalidate(&mut self) {
        for tile in &mut self.tiles {
            tile.previous = None;
        }
    }
    fn render_cached(
        &mut self,
        scene: &GpuScene,
        frame: &mut [u8],
        resolve_monitor: bool,
    ) -> Result<(), CpuSceneError> {
        let [width, height] = scene.logical_extent;
        let byte_len = (width as usize)
            .checked_mul(height as usize)
            .and_then(|area| area.checked_mul(4))
            .ok_or(CpuSceneError::InvalidFrame)?;
        if width == 0 || height == 0 || frame.len() != byte_len {
            return Err(CpuSceneError::InvalidFrame);
        }
        validate_scene(scene)?;
        self.texture_regions.update(scene)?;
        self.tables.update(scene);
        if self.extent != scene.logical_extent {
            self.extent = scene.logical_extent;
            self.tiles.clear();
            for y in (0..height).step_by(tiles::TILE_SIZE as usize) {
                for x in (0..width).step_by(tiles::TILE_SIZE as usize) {
                    self.tiles.push(tiles::Tile::new(Rect::new(
                        x as i32,
                        y as i32,
                        tiles::TILE_SIZE.min(width - x),
                        tiles::TILE_SIZE.min(height - y),
                    )));
                }
            }
        }
        self.cached_pixels.resize(byte_len, 0);
        let key = frame_key(scene);
        for tile in &mut self.tiles {
            tile.atoms.clear();
            tile.hash = std::collections::hash_map::DefaultHasher::new();
            key.hash(&mut tile.hash);
            resolve_monitor.hash(&mut tile.hash);
        }
        let columns = width.div_ceil(tiles::TILE_SIZE) as usize;
        for (index, command) in scene.commands.iter().enumerate() {
            for atom in 0..tiles::atom_count(command) {
                let Some(bounds) = command.logical_atom_bounds_with_raster_padding(
                    scene.logical_extent,
                    1.0,
                    atom,
                ) else {
                    continue;
                };
                let hash = tiles::atom_hash(scene, command, atom);
                let left = bounds.x.max(0) as u32 / tiles::TILE_SIZE;
                let top = bounds.y.max(0) as u32 / tiles::TILE_SIZE;
                let right = (bounds.x as u32 + bounds.width - 1) / tiles::TILE_SIZE;
                let bottom = (bounds.y as u32 + bounds.height - 1) / tiles::TILE_SIZE;
                for y in top..=bottom {
                    for x in left..=right {
                        let tile = &mut self.tiles[y as usize * columns + x as usize];
                        hash.hash(&mut tile.hash);
                        if let GpuCommand::Landscape {
                            base,
                            liquid_mask,
                            liquid,
                            vertices,
                            ..
                        } = command
                        {
                            let sampled = textures::landscape_samples(scene, vertices, tile.bounds);
                            self.texture_regions.hash(*base, sampled, &mut tile.hash);
                            if let Some(mask) = liquid_mask {
                                let matching_extent = scene
                                    .texture(*base)
                                    .zip(scene.texture(*mask))
                                    .is_some_and(|(base, mask)| base.extent == mask.extent);
                                self.texture_regions.hash(
                                    *mask,
                                    matching_extent.then_some(sampled).flatten(),
                                    &mut tile.hash,
                                );
                            }
                            if let Some(liquid) = liquid {
                                self.texture_regions.hash(*liquid, None, &mut tile.hash);
                            }
                        }
                        tile.atoms.push(tiles::Atom {
                            command: index,
                            primitive: atom,
                        });
                    }
                }
            }
        }
        let row_bytes = width as usize * tiles::TILE_SIZE as usize * 4;
        let tables = &self.tables;
        self.stats = self
            .tiles
            .par_chunks_mut(columns)
            .zip(self.cached_pixels.par_chunks_mut(row_bytes))
            .enumerate()
            .map(|(row, (tiles, pixels))| {
                let mut stats = CpuSceneStats::default();
                for tile in tiles {
                    if raster_tile(
                        scene,
                        tile,
                        pixels,
                        row as u32 * tiles::TILE_SIZE,
                        tables,
                        resolve_monitor,
                    )? {
                        stats.rasterized_tiles += 1;
                    } else {
                        stats.reused_tiles += 1;
                    }
                }
                Ok(stats)
            })
            .try_reduce(CpuSceneStats::default, |a, b| {
                Ok(CpuSceneStats {
                    rasterized_tiles: a.rasterized_tiles + b.rasterized_tiles,
                    reused_tiles: a.reused_tiles + b.reused_tiles,
                })
            })?;
        frame.copy_from_slice(&self.cached_pixels);
        Ok(())
    }
}

struct RasterTarget<'a> {
    pixels: &'a mut [u8],
    bounds: Rect,
    tables: &'a SpanTables,
    row_origin: u32,
    gamma_override: Option<std::sync::Arc<[[u16; 256]; 3]>>,
    gamma_raw: Option<&'a [[f32; 256]; 3]>,
}

fn raster_tile(
    scene: &GpuScene,
    tile: &mut tiles::Tile,
    pixels: &mut [u8],
    row_origin: u32,
    tables: &SpanTables,
    resolve_monitor: bool,
) -> Result<bool, CpuSceneError> {
    let hash = tile.hash.finish();
    if tile.previous == Some(hash) {
        return Ok(false);
    }
    tile.previous = None;
    let width = scene.logical_extent[0] as usize;
    let mut target = RasterTarget {
        pixels,
        bounds: tile.bounds,
        tables,
        row_origin,
        gamma_raw: None,
        gamma_override: None,
    };
    for y in tile.bounds.y..tile.bounds.y + tile.bounds.height as i32 {
        let start = ((y as u32 - row_origin) as usize * width + tile.bounds.x as usize) * 4;
        for pixel in
            target.pixels[start..start + tile.bounds.width as usize * 4].chunks_exact_mut(4)
        {
            pixel.copy_from_slice(&[scene.clear.r, scene.clear.g, scene.clear.b, scene.clear.a]);
        }
    }
    for atom in &tile.atoms {
        execute_atom(
            scene,
            &mut target,
            &scene.commands[atom.command],
            atom.command,
            atom.primitive,
        )?;
    }
    if resolve_monitor && scene.gamma_mode.monitor_postpass() {
        for y in tile.bounds.y..tile.bounds.y + tile.bounds.height as i32 {
            let start = ((y as u32 - row_origin) as usize * width + tile.bounds.x as usize) * 4;
            for pixel in
                target.pixels[start..start + tile.bounds.width as usize * 4].chunks_exact_mut(4)
            {
                for (channel, value) in pixel.iter_mut().take(3).enumerate() {
                    *value = ((u32::from(scene.gamma.channels[channel][usize::from(*value)]) + 128)
                        / 257) as u8;
                }
            }
        }
    }
    tile.previous = Some(hash);
    Ok(true)
}

struct SpanTables {
    standard_encoding: bool,
    affine_copy: Option<spans::AffineGammaCopy>,
    gamma: Option<std::sync::Arc<[[u16; 256]; 3]>>,
    raw: [[f32; 256]; 3],
    encoded: [[u8; 256]; 3],
    products: [Vec<u8>; 3],
    cached: Vec<PreparedGammaTables>,
}

struct PreparedGammaTables {
    gamma: std::sync::Arc<[[u16; 256]; 3]>,
    raw: [[f32; 256]; 3],
    encoded: [[u8; 256]; 3],
    products: [Vec<u8>; 3],
    standard_encoding: bool,
    affine_copy: Option<spans::AffineGammaCopy>,
}

impl Default for SpanTables {
    fn default() -> Self {
        Self {
            standard_encoding: false,
            affine_copy: None,
            gamma: None,
            raw: [[0.0; 256]; 3],
            encoded: [[0; 256]; 3],
            products: std::array::from_fn(|_| Vec::new()),
            cached: Vec::new(),
        }
    }
}

impl SpanTables {
    fn prepared_raw(&self, channels: &std::sync::Arc<[[u16; 256]; 3]>) -> Option<&[[f32; 256]; 3]> {
        // Check every retained identity before bounded content comparisons.
        // Public revisions can be stale and never identify prepared contents.
        if self
            .gamma
            .as_ref()
            .is_some_and(|gamma| std::sync::Arc::ptr_eq(gamma, channels))
        {
            return Some(&self.raw);
        }
        if let Some(entry) = self
            .cached
            .iter()
            .find(|entry| std::sync::Arc::ptr_eq(&entry.gamma, channels))
        {
            return Some(&entry.raw);
        }
        if self.gamma.as_deref() == Some(channels.as_ref()) {
            return Some(&self.raw);
        }
        self.cached
            .iter()
            .find(|entry| entry.gamma == *channels)
            .map(|entry| &entry.raw)
    }
    fn update(&mut self, scene: &GpuScene) {
        if !scene.gamma_mode.fragment_lookup()
            || self.gamma.as_deref() == Some(scene.gamma.channels.as_ref())
        {
            return;
        }
        // Eight composed ramps cover the retained capture LUT cache. Keep
        // their prepared products as well: lightning must not rebuild three
        // 64 KiB tables on every alternating frame.
        let cached = self.cached.iter().position(|tables| {
            std::sync::Arc::ptr_eq(&tables.gamma, &scene.gamma.channels)
                || tables.gamma == scene.gamma.channels
        });
        let selected = cached.map(|index| self.cached.remove(index));
        if let Some(gamma) = self.gamma.take() {
            if self.cached.len() == 7 {
                self.cached.remove(0);
            }
            self.cached.push(PreparedGammaTables {
                gamma,
                raw: self.raw,
                encoded: self.encoded,
                products: std::mem::take(&mut self.products),
                standard_encoding: self.standard_encoding,
                affine_copy: self.affine_copy,
            });
        }
        if let Some(selected) = selected {
            self.gamma = Some(selected.gamma);
            self.raw = selected.raw;
            self.encoded = selected.encoded;
            self.products = selected.products;
            self.standard_encoding = selected.standard_encoding;
            self.affine_copy = selected.affine_copy;
            return;
        }
        for channel in 0..3 {
            for source in 0..256 {
                self.raw[channel][source] =
                    f32::from(scene.gamma.channels[channel][source]) / 257.0;
                self.encoded[channel][source] = store_channel(self.raw[channel][source]);
            }
            self.products[channel].resize(65536, 0);
            for modulation in 0..256 {
                for source in 0..256 {
                    let prepared = (source * modulation) as f32 / 255.0;
                    let index = ((prepared * 256.0 / 255.0) as usize).min(255);
                    self.products[channel][modulation * 256 + source] =
                        self.encoded[channel][index];
                }
            }
        }
        self.standard_encoding = (0..3).all(|channel| {
            (0..256).all(|source| {
                self.encoded[channel][source] == (source as u8).max(1)
                    && self.products[channel][255 * 256 + source] == self.encoded[channel][source]
            })
        });
        self.affine_copy = spans::AffineGammaCopy::from_encoded(&self.encoded);
        self.gamma = Some(scene.gamma.channels.clone());
    }
}
#[inline(always)]
fn standard_gamma_pixel(pixel: [u8; 4]) -> [u8; 4] {
    let value = u32::from_le_bytes(pixel);
    // Borrow propagation can also flag a byte equal to one. OR-ing its low
    // bit leaves it unchanged; alpha is excluded from the RGB mask.
    let zeroes = ((value.wrapping_sub(0x0101_0101) & !value & 0x8080_8080) >> 7) & 0x0001_0101;
    (value | zeroes).to_le_bytes()
}
impl std::ops::Deref for RasterTarget<'_> {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.pixels
    }
}
impl std::ops::DerefMut for RasterTarget<'_> {
    fn deref_mut(&mut self) -> &mut [u8] {
        self.pixels
    }
}

fn execute_atom(
    scene: &GpuScene,
    frame: &mut RasterTarget<'_>,
    command: &GpuCommand,
    index: usize,
    atom: usize,
) -> Result<(), CpuSceneError> {
    frame.gamma_override = None;
    frame.gamma_raw = None;
    match command {
        GpuCommand::Solid {
            vertices,
            topology,
            alpha_mode,
            clip,
            blend,
            style,
        } => {
            let bounds = frame.bounds;
            let bounds = match clip {
                Some(clip) => match bounds.intersection(*clip) {
                    Some(bounds) => bounds,
                    None => return Ok(()),
                },
                None => bounds,
            };
            let size = tiles::primitive_size(*topology);
            let vertices = &vertices[atom * size..(atom + 1) * size];
            match topology {
                GpuPrimitiveTopology::PointList => {
                    for vertex in vertices {
                        let [x, y] = position(vertex.position)?;
                        put_solid(
                            scene,
                            frame,
                            bounds,
                            x.floor() as i32,
                            y.floor() as i32,
                            vertex.color,
                            *blend,
                            *alpha_mode,
                            style.gamma,
                            style.software_blend,
                        );
                    }
                }
                GpuPrimitiveTopology::TriangleList => {
                    for triangle in vertices.chunks_exact(3) {
                        draw_solid_triangle(
                            scene,
                            frame,
                            bounds,
                            triangle,
                            *blend,
                            *alpha_mode,
                            style.gamma,
                            style.software_blend,
                        )?;
                    }
                }
                GpuPrimitiveTopology::LineList => {
                    for line in vertices.chunks_exact(2) {
                        draw_solid_line(
                            scene,
                            frame,
                            bounds,
                            line,
                            *blend,
                            *alpha_mode,
                            style.gamma,
                            style.software_blend,
                            style.software_line_bounds,
                        )?;
                    }
                }
            }
        }
        GpuCommand::Quad {
            texture,
            vertices,
            clip,
            blend,
            base_mod2,
            sampler,
            gamma,
            owner_mask: None,
            ..
        } => {
            let texture = scene
                .texture(*texture)
                .filter(|resource| resource.is_valid())
                .ok_or(CpuSceneError::UnsupportedCommand(index))?;
            draw_quad(
                scene, frame, vertices, *clip, texture, *sampler, *blend, *base_mod2, *gamma, None,
            )?;
        }
        GpuCommand::SpriteBatch {
            texture,
            quads,
            clip,
            blend,
            mod2,
            gamma,
            outer_modulation,
        } => {
            let texture = scene
                .texture(*texture)
                .filter(|resource| resource.is_valid())
                .ok_or(CpuSceneError::UnsupportedCommand(index))?;
            for quad in &quads[atom..atom + 1] {
                let positions = [
                    [quad.rect[0], quad.rect[1], 1.0],
                    [quad.rect[2], quad.rect[1], 1.0],
                    [quad.rect[0], quad.rect[3], 1.0],
                    [quad.rect[2], quad.rect[3], 1.0],
                ];
                let vertices =
                    sprite_vertices(positions, quad.uv, [quad.modulation; 4], *outer_modulation)
                        .map(|mut vertex| {
                            vertex.software_sprite = quad.software_sprite;
                            vertex.software_shader = quad.software_shader;
                            vertex
                        });
                draw_quad(
                    scene,
                    frame,
                    &vertices,
                    *clip,
                    texture,
                    GpuSampler::Nearest,
                    *blend,
                    *mod2,
                    *gamma,
                    None,
                )?;
            }
        }
        GpuCommand::ObjectBatch {
            texture,
            owner_texture,
            sprites,
            clip,
            blend,
            gamma,
        } => {
            for sprite in &sprites[atom..atom + 1] {
                let texture_id = if sprite.owner_layer() {
                    owner_texture.ok_or(CpuSceneError::UnsupportedCommand(index))?
                } else {
                    *texture
                };
                let texture = scene
                    .texture(texture_id)
                    .filter(|resource| resource.is_valid())
                    .ok_or(CpuSceneError::UnsupportedCommand(index))?;
                let mut vertices = sprite_vertices(
                    sprite.positions,
                    sprite.uv,
                    sprite.modulation,
                    sprite.outer_modulation(),
                );
                for vertex in &mut vertices {
                    vertex.software_shader = sprite.software_shader();
                    vertex.software_sprite = sprite.software_sprite();
                }
                if sprite.sample_tile_size > 0.0 {
                    let size = sprite.sample_tile_size;
                    let tile = [0.0, 0.0, size, 1.0];
                    for vertex in &mut vertices {
                        vertex.sample_tile = tile;
                    }
                }
                draw_quad(
                    scene,
                    frame,
                    &vertices,
                    *clip,
                    texture,
                    sprite.sampler(),
                    *blend,
                    sprite.mod2(),
                    *gamma,
                    None,
                )?;
            }
        }
        GpuCommand::Landscape {
            base,
            liquid_mask,
            liquid,
            vertices,
            clip,
            phase,
            gamma,
        } => {
            let texture = scene
                .texture(*base)
                .filter(|resource| resource.is_valid())
                .ok_or(CpuSceneError::UnsupportedCommand(index))?;
            let mask = liquid_mask
                .map(|id| {
                    scene
                        .texture(id)
                        .filter(|resource| resource.is_valid())
                        .ok_or(CpuSceneError::UnsupportedCommand(index))
                })
                .transpose()?;
            let liquid = liquid
                .map(|id| {
                    scene
                        .texture(id)
                        .filter(|resource| resource.is_valid())
                        .ok_or(CpuSceneError::UnsupportedCommand(index))
                })
                .transpose()?;
            draw_quad(
                scene,
                frame,
                vertices,
                *clip,
                texture,
                GpuSampler::Nearest,
                GpuBlend::Normal,
                false,
                *gamma,
                Some(LandscapeSample {
                    mask,
                    liquid,
                    phase: *phase,
                }),
            )?;
        }
        _ => return Err(CpuSceneError::UnsupportedCommand(index)),
    }
    Ok(())
}

fn validate_scene(scene: &GpuScene) -> Result<(), CpuSceneError> {
    for (index, texture) in scene.textures.iter().enumerate() {
        if !texture.is_valid()
            || scene.textures[..index]
                .iter()
                .any(|previous| previous.id == texture.id)
        {
            return Err(CpuSceneError::InvalidFrame);
        }
        if !texture.dirty.is_empty() && texture.base_revision == Some(texture.revision)
            || texture.dirty.iter().any(|rect| {
                rect.x < 0
                    || rect.y < 0
                    || i64::from(rect.x) + i64::from(rect.width) > i64::from(texture.extent[0])
                    || i64::from(rect.y) + i64::from(rect.height) > i64::from(texture.extent[1])
            })
        {
            return Err(CpuSceneError::InvalidFrame);
        }
    }
    for (index, command) in scene.commands.iter().enumerate() {
        let require_texture = |id, format| -> Result<(), CpuSceneError> {
            scene
                .texture(id)
                .filter(|texture| texture.format == format)
                .map(|_| ())
                .ok_or(CpuSceneError::UnsupportedCommand(index))
        };
        let check_vertices = |vertices: &[GpuVertex]| -> Result<(), CpuSceneError> {
            for vertex in vertices {
                position(vertex.position)?;
                if !vertex
                    .uv
                    .into_iter()
                    .chain(vertex.modulation)
                    .chain(vertex.sample_tile)
                    .all(f32::is_finite)
                    || vertex.sample_tile[3] > 0.0 && vertex.sample_tile[2] < 1.0
                    || vertex
                        .software_sprite
                        .is_some_and(|id| scene.software_sprite(id).is_none())
                {
                    return Err(CpuSceneError::UnsupportedCommand(index));
                }
                if let Some(blit) = vertex.software_blit {
                    let valid_mapping = match blit.mapping {
                        crate::GpuSoftwareBlitMapping::Unscaled(_) => true,
                        crate::GpuSoftwareBlitMapping::Stretched(destination) => {
                            destination.width != 0 && destination.height != 0
                        }
                        crate::GpuSoftwareBlitMapping::Transformed { inverse, .. } => {
                            inverse.mat.iter().all(|value| value.is_finite())
                        }
                    };
                    if blit.source.width == 0
                        || blit.source.height == 0
                        || !blit.translation.iter().all(|value| value.is_finite())
                        || !valid_mapping
                    {
                        return Err(CpuSceneError::UnsupportedCommand(index));
                    }
                }
            }
            Ok(())
        };
        match command {
            GpuCommand::Quad {
                texture,
                vertices,
                owner_mask,
                ..
            } => {
                if owner_mask.is_some() {
                    return Err(CpuSceneError::UnsupportedCommand(index));
                }
                require_texture(*texture, GpuTextureFormat::Rgba8)?;
                check_vertices(vertices)?;
            }
            GpuCommand::Landscape {
                base,
                liquid_mask,
                liquid,
                vertices,
                phase,
                ..
            } => {
                require_texture(*base, GpuTextureFormat::Rgba8)?;
                if liquid_mask.is_some() != liquid.is_some() {
                    return Err(CpuSceneError::UnsupportedCommand(index));
                }
                if let Some(mask) = liquid_mask {
                    require_texture(*mask, GpuTextureFormat::R8)?;
                }
                if let Some(liquid) = liquid {
                    require_texture(*liquid, GpuTextureFormat::Rgba8)?;
                }
                check_vertices(vertices)?;
                if !phase.iter().all(|value| value.is_finite()) {
                    return Err(CpuSceneError::UnsupportedCommand(index));
                }
            }
            GpuCommand::ObjectBatch {
                texture,
                sprites,
                owner_texture,
                ..
            } => {
                if sprites.is_empty() {
                    continue;
                }
                require_texture(*texture, GpuTextureFormat::Rgba8)?;
                if let Some(owner) = owner_texture {
                    require_texture(*owner, GpuTextureFormat::Rgba8)?;
                }
                for sprite in sprites {
                    for vertex in sprite.positions {
                        position(vertex)?;
                    }
                    if !sprite.has_valid_packed_flags()
                        || !sprite.uv.iter().all(|value| value.is_finite())
                        || !sprite.sample_tile_size.is_finite()
                        || sprite.sample_tile_size > 0.0 && sprite.sample_tile_size < 1.0
                        || sprite.owner_layer() && owner_texture.is_none()
                        || sprite
                            .software_sprite()
                            .is_some_and(|id| scene.software_sprite(id).is_none())
                    {
                        return Err(CpuSceneError::UnsupportedCommand(index));
                    }
                }
            }
            GpuCommand::SpriteBatch { texture, quads, .. } => {
                if quads.is_empty() {
                    continue;
                }
                require_texture(*texture, GpuTextureFormat::Rgba8)?;
                if quads.iter().any(|quad| {
                    !quad
                        .rect
                        .iter()
                        .chain(quad.uv.iter())
                        .all(|value| value.is_finite())
                        || quad
                            .software_sprite
                            .is_some_and(|id| scene.software_sprite(id).is_none())
                }) {
                    return Err(CpuSceneError::UnsupportedCommand(index));
                }
            }
            GpuCommand::Solid {
                vertices,
                topology,
                style,
                ..
            } => {
                if vertices.len() % tiles::primitive_size(*topology) != 0 {
                    return Err(CpuSceneError::UnsupportedCommand(index));
                }
                if style
                    .software_line_bounds
                    .is_some_and(|bounds| bounds.width == 0 || bounds.height == 0)
                {
                    return Err(CpuSceneError::UnsupportedCommand(index));
                }
                for vertex in vertices {
                    position(vertex.position)?;
                    if !vertex.color.iter().all(|value| value.is_finite()) {
                        return Err(CpuSceneError::UnsupportedCommand(index));
                    }
                }
            }
        }
    }
    for sprite in &scene.software_sprites {
        if !sprite
            .destination
            .into_iter()
            .chain(sprite.source)
            .chain(sprite.inverse.mat)
            .chain(sprite.translation)
            .all(f32::is_finite)
            || sprite.destination[2] <= 0.0
            || sprite.destination[3] <= 0.0
            || sprite.source[2] <= 0.0
            || sprite.source[3] <= 0.0
            || sprite.mapping == crate::GpuSoftwareSpriteMapping::IntegerStretch
                && (sprite.destination[2] as i64 == 0 || sprite.destination[3] as i64 == 0)
        {
            return Err(CpuSceneError::InvalidFrame);
        }
        if let crate::GpuSoftwareSpriteMapping::Landscape {
            zoom,
            world_extent,
            tile_origin,
            texture_size,
            indent,
        } = sprite.mapping
        {
            if !zoom.is_finite()
                || zoom <= 0.0
                || !indent.is_finite()
                || texture_size == 0
                || texture_size > i32::MAX as u32
                || world_extent
                    .iter()
                    .any(|extent| *extent == 0 || *extent > i32::MAX as u32)
                || tile_origin.iter().any(|origin| *origin > i32::MAX as u32)
            {
                return Err(CpuSceneError::InvalidFrame);
            }
        }
        if let crate::GpuSoftwareSpriteMapping::RotatedCorner { center, cos, sin } = sprite.mapping
        {
            if !center.into_iter().chain([cos, sin]).all(f32::is_finite) {
                return Err(CpuSceneError::InvalidFrame);
            }
        }
        if let crate::GpuSoftwareSpriteMapping::Font {
            shear,
            center_y,
            texture_indent,
            physical_size,
            ..
        } = sprite.mapping
        {
            if ![shear, center_y, texture_indent, physical_size]
                .into_iter()
                .all(f32::is_finite)
                || physical_size < 1.0
                || physical_size + 2.0 * texture_indent == 0.0
                || sprite.source[2] < 1.0
                || sprite.source[3] < 1.0
            {
                return Err(CpuSceneError::InvalidFrame);
            }
        }
        if let Some(fog) = sprite.fog {
            if !fog
                .destination
                .into_iter()
                .chain(fog.source_range)
                .all(f32::is_finite)
                || fog.destination[2] <= 0.0
                || fog.destination[3] <= 0.0
                || fog.source_range[0] < 0.0
                || fog.source_range[1] < 0.0
                || fog.source_range[2] <= fog.source_range[0]
                || fog.source_range[3] <= fog.source_range[1]
                || fog.source_range[2] > sprite.source[2]
                || fog.source_range[3] > sprite.source[3]
            {
                return Err(CpuSceneError::InvalidFrame);
            }
        }
    }
    Ok(())
}

fn frame_key(scene: &GpuScene) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    scene.logical_extent.hash(&mut hash);
    [scene.clear.r, scene.clear.g, scene.clear.b, scene.clear.a].hash(&mut hash);
    (scene.gamma_mode as u8).hash(&mut hash);
    scene.gamma.channels.hash(&mut hash);
    hash.finish()
}

fn position(position: [f32; 3]) -> Result<[f32; 2], CpuSceneError> {
    let result = [position[0] / position[2], position[1] / position[2]];
    if result.iter().all(|coordinate| coordinate.is_finite()) {
        Ok(result)
    } else {
        Err(CpuSceneError::InvalidFrame)
    }
}

fn edge(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])
}

fn top_left(a: [f32; 2], b: [f32; 2]) -> bool {
    b[1] < a[1] || (b[1] == a[1] && b[0] > a[0])
}

#[allow(clippy::too_many_arguments)]
fn draw_solid_line(
    scene: &GpuScene,
    frame: &mut RasterTarget<'_>,
    bounds: Rect,
    vertices: &[GpuSolidVertex],
    blend: GpuBlend,
    alpha_mode: GpuSolidAlphaMode,
    gamma: bool,
    software_blend: GpuSoftwareBlend,
    line_bounds: Option<Rect>,
) -> Result<(), CpuSceneError> {
    let start = position(vertices[0].position)?.map(|coordinate| coordinate - 0.5);
    let end = position(vertices[1].position)?.map(|coordinate| coordinate - 0.5);
    if start == end {
        return Ok(());
    }
    if software_blend == GpuSoftwareBlend::GuiBox {
        // Compatibility GUI lines select their inclusive rectangle before
        // target clipping. Clipping must not invent an excluded endpoint.
        let [x1, y1] = start.map(|value| value as i32);
        let [x2, y2] = end.map(|value| value as i32);
        let [left, top, right, bottom] = if y1 == y2 {
            if x2 > x1 {
                [x1, y1, x2, y1 + 1]
            } else {
                [x2 + 1, y1, x1 + 1, y1 + 1]
            }
        } else if x1 == x2 {
            if y2 > y1 {
                [x1, y1, x1 + 1, y2]
            } else {
                [x1, y2 + 1, x1 + 1, y1 + 1]
            }
        } else {
            [x1, y1, x2.saturating_add(1), y2.saturating_add(1)]
        };
        for y in top.max(bounds.y)..bottom.min(bounds.y.saturating_add_unsigned(bounds.height)) {
            for x in left.max(bounds.x)..right.min(bounds.x.saturating_add_unsigned(bounds.width)) {
                put_solid(
                    scene,
                    frame,
                    bounds,
                    x,
                    y,
                    vertices[0].color,
                    blend,
                    alpha_mode,
                    gamma,
                    software_blend,
                );
            }
        }
        return Ok(());
    }
    let origin = start.map(f64::from);
    let direction = [f64::from(end[0]) - origin[0], f64::from(end[1]) - origin[1]];
    let mut enter = 0.0f64;
    let mut exit = 1.0f64;
    let line_bounds = line_bounds.unwrap_or(Rect::new(
        0,
        0,
        scene.logical_extent[0],
        scene.logical_extent[1],
    ));
    let clip = [
        (-direction[0], origin[0] - f64::from(line_bounds.x)),
        (
            direction[0],
            f64::from(line_bounds.x) + f64::from(line_bounds.width - 1) - origin[0],
        ),
        (-direction[1], origin[1] - f64::from(line_bounds.y)),
        (
            direction[1],
            f64::from(line_bounds.y) + f64::from(line_bounds.height - 1) - origin[1],
        ),
    ];
    for (direction, distance) in clip {
        if direction.abs() <= f64::EPSILON {
            if distance < 0.0 {
                return Ok(());
            }
            continue;
        }
        let ratio = distance / direction;
        if direction < 0.0 {
            enter = enter.max(ratio);
        } else {
            exit = exit.min(ratio);
        }
        if enter > exit {
            return Ok(());
        }
    }
    let clipped_start: [i64; 2] =
        std::array::from_fn(|i| ((origin[i] + enter * direction[i]) as f32).round() as i64);
    let clipped_end: [i64; 2] =
        std::array::from_fn(|i| ((origin[i] + exit * direction[i]) as f32).round() as i64);
    let [mut x, mut y] = clipped_start;
    let [last_x, last_y] = clipped_end;
    let dx = (last_x - x).abs();
    let dy = -(last_y - y).abs();
    let step_x = if x < last_x { 1 } else { -1 };
    let step_y = if y < last_y { 1 } else { -1 };
    let mut error = dx + dy;
    let flat = vertices[0].color == vertices[1].color;
    loop {
        let color = if flat {
            vertices[0].color.map(|value| value * 255.0)
        } else {
            let delta = [end[0] - start[0], end[1] - start[1]];
            let length = delta[0] * delta[0] + delta[1] * delta[1];
            let offset = [x as f32 - start[0], y as f32 - start[1]];
            let amount = if length <= f32::EPSILON {
                0.0
            } else {
                ((offset[0] * delta[0] + offset[1] * delta[1]) / length).clamp(0.0, 1.0)
            };
            std::array::from_fn(|i| {
                let start = (vertices[0].color[i] * 255.0).round();
                let end = (vertices[1].color[i] * 255.0).round();
                (start + (end - start) * amount).round()
            })
        };
        put_fragment(
            scene,
            frame,
            bounds,
            x as i32,
            y as i32,
            color,
            blend,
            alpha_mode,
            gamma,
            software_blend,
        );
        if x == last_x && y == last_y {
            break;
        }
        let doubled = error * 2;
        if doubled >= dy {
            error += dy;
            x += step_x;
        }
        if doubled <= dx {
            error += dx;
            y += step_y;
        }
        if x == last_x && y == last_y {
            break;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn draw_solid_triangle(
    scene: &GpuScene,
    frame: &mut RasterTarget<'_>,
    bounds: Rect,
    vertices: &[GpuSolidVertex],
    blend: GpuBlend,
    alpha_mode: GpuSolidAlphaMode,
    gamma: bool,
    software_blend: GpuSoftwareBlend,
) -> Result<(), CpuSceneError> {
    let mut vertices = [vertices[0], vertices[1], vertices[2]];
    let mut positions = [
        position(vertices[0].position)?,
        position(vertices[1].position)?,
        position(vertices[2].position)?,
    ];
    let mut area = edge(positions[0], positions[1], positions[2]);
    if area == 0.0 {
        return Ok(());
    }
    if area < 0.0 {
        positions.swap(1, 2);
        vertices.swap(1, 2);
        area = -area;
    }
    let min_x = positions
        .iter()
        .map(|p| p[0])
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(bounds.x as f32) as i32;
    let min_y = positions
        .iter()
        .map(|p| p[1])
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(bounds.y as f32) as i32;
    let max_x = positions
        .iter()
        .map(|p| p[0])
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(bounds.x as f32 + bounds.width as f32) as i32;
    let max_y = positions
        .iter()
        .map(|p| p[1])
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(bounds.y as f32 + bounds.height as f32) as i32;
    for y in min_y..max_y {
        for x in min_x..max_x {
            let p = [x as f32 + 0.5, y as f32 + 0.5];
            let edges = [
                edge(positions[1], positions[2], p),
                edge(positions[2], positions[0], p),
                edge(positions[0], positions[1], p),
            ];
            let included = edges.iter().enumerate().all(|(i, value)| {
                *value > 0.0
                    || (*value == 0.0 && top_left(positions[(i + 1) % 3], positions[(i + 2) % 3]))
            });
            if !included {
                continue;
            }
            let color = if vertices[0].color == vertices[1].color
                && vertices[0].color == vertices[2].color
            {
                vertices[0].color
            } else {
                std::array::from_fn(|channel| {
                    vertices[0].color[channel] * (edges[0] / area)
                        + vertices[1].color[channel] * (edges[1] / area)
                        + vertices[2].color[channel] * (edges[2] / area)
                })
            };
            put_solid(
                scene,
                frame,
                bounds,
                x,
                y,
                color,
                blend,
                alpha_mode,
                gamma,
                software_blend,
            );
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn put_solid(
    scene: &GpuScene,
    frame: &mut RasterTarget<'_>,
    bounds: Rect,
    x: i32,
    y: i32,
    color: [f32; 4],
    blend: GpuBlend,
    alpha_mode: GpuSolidAlphaMode,
    gamma: bool,
    software_blend: GpuSoftwareBlend,
) {
    put_fragment(
        scene,
        frame,
        bounds,
        x,
        y,
        color.map(|value| value * 255.0),
        blend,
        alpha_mode,
        gamma,
        software_blend,
    );
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn put_fragment(
    scene: &GpuScene,
    frame: &mut RasterTarget<'_>,
    bounds: Rect,
    x: i32,
    y: i32,
    source: [f32; 4],
    blend: GpuBlend,
    alpha_mode: GpuSolidAlphaMode,
    gamma: bool,
    software_blend: GpuSoftwareBlend,
) {
    if x < bounds.x
        || y < bounds.y
        || i64::from(x) >= i64::from(bounds.x) + i64::from(bounds.width)
        || i64::from(y) >= i64::from(bounds.y) + i64::from(bounds.height)
    {
        return;
    }
    let offset = ((y as u32 - frame.row_origin) as usize * scene.logical_extent[0] as usize
        + x as usize)
        * 4;
    if software_blend == GpuSoftwareBlend::GuiBox {
        let alpha = source[3] / 255.0;
        for channel in 0..3 {
            let encoded = if gamma && scene.gamma_mode.fragment_lookup() {
                let index = ((source[channel].clamp(0.0, 255.0) * 256.0 / 255.0) as usize).min(255);
                f32::from(store_channel(
                    f32::from(scene.gamma.channels[0][index]) / 257.0,
                ))
            } else {
                source[channel]
            };
            frame[offset + channel] =
                store_channel(encoded * alpha + f32::from(frame[offset + channel]) * (1.0 - alpha));
        }
        frame[offset + 3] =
            store_channel(255.0 * alpha + f32::from(frame[offset + 3]) * (1.0 - alpha));
        return;
    }
    if software_blend == GpuSoftwareBlend::Legacy && !gamma {
        let source = source.map(store_channel);
        let source = Color::new(source[0], source[1], source[2], source[3]);
        let dest = Color::new(
            frame[offset],
            frame[offset + 1],
            frame[offset + 2],
            frame[offset + 3],
        );
        let result = match blend {
            GpuBlend::Normal => source.blend_over(dest),
            GpuBlend::Additive => source.blend_additive(dest),
            GpuBlend::Replace => source,
        };
        frame[offset..offset + 4].copy_from_slice(&[result.r, result.g, result.b, result.a]);
        return;
    }
    #[cfg(test)]
    let specialized_fragments = !FORCE_GENERIC_SPANS.with(std::cell::Cell::get);
    #[cfg(not(test))]
    let specialized_fragments = true;
    if specialized_fragments
        && blend == GpuBlend::Normal
        && alpha_mode == GpuSolidAlphaMode::SourceOver
        && software_blend == GpuSoftwareBlend::Shader
    {
        let mut prepared = source;
        if gamma {
            let channels = frame.gamma_override.as_deref().or_else(|| {
                scene
                    .gamma_mode
                    .fragment_lookup()
                    .then_some(scene.gamma.channels.as_ref())
            });
            if let Some(channels) = channels {
                let raw = frame.gamma_raw.or_else(|| {
                    (frame.gamma_override.is_none() && scene.gamma_mode.fragment_lookup())
                        .then(|| frame.tables.gamma.as_ref().map(|_| &frame.tables.raw))
                        .flatten()
                });
                for channel in 0..3 {
                    let index =
                        ((source[channel].clamp(0.0, 255.0) * 256.0 / 255.0) as usize).min(255);
                    prepared[channel] = raw.map_or_else(
                        || {
                            #[cfg(test)]
                            SHADER_GAMMA_DIVISIONS.with(|count| count.set(count.get() + 1));
                            f32::from(channels[channel][index]) / 257.0
                        },
                        |raw| raw[channel][index],
                    );
                }
            }
        }
        let destination = [
            frame[offset],
            frame[offset + 1],
            frame[offset + 2],
            frame[offset + 3],
        ];
        let output = spans::blend_shader_source_over(prepared, destination);
        frame[offset..offset + 4].copy_from_slice(&output);
        return;
    }
    let alpha = (source[3] / 255.0).clamp(0.0, 1.0);
    for channel in 0..4 {
        let value = if channel < 3 && gamma {
            let channels = frame.gamma_override.as_deref().or_else(|| {
                scene
                    .gamma_mode
                    .fragment_lookup()
                    .then_some(scene.gamma.channels.as_ref())
            });
            channels.map_or(source[channel], |channels| {
                let index = ((source[channel].clamp(0.0, 255.0) * 256.0 / 255.0) as usize).min(255);
                f32::from(channels[channel][index]) / 257.0
            })
        } else {
            source[channel]
        };
        let dest = f32::from(frame[offset + channel]);
        let value = match blend {
            GpuBlend::Replace => value,
            GpuBlend::Additive if channel == 3 => dest,
            GpuBlend::Additive => dest + value * alpha,
            GpuBlend::Normal if channel == 3 && alpha_mode != GpuSolidAlphaMode::NonSeparate => {
                value + dest * (1.0 - alpha)
            }
            GpuBlend::Normal => value * alpha + dest * (1.0 - alpha),
        };
        frame[offset + channel] = if channel == 3 && software_blend != GpuSoftwareBlend::Shader {
            let source = Color::new(0, 0, 0, source[3] as u8);
            let dest = Color::new(0, 0, 0, frame[offset + 3]);
            match blend {
                GpuBlend::Normal => source.blend_over(dest).a,
                GpuBlend::Additive => dest.a,
                GpuBlend::Replace => source.a,
            }
        } else {
            store_channel(value)
        };
    }
}

// All framebuffer channels are nonnegative. Avoid the generic floating-point
// round libcall while preserving its half-up result, including values just
// below a half-integer (adding 0.5 first can round those across the boundary).
#[inline(always)]
fn store_channel(value: f32) -> u8 {
    if value <= 0.0 || value.is_nan() {
        return 0;
    }
    if value >= 255.0 {
        return 255;
    }
    let integer = value as u8;
    integer + u8::from(value - f32::from(integer) >= 0.5)
}

fn sprite_vertices(
    positions: [[f32; 3]; 4],
    uv: [f32; 4],
    modulation: [u32; 4],
    outer: crate::GpuOuterModulation,
) -> [GpuVertex; 4] {
    let uv = [
        [uv[0], uv[1]],
        [uv[2], uv[1]],
        [uv[0], uv[3]],
        [uv[2], uv[3]],
    ];
    std::array::from_fn(|i| {
        let packed = modulation[i];
        let modulation = [
            ((packed >> 16) & 255) as f32 / 255.0,
            ((packed >> 8) & 255) as f32 / 255.0,
            (packed & 255) as f32 / 255.0,
            (packed >> 24) as f32 / 255.0,
        ];
        GpuVertex::new(positions[i], uv[i], modulation).with_outer_modulation(outer)
    })
}

#[derive(Clone, Copy)]
struct LandscapeSample<'a> {
    mask: Option<&'a GpuTextureResource>,
    liquid: Option<&'a GpuTextureResource>,
    phase: [f32; 3],
}

#[allow(clippy::too_many_arguments)]
fn draw_quad(
    scene: &GpuScene,
    frame: &mut RasterTarget<'_>,
    vertices: &[GpuVertex; 4],
    clip: Option<Rect>,
    texture: &GpuTextureResource,
    sampler: GpuSampler,
    blend: GpuBlend,
    mod2: bool,
    gamma: bool,
    landscape: Option<LandscapeSample<'_>>,
) -> Result<(), CpuSceneError> {
    let mut positions = [[0.0; 2]; 4];
    for (target, vertex) in positions.iter_mut().zip(vertices) {
        *target = position(vertex.position)?;
    }
    let bounds = frame.bounds;
    let bounds = match clip {
        Some(clip) => match bounds.intersection(clip) {
            Some(bounds) => bounds,
            None => return Ok(()),
        },
        None => bounds,
    };
    let min_x = positions
        .iter()
        .map(|p| p[0])
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(bounds.x as f32) as i32;
    let min_y = positions
        .iter()
        .map(|p| p[1])
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(bounds.y as f32) as i32;
    let max_x = positions
        .iter()
        .map(|p| p[0])
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(bounds.x as f32 + bounds.width as f32) as i32;
    let max_y = positions
        .iter()
        .map(|p| p[1])
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil()
        .min(bounds.y as f32 + bounds.height as f32) as i32;
    if min_x >= max_x || min_y >= max_y {
        return Ok(());
    }
    let axis_aligned = positions[0][0] == positions[2][0]
        && positions[1][0] == positions[3][0]
        && positions[0][1] == positions[1][1]
        && positions[2][1] == positions[3][1]
        && positions[1][0] > positions[0][0]
        && positions[2][1] > positions[0][1]
        && vertices
            .iter()
            .all(|v| v.position[2] == vertices[0].position[2]);
    let software = vertices[0]
        .software_sprite
        .and_then(|id| scene.software_sprite(id));
    let explicit_gamma = software.and_then(|software| software.gamma.as_ref());
    frame.gamma_override = explicit_gamma.map(|gamma| std::sync::Arc::clone(&gamma.channels));
    frame.gamma_raw = if !gamma {
        None
    } else if let Some(gamma) = explicit_gamma {
        frame.tables.prepared_raw(&gamma.channels)
    } else if scene.gamma_mode.fragment_lookup() {
        frame.tables.gamma.as_ref().map(|_| &frame.tables.raw)
    } else {
        None
    };
    let gamma_needs_generic = gamma
        && explicit_gamma.is_some_and(|gamma| {
            !scene.gamma_mode.fragment_lookup()
                || !(std::sync::Arc::ptr_eq(&gamma.channels, &scene.gamma.channels)
                    || gamma.channels == scene.gamma.channels)
        });
    #[cfg(test)]
    let specialized_spans = !FORCE_GENERIC_SPANS.with(std::cell::Cell::get);
    #[cfg(not(test))]
    let specialized_spans = true;
    if specialized_spans
        && axis_aligned
        && sampler == GpuSampler::Linear
        && blend == GpuBlend::Normal
        && !mod2
        && landscape.is_none()
        && vertices[0].software_blit.is_none()
    {
        if let Some(software) = software {
            if draw_gui_linear_span(
                scene,
                frame,
                vertices,
                texture,
                software,
                gamma,
                [min_x, min_y, max_x, max_y],
            ) {
                return Ok(());
            }
        }
    }
    if specialized_spans
        && blend == GpuBlend::Normal
        && !mod2
        && landscape.is_none()
        && (sampler == GpuSampler::Linear
            || !axis_aligned
            || software.is_some_and(|software| {
                matches!(
                    software.mapping,
                    crate::GpuSoftwareSpriteMapping::RotatedCorner { .. }
                )
            }))
        && vertices[0].software_blit.is_none()
    {
        if let Some(software) = software {
            if draw_black_sprite_span(
                scene,
                frame,
                vertices,
                texture,
                software,
                sampler,
                gamma,
                [min_x, min_y, max_x, max_y],
            ) {
                return Ok(());
            }
        }
    }
    if let Some((software, landscape)) = software
        .zip(landscape)
        .filter(|_| specialized_spans && !gamma_needs_generic)
    {
        if draw_landscape_span(
            scene,
            frame,
            vertices,
            texture,
            software,
            landscape,
            gamma,
            [min_x, min_y, max_x, max_y],
        ) {
            return Ok(());
        }
    }
    if specialized_spans
        && sampler == GpuSampler::Nearest
        && blend == GpuBlend::Normal
        && !mod2
        && landscape.is_none()
        && axis_aligned
        && vertices[0].software_blit.is_none()
    {
        if let Some(software) = software {
            if draw_nearest_sprite_span(
                scene,
                frame,
                vertices,
                texture,
                software,
                gamma,
                !gamma_needs_generic,
                [min_x, min_y, max_x, max_y],
            ) {
                return Ok(());
            }
        }
    }
    let identity_inverse =
        software.is_some_and(|software| software.inverse == crate::Transform::identity());
    if specialized_spans
        && !gamma_needs_generic
        && sampler == GpuSampler::Nearest
        && texture.format == GpuTextureFormat::Rgba8
        && axis_aligned
        && identity_inverse
        && software
            .is_some_and(|software| software.mapping == crate::GpuSoftwareSpriteMapping::TileCopy)
    {
        let software = software.ok_or(CpuSceneError::InvalidFrame)?;
        for y in min_y..max_y {
            let source_y = (y as f32 - software.translation[1] - software.destination[1]
                + software.source[1]) as i32;
            if !(0..texture.extent[1] as i32).contains(&source_y) {
                continue;
            }
            let source_x = (min_x as f32 - software.translation[0] - software.destination[0]
                + software.source[0]) as i32;
            if source_x < 0 || source_x as u32 + (max_x - min_x) as u32 > texture.extent[0] {
                return Err(CpuSceneError::InvalidFrame);
            }
            let source_start =
                (source_y as usize * texture.extent[0] as usize + source_x as usize) * 4;
            let target_start = ((y as u32 - frame.row_origin) as usize
                * scene.logical_extent[0] as usize
                + min_x as usize)
                * 4;
            let count = (max_x - min_x) as usize * 4;
            let source = &texture.pixels[source_start..source_start + count];
            let destination = &mut frame.pixels[target_start..target_start + count];
            if gamma && scene.gamma_mode.fragment_lookup() {
                if frame.tables.standard_encoding {
                    spans::copy_standard_gamma(source, destination);
                } else if let Some(affine) = &frame.tables.affine_copy {
                    affine.copy(source, destination);
                } else {
                    for (destination, source) in
                        destination.chunks_exact_mut(4).zip(source.chunks_exact(4))
                    {
                        for channel in 0..3 {
                            destination[channel] =
                                frame.tables.encoded[channel][usize::from(source[channel])];
                        }
                        destination[3] = source[3];
                    }
                }
            } else {
                destination.copy_from_slice(source);
            }
        }
        return Ok(());
    }
    let corner_modulation =
        vertices.map(|vertex| vertex.modulation.map(|value| (value * 255.0).round()));
    let flat_modulation = corner_modulation
        .iter()
        .all(|corner| *corner == corner_modulation[0]);
    let flat_bytes = corner_modulation[0].map(store_channel);
    let unmodulated_nearest = !gamma_needs_generic
        && sampler == GpuSampler::Nearest
        && !mod2
        && vertices.iter().all(|vertex| {
            vertex.outer_modulation == crate::GpuOuterModulation::Inherit
                && vertex.modulation == [1.0, 1.0, 1.0, 0.0]
        });
    for y in min_y..max_y {
        for x in min_x..max_x {
            let point = [x as f32 + 0.5, y as f32 + 0.5];
            let native_coordinates = if let Some(software) = software {
                let (local_x, local_y) =
                    if let crate::GpuSoftwareSpriteMapping::RotatedCorner { center, cos, sin } =
                        software.mapping
                    {
                        let dx = x as f32 - software.translation[0] - center[0];
                        let dy = y as f32 - software.translation[1] - center[1];
                        (
                            dx * cos + dy * sin + software.destination[2] / 2.0,
                            -dx * sin + dy * cos + software.destination[3] / 2.0,
                        )
                    } else {
                        let (local_x, local_y) = if identity_inverse {
                            (
                                point[0] - software.translation[0],
                                point[1] - software.translation[1],
                            )
                        } else {
                            software.inverse.transform_point(
                                point[0] - software.translation[0],
                                point[1] - software.translation[1],
                            )
                        };
                        let local_x =
                            if let crate::GpuSoftwareSpriteMapping::Font {
                                shear, center_y, ..
                            } = software.mapping
                            {
                                local_x - shear * (local_y - center_y)
                            } else {
                                local_x
                            };
                        let local_x = local_x - software.destination[0];
                        let local_y = local_y - software.destination[1];
                        (local_x, local_y)
                    };
                let inclusive = matches!(
                    software.mapping,
                    crate::GpuSoftwareSpriteMapping::RotatedCorner { .. }
                );
                if local_x < 0.0
                    || local_y < 0.0
                    || if inclusive {
                        local_x > software.destination[2] || local_y > software.destination[3]
                    } else {
                        local_x >= software.destination[2] || local_y >= software.destination[3]
                    }
                {
                    continue;
                }
                Some(
                    if matches!(
                        software.mapping,
                        crate::GpuSoftwareSpriteMapping::Native
                            | crate::GpuSoftwareSpriteMapping::RotatedCorner { .. }
                            | crate::GpuSoftwareSpriteMapping::GuiLinear { .. }
                            | crate::GpuSoftwareSpriteMapping::GuiFacetLinear
                            | crate::GpuSoftwareSpriteMapping::Font { .. }
                    ) {
                        [
                            local_x / software.destination[2],
                            local_y / software.destination[3],
                        ]
                    } else {
                        [0.0; 2]
                    },
                )
            } else {
                None
            };
            let weights = if let Some((software, [mut u, mut v])) = software.zip(native_coordinates)
            {
                if let Some(fog) = software.fog {
                    let (local_x, local_y) = if identity_inverse {
                        (
                            point[0] - software.translation[0],
                            point[1] - software.translation[1],
                        )
                    } else {
                        software.inverse.transform_point(
                            point[0] - software.translation[0],
                            point[1] - software.translation[1],
                        )
                    };
                    let (local_x, local_y) =
                        if let crate::GpuSoftwareSpriteMapping::RotatedCorner { center, cos, sin } =
                            software.mapping
                        {
                            let dx = point[0] - software.translation[0] - center[0];
                            let dy = point[1] - software.translation[1] - center[1];
                            (
                                ((dx * cos + dy * sin + software.destination[2] / 2.0)
                                    / software.destination[2])
                                    .clamp(0.0, 1.0)
                                    * software.source[2],
                                ((-dx * sin + dy * cos + software.destination[3] / 2.0)
                                    / software.destination[3])
                                    .clamp(0.0, 1.0)
                                    * software.source[3],
                            )
                        } else {
                            (
                                ((local_x - fog.destination[0]) / fog.destination[2])
                                    .clamp(0.0, 1.0)
                                    * software.source[2],
                                ((local_y - fog.destination[1]) / fog.destination[3])
                                    .clamp(0.0, 1.0)
                                    * software.source[3],
                            )
                        };
                    let [left, top, right, bottom] = fog.source_range;
                    if local_x < left
                        || (local_x >= right && right < software.source[2])
                        || local_y < top
                        || (local_y >= bottom && bottom < software.source[3])
                    {
                        continue;
                    }
                    u = ((local_x - left) / (right - left)).clamp(0.0, 1.0);
                    v = ((local_y - top) / (bottom - top)).clamp(0.0, 1.0);
                }
                if u + v <= 1.0 {
                    [1.0 - u - v, u, v, 0.0]
                } else {
                    [0.0, 1.0 - v, 1.0 - u, u + v - 1.0]
                }
            } else if vertices[0].software_blit.is_some() {
                [1.0, 0.0, 0.0, 0.0]
            } else if axis_aligned {
                let u = (point[0] - positions[0][0]) / (positions[1][0] - positions[0][0]);
                let v = (point[1] - positions[0][1]) / (positions[2][1] - positions[0][1]);
                if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                    continue;
                }
                if u + v <= 1.0 {
                    [1.0 - u - v, u, v, 0.0]
                } else {
                    [0.0, 1.0 - v, 1.0 - u, u + v - 1.0]
                }
            } else {
                let Some(weights) = quad_weights(positions, vertices.map(|v| v.position[2]), point)
                else {
                    continue;
                };
                weights
            };
            let uv = if software.is_some() {
                [0.0; 2]
            } else if axis_aligned {
                let u = (point[0] - positions[0][0]) / (positions[1][0] - positions[0][0]);
                let v = (point[1] - positions[0][1]) / (positions[2][1] - positions[0][1]);
                [
                    vertices[0].uv[0] + u * (vertices[1].uv[0] - vertices[0].uv[0]),
                    vertices[0].uv[1] + v * (vertices[2].uv[1] - vertices[0].uv[1]),
                ]
            } else {
                std::array::from_fn(|channel| {
                    (0..4).map(|i| vertices[i].uv[channel] * weights[i]).sum()
                })
            };
            let landscape_coordinates = software
                .and_then(|software| landscape_coordinates(software, point, texture.extent));
            if software.is_some_and(|software| {
                matches!(
                    software.mapping,
                    crate::GpuSoftwareSpriteMapping::Landscape { .. }
                )
            }) && landscape_coordinates.is_none()
            {
                continue;
            }
            let source = if let Some(software) = software.filter(|software| {
                matches!(
                    software.mapping,
                    crate::GpuSoftwareSpriteMapping::Font { .. }
                )
            }) {
                sample_font_texture(texture, software, point)
            } else if let Some((edges, _)) = landscape_coordinates {
                sample_texture_edge(texture, edges, sampler, [0.0; 4])
            } else if let Some((software, [u, v])) = software.zip(native_coordinates) {
                let [source_x, source_y, source_width, source_height] = software.source;
                let sample_width = if software.inclusive_source_end {
                    (source_width - 1.0).max(0.0)
                } else {
                    source_width
                };
                let sample_height = if software.inclusive_source_end {
                    (source_height - 1.0).max(0.0)
                } else {
                    source_height
                };
                let u = if software.flip_x { 1.0 - u } else { u };
                let edges = match software.mapping {
                    crate::GpuSoftwareSpriteMapping::Native => {
                        [source_x + u * sample_width, source_y + v * sample_height]
                    }
                    crate::GpuSoftwareSpriteMapping::RotatedCorner { .. } => {
                        let u = if software.flip_x { 1.0 - u } else { u };
                        let sx = (u * sample_width).clamp(0.0, sample_width).floor();
                        [
                            source_x
                                + if software.flip_x {
                                    sample_width - sx
                                } else {
                                    sx
                                },
                            source_y + (v * sample_height).clamp(0.0, sample_height).floor(),
                        ]
                    }
                    crate::GpuSoftwareSpriteMapping::PixelCorner => {
                        let local_x =
                            point[0] - 0.5 - software.translation[0] - software.destination[0];
                        let local_y =
                            point[1] - 0.5 - software.translation[1] - software.destination[1];
                        let x = ((local_x / software.destination[2]) * source_width)
                            .floor()
                            .clamp(0.0, source_width - 1.0);
                        let y = ((local_y / software.destination[3]) * source_height)
                            .floor()
                            .clamp(0.0, source_height - 1.0);
                        [
                            source_x
                                + if software.flip_x {
                                    source_width - 1.0 - x
                                } else {
                                    x
                                },
                            source_y + y,
                        ]
                    }
                    crate::GpuSoftwareSpriteMapping::GuiNearest => [
                        source_x
                            + ((point[0]
                                - 0.5
                                - software.translation[0]
                                - software.destination[0])
                                / (software.destination[2] / source_width))
                                .floor()
                                .clamp(0.0, source_width - 1.0),
                        source_y
                            + ((point[1]
                                - 0.5
                                - software.translation[1]
                                - software.destination[1])
                                / (software.destination[3] / source_height))
                                .floor()
                                .clamp(0.0, source_height - 1.0),
                    ],
                    crate::GpuSoftwareSpriteMapping::TileCopy => [
                        source_x + point[0]
                            - 0.5
                            - software.translation[0]
                            - software.destination[0],
                        source_y + point[1]
                            - 0.5
                            - software.translation[1]
                            - software.destination[1],
                    ],
                    crate::GpuSoftwareSpriteMapping::GuiLinear { .. }
                    | crate::GpuSoftwareSpriteMapping::GuiFacetLinear => [
                        source_x
                            + (point[0] - software.translation[0] - software.destination[0])
                                / (software.destination[2] / source_width),
                        source_y
                            + (point[1] - software.translation[1] - software.destination[1])
                                / (software.destination[3] / source_height),
                    ],
                    crate::GpuSoftwareSpriteMapping::Landscape { .. } => {
                        unreachable!("landscape coordinates were sampled above")
                    }
                    crate::GpuSoftwareSpriteMapping::Font { .. } => {
                        unreachable!("font coordinates were sampled above")
                    }
                    crate::GpuSoftwareSpriteMapping::IntegerStretch => [
                        (source_x as i64
                            + if source_width == software.destination[2] {
                                (point[0] - 0.5 - software.translation[0] - software.destination[0])
                                    as i64
                            } else {
                                (point[0] - 0.5 - software.translation[0] - software.destination[0])
                                    as i64
                                    * source_width as i64
                                    / software.destination[2] as i64
                            }) as f32,
                        (source_y as i64
                            + if source_height == software.destination[3] {
                                (point[1] - 0.5 - software.translation[1] - software.destination[1])
                                    as i64
                            } else {
                                (point[1] - 0.5 - software.translation[1] - software.destination[1])
                                    as i64
                                    * source_height as i64
                                    / software.destination[3] as i64
                            }) as f32,
                    ],
                };
                sample_texture_edge_impl(
                    texture,
                    edges,
                    sampler,
                    vertices[0].sample_tile,
                    matches!(software.mapping, crate::GpuSoftwareSpriteMapping::Native),
                )
            } else {
                sample_texture(texture, uv, sampler, vertices[0].sample_tile)
            };
            if source[3] == 0.0
                && blend != GpuBlend::Replace
                && vertices[0].software_blit.is_none()
                && !software.is_some_and(|software| {
                    software.mapping == crate::GpuSoftwareSpriteMapping::TileCopy
                })
            {
                continue;
            }
            let modulation: [u8; 4] = if flat_modulation {
                flat_bytes
            } else {
                std::array::from_fn(|channel| {
                    store_channel(
                        (0..4)
                            .map(|i| corner_modulation[i][channel] * weights[i])
                            .sum::<f32>(),
                    )
                })
            };
            let offset = ((y as u32 - frame.row_origin) as usize
                * scene.logical_extent[0] as usize
                + x as usize)
                * 4;
            let liquid_delta = landscape.and_then(|landscape| {
                let (mask, liquid) = landscape.mask.zip(landscape.liquid)?;
                let mask_sample = landscape_coordinates.map_or_else(
                    || sample_texture(mask, uv, GpuSampler::Nearest, [0.0; 4]),
                    |(edges, _)| sample_texture_edge(mask, edges, GpuSampler::Nearest, [0.0; 4]),
                );
                if mask_sample[0] == 0.0 {
                    return None;
                }
                let [source_x, source_y] = landscape_coordinates.map_or_else(
                    || {
                        [
                            (uv[0] * texture.extent[0] as f32).floor() as i32,
                            (uv[1] * texture.extent[1] as f32).floor() as i32,
                        ]
                    },
                    |(_, liquid)| liquid,
                );
                let x = source_x.rem_euclid(liquid.extent[0] as i32) as usize;
                let y = source_y.rem_euclid(liquid.extent[1] as i32) as usize;
                let offset = (y * liquid.extent[0] as usize + x) * 4;
                Some(
                    (0..3)
                        .map(|channel| {
                            (f32::from(liquid.pixels[offset + channel]) / 255.0 - 0.5)
                                * landscape.phase[channel]
                        })
                        .sum::<f32>(),
                )
            });
            let opaque_shader = !gamma_needs_generic
                && sampler == GpuSampler::Nearest
                && source[3] == 255.0
                && blend == GpuBlend::Normal
                && (modulation[3] == 0 || mod2 && vertices[0].software_shader)
                && liquid_delta.is_none()
                && vertices[0].software_blit.is_none()
                && !software.is_some_and(|software| {
                    matches!(
                        software.mapping,
                        crate::GpuSoftwareSpriteMapping::GuiLinear { .. }
                    )
                });
            if opaque_shader {
                for channel in 0..3 {
                    let source = source[channel] as usize;
                    let modulation = usize::from(modulation[channel]);
                    frame[offset + channel] = if mod2 {
                        let value = (2 * source as i32 + 2 * modulation as i32 - 255).clamp(0, 255)
                            as usize;
                        if gamma && scene.gamma_mode.fragment_lookup() {
                            frame.tables.encoded[channel][value]
                        } else {
                            value as u8
                        }
                    } else if gamma && scene.gamma_mode.fragment_lookup() {
                        frame.tables.products[channel][modulation * 256 + source]
                    } else {
                        ((source * modulation + 127) / 255) as u8
                    };
                }
                frame[offset + 3] = 255;
            } else if software.is_some_and(|software| {
                software.mapping == crate::GpuSoftwareSpriteMapping::TileCopy
            }) {
                put_fragment(
                    scene,
                    frame,
                    bounds,
                    x,
                    y,
                    source,
                    GpuBlend::Replace,
                    vertices[0].software_alpha_mode,
                    gamma,
                    GpuSoftwareBlend::Legacy,
                );
            } else if software.is_some_and(|software| {
                software.mapping == crate::GpuSoftwareSpriteMapping::GuiFacetLinear
            }) {
                let alpha = (source[3] / 255.0).clamp(0.0, 1.0);
                let channels = frame.gamma_override.as_deref().or_else(|| {
                    scene
                        .gamma_mode
                        .fragment_lookup()
                        .then_some(scene.gamma.channels.as_ref())
                });
                for (channel, source) in source.iter().copied().take(3).enumerate() {
                    let value = if gamma {
                        channels.map_or(source, |channels| {
                            let index =
                                ((source.clamp(0.0, 255.0) * 256.0 / 255.0) as usize).min(255);
                            f32::from(channels[0][index]) / 257.0
                        })
                    } else {
                        source
                    };
                    let encoded = f32::from(store_channel(value));
                    frame.pixels[offset + channel] = store_channel(
                        encoded * alpha + f32::from(frame.pixels[offset + channel]) * (1.0 - alpha),
                    );
                }
                frame.pixels[offset + 3] = store_channel(
                    255.0 * alpha + f32::from(frame.pixels[offset + 3]) * (1.0 - alpha),
                );
            } else if let Some(crate::GpuSoftwareSprite {
                mapping: crate::GpuSoftwareSpriteMapping::GuiLinear { modulation },
                ..
            }) = software
            {
                let mut prepared = source;
                if let Some(modulation) = modulation {
                    let bytes = [
                        (modulation >> 16) as u8,
                        (modulation >> 8) as u8,
                        *modulation as u8,
                        (modulation >> 24) as u8,
                    ];
                    for channel in 0..3 {
                        prepared[channel] *= f32::from(bytes[channel]) / 255.0;
                    }
                    prepared[3] = (prepared[3] - f32::from(bytes[3])).max(0.0);
                }
                put_fragment(
                    scene,
                    frame,
                    bounds,
                    x,
                    y,
                    prepared,
                    blend,
                    vertices[0].software_alpha_mode,
                    gamma,
                    GpuSoftwareBlend::Shader,
                );
            } else if let Some(delta) = liquid_delta {
                let mut prepared = source;
                for channel in 0..3 {
                    prepared[channel] = (source[channel] / 255.0 + delta).clamp(0.0, 1.0)
                        * f32::from(modulation[channel]);
                }
                prepared[3] = (source[3] - f32::from(modulation[3])).max(0.0);
                put_fragment(
                    scene,
                    frame,
                    bounds,
                    x,
                    y,
                    prepared,
                    GpuBlend::Normal,
                    vertices[0].software_alpha_mode,
                    gamma,
                    GpuSoftwareBlend::Shader,
                );
            } else if let Some(blit) = vertices[0].software_blit {
                let Some(source) = sample_surface_blit(texture, blit, x, y) else {
                    continue;
                };
                let dest = Color::new(
                    frame[offset],
                    frame[offset + 1],
                    frame[offset + 2],
                    frame[offset + 3],
                );
                let output = blit.mode.map_or(source, |mode| {
                    let source = mode.prepare_source(source, blit.modulation);
                    match mode {
                        crate::BlitMode::Normal => source.blend_over(dest),
                        crate::BlitMode::Additive => source.blend_additive(dest),
                        crate::BlitMode::Mod2 => source.blend_shader_over(dest),
                        crate::BlitMode::Mod2Additive => source.blend_shader_additive(dest),
                    }
                });
                frame[offset..offset + 4]
                    .copy_from_slice(&[output.r, output.g, output.b, output.a]);
            } else if unmodulated_nearest {
                put_fragment(
                    scene,
                    frame,
                    bounds,
                    x,
                    y,
                    source,
                    blend,
                    vertices[0].software_alpha_mode,
                    gamma,
                    if blend == GpuBlend::Additive {
                        GpuSoftwareBlend::RoundedLegacy
                    } else {
                        GpuSoftwareBlend::Legacy
                    },
                );
            } else {
                let mut prepared = source;
                for channel in 0..3 {
                    prepared[channel] = if mod2 {
                        (2.0 * source[channel] + 2.0 * f32::from(modulation[channel]) - 255.0)
                            .clamp(0.0, 255.0)
                    } else {
                        source[channel] * f32::from(modulation[channel]) / 255.0
                    };
                }
                prepared[3] = if mod2 && vertices[0].software_shader {
                    source[3]
                } else {
                    (source[3] - f32::from(modulation[3])).max(0.0)
                };
                put_fragment(
                    scene,
                    frame,
                    bounds,
                    x,
                    y,
                    prepared,
                    blend,
                    vertices[0].software_alpha_mode,
                    gamma,
                    GpuSoftwareBlend::Shader,
                );
            }
        }
    }
    Ok(())
}

fn landscape_coordinates(
    software: &crate::GpuSoftwareSprite,
    point: [f32; 2],
    tile_extent: [u32; 2],
) -> Option<([f32; 2], [i32; 2])> {
    let crate::GpuSoftwareSpriteMapping::Landscape {
        zoom,
        world_extent,
        tile_origin,
        texture_size,
        indent,
    } = software.mapping
    else {
        return None;
    };
    let tile_end = [
        tile_origin[0].checked_add(tile_extent[0])?,
        tile_origin[1].checked_add(tile_extent[1])?,
    ];
    if tile_end.iter().any(|end| *end > i32::MAX as u32) {
        return None;
    }
    let raw: [f32; 2] = std::array::from_fn(|i| {
        let offset = point[i] - software.translation[i] - software.destination[i];
        software.source[i] + if zoom == 1.0 { offset } else { offset / zoom }
    });
    if (0..2).any(|i| raw[i] < 0.0 || raw[i] >= world_extent[i] as f32) {
        return None;
    }
    let world: [i32; 2] = std::array::from_fn(|i| (raw[i] + indent).floor() as i32);
    if (0..2).any(|i| raw[i] < tile_origin[i] as f32 || raw[i] >= tile_end[i] as f32) {
        return None;
    }
    let liquid = std::array::from_fn(|i| {
        if indent == 0.0 {
            (raw[i].floor() as i32).rem_euclid(texture_size as i32)
        } else {
            (raw[i] - tile_origin[i] as f32 + indent).floor() as i32
        }
    });
    let local = std::array::from_fn(|i| {
        world[i].clamp(tile_origin[i] as i32, (tile_end[i] - 1) as i32) as f32
            - tile_origin[i] as f32
    });
    Some((local, liquid))
}

#[allow(clippy::too_many_arguments)]
fn draw_landscape_span(
    scene: &GpuScene,
    frame: &mut RasterTarget<'_>,
    vertices: &[GpuVertex; 4],
    texture: &GpuTextureResource,
    software: &crate::GpuSoftwareSprite,
    landscape: LandscapeSample<'_>,
    gamma: bool,
    limits: [i32; 4],
) -> bool {
    let crate::GpuSoftwareSpriteMapping::Landscape {
        zoom: 1.0,
        indent: 0.0,
        world_extent,
        tile_origin,
        texture_size,
    } = software.mapping
    else {
        return false;
    };
    if software.inverse != crate::Transform::identity()
        || texture.format != GpuTextureFormat::Rgba8
        || tile_origin
            .into_iter()
            .zip(texture.extent)
            .any(|(origin, extent)| {
                origin
                    .checked_add(extent)
                    .is_none_or(|end| end > i32::MAX as u32)
            })
        || software
            .destination
            .iter()
            .chain(software.source[..2].iter())
            .chain(software.translation.iter())
            .any(|value| value.fract() != 0.0 || value.abs() > 4_000_000.0)
        || landscape.mask.is_some_and(|mask| {
            mask.format != GpuTextureFormat::R8 || mask.extent != texture.extent
        })
        || landscape
            .liquid
            .is_some_and(|liquid| liquid.format != GpuTextureFormat::Rgba8)
    {
        return false;
    }
    let [mut left, mut top, mut right, mut bottom] = limits;
    if right - left > 64 {
        return false;
    }
    // The generic path forms f32 pixel centers before subtracting origins.
    // Integer offsets are equivalent only while every intermediate retains
    // its half pixel. Larger coordinates use that original arithmetic.
    for (axis, range) in [[left, right], [top, bottom]].into_iter().enumerate() {
        for coordinate in range {
            let point = f64::from(coordinate) + 0.5;
            let translated = point - f64::from(software.translation[axis]);
            let local = translated - f64::from(software.destination[axis]);
            if [
                point,
                translated,
                local,
                local + f64::from(software.source[axis]),
            ]
            .into_iter()
            .any(|value| value.abs() > 4_000_000.0)
            {
                return false;
            }
        }
    }
    let screen_x = (software.destination[0] + software.translation[0]) as i32;
    let screen_y = (software.destination[1] + software.translation[1]) as i32;
    let world_x_offset = software.source[0] as i32 - screen_x;
    let world_y_offset = software.source[1] as i32 - screen_y;
    let offset_x = i64::from(world_x_offset);
    let offset_y = i64::from(world_y_offset);
    let clipped_left = i64::from(left)
        .max(i64::from(screen_x))
        .max(i64::from(tile_origin[0]) - offset_x)
        .max(-offset_x);
    let clipped_top = i64::from(top)
        .max(i64::from(screen_y))
        .max(i64::from(tile_origin[1]) - offset_y)
        .max(-offset_y);
    let clipped_right = i64::from(right)
        .min(i64::from(screen_x) + software.destination[2] as i64)
        .min(i64::from(world_extent[0]) - offset_x)
        .min(i64::from(tile_origin[0]) + i64::from(texture.extent[0]) - offset_x);
    let clipped_bottom = i64::from(bottom)
        .min(i64::from(screen_y) + software.destination[3] as i64)
        .min(i64::from(world_extent[1]) - offset_y)
        .min(i64::from(tile_origin[1]) + i64::from(texture.extent[1]) - offset_y);
    if clipped_left >= clipped_right || clipped_top >= clipped_bottom {
        return true;
    }
    left = clipped_left as i32;
    top = clipped_top as i32;
    right = clipped_right as i32;
    bottom = clipped_bottom as i32;
    let corners = vertices.map(|vertex| vertex.modulation.map(|value| (value * 255.0).round()));
    let flat = corners.iter().all(|corner| *corner == corners[0]);
    let flat_bytes = corners[0].map(store_channel);
    if !flat && software.fog.is_none() {
        return false;
    }
    let grey = corners
        .iter()
        .all(|corner| corner[0] == corner[1] && corner[1] == corner[2]);
    let opaque_modulation = corners.iter().all(|corner| corner[3] == 0.0);
    let legacy = vertices.iter().all(|vertex| {
        vertex.outer_modulation == crate::GpuOuterModulation::Inherit
            && vertex.modulation == [1.0, 1.0, 1.0, 0.0]
    });
    let fragment_gamma = gamma && scene.gamma_mode.fragment_lookup();
    let opaque_black = std::array::from_fn(|channel| {
        if channel == 3 {
            255
        } else if fragment_gamma {
            frame.tables.encoded[channel][0]
        } else {
            0
        }
    });
    let grey_span = grey
        && opaque_modulation
        && corners
            .iter()
            .all(|corner| (0.0..=255.0).contains(&corner[0]));
    let mut grey_values = [0u8; 64];
    let mut horizontal = [Some(0.0f32); 64];
    if let Some(fog) = software.fog {
        for x in left..right {
            let source_x = ((x as f32 + 0.5 - software.translation[0] - fog.destination[0])
                / fog.destination[2])
                .clamp(0.0, 1.0)
                * software.source[2];
            horizontal[(x - left) as usize] = if source_x < fog.source_range[0]
                || source_x >= fog.source_range[2] && fog.source_range[2] < software.source[2]
            {
                None
            } else {
                Some(
                    ((source_x - fog.source_range[0])
                        / (fog.source_range[2] - fog.source_range[0]))
                        .clamp(0.0, 1.0),
                )
            };
        }
    }
    let horizontal_amounts = horizontal.map(|value| value.unwrap_or(0.0));
    let opaque_grey_span = grey_span
        && fragment_gamma
        && horizontal[..(right - left) as usize]
            .iter()
            .all(Option::is_some);
    for y in top..bottom {
        let world_y = y + world_y_offset;
        let local_y = (world_y - tile_origin[1] as i32) as usize;
        let source_row = local_y * texture.extent[0] as usize;
        let target_row = ((y as u32 - frame.row_origin) as usize
            * scene.logical_extent[0] as usize
            + left as usize)
            * 4;
        let vertical = if let Some(fog) = software.fog {
            let source_y = ((y as f32 + 0.5 - software.translation[1] - fog.destination[1])
                / fog.destination[3])
                .clamp(0.0, 1.0)
                * software.source[3];
            if source_y < fog.source_range[1]
                || source_y >= fog.source_range[3] && fog.source_range[3] < software.source[3]
            {
                continue;
            }
            ((source_y - fog.source_range[1]) / (fog.source_range[3] - fog.source_range[1]))
                .clamp(0.0, 1.0)
        } else {
            0.0
        };
        let count = (right - left) as usize;
        let first = source_row + (left + world_x_offset - tile_origin[0] as i32) as usize;
        let source = &texture.pixels[first * 4..(first + count) * 4];
        let source_is_opaque = source.chunks_exact(4).all(|pixel| pixel[3] == 255);
        if !source_is_opaque && source.chunks_exact(4).all(|pixel| pixel[3] == 0) {
            continue;
        }
        let liquid_free = source_is_opaque
            && landscape
                .mask
                .zip(landscape.liquid)
                .is_none_or(|(mask, _)| {
                    mask.pixels[first..first + count]
                        .iter()
                        .all(|value| *value == 0)
                });
        if flat
            && flat_bytes == [0; 4]
            && liquid_free
            && horizontal[..count].iter().all(Option::is_some)
        {
            spans::fill_opaque(
                &mut frame.pixels[target_row..target_row + count * 4],
                opaque_black,
            );
            continue;
        }
        if grey_span && !flat {
            spans::interpolate_grey(
                corners.map(|corner| corner[0]),
                &horizontal_amounts[..(right - left) as usize],
                vertical,
                &mut grey_values[..(right - left) as usize],
            );
        }
        if opaque_grey_span && liquid_free {
            if flat {
                grey_values[..count].fill(flat_bytes[0]);
            }
            spans::shade_opaque_grey(
                source,
                &grey_values[..count],
                &mut frame.pixels[target_row..target_row + count * 4],
                (!frame.tables.standard_encoding).then_some(&frame.tables.encoded),
            );
            continue;
        }
        for x in left..right {
            let world_x = x + world_x_offset;
            let local_x = (world_x - tile_origin[0] as i32) as usize;
            let source_index = source_row + local_x;
            let source = &texture.pixels[source_index * 4..source_index * 4 + 4];
            if source[3] == 0 {
                continue;
            }
            let Some(horizontal) = horizontal[(x - left) as usize] else {
                continue;
            };
            let modulation = if flat {
                flat_bytes
            } else if grey_span {
                let value = grey_values[(x - left) as usize];
                [value, value, value, 0]
            } else {
                let weights = if horizontal + vertical <= 1.0 {
                    [1.0 - horizontal - vertical, horizontal, vertical, 0.0]
                } else {
                    [
                        0.0,
                        1.0 - vertical,
                        1.0 - horizontal,
                        horizontal + vertical - 1.0,
                    ]
                };
                let interpolate = |channel| {
                    store_channel(
                        (0..4)
                            .map(|corner| corners[corner][channel] * weights[corner])
                            .sum::<f32>(),
                    )
                };
                if grey {
                    let value = interpolate(0);
                    [
                        value,
                        value,
                        value,
                        if opaque_modulation { 0 } else { interpolate(3) },
                    ]
                } else {
                    std::array::from_fn(interpolate)
                }
            };
            let alpha = source[3].saturating_sub(modulation[3]);
            if alpha == 0 {
                continue;
            }
            let output_index = target_row + (x - left) as usize * 4;
            let liquid = landscape
                .mask
                .zip(landscape.liquid)
                .filter(|(mask, _)| mask.pixels[source_index] != 0);
            if alpha == 255 && liquid.is_none() {
                if modulation[..3] == [0; 3] {
                    frame[output_index..output_index + 4].copy_from_slice(&opaque_black);
                    continue;
                }
                for channel in 0..3 {
                    let source = usize::from(source[channel]);
                    let modulation = usize::from(modulation[channel]);
                    frame[output_index + channel] = if fragment_gamma {
                        frame.tables.products[channel][modulation * 256 + source]
                    } else {
                        ((source * modulation + 127) / 255) as u8
                    };
                }
                frame[output_index + 3] = 255;
            } else {
                let mut prepared = [0.0; 4];
                if let Some((_, liquid)) = liquid {
                    let x = (world_x % texture_size as i32) as usize % liquid.extent[0] as usize;
                    let y = (world_y % texture_size as i32) as usize % liquid.extent[1] as usize;
                    let index = (y * liquid.extent[0] as usize + x) * 4;
                    let delta = (0..3)
                        .map(|channel| {
                            (f32::from(liquid.pixels[index + channel]) / 255.0 - 0.5)
                                * landscape.phase[channel]
                        })
                        .sum::<f32>();
                    for channel in 0..3 {
                        prepared[channel] = (f32::from(source[channel]) / 255.0 + delta)
                            .clamp(0.0, 1.0)
                            * f32::from(modulation[channel]);
                    }
                } else {
                    for channel in 0..3 {
                        prepared[channel] =
                            f32::from(source[channel]) * f32::from(modulation[channel]) / 255.0;
                    }
                }
                prepared[3] = f32::from(alpha);
                put_fragment(
                    scene,
                    frame,
                    frame.bounds,
                    x,
                    y,
                    prepared,
                    GpuBlend::Normal,
                    GpuSolidAlphaMode::SourceOver,
                    gamma,
                    if legacy && liquid.is_none() {
                        GpuSoftwareBlend::Legacy
                    } else {
                        GpuSoftwareBlend::Shader
                    },
                );
            }
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn draw_gui_linear_span(
    scene: &GpuScene,
    frame: &mut RasterTarget<'_>,
    vertices: &[GpuVertex; 4],
    texture: &GpuTextureResource,
    software: &crate::GpuSoftwareSprite,
    gamma: bool,
    [left, top, right, bottom]: [i32; 4],
) -> bool {
    let crate::GpuSoftwareSpriteMapping::GuiLinear { modulation } = software.mapping else {
        return false;
    };
    if right - left > 64
        || bottom - top > 64
        || texture.format != GpuTextureFormat::Rgba8
        || software.inverse != crate::Transform::identity()
        || vertices.iter().any(|vertex| {
            vertex.sample_tile != vertices[0].sample_tile
                || vertex.software_sprite != vertices[0].software_sprite
                || vertex.software_alpha_mode != GpuSolidAlphaMode::SourceOver
        })
        || [left, top, right, bottom]
            .into_iter()
            .any(|value| value.abs() > 4_000_000)
        || software
            .destination
            .into_iter()
            .chain(software.source)
            .chain(software.translation)
            .chain(vertices[0].sample_tile)
            .any(|value| value.abs() > 4_000_000.0)
        || texture.extent.iter().any(|extent| *extent > 4_000_000)
        || vertices[0].sample_tile[3] != 0.0
            && (vertices[0].sample_tile[2] < 1.0 || vertices[0].sample_tile[3] < 1.0)
    {
        return false;
    }
    let source_axis = |axis: usize, coordinate: i32| {
        let point = coordinate as f32 + 0.5;
        let untranslated = point - software.translation[axis];
        let local = untranslated - software.destination[axis];
        if local < 0.0 || local >= software.destination[axis + 2] {
            return None;
        }
        if let Some(fog) = software.fog {
            let amount = ((untranslated - fog.destination[axis]) / fog.destination[axis + 2])
                .clamp(0.0, 1.0)
                * software.source[axis + 2];
            if amount < fog.source_range[axis]
                || amount >= fog.source_range[axis + 2]
                    && fog.source_range[axis + 2] < software.source[axis + 2]
            {
                return None;
            }
        }
        // StdDDraw2.cpp:738-751 keeps local division before adding the source
        // inset. Preserve those operations rather than normalizing atlas UVs.
        Some(
            software.source[axis]
                + local / (software.destination[axis + 2] / software.source[axis + 2]),
        )
    };
    let mut horizontal = [None; 64];
    let mut vertical = [None; 64];
    for x in left..right {
        horizontal[(x - left) as usize] = source_axis(0, x);
    }
    for y in top..bottom {
        vertical[(y - top) as usize] = source_axis(1, y);
    }
    if horizontal
        .iter()
        .chain(&vertical)
        .flatten()
        .any(|coordinate| !coordinate.is_finite() || coordinate.abs() > 4_000_000.0)
    {
        return false;
    }
    let bytes = modulation.map(|modulation| {
        [
            (modulation >> 16) as u8,
            (modulation >> 8) as u8,
            modulation as u8,
            (modulation >> 24) as u8,
        ]
    });
    let tile = vertices[0].sample_tile;
    let sample_axis = |axis: usize, coordinate: f32| {
        let edge = coordinate - 0.5;
        let first = edge.floor() as i32;
        let fraction = edge - edge.floor();
        let indices = [first, first + 1].map(|index| {
            let index = if tile[3] != 0.0 {
                // Match sample_texture_edge_impl: both tile origins and both
                // clamps use the physical texture's square tile width.
                let origin = (coordinate / tile[2]).floor() * tile[2];
                index.clamp(origin as i32, (origin + tile[2] - 1.0) as i32)
            } else {
                index.clamp(0, texture.extent[axis] as i32 - 1)
            };
            (index >= 0 && index < texture.extent[axis] as i32).then_some(index as usize)
        });
        (indices, fraction)
    };
    let horizontal =
        horizontal.map(|coordinate| coordinate.map(|coordinate| sample_axis(0, coordinate)));
    let vertical = vertical.map(|coordinate| {
        coordinate.map(|coordinate| {
            let (indices, fraction) = sample_axis(1, coordinate);
            (
                indices.map(|index| index.map(|index| index * texture.extent[0] as usize)),
                fraction,
            )
        })
    });
    let channels = if gamma {
        frame.gamma_override.as_deref().or_else(|| {
            scene
                .gamma_mode
                .fragment_lookup()
                .then_some(scene.gamma.channels.as_ref())
        })
    } else {
        None
    };
    let factors = bytes
        .map(|bytes| std::array::from_fn::<_, 3, _>(|channel| f32::from(bytes[channel]) / 255.0));
    let transparency = bytes.map_or(0.0, |bytes| f32::from(bytes[3]));
    let black_modulation = bytes.is_some_and(|bytes| bytes[..3] == [0; 3]);
    let black = std::array::from_fn(|channel| {
        channels.map_or(0.0, |channels| f32::from(channels[channel][0]) / 257.0)
    });
    let mut black_alpha = [0.0; 64];
    for y in top..bottom {
        let Some(([row0, row1], fy)) = vertical[(y - top) as usize] else {
            continue;
        };
        let target_row = ((y as u32 - frame.row_origin) as usize
            * scene.logical_extent[0] as usize
            + left as usize)
            * 4;
        let count = (right - left) as usize;
        if black_modulation {
            black_alpha[..count].fill(0.0);
        }
        for x in left..right {
            let Some(([column0, column1], fx)) = horizontal[(x - left) as usize] else {
                continue;
            };
            let sample = |row: Option<usize>, column: Option<usize>, channel: usize| {
                row.zip(column)
                    .map_or(if channel == 3 { 0.0 } else { 255.0 }, |(row, column)| {
                        f32::from(texture.pixels[(row + column) * 4 + channel])
                    })
            };
            let filtered = |channel| {
                // Preserve the scalar bilinear sample's two horizontal sums
                // followed by its vertical sum, without transparent RGB normalization.
                let top = sample(row0, column0, channel) * (1.0 - fx)
                    + sample(row0, column1, channel) * fx;
                let bottom = sample(row1, column0, channel) * (1.0 - fx)
                    + sample(row1, column1, channel) * fx;
                top * (1.0 - fy) + bottom * fy
            };
            if black_modulation {
                black_alpha[(x - left) as usize] = (filtered(3) - transparency).max(0.0);
                continue;
            }
            let mut prepared = std::array::from_fn::<_, 4, _>(filtered);
            if prepared[3] == 0.0 {
                continue;
            }
            if let Some(factors) = factors {
                for channel in 0..3 {
                    prepared[channel] *= factors[channel];
                }
                prepared[3] = (prepared[3] - transparency).max(0.0);
            }
            if prepared[3] == 0.0 {
                continue;
            }
            let coverage = (prepared[3] / 255.0).clamp(0.0, 1.0);
            let inverse = 1.0 - coverage;
            let offset = target_row + (x - left) as usize * 4;
            for channel in 0..3 {
                let source = channels.map_or(prepared[channel], |channels| {
                    let index =
                        ((prepared[channel].clamp(0.0, 255.0) * 256.0 / 255.0) as usize).min(255);
                    f32::from(channels[channel][index]) / 257.0
                });
                frame.pixels[offset + channel] = store_channel(
                    source * coverage + f32::from(frame.pixels[offset + channel]) * inverse,
                );
            }
            frame.pixels[offset + 3] =
                store_channel(prepared[3] + f32::from(frame.pixels[offset + 3]) * inverse);
        }
        if black_modulation {
            spans::blend_black(
                &black_alpha[..count],
                black,
                &mut frame.pixels[target_row..target_row + count * 4],
            );
        }
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn draw_nearest_sprite_span(
    scene: &GpuScene,
    frame: &mut RasterTarget<'_>,
    vertices: &[GpuVertex; 4],
    texture: &GpuTextureResource,
    software: &crate::GpuSoftwareSprite,
    gamma: bool,
    legacy_blend_allowed: bool,
    [left, top, right, bottom]: [i32; 4],
) -> bool {
    use crate::GpuSoftwareSpriteMapping as Mapping;
    if right - left > 64
        || texture.format != GpuTextureFormat::Rgba8
        || software.inverse != crate::Transform::identity()
        || vertices
            .iter()
            .any(|vertex| vertex.software_alpha_mode != GpuSolidAlphaMode::SourceOver)
        || vertices.iter().any(|vertex| vertex.sample_tile != [0.0; 4])
        || !matches!(
            software.mapping,
            Mapping::Native | Mapping::PixelCorner | Mapping::GuiNearest | Mapping::IntegerStretch
        )
        || [left, top, right, bottom]
            .into_iter()
            .any(|value| value.abs() > 4_000_000)
        || software
            .destination
            .into_iter()
            .chain(software.source)
            .chain(software.translation)
            .any(|value| value.abs() > 4_000_000.0)
    {
        return false;
    }
    let corners = vertices.map(|vertex| vertex.modulation.map(|value| (value * 255.0).round()));
    let flat = corners.iter().all(|corner| *corner == corners[0]);
    let flat_bytes = corners[0].map(store_channel);
    let grey = corners
        .iter()
        .all(|corner| corner[0] == corner[1] && corner[1] == corner[2]);
    let opaque_modulation = corners.iter().all(|corner| corner[3] == 0.0);
    // Preserve the generic dispatcher: an explicit differing/monitor widget
    // LUT disables unmodulated_nearest and retains shader alpha rounding.
    let legacy = legacy_blend_allowed
        && vertices.iter().all(|vertex| {
            vertex.outer_modulation == crate::GpuOuterModulation::Inherit
                && vertex.modulation == [1.0, 1.0, 1.0, 0.0]
        });
    let channels = if gamma {
        frame.gamma_override.as_deref().or_else(|| {
            scene
                .gamma_mode
                .fragment_lookup()
                .then_some(scene.gamma.channels.as_ref())
        })
    } else {
        None
    };
    let tables = frame.tables;
    let prepared = channels.and_then(|channels| {
        // Try every retained backing identity before comparing channel bytes.
        tables
            .gamma
            .as_ref()
            .filter(|stored| std::ptr::eq(stored.as_ref(), channels))
            .map(|_| (&tables.encoded, &tables.products, tables.standard_encoding))
            .or_else(|| {
                tables
                    .cached
                    .iter()
                    .find(|entry| std::ptr::eq(entry.gamma.as_ref(), channels))
                    .map(|entry| (&entry.encoded, &entry.products, entry.standard_encoding))
            })
            .or_else(|| {
                tables
                    .gamma
                    .as_deref()
                    .filter(|stored| *stored == channels)
                    .map(|_| (&tables.encoded, &tables.products, tables.standard_encoding))
            })
            .or_else(|| {
                tables
                    .cached
                    .iter()
                    .find(|entry| entry.gamma.as_ref() == channels)
                    .map(|entry| (&entry.encoded, &entry.products, entry.standard_encoding))
            })
    });
    let fragment_gamma = channels.is_some();
    if fragment_gamma && prepared.is_none() {
        // A widget-only cold LUT keeps the scalar path; never prepare three
        // 64 KiB product tables for an individual command or tile.
        return false;
    }
    let (encoded, products, standard_encoding) =
        prepared.unwrap_or((&tables.encoded, &tables.products, false));
    let opaque_black = std::array::from_fn(|channel| {
        if channel == 3 {
            255
        } else if fragment_gamma {
            encoded[channel][0]
        } else {
            0
        }
    });
    let grey_span = grey
        && opaque_modulation
        && corners
            .iter()
            .all(|corner| (0.0..=255.0).contains(&corner[0]));
    let mut grey_values = [0u8; 64];
    let black = std::array::from_fn(|channel| {
        channels.map_or(0.0, |channels| f32::from(channels[channel][0]) / 257.0)
    });
    let mut black_alpha = [0.0; 64];
    let source_axis = |axis: usize, coordinate: i32| -> Option<(usize, f32)> {
        let point = coordinate as f32 + 0.5;
        let untranslated = point - software.translation[axis];
        let local = untranslated - software.destination[axis];
        let destination = software.destination[axis + 2];
        let extent = software.source[axis + 2];
        if local < 0.0 || local >= destination {
            return None;
        }
        let mut amount = if software.mapping == Mapping::Native {
            local / destination
        } else {
            0.0
        };
        let sample = match software.mapping {
            Mapping::Native => {
                let sample_extent = if software.inclusive_source_end {
                    (extent - 1.0).max(0.0)
                } else {
                    extent
                };
                let sample_amount = if axis == 0 && software.flip_x {
                    1.0 - amount
                } else {
                    amount
                };
                software.source[axis] + sample_amount * sample_extent
            }
            Mapping::PixelCorner => {
                let sample =
                    ((point - 0.5 - software.translation[axis] - software.destination[axis])
                        / destination
                        * extent)
                        .floor()
                        .clamp(0.0, extent - 1.0);
                software.source[axis]
                    + if axis == 0 && software.flip_x {
                        extent - 1.0 - sample
                    } else {
                        sample
                    }
            }
            Mapping::GuiNearest => {
                software.source[axis]
                    + ((point - 0.5 - software.translation[axis] - software.destination[axis])
                        / (destination / extent))
                        .floor()
                        .clamp(0.0, extent - 1.0)
            }
            Mapping::IntegerStretch => {
                let offset =
                    (point - 0.5 - software.translation[axis] - software.destination[axis]) as i64;
                (software.source[axis] as i64
                    + if extent == destination {
                        offset
                    } else {
                        offset * extent as i64 / destination as i64
                    }) as f32
            }
            _ => return None,
        };
        if let Some(fog) = software.fog {
            let value = ((untranslated - fog.destination[axis]) / fog.destination[axis + 2])
                .clamp(0.0, 1.0)
                * extent;
            let start = fog.source_range[axis];
            let end = fog.source_range[axis + 2];
            if value < start || value >= end && end < extent {
                return None;
            }
            amount = ((value - start) / (end - start)).clamp(0.0, 1.0);
        }
        Some((
            sample.clamp(0.0, texture.extent[axis] as f32 - 1.0) as usize,
            amount,
        ))
    };
    let mut horizontal = [None; 64];
    for x in left..right {
        horizontal[(x - left) as usize] = source_axis(0, x);
    }
    let horizontal_amounts = horizontal.map(|value| value.map_or(0.0, |(_, u)| u));
    let contiguous_source = horizontal[0].map(|(x, _)| x).filter(|first| {
        horizontal[..(right - left) as usize]
            .iter()
            .enumerate()
            .all(|(offset, sample)| sample.is_some_and(|(x, _)| x == first + offset))
    });
    let opaque_grey_span = grey_span && fragment_gamma;
    let mut opaque_row: Option<(usize, usize)> = None;
    for y in top..bottom {
        let Some((source_y, v)) = source_axis(1, y) else {
            continue;
        };
        let source_row = source_y * texture.extent[0] as usize * 4;
        let target_row = ((y as u32 - frame.row_origin) as usize
            * scene.logical_extent[0] as usize
            + left as usize)
            * 4;
        // Nearest stretching often repeats an opaque source row. Its final
        // bytes are independent of the destination, including custom gamma.
        // Transparent or spatially modulated rows keep the original blending.
        if let Some((_, previous_row)) =
            opaque_row.filter(|(previous_y, _)| *previous_y == source_y)
        {
            frame.pixels.copy_within(
                previous_row..previous_row + (right - left) as usize * 4,
                target_row,
            );
            continue;
        }
        let row_is_opaque = flat
            && flat_bytes[3] == 0
            && horizontal[..(right - left) as usize].iter().all(|sample| {
                sample.is_some_and(|(source_x, _)| {
                    texture.pixels[source_row + source_x * 4 + 3] == 255
                })
            });
        if row_is_opaque && flat_bytes[..3] == [0; 3] {
            spans::fill_opaque(
                &mut frame.pixels[target_row..target_row + (right - left) as usize * 4],
                opaque_black,
            );
            opaque_row = Some((source_y, target_row));
            continue;
        }
        if flat && flat_bytes[..3] == [0; 3] {
            let count = (right - left) as usize;
            for (alpha, sample) in black_alpha[..count].iter_mut().zip(&horizontal[..count]) {
                *alpha = sample.map_or(0.0, |(source_x, _)| {
                    f32::from(
                        texture.pixels[source_row + source_x * 4 + 3].saturating_sub(flat_bytes[3]),
                    )
                });
            }
            spans::blend_black(
                &black_alpha[..count],
                black,
                &mut frame.pixels[target_row..target_row + count * 4],
            );
            continue;
        }
        if grey_span && !flat {
            spans::interpolate_grey(
                corners.map(|corner| corner[0]),
                &horizontal_amounts[..(right - left) as usize],
                v,
                &mut grey_values[..(right - left) as usize],
            );
        }
        if let Some(first) = contiguous_source.filter(|_| opaque_grey_span) {
            let count = (right - left) as usize;
            let source = &texture.pixels[source_row + first * 4..source_row + (first + count) * 4];
            if source.chunks_exact(4).all(|pixel| pixel[3] == 255) {
                if flat {
                    grey_values[..count].fill(flat_bytes[0]);
                }
                spans::shade_opaque_grey(
                    source,
                    &grey_values[..count],
                    &mut frame.pixels[target_row..target_row + count * 4],
                    (!standard_encoding).then_some(encoded),
                );
                opaque_row = flat.then_some((source_y, target_row));
                continue;
            }
        }
        for x in left..right {
            let Some((source_x, u)) = horizontal[(x - left) as usize] else {
                continue;
            };
            let source = &texture.pixels[source_row + source_x * 4..source_row + source_x * 4 + 4];
            if source[3] == 0 {
                continue;
            }
            let modulation = if flat {
                flat_bytes
            } else if grey_span {
                let value = grey_values[(x - left) as usize];
                [value, value, value, 0]
            } else {
                let weights = if u + v <= 1.0 {
                    [1.0 - u - v, u, v, 0.0]
                } else {
                    [0.0, 1.0 - v, 1.0 - u, u + v - 1.0]
                };
                let interpolate = |channel| {
                    store_channel(
                        (0..4)
                            .map(|corner| corners[corner][channel] * weights[corner])
                            .sum::<f32>(),
                    )
                };
                if grey {
                    let value = interpolate(0);
                    [
                        value,
                        value,
                        value,
                        if opaque_modulation { 0 } else { interpolate(3) },
                    ]
                } else {
                    std::array::from_fn(interpolate)
                }
            };
            let alpha = source[3].saturating_sub(modulation[3]);
            if alpha == 0 {
                continue;
            }
            let offset = target_row + (x - left) as usize * 4;
            if alpha == 255 && modulation[..3] == [0; 3] {
                frame[offset..offset + 4].copy_from_slice(&opaque_black);
            } else if alpha == 255
                && fragment_gamma
                && standard_encoding
                && modulation[..3] == [255; 3]
            {
                frame[offset..offset + 4].copy_from_slice(&standard_gamma_pixel([
                    source[0], source[1], source[2], 255,
                ]));
            } else if alpha == 255 {
                for channel in 0..3 {
                    let source = usize::from(source[channel]);
                    let modulation = usize::from(modulation[channel]);
                    frame[offset + channel] = if fragment_gamma {
                        products[channel][modulation * 256 + source]
                    } else {
                        ((source * modulation + 127) / 255) as u8
                    };
                }
                frame[offset + 3] = 255;
            } else if modulation[..3] == [0; 3] {
                let coverage = f32::from(alpha) / 255.0;
                let inverse = 1.0 - coverage;
                for channel in 0..3 {
                    frame[offset + channel] = store_channel(
                        black[channel] * coverage + f32::from(frame[offset + channel]) * inverse,
                    );
                }
                frame[offset + 3] =
                    store_channel(f32::from(alpha) + f32::from(frame[offset + 3]) * inverse);
            } else {
                let mut prepared = [0.0; 4];
                for channel in 0..3 {
                    prepared[channel] =
                        f32::from(source[channel]) * f32::from(modulation[channel]) / 255.0;
                }
                prepared[3] = f32::from(alpha);
                put_fragment(
                    scene,
                    frame,
                    frame.bounds,
                    x,
                    y,
                    prepared,
                    GpuBlend::Normal,
                    GpuSolidAlphaMode::SourceOver,
                    gamma,
                    if legacy {
                        GpuSoftwareBlend::Legacy
                    } else {
                        GpuSoftwareBlend::Shader
                    },
                );
            }
        }
        opaque_row = row_is_opaque.then_some((source_y, target_row));
    }
    true
}

#[allow(clippy::too_many_arguments)]
fn draw_black_sprite_span(
    scene: &GpuScene,
    frame: &mut RasterTarget<'_>,
    vertices: &[GpuVertex; 4],
    texture: &GpuTextureResource,
    software: &crate::GpuSoftwareSprite,
    sampler: GpuSampler,
    gamma: bool,
    [left, top, right, bottom]: [i32; 4],
) -> bool {
    use crate::GpuSoftwareSpriteMapping as Mapping;
    if right - left > 64
        || !matches!(
            software.mapping,
            Mapping::Native | Mapping::RotatedCorner { .. }
        )
        || texture.format != GpuTextureFormat::Rgba8
        || vertices.iter().any(|v| {
            v.modulation != vertices[0].modulation
                || v.modulation[..3] != [0.0; 3]
                || v.sample_tile != vertices[0].sample_tile
                || v.software_alpha_mode != GpuSolidAlphaMode::SourceOver
        })
        || software.inverse.mat[6..] != [0.0, 0.0, 1.0]
    {
        return false;
    }
    let transparency = f32::from(store_channel((vertices[0].modulation[3] * 255.0).round()));
    let identity = software.inverse == crate::Transform::identity();
    let sample_size = std::array::from_fn::<_, 2, _>(|axis| {
        if software.inclusive_source_end {
            (software.source[axis + 2] - 1.0).max(0.0)
        } else {
            software.source[axis + 2]
        }
    });
    let channels = if gamma {
        frame.gamma_override.as_deref().or_else(|| {
            scene
                .gamma_mode
                .fragment_lookup()
                .then_some(scene.gamma.channels.as_ref())
        })
    } else {
        None
    };
    let black = std::array::from_fn(|channel| {
        channels.map_or(0.0, |channels| f32::from(channels[channel][0]) / 257.0)
    });
    let count = (right - left) as usize;
    let mut black_alpha = [0.0; 64];
    for y in top..bottom {
        black_alpha[..count].fill(0.0);
        for x in left..right {
            let px = x as f32 + 0.5 - software.translation[0];
            let py = y as f32 + 0.5 - software.translation[1];
            let (px, py) = if identity {
                (px, py)
            } else {
                software.inverse.transform_point(px, py)
            };
            let local = if let Mapping::RotatedCorner { center, cos, sin } = software.mapping {
                let dx = x as f32 - software.translation[0] - center[0];
                let dy = y as f32 - software.translation[1] - center[1];
                [
                    dx * cos + dy * sin + software.destination[2] / 2.0,
                    -dx * sin + dy * cos + software.destination[3] / 2.0,
                ]
            } else {
                [px - software.destination[0], py - software.destination[1]]
            };
            let inclusive = matches!(software.mapping, Mapping::RotatedCorner { .. });
            if (0..2).any(|i| {
                local[i] < 0.0
                    || if inclusive {
                        local[i] > software.destination[i + 2]
                    } else {
                        local[i] >= software.destination[i + 2]
                    }
            }) {
                continue;
            }
            if let Some(fog) = software.fog {
                let amounts = if let Mapping::RotatedCorner { center, cos, sin } = software.mapping
                {
                    let dx = x as f32 + 0.5 - software.translation[0] - center[0];
                    let dy = y as f32 + 0.5 - software.translation[1] - center[1];
                    [
                        ((dx * cos + dy * sin + software.destination[2] / 2.0)
                            / software.destination[2])
                            .clamp(0.0, 1.0)
                            * software.source[2],
                        ((-dx * sin + dy * cos + software.destination[3] / 2.0)
                            / software.destination[3])
                            .clamp(0.0, 1.0)
                            * software.source[3],
                    ]
                } else {
                    let points = [px, py];
                    std::array::from_fn(|i| {
                        ((points[i] - fog.destination[i]) / fog.destination[i + 2]).clamp(0.0, 1.0)
                            * software.source[i + 2]
                    })
                };
                if (0..2).any(|i| {
                    let amount = amounts[i];
                    amount < fog.source_range[i]
                        || amount >= fog.source_range[i + 2]
                            && fog.source_range[i + 2] < software.source[i + 2]
                }) {
                    continue;
                }
            }
            let mut u = local[0] / software.destination[2];
            if software.flip_x {
                u = 1.0 - u;
            }
            let [sx, sy] = if matches!(software.mapping, Mapping::RotatedCorner { .. }) {
                // Retain the generic mapping's second flip and its rounding;
                // source membership uses corners, while fog above uses centers.
                let u = if software.flip_x { 1.0 - u } else { u };
                let sx = (u * sample_size[0]).clamp(0.0, sample_size[0]).floor();
                [
                    software.source[0]
                        + if software.flip_x {
                            sample_size[0] - sx
                        } else {
                            sx
                        },
                    software.source[1]
                        + (local[1] / software.destination[3] * sample_size[1])
                            .clamp(0.0, sample_size[1])
                            .floor(),
                ]
            } else {
                [
                    software.source[0] + u * sample_size[0],
                    software.source[1] + local[1] / software.destination[3] * sample_size[1],
                ]
            };
            let tile = vertices[0].sample_tile;
            let origin = [
                (sx / tile[2]).floor() * tile[2],
                (sy / tile[2]).floor() * tile[2],
            ];
            let alpha_at = |x: i32, y: i32| {
                let [x, y] = if tile[3] != 0.0 {
                    [
                        x.clamp(origin[0] as i32, (origin[0] + tile[2] - 1.0) as i32),
                        y.clamp(origin[1] as i32, (origin[1] + tile[2] - 1.0) as i32),
                    ]
                } else {
                    [
                        x.clamp(0, texture.extent[0] as i32 - 1),
                        y.clamp(0, texture.extent[1] as i32 - 1),
                    ]
                };
                if x < 0 || y < 0 || x >= texture.extent[0] as i32 || y >= texture.extent[1] as i32
                {
                    0.0
                } else {
                    f32::from(
                        texture.pixels
                            [(y as usize * texture.extent[0] as usize + x as usize) * 4 + 3],
                    )
                }
            };
            let alpha = if sampler == GpuSampler::Nearest {
                alpha_at(sx.floor() as i32, sy.floor() as i32)
            } else {
                let [sx, sy] = [sx - 0.5, sy - 0.5];
                let [ix, iy] = [sx.floor() as i32, sy.floor() as i32];
                let [fx, fy] = [sx - sx.floor(), sy - sy.floor()];
                let top = alpha_at(ix, iy) * (1.0 - fx) + alpha_at(ix + 1, iy) * fx;
                let bottom = alpha_at(ix, iy + 1) * (1.0 - fx) + alpha_at(ix + 1, iy + 1) * fx;
                top * (1.0 - fy) + bottom * fy
            };
            if alpha == 0.0 {
                continue;
            }
            black_alpha[(x - left) as usize] = (alpha - transparency).max(0.0);
        }
        let target_row = ((y as u32 - frame.row_origin) as usize
            * scene.logical_extent[0] as usize
            + left as usize)
            * 4;
        spans::blend_black(
            &black_alpha[..count],
            black,
            &mut frame.pixels[target_row..target_row + count * 4],
        );
    }
    true
}

fn sample_surface_blit(
    texture: &GpuTextureResource,
    blit: crate::GpuSoftwareBlit,
    x: i32,
    y: i32,
) -> Option<Color> {
    let x = x as f32 - blit.translation[0];
    let y = y as f32 - blit.translation[1];
    let (local_x, local_y) = match blit.mapping {
        crate::GpuSoftwareBlitMapping::Unscaled(origin) => {
            (x - origin.x as f32, y - origin.y as f32)
        }
        crate::GpuSoftwareBlitMapping::Stretched(dest) => {
            let local_x = x - dest.x as f32;
            let local_y = y - dest.y as f32;
            if local_x < 0.0
                || local_y < 0.0
                || local_x >= dest.width as f32
                || local_y >= dest.height as f32
            {
                return None;
            }
            (
                (local_x as u32 * blit.source.width / dest.width) as f32,
                (local_y as u32 * blit.source.height / dest.height) as f32,
            )
        }
        crate::GpuSoftwareBlitMapping::Transformed { origin, inverse } => {
            let (source_x, source_y) = inverse.transform_point(x + 0.5, y + 0.5);
            (
                (source_x - origin.x as f32).floor(),
                (source_y - origin.y as f32).floor(),
            )
        }
    };
    if !local_x.is_finite()
        || !local_y.is_finite()
        || local_x < 0.0
        || local_y < 0.0
        || local_x >= blit.source.width as f32
        || local_y >= blit.source.height as f32
    {
        return None;
    }
    let source_x = blit.source.x as u32 + local_x as u32;
    let source_y = blit.source.y as u32 + local_y as u32;
    let offset = (source_y as usize * texture.extent[0] as usize + source_x as usize) * 4;
    let pixel = texture.pixels.get(offset..offset + 4)?;
    Some(Color::new(pixel[0], pixel[1], pixel[2], pixel[3]))
}

fn quad_weights(positions: [[f32; 2]; 4], w: [f32; 4], point: [f32; 2]) -> Option<[f32; 4]> {
    for indices in [[0, 1, 2], [2, 1, 3]] {
        let [a, b, c] = indices;
        let area = edge(positions[a], positions[b], positions[c]);
        if area == 0.0 {
            continue;
        }
        let mut weights = [0.0; 4];
        weights[a] = edge(positions[b], positions[c], point) / area;
        weights[b] = edge(positions[c], positions[a], point) / area;
        weights[c] = edge(positions[a], positions[b], point) / area;
        if indices.iter().any(|i| weights[*i] < 0.0) {
            continue;
        }
        let denominator: f32 = indices.iter().map(|i| weights[*i] / w[*i]).sum();
        for i in indices {
            weights[i] = weights[i] / w[i] / denominator;
        }
        return Some(weights);
    }
    None
}

fn sample_texture(
    texture: &GpuTextureResource,
    uv: [f32; 2],
    sampler: GpuSampler,
    tile: [f32; 4],
) -> [f32; 4] {
    sample_texture_edge(
        texture,
        [
            uv[0] * texture.extent[0] as f32,
            uv[1] * texture.extent[1] as f32,
        ],
        sampler,
        tile,
    )
}

fn sample_font_texture(
    texture: &GpuTextureResource,
    software: &crate::GpuSoftwareSprite,
    point: [f32; 2],
) -> [f32; 4] {
    let crate::GpuSoftwareSpriteMapping::Font {
        shear,
        center_y,
        texture_indent,
        physical_size,
        normalize_transparent,
    } = software.mapping
    else {
        return [0.0; 4];
    };
    let physical_size = if normalize_transparent {
        physical_size
    } else {
        let need = software.source[2].min(software.source[3]).max(1.0) as u32;
        need.checked_next_power_of_two()
            .unwrap_or(4096)
            .clamp(2, 4096) as f32
    };
    // StdFont.cpp:814-903 keeps the glyph's local division before adding its
    // atlas inset. Normalizing atlas UVs first changes half-byte boundaries.
    let raw = [
        (point[0] - shear * (point[1] - center_y) - software.destination[0])
            / (software.destination[2] / software.source[2]),
        (point[1] - software.destination[1]) / (software.destination[3] / software.source[3]),
    ];
    let tile_origin: [i32; 2] = std::array::from_fn(|axis| {
        let tiles = (software.source[axis + 2] as i32 - 1) / physical_size as i32 + 1;
        ((raw[axis] / physical_size).floor() as i32).clamp(0, tiles - 1) * physical_size as i32
    });
    let edge: [f32; 2] = std::array::from_fn(|axis| {
        if texture_indent == 0.0 {
            return raw[axis];
        }
        let start = tile_origin[axis] as f32;
        let adjusted = start
            + texture_indent
            + (raw[axis] - start) * physical_size / (physical_size + 2.0 * texture_indent);
        adjusted.clamp(start, start + physical_size)
    });
    let local: [f32; 2] = std::array::from_fn(|axis| edge[axis] - 0.5 - tile_origin[axis] as f32);
    let left = local[0].floor() as i32;
    let top = local[1].floor() as i32;
    let fx = local[0] - left as f32;
    let fy = local[1] - top as f32;
    let texel = |x: i32, y: i32| {
        let x = tile_origin[0] + x.clamp(0, physical_size as i32 - 1);
        let y = tile_origin[1] + y.clamp(0, physical_size as i32 - 1);
        // Each glyph is a separate C4Surface facet even when retained in an
        // atlas. Its unused physical texture storage is transparent white.
        if x < 0 || y < 0 || x >= software.source[2] as i32 || y >= software.source[3] as i32 {
            return [255.0, 255.0, 255.0, 0.0];
        }
        let x = software.source[0] as i32 + x;
        let y = software.source[1] as i32 + y;
        if x < 0 || y < 0 || x >= texture.extent[0] as i32 || y >= texture.extent[1] as i32 {
            return [255.0, 255.0, 255.0, 0.0];
        }
        let offset = (y as usize * texture.extent[0] as usize + x as usize) * 4;
        if normalize_transparent && texture.pixels[offset + 3] == 0 {
            [0.0; 4]
        } else {
            std::array::from_fn(|channel| f32::from(texture.pixels[offset + channel]))
        }
    };
    let samples = [
        texel(left, top),
        texel(left + 1, top),
        texel(left, top + 1),
        texel(left + 1, top + 1),
    ];
    std::array::from_fn(|channel| {
        let top = samples[0][channel] * (1.0 - fx) + samples[1][channel] * fx;
        let bottom = samples[2][channel] * (1.0 - fx) + samples[3][channel] * fx;
        top * (1.0 - fy) + bottom * fy
    })
}

fn sample_texture_edge(
    texture: &GpuTextureResource,
    edges: [f32; 2],
    sampler: GpuSampler,
    tile: [f32; 4],
) -> [f32; 4] {
    sample_texture_edge_impl(texture, edges, sampler, tile, false)
}

fn sample_texture_edge_impl(
    texture: &GpuTextureResource,
    [x, y]: [f32; 2],
    sampler: GpuSampler,
    tile: [f32; 4],
    normalize_transparent: bool,
) -> [f32; 4] {
    let tile = if tile[3] != 0.0 {
        [
            (x / tile[2]).floor() * tile[2],
            (y / tile[2]).floor() * tile[2],
            tile[2],
            tile[3],
        ]
    } else {
        tile
    };
    let sample = |x: i32, y: i32| {
        let (x, y) = if tile[3] != 0.0 {
            (
                x.clamp(tile[0] as i32, (tile[0] + tile[2] - 1.0) as i32),
                y.clamp(tile[1] as i32, (tile[1] + tile[2] - 1.0) as i32),
            )
        } else {
            (
                x.clamp(0, texture.extent[0] as i32 - 1),
                y.clamp(0, texture.extent[1] as i32 - 1),
            )
        };
        if x < 0 || y < 0 || x >= texture.extent[0] as i32 || y >= texture.extent[1] as i32 {
            return [255.0, 255.0, 255.0, 0.0];
        }
        let offset = y as usize * texture.extent[0] as usize + x as usize;
        match texture.format {
            GpuTextureFormat::Rgba8 => {
                if normalize_transparent && texture.pixels[offset * 4 + 3] == 0 {
                    [0.0; 4]
                } else {
                    std::array::from_fn(|channel| f32::from(texture.pixels[offset * 4 + channel]))
                }
            }
            GpuTextureFormat::R8 => {
                let value = f32::from(texture.pixels[offset]);
                [value, value, value, 255.0]
            }
        }
    };
    match sampler {
        GpuSampler::Nearest => sample(x.floor() as i32, y.floor() as i32),
        GpuSampler::Linear => {
            let [x, y] = [x - 0.5, y - 0.5];
            let [left, top] = [x.floor() as i32, y.floor() as i32];
            let [fx, fy] = [x - x.floor(), y - y.floor()];
            let samples = [
                sample(left, top),
                sample(left + 1, top),
                sample(left, top + 1),
                sample(left + 1, top + 1),
            ];
            std::array::from_fn(|channel| {
                let top = samples[0][channel] * (1.0 - fx) + samples[1][channel] * fx;
                let bottom = samples[2][channel] * (1.0 - fx) + samples[3][channel] * fx;
                top * (1.0 - fy) + bottom * fy
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Color, GammaRamp, GpuSceneRecorder};

    fn validation_scene(textures: Vec<GpuTextureResource>, commands: Vec<GpuCommand>) -> GpuScene {
        GpuScene::new(
            [1, 1],
            Color::transparent(),
            crate::GpuGammaLut::from_ramp(&GammaRamp::identity()),
            crate::GpuGammaMode::Disabled,
            textures,
            commands,
        )
    }

    fn assert_scene_rejected_before_output_changes(scene: &GpuScene) {
        for loaded in [false, true] {
            let mut renderer = CpuSceneRenderer::default();
            let mut frame = [17; 4];
            let result = if loaded {
                renderer.render_loaded(scene, &mut frame)
            } else {
                renderer.render(scene, &mut frame)
            };
            assert!(result.is_err(), "loaded={loaded}");
            assert_eq!(frame, [17; 4], "loaded={loaded}");
        }
    }

    fn landscape_span_scene(
        software: crate::GpuSoftwareSprite,
        positions: [[f32; 3]; 4],
    ) -> GpuScene {
        let texture = GpuTextureResource::immutable_rgba(
            crate::GpuTextureId::fresh(),
            1,
            1,
            std::sync::Arc::from([20, 30, 40, 128]),
        );
        let mut recorder = crate::GpuSceneRecorder::default();
        let id = recorder.add_software_sprite(software).unwrap();
        recorder.add_texture(texture.clone());
        let vertices = sprite_vertices(
            positions,
            [0.0, 0.0, 1.0, 1.0],
            [0x00ff_ffff; 4],
            crate::GpuOuterModulation::Inherit,
        )
        .map(|mut vertex| {
            vertex.software_sprite = Some(id);
            vertex
        });
        recorder.push(GpuCommand::Landscape {
            base: texture.id,
            liquid_mask: None,
            liquid: None,
            vertices,
            clip: None,
            phase: [0.0; 3],
            gamma: false,
        });
        recorder.into_scene([2, 1], Color::transparent(), &crate::GammaRamp::identity())
    }
    fn span_software() -> crate::GpuSoftwareSprite {
        crate::GpuSoftwareSprite {
            destination: [0.0, 0.0, 1.0, 1.0],
            source: [0.0, 0.0, 1.0, 1.0],
            inverse: crate::Transform::identity(),
            translation: [0.0; 2],
            flip_x: false,
            inclusive_source_end: false,
            gamma: None,
            mapping: crate::GpuSoftwareSpriteMapping::Landscape {
                zoom: 1.0,
                indent: 0.0,
                world_extent: [1, 1],
                tile_origin: [0, 0],
                texture_size: 1,
            },
            fog: None,
        }
    }
    fn black_span_differential_scene(
        landscape: bool,
        alpha_pattern: u8,
        fog_holes: bool,
        modulation: [u32; 4],
        liquid: bool,
    ) -> GpuScene {
        let [width, height] = [129, 4];
        let positions = [
            [0.0, 0.0, 1.0],
            [width as f32, 0.0, 1.0],
            [0.0, height as f32, 1.0],
            [width as f32, height as f32, 1.0],
        ];
        let background = GpuTextureResource::immutable_rgba(
            crate::GpuTextureId::fresh(),
            width,
            height,
            (0..width * height)
                .flat_map(|pixel| {
                    let row = pixel / width;
                    [
                        pixel as u8,
                        (pixel * 7) as u8,
                        (row * 51) as u8,
                        80 + row as u8 * 31,
                    ]
                })
                .collect::<Vec<_>>()
                .into(),
        );
        let mut recorder = GpuSceneRecorder::default();
        recorder.add_texture(background.clone());
        recorder.push(GpuCommand::Quad {
            texture: background.id,
            owner_mask: None,
            vertices: sprite_vertices(
                positions,
                [0.0, 0.0, 1.0, 1.0],
                [0x00ff_ffff; 4],
                crate::GpuOuterModulation::Inherit,
            ),
            clip: None,
            blend: GpuBlend::Normal,
            base_mod2: false,
            owner_mod2: false,
            sampler: GpuSampler::Nearest,
            gamma: false,
        });
        let source_width = if landscape { width } else { 65 };
        let source_height = if landscape { height } else { 2 };
        let pixels = (0..source_width * source_height)
            .flat_map(|pixel| {
                let x = pixel % source_width;
                let y = pixel / source_width;
                let alpha = match alpha_pattern {
                    1 if y == 0 && x == source_width / 2 - 1 => 128,
                    1 if y == 0 && x == source_width / 2 => 0,
                    2 if y == 0 => 0,
                    3 => [0, 127, 255][pixel as usize % 3],
                    _ => 255,
                };
                [
                    (pixel * 19) as u8,
                    (pixel * 37) as u8,
                    (pixel * 61) as u8,
                    alpha,
                ]
            })
            .collect::<Vec<_>>();
        let texture = GpuTextureResource::immutable_rgba(
            crate::GpuTextureId::fresh(),
            source_width,
            source_height,
            pixels.into(),
        );
        let mut software = span_software();
        software.destination = [0.0, 0.0, width as f32, height as f32];
        software.source = [0.0, 0.0, source_width as f32, source_height as f32];
        software.mapping = if landscape {
            crate::GpuSoftwareSpriteMapping::Landscape {
                zoom: 1.0,
                indent: 0.0,
                world_extent: [source_width, source_height],
                tile_origin: [0, 0],
                texture_size: 8,
            }
        } else {
            crate::GpuSoftwareSpriteMapping::IntegerStretch
        };
        software.fog = Some(crate::GpuSoftwareFog {
            destination: software.destination,
            source_range: if fog_holes {
                let scale = if landscape { 2.0 } else { 1.0 };
                [16.0 * scale, 0.0, 32.0 * scale, source_height as f32]
            } else {
                [0.0, 0.0, source_width as f32, source_height as f32]
            },
        });
        let software_id = recorder.add_software_sprite(software).unwrap();
        let vertices = sprite_vertices(
            positions,
            [0.0, 0.0, 1.0, 1.0],
            modulation,
            crate::GpuOuterModulation::Combine,
        )
        .map(|mut vertex| {
            vertex.software_sprite = Some(software_id);
            vertex
        });
        recorder.add_texture(texture.clone());
        if landscape {
            let (liquid_mask, liquid_texture) = if liquid {
                let mut mask = texture.clone();
                mask.id = crate::GpuTextureId::fresh();
                mask.format = GpuTextureFormat::R8;
                mask.pixels = (0..source_width * source_height)
                    .map(|index| u8::from(index % 3 != 0))
                    .collect::<Vec<_>>()
                    .into();
                let liquid = GpuTextureResource::immutable_rgba(
                    crate::GpuTextureId::fresh(),
                    8,
                    8,
                    [30, 220, 85, 255].repeat(64).into(),
                );
                recorder.add_texture(mask.clone());
                recorder.add_texture(liquid.clone());
                (Some(mask.id), Some(liquid.id))
            } else {
                (None, None)
            };
            recorder.push(GpuCommand::Landscape {
                base: texture.id,
                liquid_mask,
                liquid: liquid_texture,
                vertices,
                clip: None,
                phase: [0.13, -0.07, 0.11],
                gamma: true,
            });
        } else {
            recorder.push(GpuCommand::Quad {
                texture: texture.id,
                owner_mask: None,
                vertices,
                clip: None,
                blend: GpuBlend::Normal,
                base_mod2: false,
                owner_mod2: false,
                sampler: GpuSampler::Nearest,
                gamma: true,
            });
        }
        recorder.into_scene(
            [width, height],
            Color::new(37, 63, 89, 97),
            &GammaRamp::standard(),
        )
    }

    #[test]
    fn black_span_uses_fractional_widget_gamma_before_monitor_resolve() {
        // StdGL.cpp:1081-1087 samples the per-channel R16 gamma textures
        // before source-over. Their fractional values survive until store.
        let texture = GpuTextureResource::immutable_rgba(
            crate::GpuTextureId::fresh(),
            1,
            1,
            std::sync::Arc::from([221, 143, 97, 127]),
        );
        let mut scene = validation_scene(vec![texture.clone()], Vec::new());
        scene.gamma_mode = crate::GpuGammaMode::Monitor;
        let mut software = span_software();
        software.mapping = crate::GpuSoftwareSpriteMapping::Native;
        let vertices = sprite_vertices(
            [
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
            ],
            [0.0, 0.0, 1.0, 1.0],
            [0; 4],
            crate::GpuOuterModulation::Combine,
        );
        let widget = std::sync::Arc::new(std::array::from_fn(|channel| {
            [[256, 32767, 65534][channel]; 256]
        }));
        for (gamma, expected) in [(true, [19, 108, 233, 176]), (false, [19, 45, 106, 176])] {
            let mut pixels = [37, 89, 211, 97];
            let tables = SpanTables::default();
            let mut target = RasterTarget {
                pixels: &mut pixels,
                bounds: Rect::new(0, 0, 1, 1),
                tables: &tables,
                row_origin: 0,
                gamma_raw: None,
                gamma_override: Some(widget.clone()),
            };
            assert!(draw_black_sprite_span(
                &scene,
                &mut target,
                &vertices,
                &texture,
                &software,
                GpuSampler::Nearest,
                gamma,
                [0, 0, 1, 1]
            ));
            assert_eq!(pixels, expected, "gamma={gamma}");
        }
    }

    #[test]
    fn nearest_span_uses_current_or_cached_widget_tables_in_monitor_mode() {
        // StdGL.cpp:1081-1087 samples the supplied R16 RGB ramps independently;
        // an explicit widget ramp remains a fragment operation before monitor resolve.
        let texture = GpuTextureResource::immutable_rgba(
            crate::GpuTextureId::fresh(),
            1,
            1,
            std::sync::Arc::from([193, 101, 37, 255]),
        );
        let widget = crate::GpuGammaLut::from_ramp(&GammaRamp::from_control_points([
            0x172b43, 0x698bad, 0xdff1fb,
        ]));
        let mut software = span_software();
        software.mapping = crate::GpuSoftwareSpriteMapping::Native;
        let vertices = sprite_vertices(
            [
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
            ],
            [0.0, 0.0, 1.0, 1.0],
            [0x00ff_ffff; 4],
            crate::GpuOuterModulation::Inherit,
        );
        let expected = std::array::from_fn::<_, 4, _>(|channel| {
            if channel == 3 {
                255
            } else {
                store_channel(
                    f32::from(widget.channels[channel][usize::from(texture.pixels[channel])])
                        / 257.0,
                )
            }
        });
        for cached in [false, true] {
            let mut scene = validation_scene(vec![texture.clone()], Vec::new());
            scene.gamma_mode = crate::GpuGammaMode::Fragment;
            scene.gamma = widget.clone();
            let mut tables = SpanTables::default();
            tables.update(&scene);
            scene.gamma = crate::GpuGammaLut::from_ramp(&GammaRamp::standard());
            if cached {
                tables.update(&scene);
            }
            scene.gamma_mode = crate::GpuGammaMode::Monitor;
            for same_backing in [false, true] {
                let mut pixels = [37, 89, 211, 97];
                let channels = if same_backing {
                    widget.channels.clone()
                } else {
                    std::sync::Arc::new(*widget.channels)
                };
                let mut target = RasterTarget {
                    pixels: &mut pixels,
                    bounds: Rect::new(0, 0, 1, 1),
                    tables: &tables,
                    row_origin: 0,
                    gamma_raw: None,
                    gamma_override: Some(channels),
                };
                assert!(draw_nearest_sprite_span(
                    &scene,
                    &mut target,
                    &vertices,
                    &texture,
                    &software,
                    true,
                    false,
                    [0, 0, 1, 1]
                ));
                assert_eq!(
                    pixels, expected,
                    "cached={cached} same_backing={same_backing}"
                );
            }
        }
    }

    #[test]
    fn widget_gamma_spans_match_generic_across_cold_cache_and_alternating_ramps() {
        // StdGL.cpp:1081-1087 applies explicitly supplied per-channel R16
        // ramps at fragment time; monitor resolution remains a separate pass.
        // Keep source geometry identical and disable only the dispatcher in
        // the oracle, including the legacy alpha equation for white modulation.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        pool.install(|| {
            struct ResetGeneric;
            impl Drop for ResetGeneric {
                fn drop(&mut self) {FORCE_GENERIC_SPANS.with(|flag| flag.set(false));}
            }
            let _reset = ResetGeneric;
            let standard = crate::GpuGammaLut::from_ramp(&GammaRamp::standard());
            let widget = crate::GpuGammaLut::from_ramp(&GammaRamp::from_control_points([0x172b43, 0x698bad, 0xdff1fb]));
            let lightning = crate::GpuGammaLut::from_ramp(&GammaRamp::from_control_points([0x031547, 0xafc579, 0xe7fff3]));
            for mapping in 0..4 {
                for alpha in [0, 3] {
                    for holes in [false, true] {
                        for modulation in [[0; 4], [0x00ff_ffff; 4], [0x4081_33e7; 4], [0, 0x007f_7f7f, 0x00ff_ffff, 0]] {
                            let mut original = black_span_differential_scene(false, alpha, holes, modulation, false);
                            original.software_sprites[0].mapping = if mapping == 1 {
                                crate::GpuSoftwareSpriteMapping::GuiNearest
                            } else if mapping == 3 {
                                crate::GpuSoftwareSpriteMapping::RotatedCorner {center: [64.5, 2.0], cos: 0.9998, sin: 0.02}
                            } else {crate::GpuSoftwareSpriteMapping::Native};
                            if let GpuCommand::Quad {sampler, vertices, ..} = original.commands.last_mut().unwrap() {
                                *sampler = if mapping == 2 {GpuSampler::Linear} else {GpuSampler::Nearest};
                                if modulation == [0x00ff_ffff; 4] {
                                    vertices.iter_mut().for_each(|vertex| vertex.outer_modulation = crate::GpuOuterModulation::Inherit);
                                }
                                if mapping == 2 {
                                    vertices.iter_mut().for_each(|vertex| vertex.sample_tile = [0.0, 0.0, 16.0, 2.0]);
                                }
                            }
                            for warm in [false, true] {
                                for mode in [crate::GpuGammaMode::Disabled, crate::GpuGammaMode::Fragment, crate::GpuGammaMode::Monitor] {
                                    let mut actual_renderer = CpuSceneRenderer::default();
                                    let mut generic_renderer = CpuSceneRenderer::default();
                                    if warm {
                                        let mut preparation = original.clone();
                                        preparation.gamma_mode = crate::GpuGammaMode::Fragment;
                                        for gamma in [&standard, &widget] {
                                            preparation.gamma = gamma.clone();
                                            actual_renderer.tables.update(&preparation);
                                            generic_renderer.tables.update(&preparation);
                                        }
                                    }
                                    let mut actual = vec![0; 129 * 4 * 4];
                                    let mut expected = actual.clone();
                                    for global in [&standard, &lightning, &standard, &lightning] {
                                        for override_gamma in [None, Some(&standard), Some(&widget)] {
                                            for gamma in [false, true] {
                                                let mut scene = original.clone();
                                                scene.gamma = global.clone();
                                                scene.gamma_mode = mode;
                                                scene.software_sprites[0].gamma = override_gamma.cloned();
                                                if let GpuCommand::Quad {gamma: selected, ..} = scene.commands.last_mut().unwrap() {*selected = gamma;}
                                                actual_renderer.invalidate();
                                                generic_renderer.invalidate();
                                                FORCE_GENERIC_SPANS.with(|flag| flag.set(false));
                                                actual_renderer.render(&scene, &mut actual).unwrap();
                                                FORCE_GENERIC_SPANS.with(|flag| flag.set(true));
                                                generic_renderer.render(&scene, &mut expected).unwrap();
                                                assert_eq!(actual.iter().zip(&expected).position(|(a,b)| a != b), None,
                                                    "mapping={mapping} alpha={alpha} holes={holes} modulation={modulation:?} warm={warm} mode={mode:?} global={} override={:?} gamma={gamma}", global.revision, override_gamma.map(|gamma| gamma.revision));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    #[test]
    fn differing_widget_gamma_keeps_shader_alpha_for_partial_white_nearest() {
        let texture = GpuTextureResource::immutable_rgba(
            crate::GpuTextureId::fresh(),
            1,
            1,
            std::sync::Arc::from([193, 101, 37, 127]),
        );
        let widget = crate::GpuGammaLut::from_ramp(&GammaRamp::from_control_points([
            0x172b43, 0x698bad, 0xdff1fb,
        ]));
        let mut recorder = GpuSceneRecorder::default();
        recorder.add_texture(texture.clone());
        let mut software = span_software();
        software.mapping = crate::GpuSoftwareSpriteMapping::Native;
        software.gamma = Some(widget.clone());
        let id = recorder.add_software_sprite(software).unwrap();
        let vertices = sprite_vertices(
            [
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
            ],
            [0.0, 0.0, 1.0, 1.0],
            [0x00ff_ffff; 4],
            crate::GpuOuterModulation::Inherit,
        )
        .map(|mut vertex| {
            vertex.software_sprite = Some(id);
            vertex
        });
        recorder.push(GpuCommand::Quad {
            texture: texture.id,
            owner_mask: None,
            vertices,
            clip: None,
            blend: GpuBlend::Normal,
            base_mod2: false,
            owner_mod2: false,
            sampler: GpuSampler::Nearest,
            gamma: true,
        });
        let scene =
            recorder.into_scene([1, 1], Color::new(37, 89, 211, 97), &GammaRamp::standard());
        let mut preparation = scene.clone();
        preparation.gamma = widget;
        let mut renderer = CpuSceneRenderer::default();
        renderer.tables.update(&preparation);
        let mut output = [0; 4];
        renderer.render(&scene, &mut output).unwrap();
        // Differing explicit gamma disables the generic unmodulated_nearest
        // shortcut even for white vertices. Its Shader alpha is round(127 +
        // 97 * (1 - 127/255)) = 176; Legacy's integer alpha would be 175.
        assert_eq!(output[3], 176);
    }

    #[test]
    fn gui_linear_span_preserves_rgb_of_transparent_filter_samples() {
        // StdDDraw2.cpp:731-742 retains source/destination tile edges;
        // StdGL.cpp:1076-1086 filters raw RGB before modulation and gamma.
        let texture = GpuTextureResource::immutable_rgba(
            crate::GpuTextureId::fresh(),
            2,
            1,
            std::sync::Arc::from([255, 17, 99, 0, 31, 177, 211, 127]),
        );
        let mut recorder = GpuSceneRecorder::default();
        recorder.add_texture(texture.clone());
        let mut software = span_software();
        software.source = [0.0, 0.0, 2.0, 1.0];
        software.mapping = crate::GpuSoftwareSpriteMapping::GuiLinear { modulation: None };
        let id = recorder.add_software_sprite(software).unwrap();
        let vertices = sprite_vertices(
            [
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
            ],
            [0.0, 0.0, 1.0, 1.0],
            [0x00ff_ffff; 4],
            crate::GpuOuterModulation::Inherit,
        )
        .map(|mut vertex| {
            vertex.software_sprite = Some(id);
            vertex
        });
        recorder.push(GpuCommand::Quad {
            texture: texture.id,
            owner_mask: None,
            vertices,
            clip: None,
            blend: GpuBlend::Normal,
            base_mod2: false,
            owner_mod2: false,
            sampler: GpuSampler::Linear,
            gamma: false,
        });
        let scene = recorder.into_scene([1, 1], Color::new(11, 33, 55, 77), &GammaRamp::identity());
        let mut actual = [0; 4];
        CpuSceneRenderer::default()
            .render(&scene, &mut actual)
            .unwrap();
        assert_eq!(actual, [44, 49, 80, 121]);
    }

    #[test]
    fn gui_linear_spans_match_generic_with_widget_gamma_and_tile_edges() {
        // StdGL.cpp:1076-1086 applies modulation and independent RGB gamma
        // lookups before source-over. StdDDraw2.cpp:731-751 keeps tile edges
        // and the source inset's division order. Neither execution changes
        // coordinates; the reference disables only the specialized dispatcher.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        pool.install(|| {
            struct ResetGeneric;
            impl Drop for ResetGeneric {
                fn drop(&mut self) {FORCE_GENERIC_SPANS.with(|flag| flag.set(false));}
            }
            let _reset = ResetGeneric;
            let widget_gamma = crate::GpuGammaLut::from_ramp(&GammaRamp::from_control_points([
                0x17314b, 0x93bd67, 0xe5f9db,
            ]));
            for translated in [false, true] {
                for holes in [false, true] {
                    for modulation in [None, Some(0), Some(0x4081_33e7), Some(0xff7f_31a3)] {
                        for tiled in [false, true] {
                            for clipped in [false, true] {
                                let mut original = black_span_differential_scene(false, 3, holes, [0x00ff_ffff; 4], false);
                                let software = &mut original.software_sprites[0];
                                software.mapping = crate::GpuSoftwareSpriteMapping::GuiLinear {modulation};
                                if translated {
                                    software.translation = [0.375, 0.125];
                                    software.source = [0.125, 0.25, 64.75, 1.75];
                                    let fog = software.fog.as_mut().unwrap();
                                    fog.source_range[3] = 1.75;
                                    if !holes {fog.source_range[2] = 64.75;}
                                }
                                if let GpuCommand::Quad {sampler, vertices, clip, ..} = original.commands.last_mut().unwrap() {
                                    *sampler = GpuSampler::Linear;
                                    if tiled {vertices.iter_mut().for_each(|vertex| vertex.sample_tile = [0.0, 0.0, 16.0, 2.0]);}
                                    if clipped {*clip = Some(Rect::new(61, 1, 6, 2));}
                                }
                                for mode in [crate::GpuGammaMode::Disabled, crate::GpuGammaMode::Fragment, crate::GpuGammaMode::Monitor] {
                                    for asymmetric in [false, true] {
                                        for widget in 0..3 {
                                            for gamma in [false, true] {
                                                let mut scene = original.clone();
                                                scene.gamma_mode = mode;
                                                if asymmetric {scene.gamma = widget_gamma.clone();}
                                                scene.software_sprites[0].gamma = match widget {
                                                    1 => Some(scene.gamma.clone()),
                                                    2 => Some(widget_gamma.clone()),
                                                    _ => None,
                                                };
                                                if let GpuCommand::Quad {gamma: selected, ..} = scene.commands.last_mut().unwrap() {*selected = gamma;}
                                                let mut actual = vec![0; 129 * 4 * 4];
                                                let mut expected = actual.clone();
                                                FORCE_GENERIC_SPANS.with(|flag| flag.set(false));
                                                CpuSceneRenderer::default().render(&scene, &mut actual).unwrap();
                                                FORCE_GENERIC_SPANS.with(|flag| flag.set(true));
                                                CpuSceneRenderer::default().render(&scene, &mut expected).unwrap();
                                                assert_eq!(actual.iter().zip(&expected).position(|(a,b)| a != b), None,
                                                    "translated={translated} holes={holes} modulation={modulation:?} tiled={tiled} clipped={clipped} mode={mode:?} asymmetric={asymmetric} widget={widget} gamma={gamma}");
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    #[test]
    fn gui_linear_unsupported_span_geometry_preserves_generic_output() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        pool.install(|| {
            struct ResetGeneric;
            impl Drop for ResetGeneric {
                fn drop(&mut self) {
                    FORCE_GENERIC_SPANS.with(|flag| flag.set(false));
                }
            }
            let _reset = ResetGeneric;
            for unsupported in 0..8 {
                let mut scene =
                    black_span_differential_scene(false, 3, false, [0x00ff_ffff; 4], false);
                scene.software_sprites[0].mapping = crate::GpuSoftwareSpriteMapping::GuiLinear {
                    modulation: Some(0x4081_33e7),
                };
                if unsupported == 0 {
                    scene.software_sprites[0].inverse =
                        crate::Transform::set(0.97, 0.13, 0.25, -0.003, 1.0, 0.125, 0.0, 0.0, 1.0);
                } else if unsupported == 5 {
                    scene.software_sprites[0].source[0] = 4_000_001.0;
                }
                let GpuCommand::Quad {
                    sampler,
                    vertices,
                    blend,
                    base_mod2,
                    ..
                } = scene.commands.last_mut().unwrap()
                else {
                    unreachable!()
                };
                *sampler = GpuSampler::Linear;
                match unsupported {
                    1 => {
                        vertices[1].position[1] = 0.25;
                        vertices[3].position[1] += 0.25;
                    }
                    2 => *base_mod2 = true,
                    3 => *blend = GpuBlend::Additive,
                    4 => vertices.iter_mut().for_each(|vertex| {
                        vertex.software_alpha_mode = GpuSolidAlphaMode::NonSeparate
                    }),
                    6 => vertices[1].position[2] = 1.001,
                    7 => vertices
                        .iter_mut()
                        .for_each(|vertex| vertex.sample_tile = [0.0, 0.0, 16.0, -1.0]),
                    _ => {}
                }
                let mut actual = vec![0; 129 * 4 * 4];
                let mut expected = actual.clone();
                FORCE_GENERIC_SPANS.with(|flag| flag.set(false));
                CpuSceneRenderer::default()
                    .render(&scene, &mut actual)
                    .unwrap();
                FORCE_GENERIC_SPANS.with(|flag| flag.set(true));
                CpuSceneRenderer::default()
                    .render(&scene, &mut expected)
                    .unwrap();
                assert_eq!(actual, expected, "unsupported={unsupported}");
            }
        });
    }

    #[test]
    fn gui_linear_texture_format_is_rejected_before_span_or_generic_execution() {
        let mut scene = black_span_differential_scene(false, 3, false, [0x00ff_ffff; 4], false);
        scene.software_sprites[0].mapping =
            crate::GpuSoftwareSpriteMapping::GuiLinear { modulation: None };
        let GpuCommand::Quad {
            sampler, texture, ..
        } = scene.commands.last_mut().unwrap()
        else {
            unreachable!()
        };
        *sampler = GpuSampler::Linear;
        let texture = scene
            .textures
            .iter_mut()
            .find(|resource| resource.id == *texture)
            .unwrap();
        texture.format = GpuTextureFormat::R8;
        texture.pixels = texture
            .pixels
            .chunks_exact(4)
            .map(|pixel| pixel[0])
            .collect::<Vec<_>>()
            .into();
        // R8 is a mask resource; the retained Quad contract requires Rgba8.
        // This rejection must remain before any span eligibility or output write.
        for generic in [false, true] {
            FORCE_GENERIC_SPANS.with(|flag| flag.set(generic));
            for loaded in [false, true] {
                let mut output = vec![17; 129 * 4 * 4];
                let mut renderer = CpuSceneRenderer::default();
                let result = if loaded {
                    renderer.render_loaded(&scene, &mut output)
                } else {
                    renderer.render(&scene, &mut output)
                };
                assert!(
                    matches!(result, Err(CpuSceneError::UnsupportedCommand(1))),
                    "generic={generic} loaded={loaded}"
                );
                assert!(output.iter().all(|byte| *byte == 17));
            }
        }
        FORCE_GENERIC_SPANS.with(|flag| flag.set(false));
    }

    #[test]
    fn warmed_widget_shader_fragments_keep_raw_fractions_without_repeated_divisions() {
        struct ResetGeneric;
        impl Drop for ResetGeneric {
            fn drop(&mut self) {
                FORCE_GENERIC_SPANS.with(|flag| flag.set(false));
            }
        }
        let _reset = ResetGeneric;
        FORCE_GENERIC_SPANS.with(|flag| flag.set(false));
        let widget = crate::GpuGammaLut {
            revision: 0,
            channels: std::sync::Arc::new([[32895; 256], [32767; 256], [60000; 256]]),
        };
        let mut scene = validation_scene(Vec::new(), Vec::new());
        scene.gamma_mode = crate::GpuGammaMode::Fragment;
        scene.gamma = widget.clone();
        let mut tables = SpanTables::default();
        tables.update(&scene);
        scene.gamma = crate::GpuGammaLut::from_ramp(&GammaRamp::standard());
        tables.update(&scene);
        scene.gamma_mode = crate::GpuGammaMode::Monitor;
        SHADER_GAMMA_DIVISIONS.with(|count| count.set(0));
        for _ in 0..16 {
            let mut pixels = [13, 27, 39, 97];
            let mut target = RasterTarget {
                pixels: &mut pixels,
                bounds: Rect::new(0, 0, 1, 1),
                tables: &tables,
                row_origin: 0,
                gamma_override: Some(widget.channels.clone()),
                gamma_raw: tables.prepared_raw(&widget.channels),
            };
            put_fragment(
                &scene,
                &mut target,
                Rect::new(0, 0, 1, 1),
                0,
                0,
                [0.0, 127.5, 255.0, 127.5],
                GpuBlend::Normal,
                GpuSolidAlphaMode::SourceOver,
                true,
                GpuSoftwareBlend::Shader,
            );
            // Unrounded red32895/257 gives70; prematurely encoding128 gives71.
            assert_eq!(pixels, [70, 77, 136, 176]);
        }
        assert_eq!(SHADER_GAMMA_DIVISIONS.with(std::cell::Cell::get), 0);
    }

    fn arbitrary_raw_gamma(seed: u16) -> crate::GpuGammaLut {
        let edge = [0, 1, 256, 257, 32767, 32895, 65534, 65535];
        crate::GpuGammaLut {
            revision: 17,
            channels: std::sync::Arc::new(std::array::from_fn(|channel| {
                std::array::from_fn(|input| {
                    if input < edge.len() {
                        edge[(input + channel) % edge.len()]
                    } else {
                        (input as u16)
                            .wrapping_mul(239)
                            .wrapping_add(seed)
                            .wrapping_add(channel as u16 * 17491)
                    }
                })
            })),
        }
    }

    fn assert_raw_gamma_bits(raw: &[[f32; 256]; 3], channels: &[[u16; 256]; 3]) {
        for channel in 0..3 {
            for input in 0..256 {
                assert_eq!(
                    raw[channel][input].to_bits(),
                    (f32::from(channels[channel][input]) / 257.0).to_bits(),
                    "channel={channel} input={input}"
                );
            }
        }
    }

    #[test]
    fn prepared_raw_gamma_preserves_division_bits_through_eviction_and_alternation() {
        let ramps = (0..12).map(arbitrary_raw_gamma).collect::<Vec<_>>();
        let mut scene = validation_scene(Vec::new(), Vec::new());
        scene.gamma_mode = crate::GpuGammaMode::Fragment;
        let mut tables = SpanTables::default();
        for (index, ramp) in ramps.iter().enumerate() {
            scene.gamma = ramp.clone();
            tables.update(&scene);
            assert_raw_gamma_bits(&tables.raw, &ramp.channels);
            assert!(tables.cached.len() <= 7);
            for (previous, ramp) in ramps[..=index].iter().enumerate() {
                let selected = tables.prepared_raw(&ramp.channels);
                if previous + 8 > index {
                    let selected = selected.unwrap();
                    assert_raw_gamma_bits(selected, &ramp.channels);
                    let identical = std::sync::Arc::new(*ramp.channels);
                    assert!(std::ptr::eq(
                        selected,
                        tables.prepared_raw(&identical).unwrap()
                    ));
                } else {
                    assert!(selected.is_none());
                }
            }
        }
        for index in [11, 7, 10, 6, 9, 5, 8, 4].into_iter().cycle().take(64) {
            scene.gamma = ramps[index].clone();
            tables.update(&scene);
            assert_raw_gamma_bits(&tables.raw, &scene.gamma.channels);
            assert!(tables.cached.len() <= 7);
            for ramp in &ramps[4..] {
                assert_raw_gamma_bits(tables.prepared_raw(&ramp.channels).unwrap(), &ramp.channels);
            }
        }
    }

    #[test]
    fn prepared_raw_gamma_follows_cow_contents_despite_stale_public_revision() {
        FORCE_GENERIC_SPANS.with(|flag| flag.set(false));
        let mut scene = validation_scene(Vec::new(), Vec::new());
        scene.gamma_mode = crate::GpuGammaMode::Fragment;
        scene.gamma = crate::GpuGammaLut {
            revision: 17,
            channels: std::sync::Arc::new([[32895; 256], [32767; 256], [60000; 256]]),
        };
        let held = scene.gamma.clone();
        let mut tables = SpanTables::default();
        tables.update(&scene);
        std::sync::Arc::make_mut(&mut scene.gamma.channels)[0][128] = 32896;
        tables.update(&scene);
        assert_eq!(scene.gamma.revision, held.revision);
        assert_eq!(held.channels[0][128], 32895);
        assert_eq!(scene.gamma.channels[0][128], 32896);
        assert_raw_gamma_bits(&tables.raw, &scene.gamma.channels);
        assert_raw_gamma_bits(tables.prepared_raw(&held.channels).unwrap(), &held.channels);
        for (override_gamma, red) in [(None, 71), (Some(&held), 70)] {
            SHADER_GAMMA_DIVISIONS.with(|count| count.set(0));
            let mut pixels = [13, 27, 39, 97];
            let mut target = RasterTarget {
                pixels: &mut pixels,
                bounds: Rect::new(0, 0, 1, 1),
                tables: &tables,
                row_origin: 0,
                gamma_override: override_gamma.map(|gamma| gamma.channels.clone()),
                gamma_raw: override_gamma.and_then(|gamma| tables.prepared_raw(&gamma.channels)),
            };
            put_fragment(
                &scene,
                &mut target,
                Rect::new(0, 0, 1, 1),
                0,
                0,
                [127.5, 127.5, 255.0, 127.5],
                GpuBlend::Normal,
                GpuSolidAlphaMode::SourceOver,
                true,
                GpuSoftwareBlend::Shader,
            );
            assert_eq!(pixels, [red, 77, 136, 176]);
            assert_eq!(SHADER_GAMMA_DIVISIONS.with(std::cell::Cell::get), 0);
        }
    }

    #[test]
    fn prepared_raw_shader_gamma_matches_scalar_at_float_sample_boundary_ulps() {
        struct ResetGeneric;
        impl Drop for ResetGeneric {
            fn drop(&mut self) {
                FORCE_GENERIC_SPANS.with(|flag| flag.set(false));
            }
        }
        let _reset = ResetGeneric;
        let global = arbitrary_raw_gamma(3);
        let widget = arbitrary_raw_gamma(37);
        let unknown = arbitrary_raw_gamma(91);
        let mut scene = validation_scene(Vec::new(), Vec::new());
        scene.gamma_mode = crate::GpuGammaMode::Fragment;
        let mut tables = SpanTables::default();
        scene.gamma = widget.clone();
        tables.update(&scene);
        scene.gamma = global.clone();
        tables.update(&scene);
        let values = [
            0.0,
            f32::from_bits(127.5f32.to_bits() - 1),
            127.5,
            f32::from_bits(127.5f32.to_bits() + 1),
            f32::from_bits(255.0f32.to_bits() - 1),
            255.0,
        ];
        for mode in [
            crate::GpuGammaMode::Disabled,
            crate::GpuGammaMode::Fragment,
            crate::GpuGammaMode::Monitor,
        ] {
            scene.gamma_mode = mode;
            for gamma in [false, true] {
                for (which, override_gamma) in [None, Some(&widget), Some(&unknown)]
                    .into_iter()
                    .enumerate()
                {
                    for &value in &values {
                        for alpha in [0.25, 127.5, 254.5] {
                            let source = [value, 127.5, 255.0, alpha];
                            let render = |generic| {
                                FORCE_GENERIC_SPANS.with(|flag| flag.set(generic));
                                let mut pixels = [13, 27, 39, 97];
                                let mut target = RasterTarget {
                                    pixels: &mut pixels,
                                    bounds: Rect::new(0, 0, 1, 1),
                                    tables: &tables,
                                    row_origin: 0,
                                    gamma_override: override_gamma
                                        .map(|gamma| gamma.channels.clone()),
                                    gamma_raw: override_gamma
                                        .and_then(|gamma| tables.prepared_raw(&gamma.channels)),
                                };
                                put_fragment(
                                    &scene,
                                    &mut target,
                                    Rect::new(0, 0, 1, 1),
                                    0,
                                    0,
                                    source,
                                    GpuBlend::Normal,
                                    GpuSolidAlphaMode::SourceOver,
                                    gamma,
                                    GpuSoftwareBlend::Shader,
                                );
                                pixels
                            };
                            SHADER_GAMMA_DIVISIONS.with(|count| count.set(0));
                            let fast = render(false);
                            assert_eq!(
                                SHADER_GAMMA_DIVISIONS.with(std::cell::Cell::get),
                                if gamma && which == 2 { 3 } else { 0 }
                            );
                            assert_eq!(
                                fast,
                                render(true),
                                "mode={mode:?} gamma={gamma} widget={which} source={source:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn warmed_raw_widget_shader_scene_matches_cold_and_scalar_across_gamma_modes() {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        pool.install(|| {
            struct ResetGeneric;
            impl Drop for ResetGeneric {fn drop(&mut self) {FORCE_GENERIC_SPANS.with(|flag|flag.set(false));}}
            let _reset=ResetGeneric;
            let widget=arbitrary_raw_gamma(37);
            for rotated in [false,true] {
                for sampler in [GpuSampler::Nearest,GpuSampler::Linear] {
                    let mut original=black_span_differential_scene(false,3,true,[0x107f_e1df;4],false);
                    original.software_sprites[0].mapping=if rotated {crate::GpuSoftwareSpriteMapping::RotatedCorner {
                        center:[64.5,2.0],cos:0.9998,sin:0.02,
                    }} else {crate::GpuSoftwareSpriteMapping::Native};
                    original.software_sprites[0].translation=[0.25,0.125];
                    let GpuCommand::Quad {sampler:selected,clip,..}=original.commands.last_mut().unwrap() else {unreachable!()};
                    *selected=sampler;*clip=Some(Rect::new(1,0,127,4));
                    for mode in [crate::GpuGammaMode::Disabled,crate::GpuGammaMode::Fragment,crate::GpuGammaMode::Monitor] {
                        for gamma in [false,true] {
                            for explicit in [false,true] {
                                let mut scene=original.clone();scene.gamma_mode=mode;
                                scene.software_sprites[0].gamma=explicit.then(||widget.clone());
                                let GpuCommand::Quad {gamma:flag,..}=scene.commands.last_mut().unwrap() else {unreachable!()};*flag=gamma;
                                let mut preparation=scene.clone();preparation.gamma_mode=crate::GpuGammaMode::Fragment;
                                let mut warm=CpuSceneRenderer::default();let mut cold=CpuSceneRenderer::default();
                                let global=preparation.gamma.clone();preparation.gamma=widget.clone();warm.tables.update(&preparation);
                                preparation.gamma=global;warm.tables.update(&preparation);cold.tables.update(&preparation);
                                let mut warmed=vec![0;129*4*4];let mut unknown=warmed.clone();let mut scalar=warmed.clone();
                                FORCE_GENERIC_SPANS.with(|flag|flag.set(false));SHADER_GAMMA_DIVISIONS.with(|count|count.set(0));
                                warm.render(&scene,&mut warmed).unwrap();assert_eq!(SHADER_GAMMA_DIVISIONS.with(std::cell::Cell::get),0);
                                SHADER_GAMMA_DIVISIONS.with(|count|count.set(0));cold.render(&scene,&mut unknown).unwrap();
                                let cold_divisions=SHADER_GAMMA_DIVISIONS.with(std::cell::Cell::get);
                                assert_eq!(cold_divisions>0,gamma&&explicit);
                                FORCE_GENERIC_SPANS.with(|flag|flag.set(true));CpuSceneRenderer::default().render(&scene,&mut scalar).unwrap();
                                assert_eq!(warmed,unknown);assert_eq!(warmed,scalar,
                                    "rotated={rotated} sampler={sampler:?} mode={mode:?} gamma={gamma} explicit={explicit}");
                            }
                        }
                    }
                }
            }
        });
    }

    #[test]
    fn shader_fragment_kernel_preserves_gamma_routing_and_excluded_blend_equations() {
        struct ResetGeneric;
        impl Drop for ResetGeneric {
            fn drop(&mut self) {
                FORCE_GENERIC_SPANS.with(|flag| flag.set(false));
            }
        }
        let _reset = ResetGeneric;
        let mut scene = validation_scene(Vec::new(), Vec::new());
        scene.gamma = crate::GpuGammaLut::from_ramp(&GammaRamp::from_control_points([
            0x17314b, 0x93bd67, 0xe5f9db,
        ]));
        let widget = crate::GpuGammaLut {
            revision: 0,
            channels: std::sync::Arc::new(std::array::from_fn(|channel| {
                std::array::from_fn(|input| {
                    ((input as u16)
                        .wrapping_mul(239)
                        .wrapping_add(73 + channel as u16 * 17491))
                        ^ 0x5a5a
                })
            })),
        };
        let tables = SpanTables::default();
        let sources = [
            [201.0, 73.0, 129.0, 127.5],
            [0.0, 127.49999, 255.0, 0.25],
            [127.50001, 254.49998, 1.0, 254.5],
            [300.0, -1.0, f32::INFINITY, 127.0],
            [f32::NAN, 0.5, 127.5, 127.5],
            [1.0, 2.0, 3.0, f32::NAN],
        ];
        for mode in [
            crate::GpuGammaMode::Disabled,
            crate::GpuGammaMode::Fragment,
            crate::GpuGammaMode::Monitor,
        ] {
            scene.gamma_mode = mode;
            for gamma in [false, true] {
                for override_gamma in [None, Some(&scene.gamma), Some(&widget)] {
                    for blend in [GpuBlend::Normal, GpuBlend::Replace, GpuBlend::Additive] {
                        for alpha_mode in [
                            GpuSolidAlphaMode::SourceOver,
                            GpuSolidAlphaMode::NonSeparate,
                        ] {
                            for software_blend in [
                                GpuSoftwareBlend::Shader,
                                GpuSoftwareBlend::Legacy,
                                GpuSoftwareBlend::GuiBox,
                            ] {
                                for source in sources {
                                    let render = |generic| {
                                        FORCE_GENERIC_SPANS.with(|flag| flag.set(generic));
                                        let mut pixels = [13, 27, 39, 97];
                                        let mut target = RasterTarget {
                                            pixels: &mut pixels,
                                            bounds: Rect::new(0, 0, 1, 1),
                                            tables: &tables,
                                            row_origin: 0,
                                            gamma_raw: None,
                                            gamma_override: override_gamma
                                                .map(|gamma| gamma.channels.clone()),
                                        };
                                        put_fragment(
                                            &scene,
                                            &mut target,
                                            Rect::new(0, 0, 1, 1),
                                            0,
                                            0,
                                            source,
                                            blend,
                                            alpha_mode,
                                            gamma,
                                            software_blend,
                                        );
                                        pixels
                                    };
                                    assert_eq!(render(false),render(true),
                                        "mode={mode:?} gamma={gamma} override={} blend={blend:?} alpha={alpha_mode:?} software={software_blend:?} source={source:?}",override_gamma.is_some());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn colored_native_and_rotated_shader_fragments_match_scalar_across_gamma_modes() {
        // StdGL.cpp:1082-1086 and 1246-1255 supply unrounded per-channel
        // R16 nearest gamma samples. Only optimized dispatch changes in the
        // oracle; source coordinates, filtering, modulation and fog stay fixed.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        pool.install(|| {
            struct ResetGeneric;
            impl Drop for ResetGeneric {
                fn drop(&mut self) { FORCE_GENERIC_SPANS.with(|flag|flag.set(false)); }
            }
            let _reset=ResetGeneric;
            let widget=crate::GpuGammaLut::from_ramp(&GammaRamp::from_control_points([0x17314b,0x93bd67,0xe5f9db]));
            for rotated in [false,true] {
                for sampler in [GpuSampler::Nearest,GpuSampler::Linear] {
                    for holes in [false,true] {
                        for modulation in [[0x107f_e1df;4],[0x00ff_7133,0x407f_d1ff,0x808f_7fc1,0x002f_e199]] {
                            let mut original=black_span_differential_scene(false,3,holes,modulation,false);
                            let software=&mut original.software_sprites[0];
                            software.mapping=if rotated {crate::GpuSoftwareSpriteMapping::RotatedCorner {
                                center:[64.5,2.0],cos:0.9998,sin:0.02,
                            }} else {crate::GpuSoftwareSpriteMapping::Native};
                            software.translation=[0.25,0.125];
                            if let GpuCommand::Quad {sampler:selected,vertices,clip,..}=original.commands.last_mut().unwrap() {
                                *selected=sampler;
                                for vertex in vertices {vertex.sample_tile=[0.0,0.0,16.0,2.0];}
                                *clip=Some(Rect::new(1,0,127,4));
                            }
                            for mode in [crate::GpuGammaMode::Disabled,crate::GpuGammaMode::Fragment,crate::GpuGammaMode::Monitor] {
                                for gamma in [false,true] {
                                    for actual_widget in [false,true] {
                                        let mut scene=original.clone();scene.gamma_mode=mode;
                                        scene.gamma=widget.clone();
                                        scene.software_sprites[0].gamma=actual_widget.then(||crate::GpuGammaLut::from_ramp(&GammaRamp::standard()));
                                        let GpuCommand::Quad {gamma:command_gamma,..}=scene.commands.last_mut().unwrap() else {unreachable!()};
                                        *command_gamma=gamma;
                                        let mut actual=vec![0;129*4*4];let mut expected=actual.clone();
                                        FORCE_GENERIC_SPANS.with(|flag|flag.set(false));
                                        CpuSceneRenderer::default().render(&scene,&mut actual).unwrap();
                                        FORCE_GENERIC_SPANS.with(|flag|flag.set(true));
                                        CpuSceneRenderer::default().render(&scene,&mut expected).unwrap();
                                        assert_eq!(actual.iter().zip(&expected).position(|(a,b)|a!=b),None,
                                            "rotated={rotated} sampler={sampler:?} holes={holes} modulation={modulation:?} mode={mode:?} gamma={gamma} widget={actual_widget}");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    #[test]
    fn black_filtered_and_rotated_spans_match_generic_fractional_alpha() {
        // Use identical metadata and coordinates in both executions. Only the
        // dispatcher changes, so linear sample alpha and rotated corner/fog
        // center arithmetic remain part of the independent scalar reference.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        pool.install(|| {
            struct ResetGeneric;
            impl Drop for ResetGeneric {
                fn drop(&mut self) {
                    FORCE_GENERIC_SPANS.with(|flag| flag.set(false));
                }
            }
            let _reset = ResetGeneric;
            for geometry in 0..3 {
                for sampler in [GpuSampler::Nearest, GpuSampler::Linear] {
                    for transparency in [0, 0x4000_0000, 0xff00_0000] {
                        for holes in [false, true] {
                            for flip in [false, true] {
                                for inclusive in [false, true] {
                                    for custom_gamma in [false, true] {
                                        let mut scene = black_span_differential_scene(false, 3, holes, [transparency; 4], false);
                                        let software = &mut scene.software_sprites[0];
                                        software.mapping = crate::GpuSoftwareSpriteMapping::Native;
                                        software.flip_x = flip;
                                        software.inclusive_source_end = inclusive;
                                        if geometry == 1 {
                                            software.inverse = crate::Transform::set(0.97, 0.13, 0.25, -0.003, 1.0, 0.125, 0.0, 0.0, 1.0);
                                        } else if geometry == 2 {
                                            software.mapping = crate::GpuSoftwareSpriteMapping::RotatedCorner {
                                                center: [64.5, 2.0], cos: 0.9998, sin: 0.02,
                                            };
                                            software.translation = [0.25, 0.125];
                                        }
                                        if let GpuCommand::Quad { sampler: selected, vertices, .. } = scene.commands.last_mut().unwrap() {
                                            *selected = sampler;
                                            if geometry == 1 {
                                                // Force the same non-axis-aligned dispatch used by native objects.
                                                vertices[1].position[1] = 0.25;
                                                vertices[3].position[1] += 0.25;
                                                vertices.iter_mut().for_each(|vertex| vertex.sample_tile = [0.0, 0.0, 16.0, 2.0]);
                                            }
                                        }
                                        if custom_gamma {
                                            scene.gamma.channels = std::sync::Arc::new(std::array::from_fn(|channel| {
                                                std::array::from_fn(|index| if index == 0 {
                                                    [256, 32767, 65534][channel]
                                                } else {
                                                    u16::from((index as u8).wrapping_mul(37).wrapping_add(11 + channel as u8 * 86)) * 257
                                                })
                                            }));
                                        }
                                        let mut actual = vec![0; 129 * 4 * 4];
                                        let mut expected = actual.clone();
                                        FORCE_GENERIC_SPANS.with(|flag| flag.set(false));
                                        CpuSceneRenderer::default().render(&scene, &mut actual).unwrap();
                                        FORCE_GENERIC_SPANS.with(|flag| flag.set(true));
                                        CpuSceneRenderer::default().render(&scene, &mut expected).unwrap();
                                        assert_eq!(actual.iter().zip(&expected).position(|(a,b)| a != b), None,
                                            "geometry={geometry} sampler={sampler:?} transparency={transparency} holes={holes} flip={flip} inclusive={inclusive} custom={custom_gamma}");
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    #[test]
    fn black_and_empty_spans_match_generic_with_fog_holes_and_custom_gamma() {
        // StdGL.cpp:471-503 combines/interpolates fog before the fragment
        // pipeline at 1076-1084. Keep those coordinates and arithmetic unchanged
        // while disabling only the specialized dispatcher in the reference.
        // A private one-worker pool keeps the test-only switch on its worker.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        pool.install(|| {
            struct ResetGeneric;
            impl Drop for ResetGeneric {
                fn drop(&mut self) {
                    FORCE_GENERIC_SPANS.with(|flag| flag.set(false));
                }
            }
            let _reset = ResetGeneric;
            for landscape in [false, true] {
                for liquid in [false, true].into_iter().filter(|liquid| landscape || !liquid) {
                    for alpha in 0..4 {
                        for holes in [false, true] {
                            for modulation in [[0; 4], [0x4000_0000; 4], [0, 0x007f_7f7f, 0x00ff_ffff, 0]] {
                                let original = black_span_differential_scene(landscape, alpha, holes, modulation, liquid);
                                for custom_gamma in [false, true] {
                                    for mode in [crate::GpuGammaMode::Disabled, crate::GpuGammaMode::Fragment, crate::GpuGammaMode::Monitor] {
                                        for gamma in [false, true] {
                                            let mut scene = original.clone();
                                            scene.gamma_mode = mode;
                                            if custom_gamma {
                                                scene.gamma.channels = std::sync::Arc::new(std::array::from_fn(|channel| {
                                                    std::array::from_fn(|index| u16::from((index as u8).wrapping_mul(37).wrapping_add(11 + channel as u8 * 86)) * 257)
                                                }));
                                            }
                                            match scene.commands.last_mut().unwrap() {
                                                GpuCommand::Quad { gamma: command_gamma, .. } | GpuCommand::Landscape { gamma: command_gamma, .. } => *command_gamma = gamma,
                                                _ => unreachable!(),
                                            }
                                            let mut actual = vec![0; 129 * 4 * 4];
                                            let mut expected = actual.clone();
                                            FORCE_GENERIC_SPANS.with(|flag| flag.set(false));
                                            CpuSceneRenderer::default().render(&scene, &mut actual).unwrap();
                                            FORCE_GENERIC_SPANS.with(|flag| flag.set(true));
                                            CpuSceneRenderer::default().render(&scene, &mut expected).unwrap();
                                            assert_eq!(actual.iter().zip(&expected).position(|(a,b)| a != b), None,
                                                "landscape={landscape} liquid={liquid} alpha={alpha} holes={holes} modulation={modulation:?} custom={custom_gamma} mode={mode:?} gamma={gamma}");
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    #[test]
    fn standard_gamma_zero_lift_preserves_other_channels_and_alpha() {
        // StdDDraw2.cpp:240 clamps the default ramp to MinGamma=0x100.
        let gamma = GammaRamp::standard();
        for red in 0..=255u8 {
            for green in 0..=255u8 {
                for blue in [0, 1, 2, 127, 255] {
                    let alpha = red.wrapping_add(green);
                    let pixel = [red, green, blue, alpha];
                    let expected = [
                        gamma.encode_channel(crate::gamma::GammaChannel::Red, red),
                        gamma.encode_channel(crate::gamma::GammaChannel::Green, green),
                        gamma.encode_channel(crate::gamma::GammaChannel::Blue, blue),
                        alpha,
                    ];
                    assert_eq!(standard_gamma_pixel(pixel), expected);
                }
            }
        }
    }
    #[test]
    fn lightning_span_tables_prepare_exact_affine_copy_and_reuse_it_between_ramps() {
        // Lightning.c4d/Script.c:86-90 supplies [lgt, 128+lgt/2, 255].
        // CGammaControl::Set/SetClrChannel (StdDDraw2.cpp:237-286) builds
        // the actual 16-bit ramp; fitting must follow its encoded entries.
        let lightning = crate::GpuGammaLut::from_ramp(&GammaRamp::from_control_points([
            0x606060, 0xb0b0b0, 0xffffff,
        ]));
        let standard = crate::GpuGammaLut::from_ramp(&GammaRamp::standard());
        let arbitrary = crate::GpuGammaLut {
            revision: 0,
            channels: std::sync::Arc::new(std::array::from_fn(|channel| {
                std::array::from_fn(|input| {
                    u16::from((input as u8).wrapping_mul(37 + channel as u8 * 6)) * 257
                })
            })),
        };
        let ramps = [lightning, arbitrary, standard];
        let mut scene = GpuSceneRecorder::default().into_scene(
            [1, 1],
            Color::transparent(),
            &GammaRamp::standard(),
        );
        scene.gamma_mode = crate::GpuGammaMode::Fragment;
        let mut tables = SpanTables::default();
        let mut prepared = Vec::new();
        let mut products = Vec::new();
        for (index, gamma) in ramps.iter().enumerate() {
            scene.gamma = gamma.clone();
            tables.update(&scene);
            if index == 0 {
                assert!(!tables.standard_encoding);
                assert!(tables.affine_copy.is_some());
                for channel in 0..3 {
                    for input in 0..256 {
                        assert_eq!(
                            tables.encoded[channel][input],
                            ((160 * input + 24704) >> 8).clamp(97, 255) as u8
                        );
                    }
                }
            } else if index == 1 {
                assert!(tables.affine_copy.is_none());
            }
            prepared.push(tables.affine_copy);
            products.push(tables.products[0].as_ptr());
        }
        let source = (0..=255u8)
            .flat_map(|input| [input, input ^ 91, input.wrapping_mul(17), input])
            .collect::<Vec<_>>();
        let mut output = vec![0; source.len()];
        for _ in 0..16 {
            for (index, gamma) in ramps.iter().enumerate() {
                scene.gamma = gamma.clone();
                tables.update(&scene);
                assert_eq!(tables.affine_copy, prepared[index]);
                assert_eq!(tables.products[0].as_ptr(), products[index]);
                assert!(tables.cached.len() <= 7);
                if let Some(affine) = tables.affine_copy {
                    affine.copy(&source, &mut output);
                    for (input, actual) in source.chunks_exact(4).zip(output.chunks_exact(4)) {
                        for channel in 0..3 {
                            assert_eq!(
                                actual[channel],
                                store_channel(
                                    f32::from(gamma.channels[channel][usize::from(input[channel])])
                                        / 257.0
                                )
                            );
                        }
                        assert_eq!(actual[3], input[3]);
                    }
                }
            }
        }
    }

    fn tile_copy_differential_scene(geometry: usize) -> GpuScene {
        let texture = GpuTextureResource::immutable_rgba(
            crate::GpuTextureId::fresh(),
            136,
            8,
            (0..136 * 8)
                .flat_map(|index| {
                    [
                        index as u8,
                        (index as u8).wrapping_mul(17),
                        (index as u8).wrapping_add(91),
                        [0, 1, 127, 254, 255][index % 5],
                    ]
                })
                .collect::<Vec<_>>()
                .into(),
        );
        let (destination, translation, clip) = match geometry {
            0 => ([0.0, 0.0, 131.0, 6.0], [0.0, 0.0], None),
            1 => (
                [-3.0, 0.0, 131.0, 6.0],
                [3.0, 0.0],
                Some(Rect::new(2, 0, 127, 6)),
            ),
            2 => (
                [0.0, 0.0, 128.0, 4.0],
                [1.0, 1.0],
                Some(Rect::new(3, 1, 126, 4)),
            ),
            _ => (
                [0.0, 0.0, 128.0, 4.0],
                [-1.0, -1.0],
                Some(Rect::new(0, 0, 126, 3)),
            ),
        };
        let [x, y, width, height] = destination;
        let [dx, dy] = translation;
        let mut recorder = GpuSceneRecorder::default();
        let id = recorder
            .add_software_sprite(crate::GpuSoftwareSprite {
                destination,
                source: [1.0, 1.0, width, height],
                inverse: crate::Transform::identity(),
                translation,
                flip_x: false,
                inclusive_source_end: false,
                gamma: None,
                mapping: crate::GpuSoftwareSpriteMapping::TileCopy,
                fog: None,
            })
            .unwrap();
        let vertices = sprite_vertices(
            [
                [x + dx, y + dy, 1.0],
                [x + dx + width, y + dy, 1.0],
                [x + dx, y + dy + height, 1.0],
                [x + dx + width, y + dy + height, 1.0],
            ],
            [0.0, 0.0, 1.0, 1.0],
            [0x00ff_ffff; 4],
            crate::GpuOuterModulation::Inherit,
        )
        .map(|mut vertex| {
            vertex.software_sprite = Some(id);
            vertex
        });
        recorder.add_texture(texture.clone());
        recorder.push(GpuCommand::Quad {
            texture: texture.id,
            owner_mask: None,
            vertices,
            clip,
            blend: GpuBlend::Normal,
            base_mod2: false,
            owner_mod2: false,
            sampler: GpuSampler::Nearest,
            gamma: true,
        });
        recorder.into_scene([131, 6], Color::new(11, 37, 89, 97), &GammaRamp::standard())
    }

    #[test]
    fn tile_copy_gamma_bulk_matches_generic_across_modes_clips_and_alternating_ramps() {
        // Compare identical TileCopy coordinates and commands. The oracle
        // disables only optimized dispatch, retaining raw transparent RGB,
        // alpha replacement and the existing gamma/monitor operation order.
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .build()
            .unwrap();
        pool.install(|| {
            struct ResetGeneric;
            impl Drop for ResetGeneric {
                fn drop(&mut self) { FORCE_GENERIC_SPANS.with(|flag| flag.set(false)); }
            }
            let _reset = ResetGeneric;
            let lightning = crate::GpuGammaLut::from_ramp(&GammaRamp::from_control_points([0x606060,0xb0b0b0,0xffffff]));
            let standard = crate::GpuGammaLut::from_ramp(&GammaRamp::standard());
            let asymmetric = crate::GpuGammaLut {
                revision:0,
                channels:std::sync::Arc::new([
                    std::array::from_fn(|input| (((160*input+24704)>>8).clamp(97,255) as u16)*257),
                    std::array::from_fn(|input| (((256*input+127)>>8).clamp(3,239) as u16)*257),
                    std::array::from_fn(|input| (((128*input+32768)>>8) as u16)*257),
                ]),
            };
            let mut unsupported = asymmetric.clone();
            std::sync::Arc::make_mut(&mut unsupported.channels)[0][128] -= 257;
            let ramps = [standard,lightning,asymmetric,unsupported];
            for geometry in 0..4 {
                for mode in [crate::GpuGammaMode::Disabled,crate::GpuGammaMode::Fragment,crate::GpuGammaMode::Monitor] {
                    for gamma in [false,true] {
                        for explicit in [false,true] {
                            let mut scene=tile_copy_differential_scene(geometry);
                            scene.gamma_mode=mode;
                            let GpuCommand::Quad {gamma: command_gamma,..}=&mut scene.commands[0] else {unreachable!()};
                            *command_gamma=gamma;
                            let mut fast=CpuSceneRenderer::default();
                            let mut generic=CpuSceneRenderer::default();
                            for _ in 0..2 {
                                for (ramp_index,ramp) in ramps.iter().enumerate() {
                                    scene.gamma=ramp.clone();
                                    scene.software_sprites[0].gamma=explicit.then(||ramp.clone());
                                    let mut actual=vec![0;131*6*4];
                                    let mut expected=actual.clone();
                                    FORCE_GENERIC_SPANS.with(|flag|flag.set(false));
                                    fast.render(&scene,&mut actual).unwrap();
                                    FORCE_GENERIC_SPANS.with(|flag|flag.set(true));
                                    generic.render(&scene,&mut expected).unwrap();
                                    assert_eq!(actual.iter().zip(&expected).position(|(a,b)|a!=b),None,
                                        "geometry={geometry} mode={mode:?} gamma={gamma} explicit={explicit} ramp={ramp_index}");
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    #[test]
    fn repeated_gamma_ramps_reuse_prepared_span_tables() {
        // Objects.c4d/Effects.c4d/Lightning.c4d/Script.c:86-90 alternates
        // GammaRamp 5; StdDDraw2.cpp:1270-1275 installs the composed ramp.
        let mut scene = GpuSceneRecorder::default().into_scene(
            [1, 1],
            Color::transparent(),
            &GammaRamp::standard(),
        );
        scene.gamma_mode = crate::GpuGammaMode::Fragment;
        let first_gamma = scene.gamma.clone();
        let mut tables = SpanTables::default();
        tables.update(&scene);
        let first_storage = tables.products[0].as_ptr();
        let first_value = tables.products[0][127 * 256 + 193];
        scene.gamma = crate::GpuGammaLut::from_ramp(&GammaRamp::from_control_points([
            0x606060, 0xdfdfdf, 0xffffff,
        ]));
        let second_gamma = scene.gamma.clone();
        tables.update(&scene);
        let second_storage = tables.products[0].as_ptr();
        let second_value = tables.products[0][127 * 256 + 193];
        assert_ne!(first_value, second_value);
        assert_ne!(
            first_storage, second_storage,
            "both warmed ramps retain their tables"
        );
        for _ in 0..32 {
            scene.gamma = first_gamma.clone();
            tables.update(&scene);
            assert_eq!(tables.products[0].as_ptr(), first_storage);
            assert_eq!(tables.products[0][127 * 256 + 193], first_value);
            scene.gamma = second_gamma.clone();
            tables.update(&scene);
            assert_eq!(tables.products[0].as_ptr(), second_storage);
            assert_eq!(tables.products[0][127 * 256 + 193], second_value);
        }
    }
    #[test]
    fn explicit_sprite_gamma_preserves_custom_lookup_without_leaking_to_next_atom() {
        let gamma = GammaRamp::from_control_points([0x172b43, 0x698bad, 0xdff1fb]);
        let texture = GpuTextureResource::immutable_rgba(
            crate::GpuTextureId::fresh(),
            2,
            1,
            std::sync::Arc::from([200, 100, 50, 128, 200, 100, 50, 128]),
        );
        let mut software = span_software();
        software.source = [0.0, 0.0, 2.0, 1.0];
        software.destination = software.source;
        software.mapping = crate::GpuSoftwareSpriteMapping::Native;
        software.gamma = Some(crate::GpuGammaLut::from_ramp(&gamma));
        let mut recorder = GpuSceneRecorder::default();
        let id = recorder.add_software_sprite(software).unwrap();
        recorder.add_texture(texture.clone());
        let mut vertices = sprite_vertices(
            [
                [0.0, 0.0, 1.0],
                [2.0, 0.0, 1.0],
                [0.0, 1.0, 1.0],
                [2.0, 1.0, 1.0],
            ],
            [0.0, 0.0, 1.0, 1.0],
            [0x00ff_ffff; 4],
            crate::GpuOuterModulation::Inherit,
        );
        for vertex in &mut vertices {
            vertex.software_sprite = Some(id);
        }
        recorder.push(GpuCommand::Quad {
            texture: texture.id,
            owner_mask: None,
            vertices,
            clip: None,
            blend: GpuBlend::Normal,
            base_mod2: false,
            owner_mod2: false,
            sampler: GpuSampler::Nearest,
            gamma: true,
        });
        recorder.push(GpuCommand::Solid {
            vertices: vec![GpuSolidVertex {
                position: [1.5, 0.5, 1.0],
                color: [17.0 / 255.0, 31.0 / 255.0, 71.0 / 255.0, 1.0],
                outer_modulation: crate::GpuSolidOuterModulation::PackedC4,
            }],
            topology: GpuPrimitiveTopology::PointList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: None,
            blend: GpuBlend::Normal,
            style: crate::GpuSolidStyle::with_gamma(true),
        });
        let mut scene =
            recorder.into_scene([2, 1], Color::opaque(8, 12, 24), &GammaRamp::standard());
        scene.gamma_mode = crate::GpuGammaMode::Monitor;
        let mut expected = crate::Surface::new(2, 1, crate::PixelFormat::Rgba8888);
        expected.fill(Color::opaque(8, 12, 24));
        // StdGL.cpp:1081-1087 samples the explicitly supplied R16 ramp before
        // blending; the frame's monitor ramp resolves after all atoms.
        crate::SurfaceDrawTarget::blend_fragment_over(
            &mut expected,
            0,
            0,
            [200.0, 100.0, 50.0, 128.0],
            Some(&gamma),
        )
        .unwrap();
        expected.set_pixel(1, 0, Color::opaque(17, 31, 71)).unwrap();
        let mut actual = [0; 8];
        CpuSceneRenderer::default()
            .render_without_monitor(&scene, &mut actual)
            .unwrap();
        assert_eq!(actual, expected.pixels());
    }
    #[test]
    fn landscape_dirty_upload_reuses_tiles_that_sample_unchanged_texels() {
        let mut software = span_software();
        software.source = [0.0, 0.0, 128.0, 64.0];
        software.destination = software.source;
        software.mapping = crate::GpuSoftwareSpriteMapping::Landscape {
            zoom: 1.0,
            indent: 0.0,
            world_extent: [128, 64],
            tile_origin: [0, 0],
            texture_size: 128,
        };
        let mut scene = landscape_span_scene(
            software,
            [
                [0.0, 0.0, 1.0],
                [128.0, 0.0, 1.0],
                [0.0, 64.0, 1.0],
                [128.0, 64.0, 1.0],
            ],
        );
        scene.logical_extent = [128, 64];
        scene.textures[0].extent = [128, 64];
        scene.textures[0].pixels = vec![255; 128 * 64 * 4].into();
        let mut renderer = CpuSceneRenderer::default();
        let mut actual = vec![0; 128 * 64 * 4];
        renderer.render(&scene, &mut actual).unwrap();
        let mut pixels = scene.textures[0].pixels.to_vec();
        pixels[..4].copy_from_slice(&[1, 2, 3, 255]);
        scene.textures[0].pixels = pixels.into();
        scene.textures[0].base_revision = Some(0);
        scene.textures[0].revision = 1;
        scene.textures[0].dirty = vec![Rect::new(0, 0, 1, 1)];
        renderer.render(&scene, &mut actual).unwrap();
        assert_eq!(
            renderer.stats(),
            CpuSceneStats {
                rasterized_tiles: 1,
                reused_tiles: 1
            }
        );
        let mut fresh = vec![0; actual.len()];
        CpuSceneRenderer::default()
            .render(&scene, &mut fresh)
            .unwrap();
        assert_eq!(actual, fresh);
        assert_eq!(&actual[..4], &[1, 2, 3, 255]);
    }
    #[test]
    fn landscape_mask_updates_invalidate_clamped_border_samples() {
        let mut software = span_software();
        software.source = [0.0, 0.0, 128.0, 64.0];
        software.destination = software.source;
        software.mapping = crate::GpuSoftwareSpriteMapping::Landscape {
            zoom: 1.0,
            indent: 0.0,
            world_extent: [128, 64],
            tile_origin: [0, 0],
            texture_size: 128,
        };
        let mut scene = landscape_span_scene(
            software,
            [
                [0.0, 0.0, 1.0],
                [128.0, 0.0, 1.0],
                [0.0, 64.0, 1.0],
                [128.0, 64.0, 1.0],
            ],
        );
        scene.logical_extent = [128, 64];
        scene.textures[0].extent = [128, 64];
        scene.textures[0].pixels = [10, 20, 30, 255].repeat(128 * 64).into();
        let mask = GpuTextureResource {
            id: crate::GpuTextureId::fresh(),
            extent: [1, 1],
            revision: 0,
            base_revision: None,
            format: GpuTextureFormat::R8,
            pixels: std::sync::Arc::from([0]),
            dirty: Vec::new(),
        };
        let liquid = GpuTextureResource::immutable_rgba(
            crate::GpuTextureId::fresh(),
            1,
            1,
            std::sync::Arc::from([255; 4]),
        );
        if let GpuCommand::Landscape {
            liquid_mask,
            liquid: liquid_id,
            phase,
            ..
        } = &mut scene.commands[0]
        {
            *liquid_mask = Some(mask.id);
            *liquid_id = Some(liquid.id);
            *phase = [0.05; 3];
        }
        scene.textures.extend([mask, liquid]);
        let mut renderer = CpuSceneRenderer::default();
        let mut actual = vec![0; 128 * 64 * 4];
        renderer.render(&scene, &mut actual).unwrap();
        scene.textures[1].pixels = std::sync::Arc::from([1]);
        scene.textures[1].base_revision = Some(0);
        scene.textures[1].revision = 1;
        scene.textures[1].dirty = vec![Rect::new(0, 0, 1, 1)];
        renderer.render(&scene, &mut actual).unwrap();
        let mut fresh = vec![0; actual.len()];
        CpuSceneRenderer::default()
            .render(&scene, &mut fresh)
            .unwrap();
        assert_eq!(actual.iter().zip(&fresh).position(|(a, b)| a != b), None);
    }
    #[test]
    fn black_sprite_span_clamps_negative_modulation_transparency() {
        let mut software = span_software();
        software.destination = [0.0, 0.0, 2.0, 1.0];
        software.mapping = crate::GpuSoftwareSpriteMapping::Native;
        let mut scene = landscape_span_scene(
            software,
            [
                [0.0, 0.0, 1.0],
                [2.0, 0.0, 1.0],
                [0.0, 1.0, 1.0],
                [2.0, 1.0, 1.0],
            ],
        );
        let GpuCommand::Landscape { base, vertices, .. } = &scene.commands[0] else {
            unreachable!()
        };
        let texture = *base;
        let vertices = vertices.map(|mut vertex| {
            vertex.modulation = [0.0, 0.0, 0.0, -1.0];
            vertex
        });
        scene.commands[0] = GpuCommand::Quad {
            texture,
            vertices,
            owner_mask: None,
            clip: None,
            blend: GpuBlend::Normal,
            base_mod2: false,
            owner_mod2: false,
            sampler: GpuSampler::Linear,
            gamma: false,
        };
        let mut actual = [0; 8];
        CpuSceneRenderer::default()
            .render(&scene, &mut actual)
            .unwrap();
        assert_eq!(actual, [0, 0, 0, 128, 0, 0, 0, 128]);
    }
    #[test]
    fn large_landscape_positions_keep_original_float_pixel_centers() {
        let mut software = span_software();
        software.destination = [4_000_000.0, 0.0, 4_000_000.0, 1.0];
        software.source = [0.0, 0.0, 4_000_000.0, 1.0];
        software.translation = [4_000_000.0, 0.0];
        software.mapping = crate::GpuSoftwareSpriteMapping::Landscape {
            zoom: 1.0,
            indent: 0.0,
            world_extent: [4_000_000, 1],
            tile_origin: [388_609, 0],
            texture_size: 1,
        };
        let mut scene = landscape_span_scene(
            software,
            [
                [8_388_609.0, 0.0, 1.0],
                [8_388_610.0, 0.0, 1.0],
                [8_388_609.0, 1.0, 1.0],
                [8_388_610.0, 1.0, 1.0],
            ],
        );
        scene.logical_extent = [8_388_611, 1];
        scene.textures[0].extent = [2, 1];
        scene.textures[0].pixels = std::sync::Arc::from([20u8, 30, 40, 255, 80, 90, 100, 255]);
        let mut output = vec![0; 8_388_611 * 4];
        CpuSceneRenderer::default()
            .render_loaded(&scene, &mut output)
            .unwrap();
        // f32 x+0.5 rounds upward here before subtraction of the two origins.
        assert_eq!(&output[8_388_609 * 4..8_388_610 * 4], &[80, 90, 100, 255]);
    }

    #[test]
    fn landscape_span_clips_shifted_world_bounds_without_overflow() {
        let mut software = span_software();
        software.translation = [1.0, 0.0];
        software.mapping = crate::GpuSoftwareSpriteMapping::Landscape {
            zoom: 1.0,
            indent: 0.0,
            world_extent: [i32::MAX as u32, 1],
            tile_origin: [i32::MAX as u32 - 1, 0],
            texture_size: 1,
        };
        let scene = landscape_span_scene(
            software,
            [
                [1.0, 0.0, 1.0],
                [2.0, 0.0, 1.0],
                [1.0, 1.0, 1.0],
                [2.0, 1.0, 1.0],
            ],
        );
        let mut output = [0; 8];
        CpuSceneRenderer::default()
            .render(&scene, &mut output)
            .unwrap();
        assert_eq!(output, [0; 8]);
    }

    #[test]
    fn flat_landscape_fog_keeps_source_chunk_membership() {
        let mut software = span_software();
        software.fog = Some(crate::GpuSoftwareFog {
            destination: [0.0, 0.0, 1.0, 1.0],
            source_range: [0.75, 0.0, 1.0, 1.0],
        });
        let scene = landscape_span_scene(
            software,
            [
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
            ],
        );
        let mut output = [0; 8];
        CpuSceneRenderer::default()
            .render(&scene, &mut output)
            .unwrap();
        // The original source-axis fog sampler excludes source x=0.5 from
        // this chunk even when all four modulation colors are identical.
        assert_eq!(output, [0; 8]);
    }

    #[test]
    fn retained_scene_rejects_enabled_texture_tiles_without_a_texel() {
        let texture = GpuTextureResource::immutable_rgba(
            crate::GpuTextureId::fresh(),
            1,
            1,
            std::sync::Arc::from([255; 4]),
        );
        for size in [0.0, 0.5, -1.0] {
            let vertices = sprite_vertices(
                [
                    [0.0, 0.0, 1.0],
                    [1.0, 0.0, 1.0],
                    [0.0, 1.0, 1.0],
                    [1.0, 1.0, 1.0],
                ],
                [0.0, 0.0, 1.0, 1.0],
                [0x00ff_ffff; 4],
                crate::GpuOuterModulation::Inherit,
            )
            .map(|vertex| vertex.with_sample_tile(0.0, 0.0, size));
            assert_scene_rejected_before_output_changes(&validation_scene(
                vec![texture.clone()],
                vec![GpuCommand::Quad {
                    texture: texture.id,
                    owner_mask: None,
                    vertices,
                    clip: None,
                    blend: GpuBlend::Normal,
                    base_mod2: false,
                    owner_mod2: false,
                    sampler: GpuSampler::Nearest,
                    gamma: false,
                }],
            ));
        }
    }

    #[test]
    fn retained_scene_rejects_font_tiles_without_a_texel() {
        for physical_size in [0.0, 0.5] {
            let mut recorder = GpuSceneRecorder::default();
            recorder
                .add_software_sprite(crate::GpuSoftwareSprite {
                    destination: [0.0, 0.0, 1.0, 1.0],
                    source: [0.0, 0.0, 1.0, 1.0],
                    inverse: crate::Transform::identity(),
                    translation: [0.0; 2],
                    flip_x: false,
                    inclusive_source_end: false,
                    fog: None,
                    gamma: None,
                    mapping: crate::GpuSoftwareSpriteMapping::Font {
                        shear: 0.0,
                        center_y: 0.0,
                        texture_indent: 0.0,
                        physical_size,
                        normalize_transparent: true,
                    },
                })
                .unwrap();
            let scene = recorder.into_scene([1, 1], Color::transparent(), &GammaRamp::identity());
            assert_scene_rejected_before_output_changes(&scene);
        }
    }

    #[test]
    fn retained_scene_rejects_duplicate_texture_ids_before_output_changes() {
        let texture = GpuTextureResource::immutable_rgba(
            crate::GpuTextureId::fresh(),
            1,
            1,
            std::sync::Arc::from([255; 4]),
        );
        assert_scene_rejected_before_output_changes(&validation_scene(
            vec![texture.clone(), texture],
            Vec::new(),
        ));
    }

    #[test]
    fn retained_scene_rejects_dirty_rectangles_outside_texture_bounds() {
        let mut texture = GpuTextureResource::immutable_rgba(
            crate::GpuTextureId::fresh(),
            1,
            1,
            std::sync::Arc::from([255; 4]),
        );
        texture.revision = 4;
        texture.base_revision = Some(3);
        for dirty in [
            Rect::new(-1, 0, 1, 1),
            Rect::new(0, -1, 1, 1),
            Rect::new(1, 0, 1, 1),
            Rect::new(0, 1, 1, 1),
            Rect::new(0, 0, u32::MAX, 1),
        ] {
            texture.dirty = vec![dirty];
            assert_scene_rejected_before_output_changes(&validation_scene(
                vec![texture.clone()],
                Vec::new(),
            ));
        }
        texture.dirty = vec![Rect::new(1, 1, 0, 0)];
        CpuSceneRenderer::default()
            .render(&validation_scene(vec![texture], Vec::new()), &mut [0; 4])
            .unwrap();
    }

    #[test]
    fn retained_scene_rejects_dirty_data_without_advancing_revision() {
        let mut texture = GpuTextureResource::immutable_rgba(
            crate::GpuTextureId::fresh(),
            1,
            1,
            std::sync::Arc::from([255; 4]),
        );
        texture.revision = 4;
        texture.base_revision = Some(4);
        texture.dirty = vec![Rect::new(0, 0, 1, 1)];
        assert_scene_rejected_before_output_changes(&validation_scene(
            vec![texture.clone()],
            Vec::new(),
        ));
        for base_revision in [None, Some(3), Some(5), Some(u64::MAX)] {
            texture.base_revision = base_revision;
            CpuSceneRenderer::default()
                .render(
                    &validation_scene(vec![texture.clone()], Vec::new()),
                    &mut [0; 4],
                )
                .unwrap();
        }
        texture.base_revision = Some(4);
        texture.dirty.clear();
        CpuSceneRenderer::default()
            .render(&validation_scene(vec![texture], Vec::new()), &mut [0; 4])
            .unwrap();
    }

    #[test]
    fn retained_scene_rejects_invalid_packed_object_flags_before_binning() {
        #[repr(C)]
        struct RawObjectSprite {
            positions: [[f32; 3]; 4],
            uv: [f32; 4],
            modulation: [u32; 4],
            sample_tile_size: f32,
            flags: u32,
        }
        let texture = GpuTextureResource::immutable_rgba(
            crate::GpuTextureId::fresh(),
            1,
            1,
            std::sync::Arc::from([255; 4]),
        );
        for flags in [1 << 4, 0b11 << 2] {
            let raw = RawObjectSprite {
                positions: [
                    [3.0, 3.0, 1.0],
                    [4.0, 3.0, 1.0],
                    [3.0, 4.0, 1.0],
                    [4.0, 4.0, 1.0],
                ],
                uv: [0.0, 0.0, 1.0, 1.0],
                modulation: [0x00ff_ffff; 4],
                sample_tile_size: 0.0,
                flags,
            };
            // SAFETY: both repr(C) structs have the same field layout. All u32
            // flag patterns are valid memory, including semantically invalid flags.
            let sprite =
                unsafe { std::mem::transmute::<RawObjectSprite, crate::GpuObjectSprite>(raw) };
            assert_scene_rejected_before_output_changes(&validation_scene(
                vec![texture.clone()],
                vec![GpuCommand::ObjectBatch {
                    texture: texture.id,
                    owner_texture: None,
                    sprites: vec![sprite],
                    clip: None,
                    blend: GpuBlend::Normal,
                    gamma: false,
                }],
            ));
        }
    }

    #[test]
    fn retained_scene_rejects_malformed_software_blits_before_binning() {
        let texture = GpuTextureResource::immutable_rgba(
            crate::GpuTextureId::fresh(),
            1,
            1,
            std::sync::Arc::from([255; 4]),
        );
        let valid = crate::GpuSoftwareBlit {
            source: Rect::new(0, 0, 1, 1),
            mapping: crate::GpuSoftwareBlitMapping::Unscaled(crate::Point::new(3, 3)),
            modulation: Color::opaque(255, 255, 255),
            mode: Some(crate::BlitMode::Normal),
            translation: [0.0; 2],
        };
        let mut invalid_inverse = crate::Transform::identity();
        invalid_inverse.mat[0] = f32::NAN;
        for blit in [
            crate::GpuSoftwareBlit {
                source: Rect::new(0, 0, 0, 1),
                ..valid
            },
            crate::GpuSoftwareBlit {
                source: Rect::new(0, 0, 1, 0),
                ..valid
            },
            crate::GpuSoftwareBlit {
                mapping: crate::GpuSoftwareBlitMapping::Stretched(Rect::new(3, 3, 0, 1)),
                ..valid
            },
            crate::GpuSoftwareBlit {
                mapping: crate::GpuSoftwareBlitMapping::Stretched(Rect::new(3, 3, 1, 0)),
                ..valid
            },
            crate::GpuSoftwareBlit {
                mapping: crate::GpuSoftwareBlitMapping::Transformed {
                    origin: crate::Point::new(3, 3),
                    inverse: invalid_inverse,
                },
                ..valid
            },
            crate::GpuSoftwareBlit {
                translation: [f32::INFINITY, 0.0],
                ..valid
            },
            crate::GpuSoftwareBlit {
                translation: [0.0, f32::NAN],
                ..valid
            },
        ] {
            let vertices = sprite_vertices(
                [
                    [3.0, 3.0, 1.0],
                    [4.0, 3.0, 1.0],
                    [3.0, 4.0, 1.0],
                    [4.0, 4.0, 1.0],
                ],
                [0.0, 0.0, 1.0, 1.0],
                [0x00ff_ffff; 4],
                crate::GpuOuterModulation::Inherit,
            )
            .map(|vertex| vertex.with_software_blit(blit));
            assert_scene_rejected_before_output_changes(&validation_scene(
                vec![texture.clone()],
                vec![GpuCommand::Quad {
                    texture: texture.id,
                    owner_mask: None,
                    vertices,
                    clip: None,
                    blend: GpuBlend::Normal,
                    base_mod2: false,
                    owner_mod2: false,
                    sampler: GpuSampler::Nearest,
                    gamma: false,
                }],
            ));
        }
    }

    #[test]
    fn retained_scene_rejects_missing_textures_even_for_offscreen_commands() {
        let missing = crate::GpuTextureId::fresh();
        let positions = [
            [3.0, 3.0, 1.0],
            [4.0, 3.0, 1.0],
            [3.0, 4.0, 1.0],
            [4.0, 4.0, 1.0],
        ];
        let vertices = sprite_vertices(
            positions,
            [0.0, 0.0, 1.0, 1.0],
            [0x00ff_ffff; 4],
            crate::GpuOuterModulation::Inherit,
        );
        let sprite = crate::GpuObjectSprite::new(
            positions,
            [0.0, 0.0, 1.0, 1.0],
            [0x00ff_ffff; 4],
            GpuSampler::Nearest,
            0.0,
            false,
            crate::GpuOuterModulation::Inherit,
        );
        for command in [
            GpuCommand::Quad {
                texture: missing,
                owner_mask: None,
                vertices,
                clip: None,
                blend: GpuBlend::Normal,
                base_mod2: false,
                owner_mod2: false,
                sampler: GpuSampler::Nearest,
                gamma: false,
            },
            GpuCommand::SpriteBatch {
                texture: missing,
                quads: vec![crate::GpuSpriteQuad {
                    rect: [3.0, 3.0, 4.0, 4.0],
                    uv: [0.0, 0.0, 1.0, 1.0],
                    modulation: 0x00ff_ffff,
                    software_sprite: None,
                    software_shader: true,
                }],
                clip: None,
                blend: GpuBlend::Normal,
                mod2: false,
                gamma: false,
                outer_modulation: crate::GpuOuterModulation::Inherit,
            },
            GpuCommand::ObjectBatch {
                texture: missing,
                owner_texture: None,
                sprites: vec![sprite],
                clip: None,
                blend: GpuBlend::Normal,
                gamma: false,
            },
            GpuCommand::Landscape {
                base: missing,
                liquid_mask: None,
                liquid: None,
                vertices,
                clip: None,
                phase: [0.0; 3],
                gamma: false,
            },
        ] {
            assert_scene_rejected_before_output_changes(&validation_scene(
                Vec::new(),
                vec![command],
            ));
        }
        let base = GpuTextureResource::immutable_rgba(
            crate::GpuTextureId::fresh(),
            1,
            1,
            std::sync::Arc::from([255; 4]),
        );
        assert_scene_rejected_before_output_changes(&validation_scene(
            vec![base.clone()],
            vec![GpuCommand::ObjectBatch {
                texture: base.id,
                owner_texture: Some(missing),
                sprites: vec![sprite],
                clip: None,
                blend: GpuBlend::Normal,
                gamma: false,
            }],
        ));
        let mask = GpuTextureResource {
            id: crate::GpuTextureId::fresh(),
            extent: [1, 1],
            revision: 0,
            base_revision: None,
            format: GpuTextureFormat::R8,
            pixels: std::sync::Arc::from([255]),
            dirty: Vec::new(),
        };
        assert_scene_rejected_before_output_changes(&validation_scene(
            vec![base.clone(), mask.clone()],
            vec![GpuCommand::Landscape {
                base: base.id,
                liquid_mask: Some(mask.id),
                liquid: Some(missing),
                vertices,
                clip: None,
                phase: [0.0; 3],
                gamma: false,
            }],
        ));
        assert_scene_rejected_before_output_changes(&validation_scene(
            vec![base.clone()],
            vec![GpuCommand::Landscape {
                base: base.id,
                liquid_mask: Some(missing),
                liquid: Some(base.id),
                vertices,
                clip: None,
                phase: [0.0; 3],
                gamma: false,
            }],
        ));
        for command in [
            GpuCommand::ObjectBatch {
                texture: missing,
                owner_texture: Some(missing),
                sprites: Vec::new(),
                clip: None,
                blend: GpuBlend::Normal,
                gamma: false,
            },
            GpuCommand::SpriteBatch {
                texture: missing,
                quads: Vec::new(),
                clip: None,
                blend: GpuBlend::Normal,
                mod2: false,
                gamma: false,
                outer_modulation: crate::GpuOuterModulation::Inherit,
            },
        ] {
            CpuSceneRenderer::default()
                .render(&validation_scene(Vec::new(), vec![command]), &mut [0; 4])
                .unwrap();
        }
    }

    #[test]
    fn quad_cache_reuses_pixels_when_only_unused_vertex_metadata_changes() {
        let mut scene = quad_metadata_scene(false);
        let mut renderer = CpuSceneRenderer::default();
        let mut first = [0; 8];
        renderer.render(&scene, &mut first).unwrap();
        scene.software_sprites[1].translation[0] = 17.0;
        let mut fresh = [255; 8];
        renderer.render(&scene, &mut fresh).unwrap();
        assert_eq!(fresh, first);
        assert_eq!(
            renderer.stats(),
            CpuSceneStats {
                rasterized_tiles: 0,
                reused_tiles: 1,
            }
        );
    }

    #[test]
    fn quad_metadata_id_equality_invalidates_gui_linear_span_eligibility() {
        let mut scene = quad_metadata_scene(false);
        let GpuCommand::Quad { vertices, .. } = &mut scene.commands[0] else {
            unreachable!();
        };
        let second_id = vertices[1].software_sprite;
        let first_id = vertices[0].software_sprite;
        for vertex in vertices {
            vertex.software_sprite = first_id;
        }
        let eligible = |scene: &GpuScene| {
            let GpuCommand::Quad { vertices, .. } = &scene.commands[0] else {
                unreachable!();
            };
            let mut tables = SpanTables::default();
            tables.update(scene);
            let mut pixels = [0; 8];
            let mut target = RasterTarget {
                pixels: &mut pixels,
                bounds: Rect::new(0, 0, 2, 1),
                tables: &tables,
                row_origin: 0,
                gamma_override: None,
                gamma_raw: None,
            };
            draw_gui_linear_span(
                scene,
                &mut target,
                vertices,
                &scene.textures[0],
                scene
                    .software_sprite(vertices[0].software_sprite.unwrap())
                    .unwrap(),
                false,
                [0, 0, 2, 1],
            )
        };
        assert!(eligible(&scene));
        let mut renderer = CpuSceneRenderer::default();
        let mut first = [0; 8];
        renderer.render(&scene, &mut first).unwrap();
        let first_hash = tiles::atom_hash(&scene, &scene.commands[0], 0);
        let GpuCommand::Quad { vertices, .. } = &mut scene.commands[0] else {
            unreachable!();
        };
        vertices[1].software_sprite = second_id;
        assert!(!eligible(&scene));
        assert_ne!(tiles::atom_hash(&scene, &scene.commands[0], 0), first_hash);
        let mut fresh = [255; 8];
        renderer.render(&scene, &mut fresh).unwrap();
        assert_eq!(fresh, first);
        assert_eq!(
            renderer.stats(),
            CpuSceneStats {
                rasterized_tiles: 1,
                reused_tiles: 0,
            }
        );
    }

    #[test]
    fn quad_cache_invalidates_when_consumed_first_metadata_changes() {
        let mut scene = quad_metadata_scene(false);
        let mut renderer = CpuSceneRenderer::default();
        let mut first = [0; 8];
        renderer.render(&scene, &mut first).unwrap();
        scene.software_sprites[0].source = [1.0, 0.0, 1.0, 1.0];
        let mut fresh = [255; 8];
        renderer.render(&scene, &mut fresh).unwrap();
        assert_ne!(fresh, first);
        assert_eq!(
            renderer.stats(),
            CpuSceneStats {
                rasterized_tiles: 1,
                reused_tiles: 0,
            }
        );
        let mut expected = [0; 8];
        CpuSceneRenderer::default()
            .render(&scene, &mut expected)
            .unwrap();
        assert_eq!(fresh, expected);
    }

    #[test]
    fn quad_cache_reuses_pixels_when_software_ids_are_renumbered() {
        let mut scene = quad_metadata_scene(true);
        let mut renderer = CpuSceneRenderer::default();
        let mut first = [0; 8];
        renderer.render(&scene, &mut first).unwrap();
        let software = scene.software_sprites[0].clone();
        let mut unrelated = software.clone();
        unrelated.translation = [71.0, 29.0];
        let mut recorder = GpuSceneRecorder::default();
        let old_id = recorder.add_software_sprite(unrelated.clone()).unwrap();
        let new_id = recorder.add_software_sprite(software.clone()).unwrap();
        assert_ne!(old_id, new_id);
        scene.software_sprites = vec![unrelated, software];
        let GpuCommand::Quad { vertices, .. } = &mut scene.commands[0] else {
            unreachable!();
        };
        assert_eq!(vertices[0].software_sprite, Some(old_id));
        for vertex in vertices {
            vertex.software_sprite = Some(new_id);
        }
        let mut fresh = [255; 8];
        renderer.render(&scene, &mut fresh).unwrap();
        assert_eq!(fresh, first);
        assert_eq!(
            renderer.stats(),
            CpuSceneStats {
                rasterized_tiles: 0,
                reused_tiles: 1,
            }
        );
    }

    #[test]
    fn unused_quad_metadata_is_validated_before_cached_or_loaded_output_changes() {
        let original = quad_metadata_scene(false);
        let mut recorder = GpuSceneRecorder::default();
        let mut missing_id = None;
        for _ in 0..3 {
            missing_id = recorder.add_software_sprite(original.software_sprites[0].clone());
        }
        for malformed in 0..3 {
            let mut scene = original.clone();
            match malformed {
                0 => scene.software_sprites[1].translation[0] = f32::NAN,
                1 => scene.software_sprites[1].source[2] = 0.0,
                _ => {
                    let GpuCommand::Quad { vertices, .. } = &mut scene.commands[0] else {
                        unreachable!();
                    };
                    vertices[3].software_sprite = missing_id;
                }
            }
            for loaded in [false, true] {
                let mut renderer = CpuSceneRenderer::default();
                let mut first = [0; 8];
                renderer.render(&original, &mut first).unwrap();
                let mut fresh = [17; 8];
                let result = if loaded {
                    renderer.render_loaded(&scene, &mut fresh)
                } else {
                    renderer.render(&scene, &mut fresh)
                };
                assert!(result.is_err(), "malformed={malformed} loaded={loaded}");
                assert_eq!(fresh, [17; 8]);
                assert_eq!(renderer.rendered_pixels(), first);
            }
        }
    }

    fn quad_metadata_scene(equal_ids: bool) -> GpuScene {
        let texture = GpuTextureResource::immutable_rgba(
            crate::GpuTextureId::fresh(),
            2,
            1,
            std::sync::Arc::from([193, 37, 71, 129, 23, 181, 229, 211]),
        );
        let mut software = span_software();
        software.destination = [0.0, 0.0, 2.0, 1.0];
        software.source = software.destination;
        software.mapping = crate::GpuSoftwareSpriteMapping::GuiLinear { modulation: None };
        let mut recorder = GpuSceneRecorder::default();
        let first_id = recorder.add_software_sprite(software.clone()).unwrap();
        let second_id = recorder.add_software_sprite(software).unwrap();
        let mut vertices = sprite_vertices(
            [
                [0.0, 0.0, 1.0],
                [2.0, 0.0, 1.0],
                [0.0, 1.0, 1.0],
                [2.0, 1.0, 1.0],
            ],
            [0.0, 0.0, 1.0, 1.0],
            [0x00ff_ffff; 4],
            crate::GpuOuterModulation::Inherit,
        );
        for (index, vertex) in vertices.iter_mut().enumerate() {
            vertex.software_sprite = Some(if equal_ids || index == 0 {
                first_id
            } else {
                second_id
            });
        }
        recorder.add_texture(texture.clone());
        recorder.push(GpuCommand::Quad {
            texture: texture.id,
            owner_mask: None,
            vertices,
            clip: None,
            blend: GpuBlend::Normal,
            base_mod2: false,
            owner_mod2: false,
            sampler: GpuSampler::Linear,
            gamma: false,
        });
        recorder.into_scene([2, 1], Color::new(11, 29, 53, 97), &GammaRamp::identity())
    }

    #[test]
    fn retained_tiles_reuse_unchanged_output_into_a_fresh_buffer() {
        let scene = GpuSceneRecorder::default().into_scene(
            [130, 65],
            Color::opaque(31, 73, 129),
            &GammaRamp::identity(),
        );
        let mut renderer = CpuSceneRenderer::default();
        let mut first = vec![0; 130 * 65 * 4];
        renderer.render(&scene, &mut first).unwrap();
        assert_eq!(
            renderer.stats(),
            CpuSceneStats {
                rasterized_tiles: 6,
                reused_tiles: 0
            }
        );
        let mut fresh = vec![255; first.len()];
        renderer.render(&scene, &mut fresh).unwrap();
        assert_eq!(fresh, first);
        assert_eq!(
            renderer.stats(),
            CpuSceneStats {
                rasterized_tiles: 0,
                reused_tiles: 6
            }
        );
    }

    #[test]
    fn retained_loaded_layers_preserve_destination_and_defer_monitor_gamma() {
        let mut recorder = GpuSceneRecorder::default();
        recorder.push(GpuCommand::Solid {
            vertices: vec![GpuSolidVertex {
                position: [0.5, 0.5, 1.0],
                color: [0.25, 0.5, 0.75, 0.5],
                outer_modulation: crate::GpuSolidOuterModulation::SampledTexture,
            }],
            topology: GpuPrimitiveTopology::PointList,
            alpha_mode: GpuSolidAlphaMode::NonSeparate,
            clip: None,
            blend: GpuBlend::Normal,
            style: crate::GpuSolidStyle::NONE,
        });
        let mut scene = recorder.into_scene([1, 1], Color::transparent(), &GammaRamp::standard());
        scene.gamma_mode = crate::GpuGammaMode::Monitor;
        let mut actual = [17, 31, 73, 128];
        CpuSceneRenderer::default()
            .render_loaded(&scene, &mut actual)
            .unwrap();
        assert_eq!(actual, [40, 79, 132, 128]);
    }

    #[test]
    fn retained_scene_rejects_zero_landscape_texture_size_before_rasterizing() {
        use std::sync::Arc;
        let mut recorder = GpuSceneRecorder::default();
        let id = recorder
            .add_software_sprite(crate::GpuSoftwareSprite {
                destination: [0.0, 0.0, 1.0, 1.0],
                source: [0.0, 0.0, 1.0, 1.0],
                inverse: crate::Transform::identity(),
                translation: [0.0; 2],
                flip_x: false,
                inclusive_source_end: false,
                fog: None,
                gamma: None,
                mapping: crate::GpuSoftwareSpriteMapping::Landscape {
                    zoom: 1.0,
                    world_extent: [1, 1],
                    tile_origin: [0, 0],
                    texture_size: 0,
                    indent: 0.0,
                },
            })
            .unwrap();
        let texture = GpuTextureResource::immutable_rgba(
            crate::GpuTextureId::fresh(),
            1,
            1,
            Arc::from([255; 4]),
        );
        let mut vertices = sprite_vertices(
            [
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
            ],
            [0.0, 0.0, 1.0, 1.0],
            [0x00ff_ffff; 4],
            crate::GpuOuterModulation::Inherit,
        );
        for vertex in &mut vertices {
            vertex.software_sprite = Some(id);
        }
        recorder.add_texture(texture.clone());
        recorder.push(GpuCommand::Quad {
            texture: texture.id,
            owner_mask: None,
            vertices,
            clip: None,
            blend: GpuBlend::Normal,
            base_mod2: false,
            owner_mod2: false,
            sampler: GpuSampler::Nearest,
            gamma: false,
        });
        let scene = recorder.into_scene([1, 1], Color::transparent(), &GammaRamp::identity());
        assert!(CpuSceneRenderer::default()
            .render(&scene, &mut [0; 4])
            .is_err());
    }

    #[test]
    fn retained_tiles_rerasterize_only_changed_command_coverage() {
        use crate::{PixelFormat, Surface};
        let mut surface = Surface::new(130, 65, PixelFormat::Rgba8888);
        surface.begin_gpu_scene_capture();
        surface
            .blend_pixel(1, 1, Color::opaque(31, 73, 129))
            .unwrap();
        let mut scene = surface.take_gpu_scene_capture().unwrap().into_scene(
            [130, 65],
            Color::opaque(7, 13, 23),
            &GammaRamp::identity(),
        );
        let mut renderer = CpuSceneRenderer::default();
        let mut actual = vec![0; 130 * 65 * 4];
        renderer.render(&scene, &mut actual).unwrap();
        renderer.render(&scene, &mut actual).unwrap();
        assert_eq!(
            renderer.stats(),
            CpuSceneStats {
                rasterized_tiles: 0,
                reused_tiles: 6
            }
        );
        if let GpuCommand::Solid { vertices, .. } = &mut scene.commands[0] {
            vertices[0].color = [1.0, 0.0, 0.0, 1.0];
        }
        renderer.render(&scene, &mut actual).unwrap();
        assert_eq!(
            renderer.stats(),
            CpuSceneStats {
                rasterized_tiles: 1,
                reused_tiles: 5
            }
        );
        let mut fresh = vec![0; actual.len()];
        CpuSceneRenderer::default()
            .render(&scene, &mut fresh)
            .unwrap();
        assert_eq!(actual, fresh);
    }

    #[test]
    fn retained_lines_exclude_the_final_endpoint() {
        // StdGL.cpp:893-933 submits GL_LINES with pixel-centred vertices;
        // its software oracle walks the segment half-open at its end.
        let mut recorder = GpuSceneRecorder::default();
        recorder.push(GpuCommand::Solid {
            vertices: vec![
                GpuSolidVertex {
                    position: [0.5, 0.5, 1.0],
                    color: [1.0, 0.0, 0.0, 1.0],
                    outer_modulation: crate::GpuSolidOuterModulation::PackedC4,
                },
                GpuSolidVertex {
                    position: [3.5, 0.5, 1.0],
                    color: [1.0, 0.0, 0.0, 1.0],
                    outer_modulation: crate::GpuSolidOuterModulation::PackedC4,
                },
            ],
            topology: GpuPrimitiveTopology::LineList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: None,
            blend: GpuBlend::Normal,
            style: crate::GpuSolidStyle::NONE.with_software_blend(GpuSoftwareBlend::Legacy),
        });
        let scene = recorder.into_scene([4, 1], Color::opaque(0, 0, 0), &GammaRamp::identity());
        let mut actual = vec![0; 16];
        CpuSceneRenderer::default()
            .render(&scene, &mut actual)
            .unwrap();
        assert_eq!(
            actual,
            [255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 0, 0, 0, 255]
        );
    }

    #[test]
    fn retained_landscape_applies_liquid_before_framebuffer_store() {
        // StdGL.cpp:710-763: signed liquid channels perturb normalized base
        // RGB before color modulation and the final byte-domain store.
        let base = GpuTextureResource {
            id: crate::GpuTextureId::fresh(),
            extent: [1, 1],
            revision: 0,
            base_revision: None,
            format: GpuTextureFormat::Rgba8,
            pixels: std::sync::Arc::from([128, 128, 128, 255]),
            dirty: Vec::new(),
        };
        let mask = GpuTextureResource {
            id: crate::GpuTextureId::fresh(),
            extent: [1, 1],
            revision: 0,
            base_revision: None,
            format: GpuTextureFormat::R8,
            pixels: std::sync::Arc::from([255]),
            dirty: Vec::new(),
        };
        let liquid = GpuTextureResource {
            id: crate::GpuTextureId::fresh(),
            extent: [1, 1],
            revision: 0,
            base_revision: None,
            format: GpuTextureFormat::Rgba8,
            pixels: std::sync::Arc::from([255, 128, 128, 255]),
            dirty: Vec::new(),
        };
        let mut recorder = GpuSceneRecorder::default();
        recorder.push(GpuCommand::Landscape {
            base: base.id,
            liquid_mask: Some(mask.id),
            liquid: Some(liquid.id),
            vertices: [
                GpuVertex::new([0.0, 0.0, 1.0], [0.0, 0.0], [1.0, 1.0, 1.0, 0.0]),
                GpuVertex::new([1.0, 0.0, 1.0], [1.0, 0.0], [1.0, 1.0, 1.0, 0.0]),
                GpuVertex::new([0.0, 1.0, 1.0], [0.0, 1.0], [1.0, 1.0, 1.0, 0.0]),
                GpuVertex::new([1.0, 1.0, 1.0], [1.0, 1.0], [1.0, 1.0, 1.0, 0.0]),
            ],
            clip: None,
            phase: [-0.05, 0.0, 0.0],
            gamma: false,
        });
        recorder.add_texture(base);
        recorder.add_texture(mask);
        recorder.add_texture(liquid);
        let scene = recorder.into_scene([1, 1], Color::opaque(0, 0, 0), &GammaRamp::identity());
        let mut actual = [0; 4];
        CpuSceneRenderer::default()
            .render(&scene, &mut actual)
            .unwrap();
        assert_eq!(actual, [122, 122, 122, 255]);
    }

    #[test]
    fn retained_linear_sampling_keeps_oracle_multiply_add_order() {
        // StdGL.cpp:471-527 filtering precedes framebuffer rounding. The
        // immediate sampler evaluates p0*(1-f)+p1*f, without reassociation.
        let texture = GpuTextureResource {
            id: crate::GpuTextureId::fresh(),
            extent: [2, 2],
            revision: 0,
            base_revision: None,
            format: GpuTextureFormat::Rgba8,
            pixels: std::sync::Arc::from([
                100, 100, 100, 255, 198, 198, 198, 255, 55, 55, 55, 255, 233, 233, 233, 255,
            ]),
            dirty: Vec::new(),
        };
        let source = sample_texture(&texture, [0.583_333_3, 0.5], GpuSampler::Linear, [0.0; 4]);
        assert_eq!(source[0], 169.499_98);
    }

    #[test]
    fn retained_stretch_uses_integer_source_coordinates() {
        use crate::{BlitMode, PixelFormat, Surface};
        let mut source = Surface::new(2, 1, PixelFormat::Rgba8888);
        source.set_pixel(0, 0, Color::opaque(10, 20, 30)).unwrap();
        source
            .set_pixel(1, 0, Color::opaque(200, 210, 220))
            .unwrap();
        let draw = |surface: &mut Surface| {
            surface.fill(Color::opaque(0, 0, 0));
            surface
                .blit_stretched(
                    &source,
                    Rect::new(0, 0, 2, 1),
                    Rect::new(0, 0, 3, 1),
                    Color::opaque(255, 255, 255),
                    BlitMode::Normal,
                )
                .unwrap();
        };
        let mut surface = Surface::new(3, 1, PixelFormat::Rgba8888);
        draw(&mut surface);
        let expected = surface.pixels().to_vec();
        surface.begin_gpu_scene_capture();
        draw(&mut surface);
        let scene = surface.take_gpu_scene_capture().unwrap().into_scene(
            [3, 1],
            Color::transparent(),
            &GammaRamp::identity(),
        );
        let mut actual = vec![0; 12];
        CpuSceneRenderer::default()
            .render(&scene, &mut actual)
            .unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn retained_clear_overwrites_every_output_byte() {
        let scene = GpuSceneRecorder::default().into_scene(
            [3, 2],
            Color::new(1, 2, 3, 4),
            &GammaRamp::identity(),
        );
        let mut frame = vec![255; 24];
        CpuSceneRenderer::default()
            .render(&scene, &mut frame)
            .unwrap();
        assert_eq!(frame, [1, 2, 3, 4].repeat(6));
    }

    #[test]
    fn retained_point_execution_matches_both_software_blends() {
        use crate::{PixelFormat, Surface, SurfaceDrawTarget};
        for rounded in [false, true] {
            let draw = |surface: &mut Surface| {
                surface.fill(Color::opaque(0, 0, 0));
                if rounded {
                    surface
                        .blend_fragment_over(0, 0, [1.0, 1.0, 1.0, 128.0], None)
                        .unwrap();
                } else {
                    surface.blend_pixel(0, 0, Color::new(1, 1, 1, 128)).unwrap();
                }
            };
            let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);
            draw(&mut surface);
            let expected = surface.pixels().to_vec();
            surface.begin_gpu_scene_capture();
            draw(&mut surface);
            let scene = surface.take_gpu_scene_capture().unwrap().into_scene(
                [1, 1],
                Color::transparent(),
                &GammaRamp::identity(),
            );
            let mut frame = vec![0; 4];
            CpuSceneRenderer::default()
                .render(&scene, &mut frame)
                .unwrap();
            assert_eq!(frame, expected, "rounded={rounded}");
        }
    }

    #[test]
    fn retained_clipped_boxes_match_integer_surface_composition() {
        use crate::{PixelFormat, Surface};
        for clip in [Rect::new(1, 1, 3, 2), Rect::new(20, 20, 4, 4)] {
            let draw = |surface: &mut Surface| {
                surface.fill(Color::new(13, 27, 39, 67));
                surface.set_clip(clip);
                surface.fill_rect(Rect::new(-1, 0, 6, 4), Color::new(45, 91, 133, 128));
            };
            let mut surface = Surface::new(5, 4, PixelFormat::Rgba8888);
            draw(&mut surface);
            let expected = surface.pixels().to_vec();
            surface.clear_clip();
            surface.begin_gpu_scene_capture();
            draw(&mut surface);
            let mut scene = surface.take_gpu_scene_capture().unwrap().into_scene(
                [5, 4],
                Color::transparent(),
                &GammaRamp::identity(),
            );
            // An offscreen command can survive command lowering; the executor
            // must apply its scissor rather than treating it as unscissored.
            if clip.x == 20 {
                if let GpuCommand::Solid {
                    clip: command_clip,
                    vertices,
                    ..
                } = &mut scene.commands[0]
                {
                    *command_clip = Some(clip);
                    for vertex in vertices {
                        vertex.color = [1.0, 0.0, 0.0, 1.0];
                    }
                }
                scene.clear = Color::new(13, 27, 39, 67);
            }
            let mut frame = vec![0; 80];
            CpuSceneRenderer::default()
                .render(&scene, &mut frame)
                .unwrap();
            assert_eq!(frame, expected, "clip={clip:?}");
        }
    }

    #[test]
    fn retained_unfiltered_blit_matches_surface_source_preparation() {
        use crate::{BlitMode, PixelFormat, Point, Surface};
        let mut source = Surface::new(2, 1, PixelFormat::Rgba8888);
        source
            .set_pixel(0, 0, Color::new(73, 129, 201, 128))
            .unwrap();
        source
            .set_pixel(1, 0, Color::new(201, 37, 83, 255))
            .unwrap();
        for mode in [
            BlitMode::Normal,
            BlitMode::Additive,
            BlitMode::Mod2,
            BlitMode::Mod2Additive,
        ] {
            let draw = |surface: &mut Surface| {
                surface.fill(Color::new(13, 27, 39, 67));
                surface
                    .blit_region_ex(
                        &source,
                        Rect::new(0, 0, 2, 1),
                        Point::new(1, 1),
                        Color::new(177, 193, 213, 17),
                        mode,
                    )
                    .unwrap();
            };
            let mut surface = Surface::new(4, 3, PixelFormat::Rgba8888);
            draw(&mut surface);
            let expected = surface.pixels().to_vec();
            surface.begin_gpu_scene_capture();
            draw(&mut surface);
            let scene = surface.take_gpu_scene_capture().unwrap().into_scene(
                [4, 3],
                Color::transparent(),
                &GammaRamp::identity(),
            );
            let mut frame = vec![0; 48];
            CpuSceneRenderer::default()
                .render(&scene, &mut frame)
                .unwrap();
            assert_eq!(frame, expected, "mode={mode:?}");
        }
    }

    #[test]
    fn retained_batches_keep_sprite_painter_order() {
        use crate::{GpuObjectSprite, GpuOuterModulation, GpuSpriteQuad, GpuTextureId};
        use std::sync::Arc;
        let texture = GpuTextureResource::immutable_rgba(
            GpuTextureId::fresh(),
            1,
            1,
            Arc::from([201, 73, 129, 128]),
        );
        let positions = [
            [0.0, 0.0, 1.0],
            [2.0, 0.0, 1.0],
            [0.0, 2.0, 1.0],
            [2.0, 2.0, 1.0],
        ];
        let commands = [
            GpuCommand::SpriteBatch {
                texture: texture.id,
                quads: vec![
                    GpuSpriteQuad {
                        rect: [0.0, 0.0, 2.0, 2.0],
                        uv: [0.0, 0.0, 1.0, 1.0],
                        modulation: 0x00ff_ffff,
                        software_sprite: None,
                        software_shader: true,
                    };
                    2
                ],
                clip: None,
                blend: GpuBlend::Normal,
                mod2: false,
                gamma: false,
                outer_modulation: GpuOuterModulation::Inherit,
            },
            GpuCommand::ObjectBatch {
                texture: texture.id,
                owner_texture: None,
                sprites: vec![
                    GpuObjectSprite::new(
                        positions,
                        [0.0, 0.0, 1.0, 1.0],
                        [0x00ff_ffff; 4],
                        GpuSampler::Nearest,
                        0.0,
                        false,
                        GpuOuterModulation::Inherit
                    );
                    2
                ],
                clip: None,
                blend: GpuBlend::Normal,
                gamma: false,
            },
        ];
        let source = Color::new(201, 73, 129, 128);
        let expected = source.blend_over(source.blend_over(Color::opaque(13, 27, 39)));
        for command in commands {
            let scene = GpuScene::new(
                [2, 2],
                Color::opaque(13, 27, 39),
                crate::GpuGammaLut::from_ramp(&GammaRamp::identity()),
                crate::GpuGammaMode::Disabled,
                vec![texture.clone()],
                vec![command],
            );
            let mut frame = vec![0; 16];
            CpuSceneRenderer::default()
                .render(&scene, &mut frame)
                .unwrap();
            assert_eq!(
                frame,
                [expected.r, expected.g, expected.b, expected.a].repeat(4)
            );
        }
    }

    #[test]
    fn retained_monitor_gamma_resolves_after_the_complete_frame() {
        use crate::{PixelFormat, Surface};
        let gamma = GammaRamp::standard();
        let mut surface = Surface::new(2, 1, PixelFormat::Rgba8888);
        surface.fill(Color::new(0, 73, 129, 211));
        let mut expected = surface.pixels().to_vec();
        gamma.apply_to_rgba_bytes(&mut expected);
        surface.begin_gpu_scene_capture();
        surface.fill(Color::new(0, 73, 129, 211));
        let mut scene = surface.take_gpu_scene_capture().unwrap().into_scene(
            [2, 1],
            Color::transparent(),
            &gamma,
        );
        scene.gamma_mode = crate::GpuGammaMode::Monitor;
        let mut frame = vec![0; 8];
        CpuSceneRenderer::default()
            .render(&scene, &mut frame)
            .unwrap();
        assert_eq!(frame, expected);
    }
}
