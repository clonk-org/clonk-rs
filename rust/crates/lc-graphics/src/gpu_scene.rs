//! Backend-neutral retained-texture scene commands.
//!
//! The native renderer keeps image and landscape textures resident, then
//! reissues a small painter-ordered command stream to the current window each
//! frame.  This module describes that stream without coupling the renderer to
//! OpenGL or wgpu.  The software [`crate::Surface`] path remains the reference
//! implementation used by headless rendering and deterministic tests.

use crate::{Color, GammaRamp, Rect};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

static NEXT_TEXTURE_ID: AtomicU64 = AtomicU64::new(1);

/// Process-local identity of one retained sampled texture.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct GpuTextureId(u64);

impl GpuTextureId {
    /// Allocate an identity that will not be reused during this process.
    pub fn fresh() -> Self {
        let id = NEXT_TEXTURE_ID.fetch_add(1, Ordering::Relaxed);
        assert_ne!(id, 0, "GPU texture identity space exhausted");
        Self(id)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GpuTextureFormat {
    Rgba8,
    R8,
}

impl GpuTextureFormat {
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgba8 => 4,
            Self::R8 => 1,
        }
    }
}

/// Complete regeneration data plus the incremental update for this frame.
///
/// `pixels` always contains the current complete resource.  A backend uses it
/// after device recreation or a cache miss, and otherwise uploads only
/// `dirty`.  An unchanged revision must have an empty dirty list.
#[derive(Clone, Debug)]
pub struct GpuTextureResource {
    pub id: GpuTextureId,
    pub extent: [u32; 2],
    pub revision: u64,
    /// Revision against which `dirty` was calculated. A backend may apply the
    /// rectangles only when its cached revision equals this value; otherwise
    /// the complete backing below is the loss/skipped-frame fallback.
    pub base_revision: Option<u64>,
    pub format: GpuTextureFormat,
    pub pixels: Arc<[u8]>,
    pub dirty: Vec<Rect>,
}

impl GpuTextureResource {
    pub fn immutable_rgba(id: GpuTextureId, width: u32, height: u32, pixels: Arc<[u8]>) -> Self {
        Self {
            id,
            extent: [width, height],
            revision: 0,
            base_revision: None,
            format: GpuTextureFormat::Rgba8,
            pixels,
            dirty: Vec::new(),
        }
    }

    pub fn expected_len(&self) -> Option<usize> {
        usize::try_from(self.extent[0])
            .ok()?
            .checked_mul(usize::try_from(self.extent[1]).ok()?)?
            .checked_mul(self.format.bytes_per_pixel())
    }

    pub fn is_valid(&self) -> bool {
        self.extent[0] != 0 && self.extent[1] != 0 && self.expected_len() == Some(self.pixels.len())
    }
}

/// Exact native 16-bit per-channel lookup texture for one frame.
#[derive(Clone, Debug)]
pub struct GpuGammaLut {
    pub revision: u64,
    pub channels: Arc<[[u16; 256]; 3]>,
}

impl GpuGammaLut {
    pub fn from_ramp(ramp: &GammaRamp) -> Self {
        Self {
            revision: ramp.gpu_revision(),
            channels: Arc::new(ramp.channels()),
        }
    }
}

/// Mapping from logical C4 coordinates into the physical drawable.
///
/// Native anchors an oversized scaled viewport at the lower-left.  In the
/// top-down byte convention used by Rust that means subtracting `crop_top`
/// after multiplying logical Y by `scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuPresentation {
    pub physical_extent: [u32; 2],
    pub scale: f32,
    pub crop_top: u32,
}

impl GpuPresentation {
    pub fn identity(width: u32, height: u32) -> Self {
        Self {
            physical_extent: [width, height],
            scale: 1.0,
            crop_top: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuBlend {
    Replace,
    Normal,
    Additive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuSampler {
    Nearest,
    Linear,
}

/// A second single-channel texture replaces marked base texels with a grey
/// owner-colour source.  Full-RGBA owner overlays are emitted as a second
/// ordinary quad so their native painter order remains explicit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuOwnerMask {
    Scalar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuPrimitiveTopology {
    TriangleList,
    LineList,
    PointList,
}

/// Textured vertex. `position` is homogeneous logical `[x, y, w]`; retaining
/// W lets the backend preserve perspective-correct projective sampling.
/// Modulation is normalized packed-C4 `[r, g, b, transparency]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuVertex {
    pub position: [f32; 3],
    pub uv: [f32; 2],
    pub modulation: [f32; 4],
    pub owner_modulation: [f32; 4],
    /// Native texture-tile sampling metadata `[origin_x, origin_y, size,
    /// enabled]` in source texels. Linear blits use this to reproduce the
    /// independently clamped/padded `C4TexRef` tiles instead of filtering
    /// across their seams. Other draws leave it disabled.
    pub sample_tile: [f32; 4],
}

impl GpuVertex {
    pub fn new(position: [f32; 3], uv: [f32; 2], modulation: [f32; 4]) -> Self {
        Self {
            position,
            uv,
            modulation,
            owner_modulation: modulation,
            sample_tile: [0.0; 4],
        }
    }

    pub fn with_sample_tile(mut self, origin_x: f32, origin_y: f32, size: f32) -> Self {
        self.sample_tile = [origin_x, origin_y, size, 1.0];
        self
    }

    fn translate(&mut self, x: f32, y: f32) {
        self.position[0] += x * self.position[2];
        self.position[1] += y * self.position[2];
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuSolidVertex {
    pub position: [f32; 3],
    /// Straight RGBA opacity, unlike the packed-C4 modulation above.
    pub color: [f32; 4],
}

impl GpuSolidVertex {
    fn translate(&mut self, x: f32, y: f32) {
        self.position[0] += x * self.position[2];
        self.position[1] += y * self.position[2];
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum GpuCommand {
    Quad {
        texture: GpuTextureId,
        owner_mask: Option<(GpuTextureId, GpuOwnerMask)>,
        vertices: [GpuVertex; 4],
        clip: Option<Rect>,
        blend: GpuBlend,
        base_mod2: bool,
        owner_mod2: bool,
        sampler: GpuSampler,
        gamma: bool,
    },
    Landscape {
        base: GpuTextureId,
        liquid_mask: Option<GpuTextureId>,
        liquid: Option<GpuTextureId>,
        vertices: [GpuVertex; 4],
        clip: Option<Rect>,
        phase: [f32; 3],
        gamma: bool,
    },
    Solid {
        vertices: Vec<GpuSolidVertex>,
        topology: GpuPrimitiveTopology,
        clip: Option<Rect>,
        blend: GpuBlend,
        gamma: bool,
    },
}

impl GpuCommand {
    pub fn translate(&mut self, x: f32, y: f32) {
        match self {
            Self::Quad { vertices, clip, .. } | Self::Landscape { vertices, clip, .. } => {
                vertices
                    .iter_mut()
                    .for_each(|vertex| vertex.translate(x, y));
                translate_clip(clip, x, y);
            }
            Self::Solid { vertices, clip, .. } => {
                vertices
                    .iter_mut()
                    .for_each(|vertex| vertex.translate(x, y));
                translate_clip(clip, x, y);
            }
        }
    }

    pub fn clip_to(&mut self, bounds: Rect) -> bool {
        let clip = match self {
            Self::Quad { clip, .. } | Self::Landscape { clip, .. } | Self::Solid { clip, .. } => {
                clip
            }
        };
        *clip = match *clip {
            Some(current) => current.intersection(bounds),
            None => Some(bounds),
        };
        clip.is_some_and(|clip| clip.width != 0 && clip.height != 0)
    }
}

fn translate_clip(clip: &mut Option<Rect>, x: f32, y: f32) {
    let Some(clip) = clip.as_mut() else {
        return;
    };
    clip.x = clip.x.saturating_add(x.round() as i32);
    clip.y = clip.y.saturating_add(y.round() as i32);
}

#[derive(Clone, Debug)]
pub struct GpuScene {
    pub logical_extent: [u32; 2],
    pub clear: Color,
    pub gamma: GpuGammaLut,
    pub textures: Vec<GpuTextureResource>,
    pub commands: Vec<GpuCommand>,
}

/// Mutable command sink carried by recording surfaces and flattened when a
/// CPU scratch surface is presented into its parent.
#[derive(Clone, Debug, Default)]
pub struct GpuSceneRecorder {
    textures: HashMap<GpuTextureId, GpuTextureResource>,
    commands: Vec<GpuCommand>,
}

impl GpuSceneRecorder {
    pub fn add_texture(&mut self, resource: GpuTextureResource) {
        match self.textures.entry(resource.id) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(resource);
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                if entry.get().revision < resource.revision {
                    entry.insert(resource);
                } else if entry.get().revision == resource.revision
                    && entry.get().dirty.is_empty()
                    && !resource.dirty.is_empty()
                {
                    entry.insert(resource);
                }
            }
        }
    }

    pub fn push(&mut self, command: GpuCommand) {
        if let GpuCommand::Solid {
            vertices,
            topology,
            clip,
            blend,
            gamma,
        } = command
        {
            if let Some(GpuCommand::Solid {
                vertices: previous,
                topology: previous_topology,
                clip: previous_clip,
                blend: previous_blend,
                gamma: previous_gamma,
            }) = self.commands.last_mut()
            {
                if *previous_topology == topology
                    && *previous_clip == clip
                    && *previous_blend == blend
                    && *previous_gamma == gamma
                {
                    previous.extend(vertices);
                    return;
                }
            }
            self.commands.push(GpuCommand::Solid {
                vertices,
                topology,
                clip,
                blend,
                gamma,
            });
            return;
        }
        self.commands.push(command);
    }

    pub fn push_solid_vertex(
        &mut self,
        vertex: GpuSolidVertex,
        topology: GpuPrimitiveTopology,
        clip: Option<Rect>,
        blend: GpuBlend,
        gamma: bool,
    ) {
        if let Some(GpuCommand::Solid {
            vertices,
            topology: previous_topology,
            clip: previous_clip,
            blend: previous_blend,
            gamma: previous_gamma,
        }) = self.commands.last_mut()
        {
            if *previous_topology == topology
                && *previous_clip == clip
                && *previous_blend == blend
                && *previous_gamma == gamma
            {
                vertices.push(vertex);
                return;
            }
        }
        self.commands.push(GpuCommand::Solid {
            vertices: vec![vertex],
            topology,
            clip,
            blend,
            gamma,
        });
    }

    pub fn append_translated(
        &mut self,
        mut child: Self,
        offset_x: i32,
        offset_y: i32,
        child_bounds: Rect,
        destination_clip: Option<Rect>,
    ) {
        for (_, resource) in child.textures.drain() {
            self.add_texture(resource);
        }
        for mut command in child.commands.drain(..) {
            if !command.clip_to(child_bounds) {
                continue;
            }
            command.translate(offset_x as f32, offset_y as f32);
            if destination_clip.is_some_and(|clip| !command.clip_to(clip)) {
                continue;
            }
            self.commands.push(command);
        }
    }

    pub fn into_scene(self, logical_extent: [u32; 2], clear: Color, gamma: &GammaRamp) -> GpuScene {
        let Self {
            mut textures,
            commands,
        } = self;
        let mut referenced = HashSet::new();
        for command in &commands {
            match command {
                GpuCommand::Quad {
                    texture,
                    owner_mask,
                    ..
                } => {
                    referenced.insert(*texture);
                    if let Some((texture, _)) = owner_mask {
                        referenced.insert(*texture);
                    }
                }
                GpuCommand::Landscape {
                    base,
                    liquid_mask,
                    liquid,
                    ..
                } => {
                    referenced.insert(*base);
                    referenced.extend(liquid_mask.iter().copied());
                    referenced.extend(liquid.iter().copied());
                }
                GpuCommand::Solid { .. } => {}
            }
        }
        // Scratch surfaces may record resources before their commands are
        // clipped while flattening. Do not upload or pin resources that have
        // no surviving draw in the final scene.
        textures.retain(|id, _| referenced.contains(id));
        let mut textures = textures.into_values().collect::<Vec<_>>();
        textures.sort_by_key(|resource| resource.id);
        GpuScene {
            logical_extent,
            clear,
            gamma: GpuGammaLut::from_ramp(gamma),
            textures,
            commands,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translated_projective_vertex_preserves_homogeneous_position() {
        let mut vertex = GpuVertex::new([20.0, 30.0, 2.0], [0.0, 0.0], [1.0, 1.0, 1.0, 0.0]);
        vertex.translate(5.0, 7.0);
        assert_eq!(vertex.position, [30.0, 44.0, 2.0]);
    }

    #[test]
    fn texture_resource_rejects_malformed_backing() {
        let resource =
            GpuTextureResource::immutable_rgba(GpuTextureId::fresh(), 2, 2, Arc::from([0_u8; 15]));
        assert!(!resource.is_valid());
    }

    #[test]
    fn scene_omits_resources_without_surviving_commands() {
        let used = GpuTextureId::fresh();
        let clipped = GpuTextureId::fresh();
        let mut child = GpuSceneRecorder::default();
        child.add_texture(GpuTextureResource::immutable_rgba(
            used,
            1,
            1,
            Arc::from([255_u8; 4]),
        ));
        child.add_texture(GpuTextureResource::immutable_rgba(
            clipped,
            1,
            1,
            Arc::from([0_u8; 4]),
        ));
        let vertex = GpuVertex::new(
            [0.0, 0.0, 1.0],
            [0.0, 0.0],
            [1.0, 1.0, 1.0, 0.0],
        );
        child.push(GpuCommand::Quad {
            texture: used,
            owner_mask: None,
            vertices: [vertex; 4],
            clip: None,
            blend: GpuBlend::Normal,
            base_mod2: false,
            owner_mod2: false,
            sampler: GpuSampler::Nearest,
            gamma: false,
        });

        let scene = child.into_scene([1, 1], Color::transparent(), &GammaRamp::standard());
        assert_eq!(scene.textures.len(), 1);
        assert_eq!(scene.textures[0].id, used);
    }
}
