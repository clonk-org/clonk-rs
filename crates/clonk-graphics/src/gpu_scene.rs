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
#[derive(Clone, Debug, PartialEq)]
pub struct GpuGammaLut {
    pub revision: u64,
    pub channels: Arc<[[u16; 256]; 3]>,
}

#[cfg(test)]
thread_local! {
    static GPU_GAMMA_REVISION_HASHES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn gpu_gamma_revision(ramp: &GammaRamp) -> u64 {
    #[cfg(test)]
    GPU_GAMMA_REVISION_HASHES.with(|hashes| hashes.set(hashes.get() + 1));
    ramp.gpu_revision()
}

impl GpuGammaLut {
    pub fn from_ramp(ramp: &GammaRamp) -> Self {
        thread_local! {
            static LOOKUPS: std::cell::RefCell<Vec<GpuGammaLut>> = const { std::cell::RefCell::new(Vec::new()) };
        }
        let channels = ramp.channels();
        LOOKUPS.with(|cached| {
            let mut cached = cached.borrow_mut();
            let lookup = if let Some(index) = cached
                .iter()
                .position(|entry| entry.channels.as_ref() == &channels)
            {
                cached.remove(index)
            } else {
                Self {
                    revision: gpu_gamma_revision(ramp),
                    channels: Arc::new(channels),
                }
            };
            if cached.len() == 8 {
                cached.remove(0);
            }
            cached.push(lookup.clone());
            lookup
        })
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
/// Original Surface sampling and packed-color arithmetic. GPU UVs alone
/// cannot distinguish integer stretching from pixel-center sampling.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuSoftwareBlit {
    pub source: crate::Rect,
    pub mapping: GpuSoftwareBlitMapping,
    pub modulation: crate::Color,
    pub mode: Option<crate::BlitMode>,
    pub translation: [f32; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GpuSoftwareBlitMapping {
    Unscaled(crate::Point),
    Stretched(crate::Rect),
    Transformed {
        origin: crate::Point,
        inverse: crate::Transform,
    },
}

/// Original native sampling coordinates. Normalized GPU UVs and transformed
/// corner positions discard f32 evaluation order and half-open coverage.
#[derive(Clone, Debug, PartialEq)]
pub struct GpuSoftwareSprite {
    pub destination: [f32; 4],
    pub source: [f32; 4],
    pub inverse: crate::Transform,
    pub translation: [f32; 2],
    pub flip_x: bool,
    pub inclusive_source_end: bool,
    pub mapping: GpuSoftwareSpriteMapping,
    pub fog: Option<GpuSoftwareFog>,
    /// The draw-specific ramp used by the immediate renderer, independent of monitor gamma.
    pub gamma: Option<GpuGammaLut>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuSoftwareFog {
    pub destination: [f32; 4],
    pub source_range: [f32; 4],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GpuSoftwareSpriteMapping {
    Native,
    PixelCorner,
    GuiNearest,
    /// Unscaled background tiling copies texels, including alpha, verbatim.
    TileCopy,
    GuiLinear {
        modulation: Option<u32>,
    },
    /// Classic facet compatibility quantizes filtered RGB before source-over.
    GuiFacetLinear,
    Font {
        shear: f32,
        center_y: f32,
        texture_indent: f32,
        physical_size: f32,
        normalize_transparent: bool,
    },
    IntegerStretch,
    RotatedCorner {
        center: [f32; 2],
        cos: f32,
        sin: f32,
    },
    Landscape {
        zoom: f32,
        world_extent: [u32; 2],
        tile_origin: [u32; 2],
        texture_size: u32,
        indent: f32,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GpuSoftwareSpriteId(std::num::NonZeroU32);

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
    /// CPU Surface source preparation uses integer packed-color arithmetic.
    /// Native sprite fragments keep float shader precision instead. GPU
    /// transport ignores this software-oracle provenance.
    pub software_blit: Option<GpuSoftwareBlit>,
    /// The fixed-function software path subtracts modulation transparency
    /// during Mod2; the shader path retains the sampled alpha.
    pub software_shader: bool,
    pub software_sprite: Option<GpuSoftwareSpriteId>,
    pub software_alpha_mode: GpuSolidAlphaMode,
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
    pub software_sprite: Option<GpuSoftwareSpriteId>,
    pub software_shader: bool,
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
    const SOFTWARE_FIXED_FUNCTION: u32 = 1 << 31;
    const SOFTWARE_SPRITE_SHIFT: u32 = 6;
    const SOFTWARE_SPRITE_MASK: u32 = 0x7fff_ffc0;

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
        self.flags & !(Self::SOFTWARE_FIXED_FUNCTION | Self::SOFTWARE_SPRITE_MASK)
    }

    /// Whether the transport word contains only defined flags and policies.
    pub const fn has_valid_packed_flags(self) -> bool {
        self.flags
            & !(Self::DEFINED_FLAGS_MASK
                | Self::SOFTWARE_FIXED_FUNCTION
                | Self::SOFTWARE_SPRITE_MASK)
            == 0
            && self.flags & Self::OUTER_MODULATION_MASK != Self::OUTER_MODULATION_MASK
    }

    pub const fn outer_modulation(self) -> GpuOuterModulation {
        match (self.flags & Self::OUTER_MODULATION_MASK) >> Self::OUTER_MODULATION_SHIFT {
            0 => GpuOuterModulation::Inherit,
            1 => GpuOuterModulation::Combine,
            _ => GpuOuterModulation::Ignore,
        }
    }

    pub const fn software_shader(self) -> bool {
        self.flags & Self::SOFTWARE_FIXED_FUNCTION == 0
    }

    pub fn with_software_shader(mut self, shader: bool) -> Self {
        if shader {
            self.flags &= !Self::SOFTWARE_FIXED_FUNCTION;
        } else {
            self.flags |= Self::SOFTWARE_FIXED_FUNCTION;
        }
        self
    }

    pub fn with_software_sprite(mut self, id: GpuSoftwareSpriteId) -> Self {
        self.flags = (self.flags & !Self::SOFTWARE_SPRITE_MASK)
            | (id.0.get() << Self::SOFTWARE_SPRITE_SHIFT);
        self
    }

    pub fn software_sprite(self) -> Option<GpuSoftwareSpriteId> {
        std::num::NonZeroU32::new(
            (self.flags & Self::SOFTWARE_SPRITE_MASK) >> Self::SOFTWARE_SPRITE_SHIFT,
        )
        .map(GpuSoftwareSpriteId)
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
            software_blit: None,
            software_shader: true,
            software_sprite: None,
            software_alpha_mode: GpuSolidAlphaMode::SourceOver,
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

    pub fn with_software_blit(mut self, blit: GpuSoftwareBlit) -> Self {
        self.software_blit = Some(blit);
        self
    }
    pub fn with_software_alpha_mode(mut self, mode: GpuSolidAlphaMode) -> Self {
        self.software_alpha_mode = mode;
        self
    }

    fn translate(&mut self, x: f32, y: f32) {
        self.position[0] += x * self.position[2];
        self.position[1] += y * self.position[2];
        if let Some(blit) = &mut self.software_blit {
            blit.translation[0] += x;
            blit.translation[1] += y;
        }
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
/// Software arithmetic is independent of GPU blend factors and alpha mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GpuSoftwareBlend {
    #[default]
    Shader,
    Legacy,
    RoundedLegacy,
    /// Classic compatibility boxes round the red gamma LUT before blending.
    GuiBox,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GpuSolidStyle {
    /// Resolve the monitor gamma ramp in the fragment shader.
    pub gamma: bool,
    /// Break up the 8-bit quantization of an interpolated colour with a
    /// sub-LSB noise offset. Only a real gradient asks for this; a flat fill
    /// has no banding to hide.
    pub dither: bool,
    pub software_blend: GpuSoftwareBlend,
    /// Original software raster target for a line captured in a child scene.
    /// GPU scissoring ignores this CPU provenance.
    pub software_line_bounds: Option<Rect>,
}

impl GpuSolidStyle {
    pub const NONE: Self = Self {
        gamma: false,
        dither: false,
        software_blend: GpuSoftwareBlend::Shader,
        software_line_bounds: None,
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

    pub const fn with_software_blend(self, software_blend: GpuSoftwareBlend) -> Self {
        Self {
            software_blend,
            ..self
        }
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

#[derive(Clone, Copy, Debug, Default)]
struct LogicalBoundsAccumulator {
    finite: Option<[f64; 4]>,
    unbounded: bool,
}

impl LogicalBoundsAccumulator {
    fn include_cartesian(&mut self, x: f32, y: f32) {
        if !x.is_finite() || !y.is_finite() {
            self.unbounded = true;
            return;
        }
        let x = f64::from(x);
        let y = f64::from(y);
        if let Some([left, top, right, bottom]) = self.finite {
            self.finite = Some([left.min(x), top.min(y), right.max(x), bottom.max(y)]);
        } else {
            self.finite = Some([x, y, x, y]);
        }
    }

    /// Include one connected primitive. Opposite W signs mean an edge crosses
    /// the projective horizon, so its Cartesian envelope is not finite.
    fn include_homogeneous_group(&mut self, positions: impl Iterator<Item = [f32; 3]>) {
        let mut positive_w = None;
        for [x, y, w] in positions {
            if !x.is_finite() || !y.is_finite() || !w.is_finite() || w == 0.0 {
                self.unbounded = true;
                return;
            }
            let positive = w.is_sign_positive();
            if positive_w.is_some_and(|previous| previous != positive) {
                self.unbounded = true;
                return;
            }
            positive_w = Some(positive);
            self.include_cartesian(x / w, y / w);
        }
    }

    fn clipped_rect(self, clip: Rect, padding: f32) -> Option<Rect> {
        if self.unbounded || !padding.is_finite() || padding < 0.0 {
            return Some(clip);
        }
        let [min_x, min_y, max_x, max_y] = self.finite?;
        let padding = f64::from(padding);
        let clip_left = i64::from(clip.x);
        let clip_top = i64::from(clip.y);
        let clip_right = clip_left + i64::from(clip.width);
        let clip_bottom = clip_top + i64::from(clip.height);
        let left = ((min_x - padding).floor() as i64).max(clip_left);
        let top = ((min_y - padding).floor() as i64).max(clip_top);
        let right = ((max_x + padding).ceil() as i64).min(clip_right);
        let bottom = ((max_y + padding).ceil() as i64).min(clip_bottom);
        if right <= left || bottom <= top {
            return None;
        }
        let Ok(x) = i32::try_from(left) else {
            return Some(clip);
        };
        let Ok(y) = i32::try_from(top) else {
            return Some(clip);
        };
        let Ok(width) = u32::try_from(right - left) else {
            return Some(clip);
        };
        let Ok(height) = u32::try_from(bottom - top) else {
            return Some(clip);
        };
        Some(Rect::new(x, y, width, height))
    }
}

fn command_clip_in_extent(clip: Option<Rect>, [width, height]: [u32; 2]) -> Option<Rect> {
    let clip = clip.unwrap_or_else(|| Rect::new(0, 0, width, height));
    let left = i64::from(clip.x).max(0);
    let top = i64::from(clip.y).max(0);
    let right = (i64::from(clip.x) + i64::from(clip.width)).min(i64::from(width));
    let bottom = (i64::from(clip.y) + i64::from(clip.height)).min(i64::from(height));
    if right <= left || bottom <= top {
        return None;
    }
    Some(Rect::new(
        i32::try_from(left).unwrap_or(i32::MAX),
        i32::try_from(top).unwrap_or(i32::MAX),
        u32::try_from(right - left).unwrap_or(u32::MAX),
        u32::try_from(bottom - top).unwrap_or(u32::MAX),
    ))
}

impl GpuCommand {
    /// C++'s semantic primary clipper for this command.
    pub const fn clip(&self) -> Option<Rect> {
        match self {
            Self::Quad { clip, .. }
            | Self::SpriteBatch { clip, .. }
            | Self::ObjectBatch { clip, .. }
            | Self::Landscape { clip, .. }
            | Self::Solid { clip, .. } => *clip,
        }
    }

    /// Conservative logical pixel coverage at the native one-pixel point and
    /// line width. Projective positions are divided by W; a primitive crossing
    /// W=0 conservatively owns its whole effective primary clip.
    pub fn logical_bounds(&self, logical_extent: [u32; 2]) -> Option<Rect> {
        self.logical_bounds_with_raster_padding(logical_extent, 1.0)
    }

    /// As [`Self::logical_bounds`], with an explicit logical-coordinate halo
    /// for point and line rasters. Callers presenting a scene with a wider
    /// `world_zoom` can supply a physical-to-logical conservative radius;
    /// textured and solid triangle commands ignore this value.
    pub fn logical_bounds_with_raster_padding(
        &self,
        logical_extent: [u32; 2],
        raster_padding: f32,
    ) -> Option<Rect> {
        let clip = command_clip_in_extent(self.clip(), logical_extent)?;
        let mut bounds = LogicalBoundsAccumulator::default();
        let padding = match self {
            Self::Quad { vertices, .. } | Self::Landscape { vertices, .. } => {
                bounds.include_homogeneous_group(vertices.iter().map(|vertex| vertex.position));
                0.0
            }
            Self::SpriteBatch { quads, .. } => {
                for quad in quads {
                    let [left, top, right, bottom] = quad.rect;
                    bounds.include_cartesian(left, top);
                    bounds.include_cartesian(right, bottom);
                }
                0.0
            }
            Self::ObjectBatch { sprites, .. } => {
                for sprite in sprites {
                    bounds.include_homogeneous_group(sprite.positions.into_iter());
                }
                0.0
            }
            Self::Solid {
                vertices, topology, ..
            } => {
                let primitive_size = match topology {
                    GpuPrimitiveTopology::TriangleList => 3,
                    GpuPrimitiveTopology::LineList => 2,
                    GpuPrimitiveTopology::PointList => 1,
                };
                for primitive in vertices.chunks(primitive_size) {
                    bounds
                        .include_homogeneous_group(primitive.iter().map(|vertex| vertex.position));
                }
                match topology {
                    GpuPrimitiveTopology::TriangleList => 0.0,
                    GpuPrimitiveTopology::LineList | GpuPrimitiveTopology::PointList => {
                        raster_padding
                    }
                }
            }
        };
        bounds.clipped_rect(clip, padding)
    }

    /// Conservative logical coverage of one painter-order atom within this
    /// command. Sprite/object batch entries are atoms; solid commands use one
    /// point, line pair, or triangle triple. Quad and landscape commands each
    /// expose atom zero. A missing or fully clipped atom returns `None`.
    pub fn logical_atom_bounds_with_raster_padding(
        &self,
        logical_extent: [u32; 2],
        raster_padding: f32,
        atom: usize,
    ) -> Option<Rect> {
        let clip = command_clip_in_extent(self.clip(), logical_extent)?;
        let mut bounds = LogicalBoundsAccumulator::default();
        let padding = match self {
            Self::Quad { vertices, .. } | Self::Landscape { vertices, .. } => {
                if atom != 0 {
                    return None;
                }
                bounds.include_homogeneous_group(vertices.iter().map(|vertex| vertex.position));
                0.0
            }
            Self::SpriteBatch { quads, .. } => {
                let [left, top, right, bottom] = quads.get(atom)?.rect;
                bounds.include_cartesian(left, top);
                bounds.include_cartesian(right, bottom);
                0.0
            }
            Self::ObjectBatch { sprites, .. } => {
                bounds.include_homogeneous_group(sprites.get(atom)?.positions.into_iter());
                0.0
            }
            Self::Solid {
                vertices, topology, ..
            } => {
                let (primitive_size, padding) = match topology {
                    GpuPrimitiveTopology::TriangleList => (3, 0.0),
                    GpuPrimitiveTopology::LineList => (2, raster_padding),
                    GpuPrimitiveTopology::PointList => (1, raster_padding),
                };
                let start = atom.checked_mul(primitive_size)?;
                let primitive = vertices.get(start..start.checked_add(primitive_size)?)?;
                bounds.include_homogeneous_group(primitive.iter().map(|vertex| vertex.position));
                padding
            }
        };
        bounds.clipped_rect(clip, padding)
    }

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
            Self::Solid {
                vertices,
                clip,
                style,
                ..
            } => {
                vertices
                    .iter_mut()
                    .for_each(|vertex| vertex.translate(x, y));
                translate_clip(clip, x, y);
                translate_clip(&mut style.software_line_bounds, x, y);
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
    pub software_sprites: Vec<GpuSoftwareSprite>,
}

impl GpuScene {
    /// Captured resources are ID-sorted; public scene construction may supply any order.
    pub fn texture(&self, id: GpuTextureId) -> Option<&GpuTextureResource> {
        self.textures
            .binary_search_by_key(&id, |texture| texture.id)
            .ok()
            .map(|index| &self.textures[index])
            .or_else(|| self.textures.iter().find(|texture| texture.id == id))
    }
    pub fn software_sprite(&self, id: GpuSoftwareSpriteId) -> Option<&GpuSoftwareSprite> {
        self.software_sprites.get(id.0.get() as usize - 1)
    }
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
            software_sprites: Vec::new(),
        }
    }
}

// Captures can outlive their producer. Recycle only their empty storage after
// the final owner drops it, releasing every texture snapshot first. The pool
// is local to the producing thread and has a fixed aggregate byte budget.
#[derive(Default)]
struct CaptureStorage {
    bytes: usize,
    commands: Vec<Vec<GpuCommand>>,
    software: Vec<Vec<GpuSoftwareSprite>>,
    textures: Vec<Vec<GpuTextureResource>>,
    maps: Vec<HashMap<GpuTextureId, GpuTextureResource>>,
    references: Vec<HashSet<GpuTextureId>>,
    objects: Vec<Vec<GpuObjectSprite>>,
    sprites: Vec<Vec<GpuSpriteQuad>>,
    solids: Vec<Vec<GpuSolidVertex>>,
}
thread_local! {
    static CAPTURE_STORAGE: std::cell::RefCell<CaptureStorage> = std::cell::RefCell::new(CaptureStorage::default());
}
const CAPTURE_STORAGE_BYTES: usize = 32 * 1024 * 1024;
const CAPTURE_STORAGE_BUFFERS: usize = 1024;

impl CaptureStorage {
    fn make_room(&mut self, size: usize) -> bool {
        if size == 0 || size > CAPTURE_STORAGE_BYTES {
            return false;
        }
        while self.bytes > CAPTURE_STORAGE_BYTES - size {
            // The returning capture is current. Release the largest old
            // empty backing first, including storage from other categories.
            let largest = [
                largest_buffer(&self.commands),
                largest_buffer(&self.software),
                largest_buffer(&self.textures),
                self.maps
                    .iter()
                    .enumerate()
                    .map(|(index, map)| {
                        (
                            index,
                            map.capacity()
                                * std::mem::size_of::<(GpuTextureId, GpuTextureResource)>()
                                * 2,
                        )
                    })
                    .max_by_key(|(_, bytes)| *bytes),
                self.references
                    .iter()
                    .enumerate()
                    .map(|(index, references)| {
                        (
                            index,
                            references.capacity() * std::mem::size_of::<GpuTextureId>() * 2,
                        )
                    })
                    .max_by_key(|(_, bytes)| *bytes),
                largest_buffer(&self.objects),
                largest_buffer(&self.sprites),
                largest_buffer(&self.solids),
            ]
            .into_iter()
            .enumerate()
            .filter_map(|(kind, buffer)| buffer.map(|(index, bytes)| (kind, index, bytes)))
            .max_by_key(|(_, _, bytes)| *bytes);
            let Some((kind, index, bytes)) = largest else {
                return false;
            };
            match kind {
                0 => drop(self.commands.swap_remove(index)),
                1 => drop(self.software.swap_remove(index)),
                2 => drop(self.textures.swap_remove(index)),
                3 => drop(self.maps.swap_remove(index)),
                4 => drop(self.references.swap_remove(index)),
                5 => drop(self.objects.swap_remove(index)),
                6 => drop(self.sprites.swap_remove(index)),
                7 => drop(self.solids.swap_remove(index)),
                _ => unreachable!(),
            }
            self.bytes -= bytes;
        }
        true
    }
}

fn largest_buffer<T>(buffers: &[Vec<T>]) -> Option<(usize, usize)> {
    buffers
        .iter()
        .enumerate()
        .map(|(index, buffer)| (index, buffer.capacity() * std::mem::size_of::<T>()))
        .max_by_key(|(_, bytes)| *bytes)
}

fn take_buffer<T>(
    buffers: &mut Vec<Vec<T>>,
    bytes: &mut usize,
    requested: usize,
    reuse_large: bool,
) -> Vec<T> {
    let limit = requested.saturating_mul(4).max(64);
    let selected = buffers
        .iter()
        .enumerate()
        .filter(|(_, buffer)| {
            buffer.capacity() >= requested
                && (reuse_large || requested == 0 || buffer.capacity() <= limit)
        })
        .min_by_key(|(_, buffer)| buffer.capacity())
        .map(|(index, _)| index);
    selected.map_or_else(
        || Vec::with_capacity(requested),
        |index| {
            let buffer = buffers.swap_remove(index);
            *bytes -= buffer.capacity() * std::mem::size_of::<T>();
            buffer
        },
    )
}
fn return_buffer<T>(
    storage: &mut CaptureStorage,
    select: fn(&mut CaptureStorage) -> &mut Vec<Vec<T>>,
    mut buffer: Vec<T>,
) {
    buffer.clear();
    let size = buffer.capacity().saturating_mul(std::mem::size_of::<T>());
    if select(storage).len() < CAPTURE_STORAGE_BUFFERS && storage.make_room(size) {
        storage.bytes += size;
        select(storage).push(buffer);
    }
}
fn capture_buffer<T>(
    requested: usize,
    select: impl FnOnce(&mut CaptureStorage) -> (&mut Vec<Vec<T>>, &mut usize),
) -> Vec<T> {
    capture_buffer_with_reuse(requested, false, select)
}
fn capture_buffer_with_reuse<T>(
    requested: usize,
    reuse_large: bool,
    select: impl FnOnce(&mut CaptureStorage) -> (&mut Vec<Vec<T>>, &mut usize),
) -> Vec<T> {
    CAPTURE_STORAGE.with(|storage| {
        let mut storage = storage.borrow_mut();
        let (buffers, bytes) = select(&mut storage);
        take_buffer(buffers, bytes, requested, reuse_large)
    })
}
fn return_commands(storage: &mut CaptureStorage, mut commands: Vec<GpuCommand>) {
    for command in commands.drain(..) {
        match command {
            GpuCommand::ObjectBatch { sprites, .. } => {
                return_buffer(storage, |pool| &mut pool.objects, sprites)
            }
            GpuCommand::SpriteBatch { quads, .. } => {
                return_buffer(storage, |pool| &mut pool.sprites, quads)
            }
            GpuCommand::Solid { vertices, .. } => return_solid_buffer(storage, vertices),
            _ => {}
        }
    }
    return_buffer(storage, |pool| &mut pool.commands, commands);
}

fn return_solid_buffer(storage: &mut CaptureStorage, vertices: Vec<GpuSolidVertex>) {
    if storage.solids.len() >= CAPTURE_STORAGE_BUFFERS {
        if let Some((index, capacity)) = storage
            .solids
            .iter()
            .enumerate()
            .map(|(index, buffer)| (index, buffer.capacity()))
            .min_by_key(|(_, capacity)| *capacity)
        {
            let size = vertices
                .capacity()
                .saturating_mul(std::mem::size_of::<GpuSolidVertex>());
            let replaced_size = capacity.saturating_mul(std::mem::size_of::<GpuSolidVertex>());
            if vertices.capacity() > capacity && size <= CAPTURE_STORAGE_BYTES {
                storage.solids.swap_remove(index);
                storage.bytes -= replaced_size;
            }
        }
    }
    return_buffer(storage, |pool| &mut pool.solids, vertices);
}

impl Drop for GpuScene {
    fn drop(&mut self) {
        let _ = CAPTURE_STORAGE.try_with(|storage| {
            if let Ok(mut storage) = storage.try_borrow_mut() {
                let storage = &mut *storage;
                return_commands(storage, std::mem::take(&mut self.commands));
                return_buffer(
                    storage,
                    |pool| &mut pool.software,
                    std::mem::take(&mut self.software_sprites),
                );
                return_buffer(
                    storage,
                    |pool| &mut pool.textures,
                    std::mem::take(&mut self.textures),
                );
            }
        });
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
#[derive(Clone, Debug)]
pub struct GpuSceneRecorder {
    textures: HashMap<GpuTextureId, GpuTextureResource>,
    commands: Vec<GpuCommand>,
    software_sprites: Vec<GpuSoftwareSprite>,
    capture_stats: GpuSceneCaptureStats,
    object_run_capacity_hints: GpuObjectRunCapacityHints,
    next_object_run_hint: usize,
    solid_run_capacity_hints: GpuSolidRunCapacityHints,
    next_solid_run_hint: usize,
}

impl Default for GpuSceneRecorder {
    fn default() -> Self {
        Self::with_capacities(0, 0, Default::default(), Default::default())
    }
}
impl Drop for GpuSceneRecorder {
    fn drop(&mut self) {
        let _ = CAPTURE_STORAGE.try_with(|storage| {
            if let Ok(mut storage) = storage.try_borrow_mut() {
                let storage = &mut *storage;
                return_commands(storage, std::mem::take(&mut self.commands));
                return_buffer(
                    storage,
                    |pool| &mut pool.software,
                    std::mem::take(&mut self.software_sprites),
                );
                let mut textures = std::mem::take(&mut self.textures);
                textures.clear();
                let size = textures
                    .capacity()
                    .saturating_mul(std::mem::size_of::<(GpuTextureId, GpuTextureResource)>() * 2);
                if storage.maps.len() < CAPTURE_STORAGE_BUFFERS && storage.make_room(size) {
                    storage.bytes += size;
                    storage.maps.push(textures);
                }
            }
        });
    }
}

impl GpuSceneRecorder {
    pub fn add_software_sprite(
        &mut self,
        sprite: GpuSoftwareSprite,
    ) -> Option<GpuSoftwareSpriteId> {
        let id = u32::try_from(self.software_sprites.len())
            .ok()?
            .checked_add(1)?;
        if id >= (1 << 25) {
            return None;
        }
        self.software_sprites.push(sprite);
        std::num::NonZeroU32::new(id).map(GpuSoftwareSpriteId)
    }
    pub(crate) fn with_capacities(
        command_capacity: usize,
        texture_capacity: usize,
        object_run_capacity_hints: GpuObjectRunCapacityHints,
        solid_run_capacity_hints: GpuSolidRunCapacityHints,
    ) -> Self {
        Self {
            textures: CAPTURE_STORAGE.with(|storage| {
                let mut storage = storage.borrow_mut();
                let selected = storage
                    .maps
                    .iter()
                    .enumerate()
                    .filter(|(_, map)| map.capacity() >= texture_capacity)
                    .min_by_key(|(_, map)| map.capacity())
                    .map(|(index, _)| index);
                selected.map_or_else(
                    || HashMap::with_capacity(texture_capacity),
                    |index| {
                        let map = storage.maps.swap_remove(index);
                        storage.bytes -= map.capacity()
                            * std::mem::size_of::<(GpuTextureId, GpuTextureResource)>()
                            * 2;
                        map
                    },
                )
            }),
            commands: capture_buffer(command_capacity, |pool| {
                (&mut pool.commands, &mut pool.bytes)
            }),
            software_sprites: capture_buffer(0, |pool| (&mut pool.software, &mut pool.bytes)),
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
                // Reused spare capacity may belong to a much larger earlier
                // run; only the actual work predicts the next reservation.
                capacity: vertices.len().max(1),
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
        let mut vertices =
            capture_buffer_with_reuse(self.next_solid_run_capacity(key), true, |pool| {
                (&mut pool.solids, &mut pool.bytes)
            });
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
        let mut sprites = capture_buffer(self.next_object_run_capacity(key), |pool| {
            (&mut pool.objects, &mut pool.bytes)
        });
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
        let software_offset = self.software_sprites.len() as u32;
        self.software_sprites
            .extend(child.software_sprites.drain(..).map(|mut sprite| {
                sprite.translation[0] += offset_x as f32;
                sprite.translation[1] += offset_y as f32;
                sprite
            }));
        for (_, resource) in child.textures.drain() {
            self.add_texture(resource);
        }
        for mut command in child.commands.drain(..) {
            let remap = |id: &mut Option<GpuSoftwareSpriteId>| {
                if let Some(value) = id {
                    *id = std::num::NonZeroU32::new(value.0.get() + software_offset)
                        .map(GpuSoftwareSpriteId);
                }
            };
            match &mut command {
                GpuCommand::Quad { vertices, .. } | GpuCommand::Landscape { vertices, .. } => {
                    for vertex in vertices {
                        remap(&mut vertex.software_sprite);
                    }
                }
                GpuCommand::SpriteBatch { quads, .. } => {
                    for quad in quads {
                        remap(&mut quad.software_sprite);
                    }
                }
                GpuCommand::ObjectBatch { sprites, .. } => {
                    for sprite in sprites {
                        if let Some(id) = sprite.software_sprite() {
                            if let Some(id) =
                                std::num::NonZeroU32::new(id.0.get() + software_offset)
                                    .map(GpuSoftwareSpriteId)
                            {
                                *sprite = sprite.with_software_sprite(id);
                            }
                        }
                    }
                }
                _ => {}
            }
            if let GpuCommand::Solid {
                topology: GpuPrimitiveTopology::LineList,
                style,
                ..
            } = &mut command
            {
                style.software_line_bounds.get_or_insert(child_bounds);
            }
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

    pub fn into_scene(
        mut self,
        logical_extent: [u32; 2],
        clear: Color,
        gamma: &GammaRamp,
    ) -> GpuScene {
        let commands = std::mem::take(&mut self.commands);
        let software_sprites = std::mem::take(&mut self.software_sprites);
        let textures = &mut self.textures;
        let mut referenced = CAPTURE_STORAGE.with(|storage| {
            let mut storage = storage.borrow_mut();
            storage.references.pop().map_or_else(HashSet::new, |set| {
                storage.bytes -= set.capacity() * std::mem::size_of::<GpuTextureId>() * 2;
                set
            })
        });
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
        let mut texture_list =
            capture_buffer(textures.len(), |pool| (&mut pool.textures, &mut pool.bytes));
        texture_list.extend(textures.drain().map(|(_, resource)| resource));
        referenced.clear();
        CAPTURE_STORAGE.with(|storage| {
            let mut storage = storage.borrow_mut();
            let size = referenced.capacity() * std::mem::size_of::<GpuTextureId>() * 2;
            if storage.references.len() < CAPTURE_STORAGE_BUFFERS && storage.make_room(size) {
                storage.bytes += size;
                storage.references.push(referenced);
            }
        });
        let mut textures = texture_list;
        textures.sort_by_key(|resource| resource.id);
        let mut scene = GpuScene::new(
            logical_extent,
            clear,
            GpuGammaLut::from_ramp(gamma),
            GpuGammaMode::Fragment,
            textures,
            commands,
        );
        scene.software_sprites = software_sprites;
        scene
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AllocationCounter;
    std::thread_local! {
        static ALLOCATIONS: std::cell::Cell<Option<(usize, usize)>> = const { std::cell::Cell::new(None) };
    }
    #[global_allocator]
    static ALLOCATOR: AllocationCounter = AllocationCounter;
    fn record_allocation(bytes: usize) {
        let _ = ALLOCATIONS.try_with(|count| {
            if let Some((calls, allocated)) = count.get() {
                count.set(Some((calls + 1, allocated + bytes)));
            }
        });
    }
    // SAFETY: Pointer and layout arguments pass directly to System. The
    // thread-local counter observes allocation sizes without owning memory.
    unsafe impl std::alloc::GlobalAlloc for AllocationCounter {
        unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
            record_allocation(layout.size());
            unsafe { std::alloc::GlobalAlloc::alloc(&std::alloc::System, layout) }
        }
        unsafe fn dealloc(&self, pointer: *mut u8, layout: std::alloc::Layout) {
            unsafe { std::alloc::GlobalAlloc::dealloc(&std::alloc::System, pointer, layout) }
        }
        unsafe fn realloc(
            &self,
            pointer: *mut u8,
            layout: std::alloc::Layout,
            bytes: usize,
        ) -> *mut u8 {
            record_allocation(bytes);
            unsafe { std::alloc::GlobalAlloc::realloc(&std::alloc::System, pointer, layout, bytes) }
        }
    }
    fn measure_allocations<T>(action: impl FnOnce() -> T) -> (T, (usize, usize)) {
        ALLOCATIONS.with(|count| count.set(Some((0, 0))));
        let result = action();
        let allocations = ALLOCATIONS
            .with(|count| count.take())
            .expect("measurement active");
        (result, allocations)
    }

    #[test]
    fn immediate_surface_fills_do_not_allocate_command_storage() {
        let mut surface = crate::Surface::new(8, 8, crate::PixelFormat::Rgba8888);
        surface.fill(Color::opaque(3, 5, 7));
        let (_, allocations) = measure_allocations(|| {
            surface.fill(Color::opaque(11, 13, 17));
            for _ in 0..32 {
                surface.fill_rect(Rect::new(1, 2, 3, 4), Color::new(19, 23, 29, 73));
            }
        });
        assert_eq!(allocations, (0, 0));
    }

    #[test]
    fn retained_surface_fills_reuse_warmed_solid_storage() {
        let mut surface = crate::Surface::new(8, 8, crate::PixelFormat::Rgba8888);
        let record = |surface: &mut crate::Surface| {
            surface.begin_gpu_scene_capture();
            surface.fill(Color::opaque(11, 13, 17));
            for _ in 0..32 {
                surface.fill_rect(Rect::new(1, 2, 3, 4), Color::new(19, 23, 29, 73));
            }
            drop(
                surface
                    .take_gpu_scene_capture()
                    .expect("recording remains active"),
            );
        };
        for _ in 0..3 {
            record(&mut surface);
        }
        let (_, allocations) = measure_allocations(|| record(&mut surface));
        assert_eq!(allocations, (0, 0));
    }

    #[test]
    fn returned_font_storage_survives_other_capture_categories_filling_the_budget() {
        const POINTS: usize = 24576;
        let vertex = |color| GpuSolidVertex {
            position: [0.5, 0.5, 1.0],
            color,
            outer_modulation: GpuSolidOuterModulation::Ignore,
        };
        let scene = |vertices| {
            GpuScene::new(
                [1, 1],
                Color::transparent(),
                GpuGammaLut::from_ramp(&GammaRamp::identity()),
                GpuGammaMode::Disabled,
                vec![],
                vec![GpuCommand::Solid {
                    vertices,
                    topology: GpuPrimitiveTopology::PointList,
                    alpha_mode: GpuSolidAlphaMode::SourceOver,
                    clip: None,
                    blend: GpuBlend::Normal,
                    style: GpuSolidStyle::NONE,
                }],
            )
        };
        let held = scene(vec![vertex([1.0, 0.0, 0.0, 1.0]); POINTS]);
        let mut storage = CaptureStorage::default();
        let obsolete = Vec::<GpuObjectSprite>::with_capacity(
            (CAPTURE_STORAGE_BYTES - 256) / std::mem::size_of::<GpuObjectSprite>(),
        );
        return_buffer(&mut storage, |pool| &mut pool.objects, obsolete);
        let released = vec![vertex([0.0, 0.0, 1.0, 1.0]); POINTS];
        let pointer = released.as_ptr();
        return_solid_buffer(&mut storage, released);
        assert!(storage.bytes <= CAPTURE_STORAGE_BYTES);
        assert!(storage.solids.len() <= CAPTURE_STORAGE_BUFFERS);
        let (mut current, allocations) = measure_allocations(|| {
            take_buffer(&mut storage.solids, &mut storage.bytes, POINTS, true)
        });
        eprintln!(
            "font after other-category pressure: {} allocation calls, {} bytes",
            allocations.0, allocations.1
        );
        current.resize(POINTS, vertex([0.0, 1.0, 0.0, 1.0]));
        let current_pointer = current.as_ptr();
        let current = scene(current);
        // StdGL.cpp:846-891 preserves submitted primitive colours while
        // retained captures continue owning their original vertex storage.
        let mut pixels = [0; 4];
        let mut renderer = crate::CpuSceneRenderer::default();
        renderer
            .render(&current, &mut pixels)
            .expect("current scene");
        assert_eq!(pixels, [0, 255, 0, 255]);
        renderer.render(&held, &mut pixels).expect("held scene");
        assert_eq!(pixels, [255, 0, 0, 255]);
        assert!(storage.bytes <= CAPTURE_STORAGE_BYTES);
        assert!(storage.solids.len() <= CAPTURE_STORAGE_BUFFERS);
        assert_eq!(allocations, (0, 0));
        assert_eq!(current_pointer, pointer);
    }

    #[test]
    fn current_commands_and_metadata_reuse_storage_after_old_solids_fill_the_budget() {
        let old_storage = || {
            let mut storage = CaptureStorage::default();
            let obsolete = Vec::<GpuSolidVertex>::with_capacity(
                (CAPTURE_STORAGE_BYTES - 256) / std::mem::size_of::<GpuSolidVertex>(),
            );
            return_solid_buffer(&mut storage, obsolete);
            storage
        };
        let pixels = std::sync::Arc::<[u8]>::from([40, 80, 120, 255]);
        let texture =
            GpuTextureResource::immutable_rgba(GpuTextureId::fresh(), 1, 1, pixels.clone());
        let gamma = GpuGammaLut::from_ramp(&GammaRamp::identity());
        let software = GpuSoftwareSprite {
            destination: [0.0, 0.0, 1.0, 1.0],
            source: [0.0, 0.0, 1.0, 1.0],
            inverse: crate::Transform::identity(),
            translation: [0.0; 2],
            flip_x: false,
            inclusive_source_end: false,
            mapping: GpuSoftwareSpriteMapping::Native,
            fog: None,
            gamma: Some(gamma.clone()),
        };
        let mut held = GpuScene::new(
            [1, 1],
            Color::transparent(),
            gamma.clone(),
            GpuGammaMode::Disabled,
            vec![texture.clone()],
            vec![],
        );
        held.software_sprites.push(software.clone());
        let mut commands = Vec::with_capacity(64);
        commands.push(GpuCommand::Solid {
            vertices: vec![],
            topology: GpuPrimitiveTopology::PointList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: None,
            blend: GpuBlend::Normal,
            style: GpuSolidStyle::NONE,
        });
        let command_pointer = commands.as_ptr();
        let mut storage = old_storage();
        return_commands(&mut storage, commands);
        assert!(storage.bytes <= CAPTURE_STORAGE_BYTES);
        let (commands, command_allocations) = measure_allocations(|| {
            take_buffer(&mut storage.commands, &mut storage.bytes, 64, false)
        });
        assert!(commands.is_empty());
        assert_eq!(commands.as_ptr(), command_pointer);

        let mut metadata = Vec::with_capacity(64);
        metadata.push(software);
        let metadata_pointer = metadata.as_ptr();
        let gamma_owners = std::sync::Arc::strong_count(&gamma.channels);
        let mut storage = old_storage();
        return_buffer(&mut storage, |pool| &mut pool.software, metadata);
        assert_eq!(
            std::sync::Arc::strong_count(&gamma.channels),
            gamma_owners - 1
        );
        assert!(storage.bytes <= CAPTURE_STORAGE_BYTES);
        let (metadata, metadata_allocations) = measure_allocations(|| {
            take_buffer(&mut storage.software, &mut storage.bytes, 64, false)
        });
        assert!(metadata.is_empty());
        assert_eq!(metadata.as_ptr(), metadata_pointer);

        let mut resources = Vec::with_capacity(64);
        resources.push(texture);
        let resource_pointer = resources.as_ptr();
        let pixel_owners = std::sync::Arc::strong_count(&pixels);
        let mut storage = old_storage();
        return_buffer(&mut storage, |pool| &mut pool.textures, resources);
        assert_eq!(std::sync::Arc::strong_count(&pixels), pixel_owners - 1);
        assert!(storage.bytes <= CAPTURE_STORAGE_BYTES);
        let (resources, resource_allocations) = measure_allocations(|| {
            take_buffer(&mut storage.textures, &mut storage.bytes, 64, false)
        });
        assert!(resources.is_empty());
        assert_eq!(resources.as_ptr(), resource_pointer);
        assert_eq!(held.textures[0].pixels.as_ref(), [40, 80, 120, 255]);
        assert_eq!(held.software_sprites[0].gamma.as_ref(), Some(&gamma));
        eprintln!(
            "commands {:?}, metadata {:?}, textures {:?} after old-solid pressure",
            command_allocations, metadata_allocations, resource_allocations
        );
        assert_eq!(command_allocations, (0, 0));
        assert_eq!(metadata_allocations, (0, 0));
        assert_eq!(resource_allocations, (0, 0));
    }

    #[test]
    fn solid_capacity_hints_do_not_force_growth_past_available_run_storage() {
        // StdGL.cpp:846-891 preserves primitive painter order. Spare capacity
        // inherited from another capture must not change the next draw's size.
        const POINTS: usize = 8192;
        let vertex = |color| GpuSolidVertex {
            position: [0.5, 0.5, 1.0],
            color,
            outer_modulation: GpuSolidOuterModulation::Ignore,
        };
        let command = |capacity, color| {
            let mut vertices = Vec::with_capacity(capacity);
            vertices.resize(POINTS, vertex(color));
            GpuCommand::Solid {
                vertices,
                topology: GpuPrimitiveTopology::PointList,
                alpha_mode: GpuSolidAlphaMode::SourceOver,
                clip: None,
                blend: GpuBlend::Normal,
                style: GpuSolidStyle::NONE,
            }
        };
        let mut recorder =
            GpuSceneRecorder::with_capacities(1, 0, Default::default(), Default::default());
        recorder.push(command(24576, [1.0, 0.0, 0.0, 1.0]));
        recorder.retain_solid_run_capacities();
        let hints = recorder.take_solid_run_capacity_hints();
        let held = recorder.into_scene([1, 1], Color::transparent(), &GammaRamp::identity());
        let mut released =
            GpuSceneRecorder::with_capacities(1, 0, Default::default(), Default::default());
        released.push(command(16384, [0.0, 0.0, 1.0, 1.0]));
        let released = released.into_scene([1, 1], Color::transparent(), &GammaRamp::identity());
        let released_pointer = match &released.commands[0] {
            GpuCommand::Solid { vertices, .. } => vertices.as_ptr(),
            _ => unreachable!(),
        };
        drop(released);
        let mut next = GpuSceneRecorder::with_capacities(1, 0, Default::default(), hints);
        let (_, allocations) = measure_allocations(|| {
            for _ in 0..POINTS {
                next.push_solid_vertex(
                    vertex([0.0, 1.0, 0.0, 1.0]),
                    GpuPrimitiveTopology::PointList,
                    GpuSolidAlphaMode::SourceOver,
                    None,
                    GpuBlend::Normal,
                    GpuSolidStyle::NONE,
                );
            }
        });
        eprintln!(
            "warmed solid run: {} allocation calls, {} bytes",
            allocations.0, allocations.1
        );
        let scene = next.into_scene([1, 1], Color::transparent(), &GammaRamp::identity());
        let mut actual = [0; 4];
        let mut renderer = crate::CpuSceneRenderer::default();
        renderer
            .render(&scene, &mut actual)
            .expect("valid new scene");
        assert_eq!(actual, [0, 255, 0, 255]);
        renderer
            .render(&held, &mut actual)
            .expect("valid held scene");
        assert_eq!(actual, [255, 0, 0, 255]);
        let GpuCommand::Solid { vertices, .. } = &scene.commands[0] else {
            unreachable!()
        };
        assert_eq!(vertices.len(), POINTS);
        assert_eq!(
            allocations,
            (0, 0),
            "the released buffer already fits every point"
        );
        assert_eq!(vertices.as_ptr(), released_pointer);
    }

    #[test]
    fn returned_font_run_is_reused_after_many_small_solid_runs() {
        // StdGL.cpp:846-891 draws all primitive runs in submission order.
        // A released tooltip run must remain available after many short runs.
        const POINTS: usize = 24576;
        let vertex = |color| GpuSolidVertex {
            position: [0.5, 0.5, 1.0],
            color,
            outer_modulation: GpuSolidOuterModulation::Ignore,
        };
        let command = |count, color| GpuCommand::Solid {
            vertices: vec![vertex(color); count],
            topology: GpuPrimitiveTopology::PointList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: None,
            blend: GpuBlend::Normal,
            style: GpuSolidStyle::NONE,
        };
        let mut held =
            GpuSceneRecorder::with_capacities(1, 0, Default::default(), Default::default());
        held.push(command(POINTS, [1.0, 0.0, 0.0, 1.0]));
        held.retain_solid_run_capacities();
        let hints = held.take_solid_run_capacity_hints();
        let held = held.into_scene([1, 1], Color::transparent(), &GammaRamp::identity());
        let released = GpuScene::new(
            [1, 1],
            Color::transparent(),
            GpuGammaLut::from_ramp(&GammaRamp::identity()),
            GpuGammaMode::Disabled,
            vec![],
            vec![command(POINTS, [0.0, 0.0, 1.0, 1.0])],
        );
        let released_pointer = match &released.commands[0] {
            GpuCommand::Solid { vertices, .. } => vertices.as_ptr(),
            _ => unreachable!(),
        };
        let small_runs = GpuScene::new(
            [1, 1],
            Color::transparent(),
            GpuGammaLut::from_ramp(&GammaRamp::identity()),
            GpuGammaMode::Disabled,
            vec![],
            (0..1024)
                .map(|_| command(8, [0.0, 0.0, 1.0, 1.0]))
                .collect(),
        );
        drop(small_runs);
        drop(released);
        let mut next = GpuSceneRecorder::with_capacities(1, 0, Default::default(), hints);
        let (_, allocations) = measure_allocations(|| {
            for _ in 0..POINTS {
                next.push_solid_vertex(
                    vertex([0.0, 1.0, 0.0, 1.0]),
                    GpuPrimitiveTopology::PointList,
                    GpuSolidAlphaMode::SourceOver,
                    None,
                    GpuBlend::Normal,
                    GpuSolidStyle::NONE,
                );
            }
        });
        eprintln!(
            "warmed font after small runs: {} allocation calls, {} bytes",
            allocations.0, allocations.1
        );
        let scene = next.into_scene([1, 1], Color::transparent(), &GammaRamp::identity());
        let mut actual = [0; 4];
        let mut renderer = crate::CpuSceneRenderer::default();
        renderer
            .render(&scene, &mut actual)
            .expect("valid current frame");
        assert_eq!(actual, [0, 255, 0, 255]);
        renderer
            .render(&held, &mut actual)
            .expect("valid held frame");
        assert_eq!(actual, [255, 0, 0, 255]);
        assert_eq!(
            allocations,
            (0, 0),
            "released font backing must survive small-run traffic"
        );
        let GpuCommand::Solid { vertices, .. } = &scene.commands[0] else {
            unreachable!()
        };
        assert_eq!(vertices.as_ptr(), released_pointer);
        CAPTURE_STORAGE.with(|storage| {
            let storage = storage.borrow();
            assert!(storage.solids.len() <= CAPTURE_STORAGE_BUFFERS);
            assert!(storage.bytes <= CAPTURE_STORAGE_BYTES);
        });
    }

    #[test]
    fn appended_sprite_batch_keeps_its_child_sampling_metadata() {
        let metadata = || GpuSoftwareSprite {
            destination: [0.0, 0.0, 1.0, 1.0],
            source: [0.0, 0.0, 2.0, 1.0],
            inverse: crate::Transform::identity(),
            translation: [0.0; 2],
            flip_x: false,
            inclusive_source_end: false,
            mapping: GpuSoftwareSpriteMapping::PixelCorner,
            fog: None,
            gamma: None,
        };
        let texture = GpuTextureResource::immutable_rgba(
            GpuTextureId::fresh(),
            2,
            1,
            Arc::from([255, 0, 0, 255, 0, 255, 0, 255]),
        );
        let mut parent = GpuSceneRecorder::default();
        let mut unrelated = metadata();
        unrelated.source = [1.0, 0.0, 1.0, 1.0];
        unrelated.mapping = GpuSoftwareSpriteMapping::Native;
        parent
            .add_software_sprite(unrelated)
            .expect("parent metadata");
        let mut child = GpuSceneRecorder::default();
        let id = child
            .add_software_sprite(metadata())
            .expect("child metadata");
        child.add_texture(texture.clone());
        child.push(GpuCommand::SpriteBatch {
            texture: texture.id,
            quads: vec![GpuSpriteQuad {
                rect: [0.0, 0.0, 1.0, 1.0],
                uv: [0.0, 0.0, 1.0, 1.0],
                modulation: 0x00ff_ffff,
                software_sprite: Some(id),
                software_shader: true,
            }],
            clip: None,
            blend: GpuBlend::Normal,
            mod2: false,
            gamma: false,
            outer_modulation: GpuOuterModulation::Ignore,
        });
        parent.append_translated(child, 1, 0, Rect::new(0, 0, 1, 1), None);
        let scene = parent.into_scene([2, 1], Color::transparent(), &GammaRamp::identity());
        let mut actual = [0; 8];
        crate::CpuSceneRenderer::default()
            .render(&scene, &mut actual)
            .expect("valid translated scene");
        assert_eq!(
            actual,
            [0, 0, 0, 0, 255, 0, 0, 255],
            "the child samples the corner texel after its viewport translation"
        );
    }

    #[test]
    fn scene_texture_lookup_accepts_unsorted_public_resources() {
        let first = GpuTextureResource::immutable_rgba(
            GpuTextureId::fresh(),
            1,
            1,
            Arc::from([1, 2, 3, 4]),
        );
        let second = GpuTextureResource::immutable_rgba(
            GpuTextureId::fresh(),
            1,
            1,
            Arc::from([5, 6, 7, 8]),
        );
        let scene = GpuScene::new(
            [1, 1],
            Color::transparent(),
            GpuGammaLut::from_ramp(&GammaRamp::identity()),
            GpuGammaMode::Disabled,
            vec![second.clone(), first.clone()],
            vec![],
        );
        assert_eq!(
            scene
                .texture(first.id)
                .map(|texture| texture.pixels.as_ref()),
            Some(first.pixels.as_ref())
        );
        assert_eq!(
            scene
                .texture(second.id)
                .map(|texture| texture.pixels.as_ref()),
            Some(second.pixels.as_ref())
        );
        assert!(scene.texture(GpuTextureId::fresh()).is_none());
    }

    #[test]
    fn identical_gamma_captures_share_immutable_channel_storage() {
        let ramp = GammaRamp::standard();
        let first = GpuGammaLut::from_ramp(&ramp);
        let second = GpuGammaLut::from_ramp(&ramp);
        assert!(Arc::ptr_eq(&first.channels, &second.channels));
        let different = GpuGammaLut::from_ramp(&GammaRamp::identity());
        assert_ne!(*first.channels, *different.channels);
        assert_eq!(*first.channels, ramp.channels());
    }

    #[test]
    fn equal_gamma_captures_reuse_the_content_revision() {
        // CGammaControl::SetClrChannel fixes all 256 lookup entries until
        // controls change (StdDDraw2.cpp:237-271). Repeated retained draws
        // must reuse the revision as well as the exact immutable table.
        let controls = [0x172b3f, 0x7894a2, 0xdce3f1];
        let first = GpuGammaLut::from_ramp(&GammaRamp::from_control_points(controls));
        let hashes = GPU_GAMMA_REVISION_HASHES.with(std::cell::Cell::get);
        for _ in 0..32 {
            let same = GpuGammaLut::from_ramp(&GammaRamp::from_control_points(controls));
            assert_eq!(same.revision, first.revision);
            assert!(Arc::ptr_eq(&same.channels, &first.channels));
        }
        assert_eq!(GPU_GAMMA_REVISION_HASHES.with(std::cell::Cell::get), hashes);
        let changed = GammaRamp::from_control_points([0x182b3f, 0x7894a2, 0xdce3f1]);
        let changed_lut = GpuGammaLut::from_ramp(&changed);
        assert_eq!(changed_lut.revision, changed.gpu_revision());
        assert_eq!(changed_lut.channels.as_ref(), &changed.channels());
        assert_eq!(
            first.channels.as_ref(),
            &GammaRamp::from_control_points(controls).channels()
        );
    }

    #[test]
    fn alternating_gamma_captures_reuse_immutable_channel_storage() {
        let ramp = GammaRamp::standard();
        let first = GpuGammaLut::from_ramp(&ramp);
        let identity = GpuGammaLut::from_ramp(&GammaRamp::identity());
        let repeated = GpuGammaLut::from_ramp(&ramp);
        assert!(Arc::ptr_eq(&first.channels, &repeated.channels));
        assert_eq!(*identity.channels, GammaRamp::identity().channels());
        assert_eq!(*first.channels, ramp.channels());
    }

    #[test]
    fn completed_scene_returns_empty_capture_storage_without_retaining_textures() {
        let id = GpuTextureId::fresh();
        let pixels: std::sync::Arc<[u8]> = std::sync::Arc::from([1u8, 2, 3, 255]);
        let mut recorder =
            GpuSceneRecorder::with_capacities(37, 19, Default::default(), Default::default());
        recorder.add_texture(GpuTextureResource::immutable_rgba(id, 1, 1, pixels.clone()));
        recorder.push(GpuCommand::Quad {
            texture: id,
            owner_mask: None,
            vertices: std::array::from_fn(|_| {
                GpuVertex::new([0.0, 0.0, 1.0], [0.0, 0.0], [1.0, 1.0, 1.0, 0.0])
            }),
            clip: None,
            blend: GpuBlend::Normal,
            base_mod2: false,
            owner_mod2: false,
            sampler: GpuSampler::Nearest,
            gamma: false,
        });
        let scene = recorder.into_scene([1, 1], Color::new(0, 0, 0, 255), &GammaRamp::standard());
        let capacity = scene.commands.capacity();
        drop(scene);
        assert_eq!(std::sync::Arc::strong_count(&pixels), 1);
        let recorder =
            GpuSceneRecorder::with_capacities(36, 18, Default::default(), Default::default());
        assert_eq!(recorder.commands.capacity(), capacity);
        assert!(recorder.commands.is_empty());
        assert!(recorder.textures.is_empty());
    }

    #[test]
    fn changed_solid_run_hints_reuse_released_storage_and_preserve_held_scenes() {
        let pixels: Arc<[u8]> = Arc::from([17, 31, 71, 128]);
        let texture =
            GpuTextureResource::immutable_rgba(GpuTextureId::fresh(), 1, 1, Arc::clone(&pixels));
        let capture = |hints, clip, color: [f32; 4]| {
            let mut recorder = GpuSceneRecorder::with_capacities(2, 1, Default::default(), hints);
            // StdGL.cpp:846-891 keeps each primitive before the later texture
            // draw. A tooltip's thousands of sampled points form one run.
            for _ in 0..4096 {
                recorder.push_solid_vertex(
                    GpuSolidVertex {
                        position: [0.5, 0.5, 1.0],
                        color,
                        outer_modulation: GpuSolidOuterModulation::Ignore,
                    },
                    GpuPrimitiveTopology::PointList,
                    GpuSolidAlphaMode::SourceOver,
                    clip,
                    GpuBlend::Normal,
                    GpuSolidStyle::NONE,
                );
            }
            recorder.add_texture(texture.clone());
            recorder.push(GpuCommand::Quad {
                texture: texture.id,
                owner_mask: None,
                vertices: std::array::from_fn(|index| {
                    GpuVertex::new(
                        [
                            [0.0, 0.0, 1.0],
                            [1.0, 0.0, 1.0],
                            [0.0, 1.0, 1.0],
                            [1.0, 1.0, 1.0],
                        ][index],
                        [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]][index],
                        [1.0, 1.0, 1.0, 0.0],
                    )
                }),
                clip: None,
                blend: GpuBlend::Normal,
                base_mod2: false,
                owner_mod2: false,
                sampler: GpuSampler::Nearest,
                gamma: false,
            });
            recorder.retain_solid_run_capacities();
            let hints = recorder.take_solid_run_capacity_hints();
            let mut scene =
                recorder.into_scene([1, 1], Color::transparent(), &GammaRamp::identity());
            scene.gamma_mode = GpuGammaMode::Disabled;
            (hints, scene)
        };
        let backing = |scene: &GpuScene| match &scene.commands[0] {
            GpuCommand::Solid { vertices, .. } => vertices.as_ptr(),
            _ => panic!("solid points precede the textured draw"),
        };
        let (hints, first) = capture(Default::default(), None, [1.0, 0.0, 0.0, 1.0]);
        let first_pointer = backing(&first);
        let (hints, held) = capture(hints, Some(Rect::new(0, 0, 1, 1)), [0.0, 0.0, 1.0, 1.0]);
        assert_ne!(
            backing(&held),
            first_pointer,
            "live snapshots cannot share mutable storage"
        );
        drop(first);
        let (mut hints, third) = capture(hints, None, [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(
            backing(&third),
            first_pointer,
            "a shifted hint must reuse a released large run"
        );
        drop(third);
        for index in 0..16 {
            let (next_hints, scene) = capture(
                hints,
                (index % 2 == 0).then_some(Rect::new(0, 0, 1, 1)),
                [0.0, 1.0, 0.0, 1.0],
            );
            hints = next_hints;
            assert_eq!(backing(&scene), first_pointer);
            let mut actual = [0; 4];
            crate::CpuSceneRenderer::default()
                .render(&scene, &mut actual)
                .unwrap();
            let expected = Color::new(17, 31, 71, 128).blend_over(Color::opaque(0, 255, 0));
            assert_eq!(actual, [expected.r, expected.g, expected.b, expected.a]);
        }
        let mut actual = [0; 4];
        crate::CpuSceneRenderer::default()
            .render(&held, &mut actual)
            .unwrap();
        let expected = Color::new(17, 31, 71, 128).blend_over(Color::opaque(0, 0, 255));
        assert_eq!(actual, [expected.r, expected.g, expected.b, expected.a]);
        assert_eq!(
            held.texture(texture.id).unwrap().pixels.as_ref(),
            pixels.as_ref()
        );
        drop(held);
        assert_eq!(
            Arc::strong_count(&pixels),
            2,
            "empty storage cannot pin captured resources"
        );
    }

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
    fn logical_bounds_divide_homogeneous_vertices_and_intersect_primary_clip() {
        let vertex = |x, y| GpuVertex::new([x, y, 2.0], [0.0, 0.0], [1.0; 4]);
        let command = GpuCommand::Quad {
            texture: GpuTextureId::fresh(),
            owner_mask: None,
            vertices: [
                vertex(2.5, 4.5),
                vertex(15.5, 4.5),
                vertex(2.5, 19.5),
                vertex(15.5, 19.5),
            ],
            clip: Some(Rect::new(3, 1, 4, 7)),
            blend: GpuBlend::Normal,
            base_mod2: false,
            owner_mod2: false,
            sampler: GpuSampler::Nearest,
            gamma: false,
        };

        assert_eq!(command.clip(), Some(Rect::new(3, 1, 4, 7)));
        assert_eq!(
            command.logical_bounds([10, 10]),
            Some(Rect::new(3, 2, 4, 6))
        );
    }

    #[test]
    fn logical_bounds_cover_compact_sprite_and_object_batches() {
        let sprite_batch = GpuCommand::SpriteBatch {
            texture: GpuTextureId::fresh(),
            quads: vec![
                GpuSpriteQuad {
                    rect: [6.75, 8.25, 2.25, 3.5],
                    uv: [0.0; 4],
                    modulation: 0x00ff_ffff,
                    software_sprite: None,
                    software_shader: true,
                },
                GpuSpriteQuad {
                    rect: [10.0, 1.0, 12.5, 4.0],
                    uv: [0.0; 4],
                    modulation: 0x00ff_ffff,
                    software_sprite: None,
                    software_shader: true,
                },
            ],
            clip: None,
            blend: GpuBlend::Normal,
            mod2: false,
            gamma: false,
            outer_modulation: GpuOuterModulation::Inherit,
        };
        let object_batch = GpuCommand::ObjectBatch {
            texture: GpuTextureId::fresh(),
            owner_texture: None,
            sprites: vec![GpuObjectSprite::new(
                [
                    [4.0, 6.0, 2.0],
                    [14.0, 6.0, 2.0],
                    [4.0, 18.0, 2.0],
                    [14.0, 18.0, 2.0],
                ],
                [0.0; 4],
                [0x00ff_ffff; 4],
                GpuSampler::Nearest,
                0.0,
                false,
                GpuOuterModulation::Inherit,
            )],
            clip: Some(Rect::new(0, 4, 20, 3)),
            blend: GpuBlend::Normal,
            gamma: false,
        };

        assert_eq!(
            sprite_batch.logical_bounds([20, 20]),
            Some(Rect::new(2, 1, 11, 8))
        );
        assert_eq!(
            object_batch.logical_bounds([20, 20]),
            Some(Rect::new(2, 4, 5, 3))
        );
    }

    #[test]
    fn logical_bounds_pad_line_and_point_rasters() {
        let vertex = |x, y| GpuSolidVertex {
            position: [x, y, 1.0],
            color: [1.0; 4],
            outer_modulation: GpuSolidOuterModulation::Ignore,
        };
        let command = GpuCommand::Solid {
            vertices: vec![vertex(3.5, 4.5), vertex(6.5, 4.5)],
            topology: GpuPrimitiveTopology::LineList,
            alpha_mode: GpuSolidAlphaMode::SourceOver,
            clip: None,
            blend: GpuBlend::Normal,
            style: GpuSolidStyle::NONE,
        };

        assert_eq!(
            command.logical_bounds_with_raster_padding([10, 10], 1.0),
            Some(Rect::new(2, 3, 6, 3))
        );
        assert_eq!(
            command.logical_bounds_with_raster_padding([10, 10], 2.0),
            Some(Rect::new(1, 2, 8, 5))
        );
    }

    #[test]
    fn projective_horizon_falls_back_to_the_effective_primary_clip() {
        let vertex = |x, y, w| GpuVertex::new([x, y, w], [0.0, 0.0], [1.0; 4]);
        let command = GpuCommand::Landscape {
            base: GpuTextureId::fresh(),
            liquid_mask: None,
            liquid: None,
            vertices: [
                vertex(2.0, 2.0, 1.0),
                vertex(8.0, 2.0, 1.0),
                vertex(-2.0, -8.0, -1.0),
                vertex(-8.0, -8.0, -1.0),
            ],
            clip: Some(Rect::new(1, 3, 6, 5)),
            phase: [0.0; 3],
            gamma: false,
        };

        assert_eq!(
            command.logical_bounds([10, 10]),
            Some(Rect::new(1, 3, 6, 5))
        );
    }

    #[test]
    fn empty_batch_has_no_logical_bounds() {
        let command = GpuCommand::ObjectBatch {
            texture: GpuTextureId::fresh(),
            owner_texture: None,
            sprites: Vec::new(),
            clip: None,
            blend: GpuBlend::Normal,
            gamma: false,
        };

        assert_eq!(command.logical_bounds([10, 10]), None);
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
