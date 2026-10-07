//! Painter-order bins and fingerprints for the software executor.

use crate::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

pub(super) const TILE_SIZE: u32 = 64;

#[derive(Clone, Copy)]
pub(super) struct Atom {
    pub command: usize,
    pub primitive: usize,
}

pub(super) struct Tile {
    pub bounds: Rect,
    pub atoms: Vec<Atom>,
    pub hash: DefaultHasher,
    pub previous: Option<u64>,
}

impl Tile {
    pub fn new(bounds: Rect) -> Self {
        Self {
            bounds,
            atoms: Vec::new(),
            hash: DefaultHasher::new(),
            previous: None,
        }
    }
}

pub(super) fn atom_count(command: &GpuCommand) -> usize {
    match command {
        GpuCommand::Quad { .. } | GpuCommand::Landscape { .. } => 1,
        GpuCommand::SpriteBatch { quads, .. } => quads.len(),
        GpuCommand::ObjectBatch { sprites, .. } => sprites.len(),
        GpuCommand::Solid {
            vertices, topology, ..
        } => vertices.len() / primitive_size(*topology),
    }
}

pub(super) fn primitive_size(topology: GpuPrimitiveTopology) -> usize {
    match topology {
        GpuPrimitiveTopology::PointList => 1,
        GpuPrimitiveTopology::LineList => 2,
        GpuPrimitiveTopology::TriangleList => 3,
    }
}

fn floats<const N: usize>(values: [f32; N], hash: &mut DefaultHasher) {
    values.map(f32::to_bits).hash(hash);
}

fn texture(scene: &GpuScene, id: GpuTextureId, hash: &mut DefaultHasher) {
    id.hash(hash);
    if let Some(resource) = scene.texture(id) {
        resource.extent.hash(hash);
        (resource.format as u8).hash(hash);
        resource.revision.hash(hash);
    }
}

fn software(scene: &GpuScene, id: Option<GpuSoftwareSpriteId>, hash: &mut DefaultHasher) {
    let value = id.and_then(|id| scene.software_sprite(id));
    value.is_some().hash(hash);
    let Some(value) = value else {
        return;
    };
    floats(value.destination, hash);
    floats(value.source, hash);
    floats(value.inverse.mat, hash);
    floats(value.translation, hash);
    value.gamma.is_some().hash(hash);
    if let Some(gamma) = &value.gamma {
        // frame_key already fingerprints the installed ramp once. Most
        // sprites share it, so repeating 768 LUT entries per vertex wastes
        // lowering time. Independent widget ramps keep their own contents.
        let installed = std::sync::Arc::ptr_eq(&gamma.channels, &scene.gamma.channels);
        installed.hash(hash);
        if !installed {
            gamma.channels.hash(hash);
        }
    }
    value.flip_x.hash(hash);
    value.inclusive_source_end.hash(hash);
    match value.mapping {
        GpuSoftwareSpriteMapping::Native => 0u8.hash(hash),
        GpuSoftwareSpriteMapping::RotatedCorner { center, cos, sin } => {
            9u8.hash(hash);
            floats(center, hash);
            floats([cos, sin], hash);
        }
        GpuSoftwareSpriteMapping::PixelCorner => 1u8.hash(hash),
        GpuSoftwareSpriteMapping::GuiNearest => 6u8.hash(hash),
        GpuSoftwareSpriteMapping::TileCopy => 2u8.hash(hash),
        GpuSoftwareSpriteMapping::IntegerStretch => 3u8.hash(hash),
        GpuSoftwareSpriteMapping::GuiFacetLinear => 8u8.hash(hash),
        GpuSoftwareSpriteMapping::GuiLinear { modulation } => {
            4u8.hash(hash);
            modulation.hash(hash);
        }
        GpuSoftwareSpriteMapping::Font {
            shear,
            center_y,
            texture_indent,
            physical_size,
            normalize_transparent,
        } => {
            7u8.hash(hash);
            floats([shear, center_y, texture_indent, physical_size], hash);
            normalize_transparent.hash(hash);
        }
        GpuSoftwareSpriteMapping::Landscape {
            zoom,
            world_extent,
            tile_origin,
            texture_size,
            indent,
        } => {
            5u8.hash(hash);
            floats([zoom, indent], hash);
            world_extent.hash(hash);
            tile_origin.hash(hash);
            texture_size.hash(hash);
        }
    }
    value.fog.is_some().hash(hash);
    if let Some(fog) = value.fog {
        floats(fog.destination, hash);
        floats(fog.source_range, hash);
    }
}

fn vertices(scene: &GpuScene, vertices: &[GpuVertex; 4], hash: &mut DefaultHasher) {
    // CPU sampling resolves the first descriptor. GUI linear spans also
    // consume whether all four IDs match, independently of their contents.
    software(scene, vertices[0].software_sprite, hash);
    vertices
        .iter()
        .all(|vertex| vertex.software_sprite == vertices[0].software_sprite)
        .hash(hash);
    for vertex in vertices {
        floats(vertex.position, hash);
        floats(vertex.uv, hash);
        floats(vertex.modulation, hash);
        floats(vertex.owner_modulation, hash);
        floats(vertex.sample_tile, hash);
        (vertex.outer_modulation as u8).hash(hash);
        (vertex.owner_outer_modulation as u8).hash(hash);
        vertex.software_shader.hash(hash);
        (vertex.software_alpha_mode as u8).hash(hash);
        vertex.software_blit.is_some().hash(hash);
        if let Some(blit) = vertex.software_blit {
            blit.source.hash(hash);
            [
                blit.modulation.r,
                blit.modulation.g,
                blit.modulation.b,
                blit.modulation.a,
            ]
            .hash(hash);
            blit.mode.map(|mode| mode as u8).hash(hash);
            floats(blit.translation, hash);
            match blit.mapping {
                GpuSoftwareBlitMapping::Unscaled(origin) => {
                    0u8.hash(hash);
                    (origin.x, origin.y).hash(hash);
                }
                GpuSoftwareBlitMapping::Stretched(destination) => {
                    1u8.hash(hash);
                    destination.hash(hash);
                }
                GpuSoftwareBlitMapping::Transformed { origin, inverse } => {
                    2u8.hash(hash);
                    (origin.x, origin.y).hash(hash);
                    floats(inverse.mat, hash);
                }
            }
        }
    }
}

pub(super) fn atom_hash(scene: &GpuScene, command: &GpuCommand, atom: usize) -> u64 {
    let mut hash = DefaultHasher::new();
    command.clip().hash(&mut hash);
    match command {
        GpuCommand::Quad {
            texture: id,
            owner_mask,
            vertices: quad,
            blend,
            base_mod2,
            owner_mod2,
            sampler,
            gamma,
            ..
        } => {
            0u8.hash(&mut hash);
            texture(scene, *id, &mut hash);
            owner_mask.is_some().hash(&mut hash);
            if let Some((id, mask)) = owner_mask {
                texture(scene, *id, &mut hash);
                (*mask as u8).hash(&mut hash);
            }
            vertices(scene, quad, &mut hash);
            (
                *blend as u8,
                *base_mod2,
                *owner_mod2,
                *sampler as u8,
                *gamma,
            )
                .hash(&mut hash);
        }
        GpuCommand::SpriteBatch {
            texture: id,
            quads,
            blend,
            mod2,
            gamma,
            outer_modulation,
            ..
        } => {
            1u8.hash(&mut hash);
            texture(scene, *id, &mut hash);
            let quad = quads[atom];
            floats(quad.rect, &mut hash);
            floats(quad.uv, &mut hash);
            quad.modulation.hash(&mut hash);
            quad.software_shader.hash(&mut hash);
            software(scene, quad.software_sprite, &mut hash);
            (*blend as u8, *mod2, *gamma, *outer_modulation as u8).hash(&mut hash);
        }
        GpuCommand::ObjectBatch {
            texture: id,
            owner_texture,
            sprites,
            blend,
            gamma,
            ..
        } => {
            2u8.hash(&mut hash);
            let sprite = sprites[atom];
            texture(
                scene,
                if sprite.owner_layer() {
                    owner_texture.unwrap_or(*id)
                } else {
                    *id
                },
                &mut hash,
            );
            for position in sprite.positions {
                floats(position, &mut hash);
            }
            floats(sprite.uv, &mut hash);
            sprite.modulation.hash(&mut hash);
            floats([sprite.sample_tile_size], &mut hash);
            (
                sprite.sampler() as u8,
                sprite.mod2(),
                sprite.owner_layer(),
                sprite.outer_modulation() as u8,
                sprite.software_shader(),
                *blend as u8,
                *gamma,
            )
                .hash(&mut hash);
            software(scene, sprite.software_sprite(), &mut hash);
        }
        GpuCommand::Landscape {
            base,
            liquid_mask,
            liquid,
            vertices: quad,
            phase,
            gamma,
            ..
        } => {
            3u8.hash(&mut hash);
            base.hash(&mut hash);
            for id in [liquid_mask, liquid] {
                id.is_some().hash(&mut hash);
                id.hash(&mut hash);
            }
            vertices(scene, quad, &mut hash);
            floats(*phase, &mut hash);
            gamma.hash(&mut hash);
        }
        GpuCommand::Solid {
            vertices,
            topology,
            alpha_mode,
            blend,
            style,
            ..
        } => {
            4u8.hash(&mut hash);
            (
                *topology as u8,
                *alpha_mode as u8,
                *blend as u8,
                style.gamma,
                style.dither,
                style.software_blend as u8,
            )
                .hash(&mut hash);
            style.software_line_bounds.hash(&mut hash);
            let size = primitive_size(*topology);
            for vertex in &vertices[atom * size..(atom + 1) * size] {
                floats(vertex.position, &mut hash);
                floats(vertex.color, &mut hash);
                (vertex.outer_modulation as u8).hash(&mut hash);
            }
        }
    }
    hash.finish()
}
