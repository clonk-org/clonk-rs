//! Revisions of sampled texture regions, including skipped-upload recovery.

use crate::{GpuScene, GpuTextureFormat, GpuTextureId, Rect};
use std::hash::{Hash, Hasher};

const REGION_SIZE: u32 = 64;

#[derive(Default)]
pub(super) struct TextureRegions {
    entries: Vec<Entry>,
    next_epoch: u64,
}

struct Entry {
    id: GpuTextureId,
    extent: [u32; 2],
    format: GpuTextureFormat,
    revision: u64,
    epochs: Vec<u64>,
}

impl TextureRegions {
    pub fn update(&mut self, scene: &GpuScene) -> Result<(), super::CpuSceneError> {
        self.entries
            .retain(|entry| scene.textures.iter().any(|texture| texture.id == entry.id));
        for texture in &scene.textures {
            let index = self.entries.iter().position(|entry| entry.id == texture.id);
            if index.is_some_and(|index| {
                let entry = &self.entries[index];
                entry.extent == texture.extent
                    && entry.format == texture.format
                    && entry.revision == texture.revision
            }) {
                continue;
            }
            self.next_epoch = self
                .next_epoch
                .checked_add(1)
                .ok_or(super::CpuSceneError::InvalidFrame)?;
            let epoch = self.next_epoch;
            let index = index.unwrap_or_else(|| {
                self.entries.push(Entry {
                    id: texture.id,
                    extent: [0; 2],
                    format: texture.format,
                    revision: texture.revision,
                    epochs: Vec::new(),
                });
                self.entries.len() - 1
            });
            let entry = &mut self.entries[index];
            let columns = texture.extent[0].div_ceil(REGION_SIZE) as usize;
            let rows = texture.extent[1].div_ceil(REGION_SIZE) as usize;
            let incremental = entry.extent == texture.extent
                && entry.format == texture.format
                && texture.base_revision == Some(entry.revision)
                && !texture.dirty.is_empty();
            if incremental {
                for rect in &texture.dirty {
                    if rect.width == 0 || rect.height == 0 {
                        continue;
                    }
                    let left = rect.x as u32 / REGION_SIZE;
                    let top = rect.y as u32 / REGION_SIZE;
                    let right = (rect.x as u32 + rect.width - 1) / REGION_SIZE;
                    let bottom = (rect.y as u32 + rect.height - 1) / REGION_SIZE;
                    for y in top..=bottom {
                        entry.epochs[y as usize * columns + left as usize
                            ..=y as usize * columns + right as usize]
                            .fill(epoch);
                    }
                }
            } else {
                entry.epochs.resize(columns * rows, epoch);
                entry.epochs.fill(epoch);
            }
            entry.extent = texture.extent;
            entry.format = texture.format;
            entry.revision = texture.revision;
        }
        Ok(())
    }

    pub fn hash(&self, id: GpuTextureId, sampled: Option<Rect>, hash: &mut impl Hasher) {
        id.hash(hash);
        let Some(entry) = self.entries.iter().find(|entry| entry.id == id) else {
            return;
        };
        entry.extent.hash(hash);
        entry.format.hash(hash);
        let Some(sampled) = sampled else {
            entry.epochs.hash(hash);
            return;
        };
        let Some(rect) = sampled.intersection(Rect::new(0, 0, entry.extent[0], entry.extent[1]))
        else {
            return;
        };
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        let columns = entry.extent[0].div_ceil(REGION_SIZE) as usize;
        let left = rect.x as u32 / REGION_SIZE;
        let top = rect.y as u32 / REGION_SIZE;
        let right = (rect.x as u32 + rect.width - 1) / REGION_SIZE;
        let bottom = (rect.y as u32 + rect.height - 1) / REGION_SIZE;
        for y in top..=bottom {
            entry.epochs
                [y as usize * columns + left as usize..=y as usize * columns + right as usize]
                .hash(hash);
        }
    }
}

/// The integral landscape path samples exactly one texel per screen pixel.
/// Other transforms use the complete resource as a conservative dependency.
pub(super) fn landscape_samples(
    scene: &GpuScene,
    vertices: &[crate::GpuVertex; 4],
    tile: Rect,
) -> Option<Rect> {
    let sprite = vertices[0]
        .software_sprite
        .and_then(|id| scene.software_sprite(id))?;
    let crate::GpuSoftwareSpriteMapping::Landscape {
        zoom: 1.0,
        indent: 0.0,
        tile_origin,
        ..
    } = sprite.mapping
    else {
        return None;
    };
    if sprite.inverse != crate::Transform::identity()
        || sprite
            .destination
            .iter()
            .chain(sprite.source[..2].iter())
            .chain(sprite.translation.iter())
            .any(|value| value.fract() != 0.0 || value.abs() > 4_000_000.0)
        || [tile.x, tile.y]
            .into_iter()
            .any(|value| value.abs() > 4_000_000)
    {
        return None;
    }
    for (i, coordinates) in [
        [
            tile.x,
            i32::try_from(i64::from(tile.x) + i64::from(tile.width)).ok()?,
        ],
        [
            tile.y,
            i32::try_from(i64::from(tile.y) + i64::from(tile.height)).ok()?,
        ],
    ]
    .into_iter()
    .enumerate()
    {
        for coordinate in coordinates {
            let point = f64::from(coordinate) + 0.5;
            let translated = point - f64::from(sprite.translation[i]);
            let local = translated - f64::from(sprite.destination[i]);
            if [
                point,
                translated,
                local,
                local + f64::from(sprite.source[i]),
            ]
            .into_iter()
            .any(|value| value.abs() > 4_000_000.0)
            {
                return None;
            }
        }
    }
    let axis = |i: usize, coordinate: i32| {
        i64::from(coordinate) + sprite.source[i] as i64
            - sprite.destination[i] as i64
            - sprite.translation[i] as i64
            - i64::from(tile_origin[i])
    };
    Some(Rect::new(
        i32::try_from(axis(0, tile.x)).ok()?,
        i32::try_from(axis(1, tile.y)).ok()?,
        tile.width,
        tile.height,
    ))
}
