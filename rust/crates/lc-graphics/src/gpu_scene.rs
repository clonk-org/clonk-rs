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
        gamma: bool,
    },
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
            alpha_mode,
            clip,
            blend,
            gamma,
        } = command
        {
            if let Some(GpuCommand::Solid {
                vertices: previous,
                topology: previous_topology,
                alpha_mode: previous_alpha_mode,
                clip: previous_clip,
                blend: previous_blend,
                gamma: previous_gamma,
            }) = self.commands.last_mut()
            {
                if *previous_topology == topology
                    && *previous_alpha_mode == alpha_mode
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
                alpha_mode,
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
        alpha_mode: GpuSolidAlphaMode,
        clip: Option<Rect>,
        blend: GpuBlend,
        gamma: bool,
    ) {
        if let Some(GpuCommand::Solid {
            vertices,
            topology: previous_topology,
            alpha_mode: previous_alpha_mode,
            clip: previous_clip,
            blend: previous_blend,
            gamma: previous_gamma,
        }) = self.commands.last_mut()
        {
            if *previous_topology == topology
                && *previous_alpha_mode == alpha_mode
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
            alpha_mode,
            clip,
            blend,
            gamma,
        });
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
            gamma_mode: GpuGammaMode::Fragment,
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

    fn normalized_packed(packed: u32) -> [f32; 4] {
        packed_c4_to_normalized(packed)
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
            gamma: false,
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
            gamma: false,
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
            gamma: false,
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
            gamma: false,
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
            false,
        );
        recorder.push_solid_vertex(
            vertex,
            GpuPrimitiveTopology::PointList,
            GpuSolidAlphaMode::SourceOver,
            None,
            GpuBlend::Normal,
            false,
        );
        recorder.push_solid_vertex(
            vertex,
            GpuPrimitiveTopology::PointList,
            GpuSolidAlphaMode::NonSeparate,
            None,
            GpuBlend::Normal,
            false,
        );
        recorder.push(GpuCommand::Solid {
            vertices: vec![vertex],
            topology: GpuPrimitiveTopology::PointList,
            alpha_mode: GpuSolidAlphaMode::NonSeparate,
            clip: None,
            blend: GpuBlend::Normal,
            gamma: false,
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
