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

/// Where the active native gamma ramp is applied for one retained frame.
///
/// CStdGL uses fragment lookup only when both shader switches are enabled.
/// The fixed-function path leaves all draws untouched and exposes the ramp
/// after the complete framebuffer has been composed. `Disabled` bypasses
/// both operations continuously.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuGammaMode {
    #[default]
    Fragment,
    Monitor,
    Disabled,
}

impl GpuGammaMode {
    pub const fn fragment_lookup(self) -> bool {
        matches!(self, Self::Fragment)
    }

    pub const fn monitor_postpass(self) -> bool {
        matches!(self, Self::Monitor)
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
    /// The viewport zoom the world is magnified by, where `1.0` is unzoomed.
    ///
    /// Vertex *positions* already carry it through the projection, but a point
    /// or line's raster footprint is a width rather than a position, so it has
    /// to be multiplied in separately. Without it, magnifying the world would
    /// leave rain, spray, dug-material sparks and every debug line at their
    /// unzoomed width.
    ///
    /// Presentation only: nothing the lockstep simulation reads may derive
    /// from this.
    pub world_zoom: f32,
}

impl GpuPresentation {
    pub fn identity(width: u32, height: u32) -> Self {
        Self {
            physical_extent: [width, height],
            scale: 1.0,
            crop_top: 0,
            world_zoom: 1.0,
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

/// Destination-alpha provenance for a retained solid primitive.
///
/// [`GpuSolidVertex::color`] always stores straight opacity and every solid
/// producer blends RGB identically; they differ in the framebuffer-alpha
/// equation of the deterministic CPU reference. Primitive draws (quads,
/// boxes, lines, points) keep source-over alpha, while sampled-fragment
/// recovery through `SurfaceDrawTarget::blend_fragment` weights the stored
/// alpha by the same source factor as its RGB — the non-separate GL
/// equation. Backends must preserve the distinction so retained replay
/// reproduces the exact CPU-reference bytes; additive commands preserve
/// destination alpha under both modes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GpuSolidAlphaMode {
    /// Primitive fills/lines/points: `Aout = As + Ad*(1-As)`.
    SourceOver,
    /// Sampled-fragment recovery: `Aout = As*As + Ad*(1-As)`.
    NonSeparate,
}

/// How a solid vertex responds to an enclosing C++ blit modulation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuSolidOuterModulation {
    /// The vertex came from DrawBox/DrawLine/DrawQuad and therefore combines
    /// byte colors through C++ `ModulateClr` (including its `>> 8` quirk).
    #[default]
    PackedC4,
    /// The vertex is an already-filtered texture fragment retained by a CPU
    /// recovery path. Apply the outer texture shader directly to its floats.
    SampledTexture,
    /// A nested native state explicitly suppresses the enclosing modulation.
    Ignore,
}

/// How a retained texture modulation relates to an enclosing C++
/// `ActivateBlitModulation` state.
///
/// CStdDDraw does not treat its identity modulation as an ordinary color.
/// An unmodulated base blit inherits the active value directly, while an
/// already-colored owner/fog/local draw explicitly combines with it through
/// `ModulateClr`. `C4GFXBLIT_CLRSFC_OWNCLR` and nested local overrides ignore
/// the enclosing value altogether.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuOuterModulation {
    /// Replace the captured identity with the enclosing modulation.
    #[default]
    Inherit,
    /// Combine the captured packed-C4 color with the enclosing modulation.
    Combine,
    /// Preserve the captured color; the native draw suppresses/overrides the
    /// enclosing modulation.
    Ignore,
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
    /// Provenance of `modulation` relative to a later enclosing blit state.
    pub outer_modulation: GpuOuterModulation,
    /// Equivalent provenance for the legacy combined-owner representation.
    /// Lowered owner passes normally use `outer_modulation` instead.
    pub owner_outer_modulation: GpuOuterModulation,
    /// Native texture-tile sampling metadata `[origin_x, origin_y, size,
    /// enabled]` in source texels. Linear blits use this to reproduce the
    /// independently clamped/padded `C4TexRef` tiles instead of filtering
    /// across their seams. Other draws leave it disabled.
    pub sample_tile: [f32; 4],
}

/// One affine, axis-aligned sprite in a retained painter-order batch.
///
/// Positions and UVs are stored as `[left, top, right, bottom]`. Every corner
/// has homogeneous W=1, shares one packed-C4 modulation, and uses nearest
/// sampling without native tile metadata. More general textured draws remain
/// [`GpuCommand::Quad`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuSpriteQuad {
    pub rect: [f32; 4],
    pub uv: [f32; 4],
    pub modulation: u32,
}

/// One compact object face in retained painter order.
///
/// Positions retain homogeneous logical `[x, y, w]` coordinates so rotated,
/// mirrored and projective object transforms do not need the generic
/// [`GpuVertex`] layout. UVs are the axis-aligned source edges
/// `[left, top, right, bottom]`; a reversed edge represents a source flip.
/// Packed per-corner modulation preserves the exact byte-domain fog and
/// `ModulateClr` result. `sample_tile_size` is zero for nearest sampling and
/// the native `C4TexRef` tile size for linear sampling.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuObjectSprite {
    pub positions: [[f32; 3]; 4],
    pub uv: [f32; 4],
    pub modulation: [u32; 4],
    pub sample_tile_size: f32,
    flags: u32,
}

impl GpuObjectSprite {
    pub const FLAG_MOD2: u32 = 1 << 0;
    pub const FLAG_LINEAR: u32 = 1 << 1;
    /// Select the companion owner texture in a paired object batch. Bit four
    /// remains reserved for the renderer-only fragment-gamma flag.
    pub const FLAG_OWNER_LAYER: u32 = 1 << 5;
    const OUTER_MODULATION_SHIFT: u32 = 2;
    const OUTER_MODULATION_MASK: u32 = 0b11 << Self::OUTER_MODULATION_SHIFT;
    const DEFINED_FLAGS_MASK: u32 =
        Self::FLAG_MOD2 | Self::FLAG_LINEAR | Self::OUTER_MODULATION_MASK | Self::FLAG_OWNER_LAYER;

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        positions: [[f32; 3]; 4],
        uv: [f32; 4],
        modulation: [u32; 4],
        sampler: GpuSampler,
        sample_tile_size: f32,
        mod2: bool,
        outer_modulation: GpuOuterModulation,
    ) -> Self {
        let sampler_flag = match sampler {
            GpuSampler::Nearest => 0,
            GpuSampler::Linear => Self::FLAG_LINEAR,
        };
        let mod2_flag = u32::from(mod2) * Self::FLAG_MOD2;
        let outer_modulation_flag = match outer_modulation {
            GpuOuterModulation::Inherit => 0,
            GpuOuterModulation::Combine => 1,
            GpuOuterModulation::Ignore => 2,
        } << Self::OUTER_MODULATION_SHIFT;
        Self {
            positions,
            uv,
            modulation,
            sample_tile_size,
            flags: sampler_flag | mod2_flag | outer_modulation_flag,
        }
    }

    pub const fn sampler(self) -> GpuSampler {
        if self.flags & Self::FLAG_LINEAR == 0 {
            GpuSampler::Nearest
        } else {
            GpuSampler::Linear
        }
    }

    pub const fn mod2(self) -> bool {
        self.flags & Self::FLAG_MOD2 != 0
    }

    pub const fn owner_layer(self) -> bool {
        self.flags & Self::FLAG_OWNER_LAYER != 0
    }

    pub const fn with_owner_layer(mut self) -> Self {
        self.flags |= Self::FLAG_OWNER_LAYER;
        self
    }

    /// Packed renderer transport bits produced by the safe constructor.
    pub const fn packed_flags(self) -> u32 {
        self.flags
    }

    /// Whether the transport word contains only defined flags and policies.
    pub const fn has_valid_packed_flags(self) -> bool {
        self.flags & !Self::DEFINED_FLAGS_MASK == 0
            && self.flags & Self::OUTER_MODULATION_MASK != Self::OUTER_MODULATION_MASK
    }

    pub const fn outer_modulation(self) -> GpuOuterModulation {
        match (self.flags & Self::OUTER_MODULATION_MASK) >> Self::OUTER_MODULATION_SHIFT {
            0 => GpuOuterModulation::Inherit,
            1 => GpuOuterModulation::Combine,
            _ => GpuOuterModulation::Ignore,
        }
    }

    fn translate(&mut self, x: f32, y: f32) {
        self.positions.iter_mut().for_each(|position| {
            position[0] += x * position[2];
            position[1] += y * position[2];
        });
    }
}

impl GpuVertex {
    pub fn new(position: [f32; 3], uv: [f32; 2], modulation: [f32; 4]) -> Self {
        // Identity is the native "no active modulation" sentinel. Any
        // non-identity value has already been produced by a local color, fog,
        // or owner pass and therefore combines with an enclosing value.
        let outer_modulation = if modulation == [1.0, 1.0, 1.0, 0.0] {
            GpuOuterModulation::Inherit
        } else {
            GpuOuterModulation::Combine
        };
        Self {
            position,
            uv,
            modulation,
            owner_modulation: modulation,
            outer_modulation,
            owner_outer_modulation: outer_modulation,
            sample_tile: [0.0; 4],
        }
    }

    /// Override inferred provenance. This is required when a native caller
    /// explicitly supplied identity-white or suppressed the enclosing color.
    pub fn with_outer_modulation(mut self, policy: GpuOuterModulation) -> Self {
        self.outer_modulation = policy;
        self
    }

    pub fn with_owner_outer_modulation(mut self, policy: GpuOuterModulation) -> Self {
        self.owner_outer_modulation = policy;
        self
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
    pub outer_modulation: GpuSolidOuterModulation,
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
    SpriteBatch {
        texture: GpuTextureId,
        quads: Vec<GpuSpriteQuad>,
        clip: Option<Rect>,
        blend: GpuBlend,
        mod2: bool,
        gamma: bool,
        outer_modulation: GpuOuterModulation,
    },
    ObjectBatch {
        texture: GpuTextureId,
        /// Optional companion texture selected by
        /// [`GpuObjectSprite::owner_layer`]. Keeping the pair on one command
        /// lets adjacent faces retain base/owner primitive order in one draw.
        owner_texture: Option<GpuTextureId>,
        sprites: Vec<GpuObjectSprite>,
        clip: Option<Rect>,
        blend: GpuBlend,
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
        alpha_mode: GpuSolidAlphaMode,
        clip: Option<Rect>,
        blend: GpuBlend,
        style: GpuSolidStyle,
    },
}

/// Per-command fragment options for a solid primitive.
///
/// Solid draws carry more than one independent fragment decision, and every
/// one of them has to reach the shader as a vertex flag. Keeping them in one
/// value means adding another does not touch every construction site.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuSolidStyle {
    /// Resolve the monitor gamma ramp in the fragment shader.
    pub gamma: bool,
    /// Break up the 8-bit quantization of an interpolated colour with a
    /// sub-LSB noise offset. Only a real gradient asks for this; a flat fill
    /// has no banding to hide.
    pub dither: bool,
}

impl GpuSolidStyle {
    pub const NONE: Self = Self {
        gamma: false,
        dither: false,
    };

    pub const fn with_gamma(gamma: bool) -> Self {
        Self {
            gamma,
            ..Self::NONE
        }
    }

    pub const fn dithered(self, dither: bool) -> Self {
        Self { dither, ..self }
    }
}

/// Why exact packed-C4 modulation could not be applied to a retained command.
///
/// Textured commands and native solid primitives store byte-derived packed-C4
/// channels. Converting an arbitrary packed-color float back to a byte would
/// be an approximation, so that path fails closed. Vertices explicitly tagged
/// as already-filtered texture fragments retain their shader-domain floats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GpuSceneModulationError {
    #[error(
        "textured command {command}, vertex {vertex}, {channel_set} channel {channel} is not an exact normalized byte"
    )]
    AmbiguousTexturedColor {
        command: usize,
        vertex: usize,
        channel_set: &'static str,
        channel: usize,
    },
    #[error(
        "textured command {command}, vertex {vertex}, {channel_set} inherits outer modulation but is not identity-white"
    )]
    NonIdentityInheritedColor {
        command: usize,
        vertex: usize,
        channel_set: &'static str,
    },
    #[error(
        "replacement object command {command} mixes outer-modulation blend classes at sprite {sprite}"
    )]
    MixedReplaceObjectOuterModulation { command: usize, sprite: usize },
    #[error(
        "solid command {command}, vertex {vertex}, color channel {channel} is not an exact normalized byte"
    )]
    AmbiguousSolidColor {
        command: usize,
        vertex: usize,
        channel: usize,
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
            Self::SpriteBatch { quads, clip, .. } => {
                for quad in quads {
                    quad.rect[0] += x;
                    quad.rect[1] += y;
                    quad.rect[2] += x;
                    quad.rect[3] += y;
                }
                translate_clip(clip, x, y);
            }
            Self::ObjectBatch { sprites, clip, .. } => {
                sprites.iter_mut().for_each(|sprite| sprite.translate(x, y));
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
            Self::Quad { clip, .. }
            | Self::SpriteBatch { clip, .. }
            | Self::ObjectBatch { clip, .. }
            | Self::Landscape { clip, .. }
            | Self::Solid { clip, .. } => clip,
        };
        *clip = match *clip {
            Some(current) => current.intersection(bounds),
            None => Some(bounds),
        };
        clip.is_some_and(|clip| clip.width != 0 && clip.height != 0)
    }

    /// Apply one enclosing C++ `ActivateBlitModulation` value exactly.
    ///
    /// `modulation` uses packed `0xTTRRGGBB`. An unmodulated textured draw
    /// inherits it directly. A locally modulated draw combines through C++
    /// `ModulateClr`: RGB uses `(dst * src) >> 8` (including white times white
    /// producing 254), while transparency uses a screen combine. A suppressed
    /// outer state leaves the captured color unchanged. A replacement draw
    /// becomes a normal alpha blend only when the enclosing value applies and
    /// adds transparency (`StdGL.cpp:437-560,846-889`). Semantic text is not
    /// represented by [`GpuCommand`]; use [`modulate_rgba8_by_packed_c4`] for
    /// captured text colors.
    pub fn apply_packed_c4_modulation(
        &mut self,
        modulation: u32,
    ) -> Result<(), GpuSceneModulationError> {
        self.validate_packed_c4_modulation(0)?;
        self.apply_packed_c4_modulation_validated(modulation);
        Ok(())
    }

    fn validate_packed_c4_modulation(&self, command: usize) -> Result<(), GpuSceneModulationError> {
        match self {
            Self::Quad { vertices, .. } => {
                for (vertex_index, vertex) in vertices.iter().enumerate() {
                    validate_textured_channels(
                        vertex.modulation,
                        vertex.outer_modulation,
                        command,
                        vertex_index,
                        "base modulation",
                    )?;
                    validate_textured_channels(
                        vertex.owner_modulation,
                        vertex.owner_outer_modulation,
                        command,
                        vertex_index,
                        "owner modulation",
                    )?;
                }
            }
            Self::SpriteBatch {
                quads,
                outer_modulation,
                ..
            } => {
                for (quad_index, quad) in quads.iter().enumerate() {
                    if *outer_modulation == GpuOuterModulation::Inherit
                        && quad.modulation != 0x00ff_ffff
                    {
                        return Err(GpuSceneModulationError::NonIdentityInheritedColor {
                            command,
                            vertex: quad_index,
                            channel_set: "sprite modulation",
                        });
                    }
                }
            }
            Self::ObjectBatch { sprites, blend, .. } => {
                if *blend == GpuBlend::Replace {
                    if let Some(first) = sprites.first() {
                        let first_outer_applies =
                            first.outer_modulation() != GpuOuterModulation::Ignore;
                        if let Some((sprite, _)) = sprites.iter().enumerate().find(|(_, sprite)| {
                            (sprite.outer_modulation() != GpuOuterModulation::Ignore)
                                != first_outer_applies
                        }) {
                            return Err(
                                GpuSceneModulationError::MixedReplaceObjectOuterModulation {
                                    command,
                                    sprite,
                                },
                            );
                        }
                    }
                }
                for (sprite_index, sprite) in sprites.iter().enumerate() {
                    if sprite.outer_modulation() == GpuOuterModulation::Inherit
                        && sprite
                            .modulation
                            .iter()
                            .any(|&modulation| modulation != 0x00ff_ffff)
                    {
                        return Err(GpuSceneModulationError::NonIdentityInheritedColor {
                            command,
                            vertex: sprite_index,
                            channel_set: "object sprite modulation",
                        });
                    }
                }
            }
            Self::Landscape { vertices, .. } => {
                for (vertex_index, vertex) in vertices.iter().enumerate() {
                    validate_textured_channels(
                        vertex.modulation,
                        vertex.outer_modulation,
                        command,
                        vertex_index,
                        "modulation",
                    )?;
                }
            }
            Self::Solid { vertices, .. } => {
                for (vertex_index, vertex) in vertices.iter().enumerate() {
                    if vertex.outer_modulation != GpuSolidOuterModulation::PackedC4
                        && vertex
                            .color
                            .iter()
                            .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
                    {
                        continue;
                    }
                    for (channel, &value) in vertex.color.iter().enumerate() {
                        if exact_normalized_byte(value).is_none() {
                            return Err(GpuSceneModulationError::AmbiguousSolidColor {
                                command,
                                vertex: vertex_index,
                                channel,
                            });
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn apply_packed_c4_modulation_validated(&mut self, modulation: u32) {
        match self {
            Self::Quad {
                vertices, blend, ..
            } => {
                let outer_applies = vertices
                    .iter()
                    .any(|vertex| vertex.outer_modulation != GpuOuterModulation::Ignore);
                for vertex in vertices {
                    vertex.modulation = apply_outer_modulation(
                        vertex.modulation,
                        vertex.outer_modulation,
                        modulation,
                    );
                    vertex.owner_modulation = apply_outer_modulation(
                        vertex.owner_modulation,
                        vertex.owner_outer_modulation,
                        modulation,
                    );
                }
                promote_transparent_replace_blend(blend, modulation, outer_applies);
            }
            Self::SpriteBatch {
                quads,
                blend,
                outer_modulation,
                ..
            } => {
                let outer_applies = *outer_modulation != GpuOuterModulation::Ignore;
                for quad in quads {
                    quad.modulation = match *outer_modulation {
                        GpuOuterModulation::Inherit => modulation,
                        GpuOuterModulation::Combine => {
                            modulate_packed_c4(quad.modulation, modulation)
                        }
                        GpuOuterModulation::Ignore => quad.modulation,
                    };
                }
                promote_transparent_replace_blend(blend, modulation, outer_applies);
            }
            Self::ObjectBatch { sprites, blend, .. } => {
                let outer_applies = sprites
                    .iter()
                    .any(|sprite| sprite.outer_modulation() != GpuOuterModulation::Ignore);
                for sprite in sprites {
                    let outer_modulation = sprite.outer_modulation();
                    for color in &mut sprite.modulation {
                        *color = match outer_modulation {
                            GpuOuterModulation::Inherit => modulation,
                            GpuOuterModulation::Combine => modulate_packed_c4(*color, modulation),
                            GpuOuterModulation::Ignore => *color,
                        };
                    }
                }
                promote_transparent_replace_blend(blend, modulation, outer_applies);
            }
            Self::Landscape { vertices, .. } => {
                for vertex in vertices {
                    vertex.modulation = apply_outer_modulation(
                        vertex.modulation,
                        vertex.outer_modulation,
                        modulation,
                    );
                }
            }
            Self::Solid {
                vertices, blend, ..
            } => {
                let outer_applies = vertices
                    .iter()
                    .any(|vertex| vertex.outer_modulation != GpuSolidOuterModulation::Ignore);
                for vertex in vertices {
                    vertex.color = match vertex.outer_modulation {
                        GpuSolidOuterModulation::PackedC4 => {
                            let packed = solid_rgba_to_packed_c4(vertex.color)
                                .expect("solid modulation was validated before mutation");
                            packed_c4_to_solid_rgba(modulate_packed_c4(packed, modulation))
                        }
                        GpuSolidOuterModulation::SampledTexture => {
                            modulate_sampled_fragment(vertex.color, modulation)
                        }
                        GpuSolidOuterModulation::Ignore => vertex.color,
                    };
                }
                promote_transparent_replace_blend(blend, modulation, outer_applies);
            }
        }
    }
}

fn modulate_sampled_fragment(mut color: [f32; 4], modulation: u32) -> [f32; 4] {
    let normalized = packed_c4_to_normalized(modulation);
    color[0] = (color[0] * normalized[0]).clamp(0.0, 1.0);
    color[1] = (color[1] * normalized[1]).clamp(0.0, 1.0);
    color[2] = (color[2] * normalized[2]).clamp(0.0, 1.0);
    color[3] = (color[3] - normalized[3]).clamp(0.0, 1.0);
    color
}

fn promote_transparent_replace_blend(blend: &mut GpuBlend, modulation: u32, outer_applies: bool) {
    if outer_applies && modulation >> 24 != 0 && *blend == GpuBlend::Replace {
        *blend = GpuBlend::Normal;
    }
}

fn validate_textured_channels(
    channels: [f32; 4],
    policy: GpuOuterModulation,
    command: usize,
    vertex: usize,
    channel_set: &'static str,
) -> Result<(), GpuSceneModulationError> {
    if policy == GpuOuterModulation::Ignore {
        return Ok(());
    }
    for (channel, value) in channels.into_iter().enumerate() {
        if exact_normalized_byte(value).is_none() {
            return Err(GpuSceneModulationError::AmbiguousTexturedColor {
                command,
                vertex,
                channel_set,
                channel,
            });
        }
    }
    if policy == GpuOuterModulation::Inherit
        && normalized_c4_to_packed(channels) != Some(0x00ff_ffff)
    {
        return Err(GpuSceneModulationError::NonIdentityInheritedColor {
            command,
            vertex,
            channel_set,
        });
    }
    Ok(())
}

fn apply_outer_modulation(
    channels: [f32; 4],
    policy: GpuOuterModulation,
    modulation: u32,
) -> [f32; 4] {
    match policy {
        GpuOuterModulation::Inherit => packed_c4_to_normalized(modulation),
        GpuOuterModulation::Combine => modulate_normalized_c4(channels, modulation),
        GpuOuterModulation::Ignore => channels,
    }
}

fn exact_normalized_byte(value: f32) -> Option<u8> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return None;
    }
    let byte = (value * 255.0).round() as u8;
    (f32::from(byte) / 255.0)
        .to_bits()
        .eq(&value.to_bits())
        .then_some(byte)
}

fn normalized_c4_to_packed(channels: [f32; 4]) -> Option<u32> {
    let [red, green, blue, transparency] = channels.map(exact_normalized_byte);
    Some(
        (u32::from(transparency?) << 24)
            | (u32::from(red?) << 16)
            | (u32::from(green?) << 8)
            | u32::from(blue?),
    )
}

fn packed_c4_to_normalized(packed: u32) -> [f32; 4] {
    [
        ((packed >> 16) & 0xff) as u8,
        ((packed >> 8) & 0xff) as u8,
        (packed & 0xff) as u8,
        (packed >> 24) as u8,
    ]
    .map(|channel| f32::from(channel) / 255.0)
}

fn modulate_normalized_c4(channels: [f32; 4], modulation: u32) -> [f32; 4] {
    let packed = normalized_c4_to_packed(channels)
        .expect("GpuVertex packed-C4 channels were validated before mutation");
    packed_c4_to_normalized(modulate_packed_c4(packed, modulation))
}

fn solid_rgba_to_packed_c4(color: [f32; 4]) -> Option<u32> {
    let [red, green, blue, opacity] = color.map(exact_normalized_byte);
    Some(rgba8_to_packed_c4([red?, green?, blue?, opacity?]))
}

fn packed_c4_to_solid_rgba(packed: u32) -> [f32; 4] {
    packed_c4_to_rgba8(packed).map(|channel| f32::from(channel) / 255.0)
}

fn rgba8_to_packed_c4([red, green, blue, opacity]: [u8; 4]) -> u32 {
    (u32::from(255 - opacity) << 24)
        | (u32::from(red) << 16)
        | (u32::from(green) << 8)
        | u32::from(blue)
}

fn packed_c4_to_rgba8(packed: u32) -> [u8; 4] {
    [
        ((packed >> 16) & 0xff) as u8,
        ((packed >> 8) & 0xff) as u8,
        (packed & 0xff) as u8,
        255 - (packed >> 24) as u8,
    ]
}

fn modulate_packed_c4(destination: u32, source: u32) -> u32 {
    let channel = |value: u32, shift: u32| (value >> shift) & 0xff;
    let multiply = |left: u32, right: u32| (left * right) >> 8;
    let destination_transparency = channel(destination, 24);
    let source_transparency = channel(source, 24);
    let transparency = (destination_transparency + source_transparency
        - multiply(destination_transparency, source_transparency))
    .min(0xff);
    (transparency << 24)
        | (multiply(channel(destination, 16), channel(source, 16)) << 16)
        | (multiply(channel(destination, 8), channel(source, 8)) << 8)
        | multiply(channel(destination, 0), channel(source, 0))
}

/// Apply C++ `ModulateClr` to a straight RGBA byte color.
///
/// This is the exact bridge for semantic captured text: RGB uses `>> 8` and
/// packed transparency uses the native screen combine before conversion back
/// Everything the fragment-shader landscape composer reads, owned so it can
/// travel on a retained scene.
///
/// The CPU composer walks INTEGER landscape coordinates, so one pattern texel
/// per landscape pixel is its ceiling and larger material art only stretches
/// the tiling period. Composing from this plan instead evaluates the same
/// arithmetic per fragment, which is what lets a detail factor resolve finer
/// art while keeping the world-space period.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShaderLandscapePlan {
    /// Landscape-map extent, i.e. `PixelGrid::width()`/`height()`.
    pub extent: [u32; 2],
    /// One landscape byte per map pixel.
    pub index_plane: Vec<u8>,
    /// Interleaved `(lighten, darken)` amounts, two bytes per map pixel.
    /// `None` when material shading is off.
    pub shading_plane: Option<Vec<u8>>,
    /// RGBA pattern atlas; `Surface8` patterns carry their palette index in red.
    pub atlas: Vec<u8>,
    pub atlas_extent: [u32; 2],
    /// One packed texmap slot per entry, laid out exactly as the renderer's
    /// `ShaderLandscapeSlot`: `colors[4]`, `params[4]`, `primary[4]`,
    /// `overlay[4]`. Kept as a flat array so this crate does not need a third
    /// mirror of a layout that already exists on both sides.
    pub slots: Vec<[u32; 16]>,
}

/// to straight opacity.
pub fn modulate_rgba8_by_packed_c4(color: [u8; 4], modulation: u32) -> [u8; 4] {
    packed_c4_to_rgba8(modulate_packed_c4(rgba8_to_packed_c4(color), modulation))
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
    /// Device-snapshot gamma placement for this exact frame. Recorder-only
    /// callers retain the historical fragment behavior by default; the app
    /// replaces it from `AdvancedRendererConfig` before presentation.
    pub gamma_mode: GpuGammaMode,
    pub textures: Vec<GpuTextureResource>,
    pub commands: Vec<GpuCommand>,
}

impl GpuScene {
    pub fn new(
        logical_extent: [u32; 2],
        clear: Color,
        gamma: GpuGammaLut,
        gamma_mode: GpuGammaMode,
        textures: Vec<GpuTextureResource>,
        commands: Vec<GpuCommand>,
    ) -> Self {
        Self {
            logical_extent,
            clear,
            gamma,
            gamma_mode,
            textures,
            commands,
        }
    }
}

/// State that makes two compact object batches one adjacent resource run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ObjectBatchKey {
    texture: GpuTextureId,
    owner_texture: Option<GpuTextureId>,
    clip: Option<Rect>,
    blend: GpuBlend,
    gamma: bool,
    /// Replacement draws must not mix sprites that keep replacement
    /// semantics with sprites whose outer transparency promotes alpha blend.
    replace_outer_applies: Option<bool>,
}

impl ObjectBatchKey {
    fn new(
        texture: GpuTextureId,
        owner_texture: Option<GpuTextureId>,
        clip: Option<Rect>,
        blend: GpuBlend,
        gamma: bool,
        sprite: GpuObjectSprite,
    ) -> Self {
        Self {
            texture,
            owner_texture,
            clip,
            blend,
            gamma,
            replace_outer_applies: (blend == GpuBlend::Replace)
                .then(|| sprite.outer_modulation() != GpuOuterModulation::Ignore),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ObjectRunCapacityHint {
    key: ObjectBatchKey,
    capacity: usize,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct GpuObjectRunCapacityHints(Vec<ObjectRunCapacityHint>);

/// What splits one retained run of solid primitives from the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SolidRunKey {
    topology: GpuPrimitiveTopology,
    alpha_mode: GpuSolidAlphaMode,
    clip: Option<Rect>,
    blend: GpuBlend,
    style: GpuSolidStyle,
}

#[derive(Clone, Copy, Debug)]
struct SolidRunCapacityHint {
    key: SolidRunKey,
    capacity: usize,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct GpuSolidRunCapacityHints(Vec<SolidRunCapacityHint>);

/// Why one retained sprite entered the generic quad/chunk capture path.
///
/// Reasons are non-exclusive: one sprite can increment several counters while
/// the total fallback count still advances exactly once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuSpriteFallbackReasons {
    pub spatial_fog: bool,
    pub precomputed_fog_modulation: bool,
    pub texture_indent: bool,
    pub owner_mask: bool,
    pub physical_texture_tiles: bool,
}

/// Low-overhead structural evidence gathered while a retained scene is built.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuSceneCaptureStats {
    pub generic_sprite_fallbacks: usize,
    pub spatial_fog_fallbacks: usize,
    pub precomputed_fog_modulation_fallbacks: usize,
    pub texture_indent_fallbacks: usize,
    pub owner_mask_fallbacks: usize,
    pub physical_texture_tile_fallbacks: usize,
    /// Generic quad chunks produced by spatial fog expansion.
    pub fog_expanded_chunks: usize,
}

impl GpuSceneCaptureStats {
    pub fn merge(&mut self, other: Self) {
        self.generic_sprite_fallbacks = self
            .generic_sprite_fallbacks
            .saturating_add(other.generic_sprite_fallbacks);
        self.spatial_fog_fallbacks = self
            .spatial_fog_fallbacks
            .saturating_add(other.spatial_fog_fallbacks);
        self.precomputed_fog_modulation_fallbacks = self
            .precomputed_fog_modulation_fallbacks
            .saturating_add(other.precomputed_fog_modulation_fallbacks);
        self.texture_indent_fallbacks = self
            .texture_indent_fallbacks
            .saturating_add(other.texture_indent_fallbacks);
        self.owner_mask_fallbacks = self
            .owner_mask_fallbacks
            .saturating_add(other.owner_mask_fallbacks);
        self.physical_texture_tile_fallbacks = self
            .physical_texture_tile_fallbacks
            .saturating_add(other.physical_texture_tile_fallbacks);
        self.fog_expanded_chunks = self
            .fog_expanded_chunks
            .saturating_add(other.fog_expanded_chunks);
    }
}

/// Mutable command sink carried by recording surfaces and flattened when a
/// CPU scratch surface is presented into its parent.
#[derive(Clone, Debug, Default)]
pub struct GpuSceneRecorder {
    textures: HashMap<GpuTextureId, GpuTextureResource>,
    commands: Vec<GpuCommand>,
    capture_stats: GpuSceneCaptureStats,
    object_run_capacity_hints: GpuObjectRunCapacityHints,
    next_object_run_hint: usize,
    solid_run_capacity_hints: GpuSolidRunCapacityHints,
    next_solid_run_hint: usize,
}

impl GpuSceneRecorder {
    pub(crate) fn with_capacities(
        command_capacity: usize,
        texture_capacity: usize,
        object_run_capacity_hints: GpuObjectRunCapacityHints,
        solid_run_capacity_hints: GpuSolidRunCapacityHints,
    ) -> Self {
        Self {
            textures: HashMap::with_capacity(texture_capacity),
            commands: Vec::with_capacity(command_capacity),
            capture_stats: GpuSceneCaptureStats::default(),
            object_run_capacity_hints,
            next_object_run_hint: 0,
            solid_run_capacity_hints,
            next_solid_run_hint: 0,
        }
    }

    pub(crate) fn command_count(&self) -> usize {
        self.commands.len()
    }

    pub(crate) fn texture_count(&self) -> usize {
        self.textures.len()
    }

    pub const fn capture_stats(&self) -> GpuSceneCaptureStats {
        self.capture_stats
    }

    pub fn record_gpu_sprite_fallback(
        &mut self,
        reasons: GpuSpriteFallbackReasons,
        fog_expanded_chunks: usize,
    ) {
        self.capture_stats.generic_sprite_fallbacks = self
            .capture_stats
            .generic_sprite_fallbacks
            .saturating_add(1);
        self.capture_stats.spatial_fog_fallbacks = self
            .capture_stats
            .spatial_fog_fallbacks
            .saturating_add(usize::from(reasons.spatial_fog));
        self.capture_stats.precomputed_fog_modulation_fallbacks = self
            .capture_stats
            .precomputed_fog_modulation_fallbacks
            .saturating_add(usize::from(reasons.precomputed_fog_modulation));
        self.capture_stats.texture_indent_fallbacks = self
            .capture_stats
            .texture_indent_fallbacks
            .saturating_add(usize::from(reasons.texture_indent));
        self.capture_stats.owner_mask_fallbacks = self
            .capture_stats
            .owner_mask_fallbacks
            .saturating_add(usize::from(reasons.owner_mask));
        self.capture_stats.physical_texture_tile_fallbacks = self
            .capture_stats
            .physical_texture_tile_fallbacks
            .saturating_add(usize::from(reasons.physical_texture_tiles));
        self.capture_stats.fog_expanded_chunks = self
            .capture_stats
            .fog_expanded_chunks
            .saturating_add(fog_expanded_chunks);
    }

    pub(crate) fn retain_object_run_capacities(&mut self) {
        let mut retained = std::mem::take(&mut self.object_run_capacity_hints.0);
        retained.clear();
        for command in &self.commands {
            let GpuCommand::ObjectBatch {
                texture,
                owner_texture,
                sprites,
                clip,
                blend,
                gamma,
            } = command
            else {
                continue;
            };
            let Some(sprite) = sprites.first().copied() else {
                continue;
            };
            retained.push(ObjectRunCapacityHint {
                key: ObjectBatchKey::new(*texture, *owner_texture, *clip, *blend, *gamma, sprite),
                capacity: sprites.capacity().max(sprites.len()).max(1),
            });
        }
        self.object_run_capacity_hints.0 = retained;
    }

    pub(crate) fn take_object_run_capacity_hints(&mut self) -> GpuObjectRunCapacityHints {
        std::mem::take(&mut self.object_run_capacity_hints)
    }

    pub(crate) fn retain_solid_run_capacities(&mut self) {
        let mut retained = std::mem::take(&mut self.solid_run_capacity_hints.0);
        retained.clear();
        for command in &self.commands {
            let GpuCommand::Solid {
                vertices,
                topology,
                alpha_mode,
                clip,
                blend,
                style,
            } = command
            else {
                continue;
            };
            retained.push(SolidRunCapacityHint {
                key: SolidRunKey {
                    topology: *topology,
                    alpha_mode: *alpha_mode,
                    clip: *clip,
                    blend: *blend,
                    style: *style,
                },
                capacity: vertices.capacity().max(vertices.len()).max(1),
            });
        }
        self.solid_run_capacity_hints.0 = retained;
    }

    pub(crate) fn take_solid_run_capacity_hints(&mut self) -> GpuSolidRunCapacityHints {
        std::mem::take(&mut self.solid_run_capacity_hints)
    }

    #[cfg(test)]
    pub(crate) fn first_solid_run_capacity(&self) -> Option<usize> {
        self.commands.iter().find_map(|command| match command {
            GpuCommand::Solid { vertices, .. } => Some(vertices.capacity()),
            _ => None,
        })
    }

    fn next_solid_run_capacity(&mut self, key: SolidRunKey) -> usize {
        let capacity = self
            .solid_run_capacity_hints
            .0
            .get(self.next_solid_run_hint)
            .filter(|hint| hint.key == key)
            .map(|hint| hint.capacity)
            .unwrap_or(1)
            .max(1);
        self.next_solid_run_hint = self.next_solid_run_hint.saturating_add(1);
        capacity
    }

    fn open_solid_run(
        &mut self,
        key: SolidRunKey,
        endpoints: impl IntoIterator<Item = GpuSolidVertex>,
    ) {
        let mut vertices = Vec::with_capacity(self.next_solid_run_capacity(key));
        vertices.extend(endpoints);
        self.push_solid_run(key, vertices);
    }

    /// Open a run around storage the caller already owns.
    ///
    /// A whole command arrives with its own buffer, so take it rather than
    /// copying it into a fresh one, and only grow it to last frame's length.
    fn adopt_solid_run(&mut self, key: SolidRunKey, mut vertices: Vec<GpuSolidVertex>) {
        let capacity = self.next_solid_run_capacity(key);
        vertices.reserve_exact(capacity.saturating_sub(vertices.len()));
        self.push_solid_run(key, vertices);
    }

    fn push_solid_run(&mut self, key: SolidRunKey, vertices: Vec<GpuSolidVertex>) {
        self.commands.push(GpuCommand::Solid {
            vertices,
            topology: key.topology,
            alpha_mode: key.alpha_mode,
            clip: key.clip,
            blend: key.blend,
            style: key.style,
        });
    }

    #[cfg(test)]
    pub(crate) fn first_object_run_capacity(&self) -> Option<usize> {
        self.commands.iter().find_map(|command| match command {
            GpuCommand::ObjectBatch { sprites, .. } => Some(sprites.capacity()),
            _ => None,
        })
    }

    fn next_object_run_capacity(&mut self, key: ObjectBatchKey) -> usize {
        let capacity = self
            .object_run_capacity_hints
            .0
            .get(self.next_object_run_hint)
            .filter(|hint| hint.key == key)
            .map(|hint| hint.capacity)
            .unwrap_or(1)
            .max(1);
        self.next_object_run_hint = self.next_object_run_hint.saturating_add(1);
        capacity
    }

    fn last_object_batch_matches(&self, key: ObjectBatchKey) -> bool {
        self.commands.last().is_some_and(|command| {
            let GpuCommand::ObjectBatch {
                texture,
                owner_texture,
                sprites,
                clip,
                blend,
                gamma,
            } = command
            else {
                return false;
            };
            sprites.first().copied().is_some_and(|sprite| {
                ObjectBatchKey::new(*texture, *owner_texture, *clip, *blend, *gamma, sprite) == key
            })
        })
    }

    fn push_object_batch_run(
        &mut self,
        texture: GpuTextureId,
        owner_texture: Option<GpuTextureId>,
        mut sprites: Vec<GpuObjectSprite>,
        clip: Option<Rect>,
        blend: GpuBlend,
        gamma: bool,
    ) {
        let Some(first) = sprites.first().copied() else {
            return;
        };
        let key = ObjectBatchKey::new(texture, owner_texture, clip, blend, gamma, first);
        debug_assert!(sprites.iter().copied().all(|sprite| {
            ObjectBatchKey::new(texture, owner_texture, clip, blend, gamma, sprite) == key
        }));

        if self.last_object_batch_matches(key) {
            let Some(GpuCommand::ObjectBatch {
                sprites: previous, ..
            }) = self.commands.last_mut()
            else {
                unreachable!("the compatible command was an object batch");
            };
            previous.extend(sprites);
            return;
        }

        let hinted_capacity = self.next_object_run_capacity(key);
        if sprites.capacity() < hinted_capacity {
            sprites.reserve(hinted_capacity.saturating_sub(sprites.len()));
        }
        self.commands.push(GpuCommand::ObjectBatch {
            texture,
            owner_texture,
            sprites,
            clip,
            blend,
            gamma,
        });
    }

    fn push_object_batch(
        &mut self,
        texture: GpuTextureId,
        owner_texture: Option<GpuTextureId>,
        sprites: Vec<GpuObjectSprite>,
        clip: Option<Rect>,
        blend: GpuBlend,
        gamma: bool,
    ) {
        let Some(first) = sprites.first().copied() else {
            return;
        };
        if blend != GpuBlend::Replace {
            self.push_object_batch_run(texture, owner_texture, sprites, clip, blend, gamma);
            return;
        }

        let first_outer_applies = first.outer_modulation() != GpuOuterModulation::Ignore;
        if sprites.iter().all(|sprite| {
            (sprite.outer_modulation() != GpuOuterModulation::Ignore) == first_outer_applies
        }) {
            self.push_object_batch_run(texture, owner_texture, sprites, clip, blend, gamma);
            return;
        }

        let mut run = Vec::new();
        let mut run_outer_applies = first_outer_applies;
        for sprite in sprites {
            let outer_applies = sprite.outer_modulation() != GpuOuterModulation::Ignore;
            if !run.is_empty() && outer_applies != run_outer_applies {
                self.push_object_batch_run(
                    texture,
                    owner_texture,
                    std::mem::take(&mut run),
                    clip,
                    blend,
                    gamma,
                );
                run_outer_applies = outer_applies;
            }
            run.push(sprite);
        }
        self.push_object_batch_run(texture, owner_texture, run, clip, blend, gamma);
    }

    pub fn add_texture(&mut self, resource: GpuTextureResource) {
        match self.textures.entry(resource.id) {
            std::collections::hash_map::Entry::Vacant(entry) => {
                entry.insert(resource);
            }
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                if entry.get().revision < resource.revision
                    || (entry.get().revision == resource.revision
                        && entry.get().dirty.is_empty()
                        && !resource.dirty.is_empty())
                {
                    entry.insert(resource);
                }
            }
        }
    }

    pub fn push(&mut self, command: GpuCommand) {
        if matches!(&command, GpuCommand::SpriteBatch { quads, .. } if quads.is_empty())
            || matches!(&command, GpuCommand::ObjectBatch { sprites, .. } if sprites.is_empty())
        {
            return;
        }
        if let GpuCommand::ObjectBatch {
            texture,
            owner_texture,
            sprites,
            clip,
            blend,
            gamma,
        } = command
        {
            self.push_object_batch(texture, owner_texture, sprites, clip, blend, gamma);
            return;
        }
        if let GpuCommand::Solid {
            vertices,
            topology,
            alpha_mode,
            clip,
            blend,
            style,
        } = command
        {
            if let Some(GpuCommand::Solid {
                vertices: previous,
                topology: previous_topology,
                alpha_mode: previous_alpha_mode,
                clip: previous_clip,
                blend: previous_blend,
                style: previous_style,
            }) = self.commands.last_mut()
            {
                if *previous_topology == topology
                    && *previous_alpha_mode == alpha_mode
                    && *previous_clip == clip
                    && *previous_blend == blend
                    && *previous_style == style
                {
                    previous.extend(vertices);
                    return;
                }
            }
            self.adopt_solid_run(
                SolidRunKey {
                    topology,
                    alpha_mode,
                    clip,
                    blend,
                    style,
                },
                vertices,
            );
            return;
        }
        self.commands.push(command);
    }

    pub fn push_object_sprite(
        &mut self,
        texture: GpuTextureId,
        sprite: GpuObjectSprite,
        clip: Option<Rect>,
        blend: GpuBlend,
        gamma: bool,
    ) {
        self.push_object_sprite_layer(texture, None, sprite, clip, blend, gamma);
    }

    pub fn push_owner_object_sprite(
        &mut self,
        texture: GpuTextureId,
        owner_texture: GpuTextureId,
        sprite: GpuObjectSprite,
        clip: Option<Rect>,
        blend: GpuBlend,
        gamma: bool,
    ) {
        self.push_object_sprite_layer(texture, Some(owner_texture), sprite, clip, blend, gamma);
    }

    fn push_object_sprite_layer(
        &mut self,
        texture: GpuTextureId,
        owner_texture: Option<GpuTextureId>,
        sprite: GpuObjectSprite,
        clip: Option<Rect>,
        blend: GpuBlend,
        gamma: bool,
    ) {
        let key = ObjectBatchKey::new(texture, owner_texture, clip, blend, gamma, sprite);
        if self.last_object_batch_matches(key) {
            let Some(GpuCommand::ObjectBatch { sprites, .. }) = self.commands.last_mut() else {
                unreachable!("the compatible command was an object batch");
            };
            sprites.push(sprite);
            return;
        }
        let mut sprites = Vec::with_capacity(self.next_object_run_capacity(key));
        sprites.push(sprite);
        self.commands.push(GpuCommand::ObjectBatch {
            texture,
            owner_texture,
            sprites,
            clip,
            blend,
            gamma,
        });
    }

    pub fn push_solid_vertex(
        &mut self,
        vertex: GpuSolidVertex,
        topology: GpuPrimitiveTopology,
        alpha_mode: GpuSolidAlphaMode,
        clip: Option<Rect>,
        blend: GpuBlend,
        style: GpuSolidStyle,
    ) {
        if let Some(GpuCommand::Solid {
            vertices,
            topology: previous_topology,
            alpha_mode: previous_alpha_mode,
            clip: previous_clip,
            blend: previous_blend,
            style: previous_style,
        }) = self.commands.last_mut()
        {
            if *previous_topology == topology
                && *previous_alpha_mode == alpha_mode
                && *previous_clip == clip
                && *previous_blend == blend
                && *previous_style == style
            {
                vertices.push(vertex);
                return;
            }
        }
        self.open_solid_run(
            SolidRunKey {
                topology,
                alpha_mode,
                clip,
                blend,
                style,
            },
            [vertex],
        );
    }

    /// Append both endpoints of one line primitive to the open solid run.
    ///
    /// A moving PXS produces a line every frame, so handing the recorder a
    /// fresh two-element `Vec` per particle would allocate once per particle
    /// only to copy it into the run and drop it. The pair is appended as a
    /// unit: half a line list is not a line list.
    #[allow(clippy::too_many_arguments)]
    pub fn push_solid_vertex_pair(
        &mut self,
        start: GpuSolidVertex,
        end: GpuSolidVertex,
        topology: GpuPrimitiveTopology,
        alpha_mode: GpuSolidAlphaMode,
        clip: Option<Rect>,
        blend: GpuBlend,
        style: GpuSolidStyle,
    ) {
        if let Some(GpuCommand::Solid {
            vertices,
            topology: previous_topology,
            alpha_mode: previous_alpha_mode,
            clip: previous_clip,
            blend: previous_blend,
            style: previous_style,
        }) = self.commands.last_mut()
        {
            if *previous_topology == topology
                && *previous_alpha_mode == alpha_mode
                && *previous_clip == clip
                && *previous_blend == blend
                && *previous_style == style
            {
                vertices.extend([start, end]);
                return;
            }
        }
        self.open_solid_run(
            SolidRunKey {
                topology,
                alpha_mode,
                clip,
                blend,
                style,
            },
            [start, end],
        );
    }

    /// Apply one active C++ blit modulation to all retained draws atomically.
    ///
    /// Every float that must be converted back to packed C4 is validated
    /// before any command changes. Suppressed channels require no conversion.
    /// Separate semantic text captures must use
    /// [`modulate_rgba8_by_packed_c4`] before being interleaved with this
    /// command stream.
    pub fn apply_packed_c4_modulation(
        &mut self,
        modulation: u32,
    ) -> Result<(), GpuSceneModulationError> {
        for (command_index, command) in self.commands.iter().enumerate() {
            command.validate_packed_c4_modulation(command_index)?;
        }
        for command in &mut self.commands {
            command.apply_packed_c4_modulation_validated(modulation);
        }
        Ok(())
    }

    pub fn append_translated(
        &mut self,
        mut child: Self,
        offset_x: i32,
        offset_y: i32,
        child_bounds: Rect,
        destination_clip: Option<Rect>,
    ) {
        self.capture_stats.merge(child.capture_stats);
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
            self.push(command);
        }
    }

    pub fn into_scene(self, logical_extent: [u32; 2], clear: Color, gamma: &GammaRamp) -> GpuScene {
        let Self {
            mut textures,
            commands,
            ..
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
                GpuCommand::SpriteBatch { texture, quads, .. } if !quads.is_empty() => {
                    referenced.insert(*texture);
                }
                GpuCommand::SpriteBatch { .. } => {}
                GpuCommand::ObjectBatch {
                    texture,
                    owner_texture,
                    sprites,
                    ..
                } if !sprites.is_empty() => {
                    referenced.insert(*texture);
                    referenced.extend(owner_texture.iter().copied());
                }
                GpuCommand::ObjectBatch { .. } => {}
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
        GpuScene::new(
            logical_extent,
            clear,
            GpuGammaLut::from_ramp(gamma),
            GpuGammaMode::Fragment,
            textures,
            commands,
        )
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appended_child_scene_accumulates_gpu_sprite_fallback_stats() {
        let mut child = GpuSceneRecorder::default();
        child.record_gpu_sprite_fallback(
            GpuSpriteFallbackReasons {
                spatial_fog: true,
                owner_mask: true,
                ..GpuSpriteFallbackReasons::default()
            },
            3,
        );
        let mut parent = GpuSceneRecorder::default();

        parent.append_translated(child, 0, 0, Rect::new(0, 0, 1, 1), None);

        assert_eq!(
            parent.capture_stats(),
            GpuSceneCaptureStats {
                generic_sprite_fallbacks: 1,
                spatial_fog_fallbacks: 1,
                owner_mask_fallbacks: 1,
                fog_expanded_chunks: 3,
                ..GpuSceneCaptureStats::default()
            }
        );
    }

    fn normalized_packed(packed: u32) -> [f32; 4] {
        packed_c4_to_normalized(packed)
    }

    fn object_sprite(outer_modulation: GpuOuterModulation) -> GpuObjectSprite {
        GpuObjectSprite::new(
            [[0.0, 0.0, 1.0]; 4],
            [0.0, 0.0, 1.0, 1.0],
            [0x00ff_ffff; 4],
            GpuSampler::Nearest,
            0.0,
            false,
            outer_modulation,
        )
    }

    fn textured_command_with_policies(
        base: u32,
        owner: u32,
        base_policy: GpuOuterModulation,
        owner_policy: GpuOuterModulation,
    ) -> GpuCommand {
        let mut vertex = GpuVertex::new([0.0, 0.0, 1.0], [0.0, 0.0], normalized_packed(base))
            .with_outer_modulation(base_policy)
            .with_owner_outer_modulation(owner_policy);
        vertex.owner_modulation = normalized_packed(owner);
        GpuCommand::Quad {
            texture: GpuTextureId::fresh(),
            owner_mask: None,
            vertices: [vertex; 4],
            clip: None,
            blend: GpuBlend::Normal,
            base_mod2: false,
            owner_mod2: false,
            sampler: GpuSampler::Nearest,
            gamma: false,
        }
    }

    fn textured_command(base: u32, owner: u32) -> GpuCommand {
        let inferred_policy = |color| {
            if color == 0x00ff_ffff {
                GpuOuterModulation::Inherit
            } else {
                GpuOuterModulation::Combine
            }
        };
        textured_command_with_policies(base, owner, inferred_policy(base), inferred_policy(owner))
    }

    #[test]
    fn translated_projective_vertex_preserves_homogeneous_position() {
        let mut vertex = GpuVertex::new([20.0, 30.0, 2.0], [0.0, 0.0], [1.0, 1.0, 1.0, 0.0]);
        vertex.translate(5.0, 7.0);
        assert_eq!(vertex.position, [30.0, 44.0, 2.0]);
    }

    #[test]
    fn object_sprite_instance_fits_the_compact_capture_budget() {
        assert_eq!(std::mem::size_of::<GpuObjectSprite>(), 88);
    }

    /// The compact record is smaller than the four vertices a generic quad
    /// spends on the same sprite, which is the whole point of routing eligible
    /// non-object draws through it (clonk-org/clonk-rs#271).
    ///
    /// Pinned as a comparison rather than two loose numbers so the saving
    /// cannot quietly invert: a future field added to `GpuObjectSprite`
    /// without one added to `GpuVertex` would fail here rather than in a
    /// benchmark nobody runs.
    #[test]
    fn a_compact_instance_costs_less_than_the_quad_it_replaces() {
        let compact = std::mem::size_of::<GpuObjectSprite>();
        let generic = std::mem::size_of::<GpuVertex>() * 4;
        assert!(
            compact < generic,
            "compact instance is {compact} bytes against {generic} for four vertices",
        );
        assert!(
            compact <= 96,
            "clonk-org/clonk-rs#271 budgets the generalized instance at 96 bytes, got {compact}",
        );
    }

    #[test]
    fn object_sprite_rejects_reserved_packed_flags() {
        let valid = GpuObjectSprite::new(
            [[0.0, 0.0, 1.0]; 4],
            [0.0, 0.0, 1.0, 1.0],
            [0x00ff_ffff; 4],
            GpuSampler::Nearest,
            0.0,
            false,
            GpuOuterModulation::Inherit,
        );
        let reserved_bit = GpuObjectSprite {
            flags: valid.packed_flags() | (1 << 4),
            ..valid
        };
        let reserved_outer_policy = GpuObjectSprite {
            flags: valid.packed_flags() | GpuObjectSprite::OUTER_MODULATION_MASK,
            ..valid
        };

        assert!(valid.has_valid_packed_flags());
        assert!(!reserved_bit.has_valid_packed_flags());
        assert!(!reserved_outer_policy.has_valid_packed_flags());
    }

    #[test]
    fn adjacent_object_sprites_share_one_ordered_resource_run() {
        let texture = GpuTextureId::fresh();
        let sprite = GpuObjectSprite::new(
            [
                [0.0, 0.0, 1.0],
                [1.0, 0.0, 1.0],
                [0.0, 1.0, 1.0],
                [1.0, 1.0, 1.0],
            ],
            [0.0, 0.0, 1.0, 1.0],
            [0x00ff_0000; 4],
            GpuSampler::Nearest,
            0.0,
            false,
            GpuOuterModulation::Combine,
        );
        let mut recorder = GpuSceneRecorder::default();

        recorder.push_object_sprite(texture, sprite, None, GpuBlend::Normal, false);
        recorder.push_object_sprite(
            texture,
            GpuObjectSprite {
                modulation: [0x0000_ff00; 4],
                ..sprite
            },
            None,
            GpuBlend::Normal,
            false,
        );

        let [GpuCommand::ObjectBatch { sprites, .. }] = recorder.commands.as_slice() else {
            panic!("adjacent object sprites did not form one resource run");
        };
        assert_eq!(sprites.len(), 2);
        assert_eq!(sprites[0].modulation, [0x00ff_0000; 4]);
        assert_eq!(sprites[1].modulation, [0x0000_ff00; 4]);
    }

    #[test]
    fn adjacent_owner_pairs_keep_base_owner_order_in_one_resource_run() {
        let texture = GpuTextureId::fresh();
        let owner_texture = GpuTextureId::fresh();
        let base = object_sprite(GpuOuterModulation::Combine);
        let owner = object_sprite(GpuOuterModulation::Combine).with_owner_layer();
        let mut recorder = GpuSceneRecorder::default();

        for _ in 0..2 {
            recorder.push_owner_object_sprite(
                texture,
                owner_texture,
                base,
                None,
                GpuBlend::Normal,
                false,
            );
            recorder.push_owner_object_sprite(
                texture,
                owner_texture,
                owner,
                None,
                GpuBlend::Normal,
                false,
            );
        }

        let [GpuCommand::ObjectBatch {
            texture: actual_base,
            owner_texture: Some(actual_owner),
            sprites,
            ..
        }] = recorder.commands.as_slice()
        else {
            panic!("compatible owner pairs did not retain one ordered resource run");
        };
        assert_eq!((*actual_base, *actual_owner), (texture, owner_texture));
        assert_eq!(
            sprites
                .iter()
                .map(|sprite| sprite.owner_layer())
                .collect::<Vec<_>>(),
            [false, true, false, true]
        );
    }

    #[test]
    fn changed_owner_texture_splits_an_object_resource_pair_run() {
        let texture = GpuTextureId::fresh();
        let owner_textures = [GpuTextureId::fresh(), GpuTextureId::fresh()];
        let sprite = object_sprite(GpuOuterModulation::Combine);
        let mut recorder = GpuSceneRecorder::default();

        for owner_texture in owner_textures {
            recorder.push_owner_object_sprite(
                texture,
                owner_texture,
                sprite,
                None,
                GpuBlend::Normal,
                false,
            );
        }

        assert_eq!(recorder.commands.len(), 2);
        assert_eq!(
            recorder
                .commands
                .iter()
                .map(|command| match command {
                    GpuCommand::ObjectBatch { owner_texture, .. } => *owner_texture,
                    _ => None,
                })
                .collect::<Vec<_>>(),
            owner_textures.map(Some)
        );
    }

    #[test]
    fn object_pair_run_key_preserves_every_required_painter_boundary() {
        let base = GpuTextureId::fresh();
        let owner = GpuTextureId::fresh();
        let clip = Rect::new(1, 2, 30, 40);
        let combined = object_sprite(GpuOuterModulation::Combine);
        let ignored = object_sprite(GpuOuterModulation::Ignore);
        let key = ObjectBatchKey::new(
            base,
            Some(owner),
            Some(clip),
            GpuBlend::Normal,
            false,
            combined,
        );

        assert_eq!(
            key,
            ObjectBatchKey::new(
                base,
                Some(owner),
                Some(clip),
                GpuBlend::Normal,
                false,
                ignored.with_owner_layer(),
            ),
            "ordinary blending keeps per-instance outer and layer policy inside one run"
        );
        for changed in [
            ObjectBatchKey::new(
                GpuTextureId::fresh(),
                Some(owner),
                Some(clip),
                GpuBlend::Normal,
                false,
                combined,
            ),
            ObjectBatchKey::new(
                base,
                Some(GpuTextureId::fresh()),
                Some(clip),
                GpuBlend::Normal,
                false,
                combined,
            ),
            ObjectBatchKey::new(
                base,
                Some(owner),
                Some(Rect::new(2, 2, 30, 40)),
                GpuBlend::Normal,
                false,
                combined,
            ),
            ObjectBatchKey::new(
                base,
                Some(owner),
                Some(clip),
                GpuBlend::Additive,
                false,
                combined,
            ),
            ObjectBatchKey::new(
                base,
                Some(owner),
                Some(clip),
                GpuBlend::Normal,
                true,
                combined,
            ),
        ] {
            assert_ne!(key, changed);
        }
        assert_ne!(
            ObjectBatchKey::new(
                base,
                Some(owner),
                Some(clip),
                GpuBlend::Replace,
                false,
                combined,
            ),
            ObjectBatchKey::new(
                base,
                Some(owner),
                Some(clip),
                GpuBlend::Replace,
                false,
                ignored,
            ),
            "Replace must split layers whose enclosing modulation changes blend semantics"
        );
    }

    #[test]
    fn pushed_object_batches_coalesce_at_the_surface_command_boundary() {
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
        let batch = |sprite| GpuCommand::ObjectBatch {
            texture,
            owner_texture: None,
            sprites: vec![sprite],
            clip: None,
            blend: GpuBlend::Normal,
            gamma: false,
        };
        let mut recorder = GpuSceneRecorder::default();

        recorder.push(batch(sprite));
        recorder.push(batch(sprite));

        let [GpuCommand::ObjectBatch { sprites, .. }] = recorder.commands.as_slice() else {
            panic!("surface-pushed object faces did not coalesce");
        };
        assert_eq!(sprites.len(), 2);
    }

    #[test]
    fn appended_object_batch_coalesces_without_consuming_an_extra_run_hint() {
        let clip = Rect::new(0, 0, 16, 16);
        let shared_texture = GpuTextureId::fresh();
        let following_texture = GpuTextureId::fresh();
        let sprite = object_sprite(GpuOuterModulation::Inherit);
        let mut parent = GpuSceneRecorder::default();
        parent.push_object_sprite(shared_texture, sprite, Some(clip), GpuBlend::Normal, false);
        let mut child = GpuSceneRecorder::default();
        child.push_object_sprite(shared_texture, sprite, None, GpuBlend::Normal, false);

        parent.append_translated(child, 0, 0, clip, None);
        assert_eq!(parent.next_object_run_hint, 1);
        parent.push_object_sprite(
            following_texture,
            sprite,
            Some(clip),
            GpuBlend::Normal,
            false,
        );

        let [GpuCommand::ObjectBatch {
            sprites: shared, ..
        }, GpuCommand::ObjectBatch {
            sprites: following, ..
        }] = parent.commands.as_slice()
        else {
            panic!("appended adjacent object batches did not retain two resource runs");
        };
        assert_eq!(shared.len(), 2);
        assert_eq!(following.len(), 1);
        assert_eq!(parent.next_object_run_hint, 2);
    }

    #[test]
    fn replace_object_batch_splits_outer_modulation_blend_classes() {
        let mixed = GpuCommand::ObjectBatch {
            texture: GpuTextureId::fresh(),
            owner_texture: None,
            sprites: vec![
                object_sprite(GpuOuterModulation::Ignore),
                object_sprite(GpuOuterModulation::Combine),
            ],
            clip: None,
            blend: GpuBlend::Replace,
            gamma: false,
        };
        let mut direct = mixed.clone();
        assert!(matches!(
            direct.apply_packed_c4_modulation(0x80ff_ffff),
            Err(GpuSceneModulationError::MixedReplaceObjectOuterModulation {
                command: 0,
                sprite: 1,
            })
        ));
        assert_eq!(direct, mixed, "failed validation must be atomic");

        let mut recorder = GpuSceneRecorder::default();
        recorder.push(mixed);

        recorder
            .apply_packed_c4_modulation(0x80ff_ffff)
            .expect("object sprite colors are exact packed C4 values");

        let [GpuCommand::ObjectBatch {
            sprites: ignored,
            blend: GpuBlend::Replace,
            ..
        }, GpuCommand::ObjectBatch {
            sprites: combined,
            blend: GpuBlend::Normal,
            ..
        }] = recorder.commands.as_slice()
        else {
            panic!("replace object sprites with different blend semantics stayed mixed");
        };
        assert_eq!(ignored[0].modulation, [0x00ff_ffff; 4]);
        assert_eq!(combined[0].modulation, [0x80fe_fefe; 4]);
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
        let vertex = GpuVertex::new([0.0, 0.0, 1.0], [0.0, 0.0], [1.0, 1.0, 1.0, 0.0]);
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

    #[test]
    fn semantic_text_style_combines_with_cpp_shift_and_transparency_screen() {
        assert_eq!(
            modulate_rgba8_by_packed_c4([255, 128, 64, 127], 0x80ff_80ff),
            [254, 64, 63, 63]
        );
    }

    #[test]
    fn direct_textured_inherit_uses_outer_color_without_white_rounding() {
        let mut quad = textured_command_with_policies(
            0x00ff_ffff,
            0x00ff_ffff,
            GpuOuterModulation::Inherit,
            GpuOuterModulation::Inherit,
        );
        quad.apply_packed_c4_modulation(0x80ff_ffff)
            .expect("identity texture channels inherit the exact outer color");
        let GpuCommand::Quad { vertices, .. } = quad else {
            unreachable!();
        };
        assert_eq!(
            normalized_c4_to_packed(vertices[0].modulation),
            Some(0x80ff_ffff)
        );
        assert_eq!(
            normalized_c4_to_packed(vertices[0].owner_modulation),
            Some(0x80ff_ffff)
        );

        let vertex = GpuVertex::new([0.0, 0.0, 1.0], [0.0, 0.0], normalized_packed(0x00ff_ffff))
            .with_outer_modulation(GpuOuterModulation::Inherit);
        let mut landscape = GpuCommand::Landscape {
            base: GpuTextureId::fresh(),
            liquid_mask: None,
            liquid: None,
            vertices: [vertex; 4],
            clip: None,
            phase: [0.0; 3],
            gamma: false,
        };
        landscape
            .apply_packed_c4_modulation(0x80ff_ffff)
            .expect("unmodulated landscape inherits the exact outer color");
        let GpuCommand::Landscape { vertices, .. } = landscape else {
            unreachable!();
        };
        assert_eq!(
            normalized_c4_to_packed(vertices[0].modulation),
            Some(0x80ff_ffff)
        );
    }

    #[test]
    fn combined_texture_fog_and_owner_use_cpp_modulate_clr() {
        let mut quad = textured_command_with_policies(
            0x0080_4020,
            0x4020_1008,
            GpuOuterModulation::Combine,
            GpuOuterModulation::Combine,
        );
        quad.apply_packed_c4_modulation(0x80ff_80ff)
            .expect("byte-derived quad modulation is exact");
        let GpuCommand::Quad { vertices, .. } = quad else {
            unreachable!();
        };
        assert_eq!(
            normalized_c4_to_packed(vertices[0].modulation),
            Some(0x807f_201f)
        );
        assert_eq!(
            normalized_c4_to_packed(vertices[0].owner_modulation),
            Some(0xa01f_0807)
        );

        let mut explicit_white = textured_command_with_policies(
            0x00ff_ffff,
            0x00ff_ffff,
            GpuOuterModulation::Combine,
            GpuOuterModulation::Combine,
        );
        explicit_white
            .apply_packed_c4_modulation(0x00ff_ffff)
            .expect("explicit identity-white remains a combining local color");
        let GpuCommand::Quad { vertices, .. } = explicit_white else {
            unreachable!();
        };
        assert_eq!(
            normalized_c4_to_packed(vertices[0].modulation),
            Some(0x00fe_fefe)
        );
        assert_eq!(
            normalized_c4_to_packed(vertices[0].owner_modulation),
            Some(0x00fe_fefe)
        );

        let vertex = GpuVertex::new([0.0, 0.0, 1.0], [0.0, 0.0], normalized_packed(0x0080_4020))
            .with_outer_modulation(GpuOuterModulation::Combine);
        let mut landscape = GpuCommand::Landscape {
            base: GpuTextureId::fresh(),
            liquid_mask: None,
            liquid: None,
            vertices: [vertex; 4],
            clip: None,
            phase: [0.0; 3],
            gamma: false,
        };
        landscape
            .apply_packed_c4_modulation(0x80ff_80ff)
            .expect("byte-derived landscape modulation is exact");
        let GpuCommand::Landscape { vertices, .. } = landscape else {
            unreachable!();
        };
        assert_eq!(
            normalized_c4_to_packed(vertices[0].modulation),
            Some(0x807f_201f)
        );
    }

    #[test]
    fn suppressed_owner_color_ignores_outer_modulation() {
        let mut quad = textured_command_with_policies(
            0x00ff_ffff,
            0x4020_1008,
            GpuOuterModulation::Inherit,
            GpuOuterModulation::Ignore,
        );
        quad.apply_packed_c4_modulation(0x80ff_80ff)
            .expect("suppressed owner color requires no packed conversion");
        let GpuCommand::Quad { vertices, .. } = quad else {
            unreachable!();
        };
        assert_eq!(
            normalized_c4_to_packed(vertices[0].modulation),
            Some(0x80ff_80ff)
        );
        assert_eq!(
            normalized_c4_to_packed(vertices[0].owner_modulation),
            Some(0x4020_1008)
        );
    }

    #[test]
    fn solid_color_combines_and_round_trips_exact_rgba_bytes() {
        let color = [200_u8, 100, 50, 128].map(|byte| f32::from(byte) / 255.0);
        let mut command = GpuCommand::Solid {
            vertices: vec![GpuSolidVertex {
                position: [0.0, 0.0, 1.0],
                color,
                outer_modulation: GpuSolidOuterModulation::PackedC4,
            }],
            topology: GpuPrimitiveTopology::PointList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: None,
            blend: GpuBlend::Normal,
            style: GpuSolidStyle::NONE,
        };
        command
            .apply_packed_c4_modulation(0x80ff_ffff)
            .expect("byte-derived solid color is exact");
        let GpuCommand::Solid { vertices, .. } = command else {
            unreachable!();
        };
        assert_eq!(
            vertices[0]
                .color
                .map(|channel| (channel * 255.0).round() as u8),
            [199, 99, 49, 63]
        );
    }

    #[test]
    fn transparent_global_modulation_promotes_replace_draws_to_alpha_blend() {
        let mut quad = textured_command(0x00ff_ffff, 0x00ff_ffff);
        let GpuCommand::Quad { blend, .. } = &mut quad else {
            unreachable!();
        };
        *blend = GpuBlend::Replace;
        quad.apply_packed_c4_modulation(0x80ff_ffff)
            .expect("opaque byte-derived quad");
        assert!(matches!(
            quad,
            GpuCommand::Quad {
                blend: GpuBlend::Normal,
                ..
            }
        ));

        let mut solid = GpuCommand::Solid {
            vertices: vec![GpuSolidVertex {
                position: [0.0, 0.0, 1.0],
                color: [1.0; 4],
                outer_modulation: GpuSolidOuterModulation::PackedC4,
            }],
            topology: GpuPrimitiveTopology::PointList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: None,
            blend: GpuBlend::Replace,
            style: GpuSolidStyle::NONE,
        };
        solid
            .apply_packed_c4_modulation(0x80ff_ffff)
            .expect("opaque byte-derived solid");
        assert!(matches!(
            solid,
            GpuCommand::Solid {
                blend: GpuBlend::Normal,
                ..
            }
        ));

        let mut opaque = textured_command(0x00ff_ffff, 0x00ff_ffff);
        let GpuCommand::Quad { blend, .. } = &mut opaque else {
            unreachable!();
        };
        *blend = GpuBlend::Replace;
        opaque
            .apply_packed_c4_modulation(0x00ff_ffff)
            .expect("opaque modulation remains exact");
        assert!(matches!(
            opaque,
            GpuCommand::Quad {
                blend: GpuBlend::Replace,
                ..
            }
        ));

        let mut ignored = textured_command_with_policies(
            0x00ff_ffff,
            0x00ff_ffff,
            GpuOuterModulation::Ignore,
            GpuOuterModulation::Ignore,
        );
        let GpuCommand::Quad { blend, .. } = &mut ignored else {
            unreachable!();
        };
        *blend = GpuBlend::Replace;
        ignored
            .apply_packed_c4_modulation(0x80ff_ffff)
            .expect("ignored outer modulation leaves the command untouched");
        assert!(matches!(
            ignored,
            GpuCommand::Quad {
                blend: GpuBlend::Replace,
                ..
            }
        ));
    }

    #[test]
    fn inherited_non_identity_color_is_a_typed_provenance_error() {
        let mut command = textured_command_with_policies(
            0x0080_4020,
            0x00ff_ffff,
            GpuOuterModulation::Inherit,
            GpuOuterModulation::Inherit,
        );
        let before = command.clone();
        assert!(matches!(
            command.apply_packed_c4_modulation(0x80ff_ffff),
            Err(GpuSceneModulationError::NonIdentityInheritedColor {
                command: 0,
                vertex: 0,
                channel_set: "base modulation",
            })
        ));
        assert_eq!(command, before);
    }

    #[test]
    fn arbitrary_textured_float_is_a_typed_error_not_a_panic() {
        let mut command = textured_command(0x00ff_ffff, 0x00ff_ffff);
        let GpuCommand::Quad { vertices, .. } = &mut command else {
            unreachable!();
        };
        vertices[2].modulation[1] = 0.5;
        let before = command.clone();
        assert!(matches!(
            command.apply_packed_c4_modulation(0x80ff_ffff),
            Err(GpuSceneModulationError::AmbiguousTexturedColor {
                command: 0,
                vertex: 2,
                channel_set: "base modulation",
                channel: 1,
            })
        ));
        assert_eq!(command, before);
    }

    #[test]
    fn sampled_fragment_accepts_fractional_filter_output_and_applies_shader_fade() {
        let mut command = GpuCommand::Solid {
            vertices: vec![GpuSolidVertex {
                position: [0.5, 0.5, 1.0],
                color: [0.5, 0.25, 1.0, 0.75],
                outer_modulation: GpuSolidOuterModulation::SampledTexture,
            }],
            topology: GpuPrimitiveTopology::PointList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: None,
            blend: GpuBlend::Normal,
            style: GpuSolidStyle::NONE,
        };
        command
            .apply_packed_c4_modulation(0x4080_ff40)
            .expect("filtered fragments remain exactly representable as shader floats");
        let GpuCommand::Solid { vertices, .. } = command else {
            unreachable!();
        };
        let expected = [0.5 * 128.0 / 255.0, 0.25, 64.0 / 255.0, 0.75 - 64.0 / 255.0];
        for (actual, expected) in vertices[0].color.into_iter().zip(expected) {
            assert!((actual - expected).abs() < 1.0e-6);
        }
    }

    #[test]
    fn recorder_modulation_validates_all_commands_before_mutating_any() {
        let mut recorder = GpuSceneRecorder::default();
        recorder.push(textured_command(0x00ff_ffff, 0x00ff_ffff));
        recorder.push(GpuCommand::Solid {
            vertices: vec![GpuSolidVertex {
                position: [0.0, 0.0, 1.0],
                color: [0.5, 1.0, 1.0, 1.0],
                outer_modulation: GpuSolidOuterModulation::PackedC4,
            }],
            topology: GpuPrimitiveTopology::PointList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: None,
            blend: GpuBlend::Normal,
            style: GpuSolidStyle::NONE,
        });
        let before = recorder.commands.clone();
        assert!(matches!(
            recorder.apply_packed_c4_modulation(0x80ff_ffff),
            Err(GpuSceneModulationError::AmbiguousSolidColor {
                command: 1,
                vertex: 0,
                channel: 0,
            })
        ));
        assert_eq!(recorder.commands, before);
    }

    #[test]
    fn a_whole_solid_command_keeps_the_buffer_it_arrived_with() {
        // GUI lines and flattened scratch surfaces hand over a command they
        // already built. Opening its run must adopt that buffer; copying it
        // into a fresh one would allocate for every such command.
        let vertex = GpuSolidVertex {
            position: [0.5, 0.5, 1.0],
            color: [1.0; 4],
            outer_modulation: GpuSolidOuterModulation::Ignore,
        };
        let mut vertices = Vec::with_capacity(8);
        vertices.extend([vertex, vertex]);
        let mut recorder = GpuSceneRecorder::default();

        recorder.push(GpuCommand::Solid {
            vertices,
            topology: GpuPrimitiveTopology::LineList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: None,
            blend: GpuBlend::Normal,
            style: GpuSolidStyle::NONE,
        });

        assert_eq!(recorder.first_solid_run_capacity(), Some(8));
    }

    #[test]
    fn a_retained_solid_run_capacity_presizes_the_next_frame() {
        // A steady rain draws about as many endpoints every frame. Carrying the
        // run length forward means the second frame reserves once instead of
        // doubling its way back up, so allocation stops tracking particle count.
        let endpoint = |x: f32| GpuSolidVertex {
            position: [x, 0.5, 1.0],
            color: [1.0; 4],
            outer_modulation: GpuSolidOuterModulation::Ignore,
        };
        let mut recorder = GpuSceneRecorder::default();
        for index in 0..8 {
            recorder.push_solid_vertex_pair(
                endpoint(index as f32),
                endpoint(index as f32 + 0.5),
                GpuPrimitiveTopology::LineList,
                GpuSolidAlphaMode::SourceOver,
                None,
                GpuBlend::Normal,
                GpuSolidStyle::NONE,
            );
        }
        recorder.retain_solid_run_capacities();
        let hints = recorder.take_solid_run_capacity_hints();

        let mut next = GpuSceneRecorder::with_capacities(0, 0, Default::default(), hints);
        next.push_solid_vertex_pair(
            endpoint(0.0),
            endpoint(0.5),
            GpuPrimitiveTopology::LineList,
            GpuSolidAlphaMode::SourceOver,
            None,
            GpuBlend::Normal,
            GpuSolidStyle::NONE,
        );

        assert_eq!(
            next.first_solid_run_capacity(),
            Some(16),
            "the run did not reopen at last frame's length"
        );
    }

    #[test]
    fn recorder_keeps_line_endpoint_pairs_whole_inside_one_run() {
        // A moving PXS appends its two endpoints to the open run rather than
        // handing over a fresh two-element `Vec` per particle. A pair may never
        // straddle a run boundary: an odd endpoint count is not a line list.
        let endpoint = |x: f32| GpuSolidVertex {
            position: [x, 0.5, 1.0],
            color: [1.0; 4],
            outer_modulation: GpuSolidOuterModulation::Ignore,
        };
        let mut recorder = GpuSceneRecorder::default();
        let push = |recorder: &mut GpuSceneRecorder, first: f32, style| {
            recorder.push_solid_vertex_pair(
                endpoint(first),
                endpoint(first + 1.0),
                GpuPrimitiveTopology::LineList,
                GpuSolidAlphaMode::SourceOver,
                None,
                GpuBlend::Normal,
                style,
            );
        };
        push(&mut recorder, 0.5, GpuSolidStyle::NONE);
        push(&mut recorder, 2.5, GpuSolidStyle::NONE);
        push(&mut recorder, 4.5, GpuSolidStyle::with_gamma(true));

        let [GpuCommand::Solid {
            vertices: run,
            topology,
            ..
        }, GpuCommand::Solid {
            vertices: gamma_run,
            ..
        }] = recorder.commands.as_slice()
        else {
            panic!("endpoint pairs did not group by fragment style");
        };
        assert_eq!(*topology, GpuPrimitiveTopology::LineList);
        assert_eq!(
            run.iter()
                .map(|vertex| vertex.position[0])
                .collect::<Vec<_>>(),
            vec![0.5, 1.5, 2.5, 3.5]
        );
        assert_eq!(gamma_run.len(), 2, "a new run still starts with both ends");
    }

    #[test]
    fn recorder_splits_solid_batches_at_alpha_provenance_boundaries() {
        let vertex = GpuSolidVertex {
            position: [0.5, 0.5, 1.0],
            color: [1.0; 4],
            outer_modulation: GpuSolidOuterModulation::Ignore,
        };
        let mut recorder = GpuSceneRecorder::default();
        recorder.push_solid_vertex(
            vertex,
            GpuPrimitiveTopology::PointList,
            GpuSolidAlphaMode::SourceOver,
            None,
            GpuBlend::Normal,
            GpuSolidStyle::NONE,
        );
        recorder.push_solid_vertex(
            vertex,
            GpuPrimitiveTopology::PointList,
            GpuSolidAlphaMode::SourceOver,
            None,
            GpuBlend::Normal,
            GpuSolidStyle::NONE,
        );
        recorder.push_solid_vertex(
            vertex,
            GpuPrimitiveTopology::PointList,
            GpuSolidAlphaMode::NonSeparate,
            None,
            GpuBlend::Normal,
            GpuSolidStyle::NONE,
        );
        recorder.push(GpuCommand::Solid {
            vertices: vec![vertex],
            topology: GpuPrimitiveTopology::PointList,
            alpha_mode: GpuSolidAlphaMode::NonSeparate,
            clip: None,
            blend: GpuBlend::Normal,
            style: GpuSolidStyle::NONE,
        });

        assert_eq!(recorder.commands.len(), 2);
        let GpuCommand::Solid {
            vertices,
            alpha_mode,
            ..
        } = &recorder.commands[0]
        else {
            unreachable!();
        };
        assert_eq!(vertices.len(), 2);
        assert_eq!(*alpha_mode, GpuSolidAlphaMode::SourceOver);
        let GpuCommand::Solid {
            vertices,
            alpha_mode,
            ..
        } = &recorder.commands[1]
        else {
            unreachable!();
        };
        assert_eq!(vertices.len(), 2);
        assert_eq!(*alpha_mode, GpuSolidAlphaMode::NonSeparate);
    }
}
