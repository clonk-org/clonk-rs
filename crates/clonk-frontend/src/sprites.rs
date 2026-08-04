use super::*;

const DEFAULT_PLAYER_COLORS: [Color; 12] = [
    Color::opaque(0xE8, 0x00, 0x00),
    Color::opaque(0x00, 0x00, 0xF4),
    Color::opaque(0x00, 0xC8, 0x00),
    Color::opaque(0x1C, 0xF4, 0xFC),
    Color::opaque(0x44, 0x84, 0xC4),
    Color::opaque(0x30, 0x48, 0x78),
    Color::opaque(0x00, 0x44, 0xA0),
    Color::opaque(0x50, 0x80, 0xF0),
    Color::opaque(0x84, 0x84, 0x84),
    Color::opaque(0xFF, 0xFF, 0xFF),
    Color::opaque(0xF8, 0x94, 0x00),
    Color::opaque(0xC0, 0x00, 0xBC),
];

pub(crate) fn sprite_map_key(definition_id: &str, graphics_name: Option<&str>) -> String {
    match graphics_name {
        Some(name) if !name.is_empty() => {
            format!("{}::{}", definition_id, name.to_ascii_lowercase())
        }
        _ => definition_id.to_string(),
    }
}

pub fn default_owner_color(owner: i32) -> Color {
    if owner <= 0 {
        return Color::opaque(255, 255, 255);
    }
    let idx = ((owner - 1) as usize) % DEFAULT_PLAYER_COLORS.len();
    DEFAULT_PLAYER_COLORS[idx]
}

pub(crate) const CATEGORY_BACKGROUND_FLAG: i32 = 1 << 20;
pub(crate) const CATEGORY_PARALLAX_FLAG: i32 = 1 << 21;
pub(crate) const CATEGORY_FOREGROUND_FLAG: i32 = 1 << 23;
pub(crate) const CATEGORY_IGNORE_FOW_FLAG: i32 = 1 << 25;
pub(crate) const CATEGORY_MOUSE_IGNORE_FLAG: i32 = 1 << 24;

#[derive(Clone, Copy)]
pub(crate) enum ObjectRenderPass {
    Background,
    Normal,
    ForegroundNonParallax,
    ForegroundParallax,
}

/// CStdDDraw state established by C4Object::PrepareDrawing or one explicit
/// C4GraphicsOverlay. Modulation retains C4's packed transparency-alpha color.
#[derive(Clone, Copy)]
pub(crate) struct SpriteBlitState {
    pub(crate) mode: u32,
    pub(crate) modulation: Option<u32>,
    /// Per-fragment value interpolated from the active ClrModMap vertices.
    /// Kept separate from ColorMod so `C4GFXBLIT_CLRSFC_OWNCLR` can suppress
    /// the object modulation without accidentally suppressing fog.
    pub(crate) fog_modulation: Option<FogModulationSample>,
    pub(crate) renderer_config: AdvancedRendererConfig,
}

impl SpriteBlitState {
    pub(crate) const fn normal() -> Self {
        Self {
            mode: 0,
            modulation: None,
            fog_modulation: None,
            renderer_config: AdvancedRendererConfig::DEFAULT,
        }
    }

    pub(crate) fn with_renderer_config(mut self, renderer_config: AdvancedRendererConfig) -> Self {
        self.mode = renderer_config.masked_blit_mode(self.mode);
        self.renderer_config = renderer_config;
        self
    }

    pub(crate) fn for_object(object: &ObjectSnapshot) -> Self {
        let mode = object.blit_mode;
        let modulation = (object.color_modulation != 0
            || mode & (C4GFXBLIT_MOD2 | C4GFXBLIT_CLRSFC_MOD2) != 0)
            .then_some(object.color_modulation);
        Self {
            mode,
            modulation,
            fog_modulation: None,
            renderer_config: AdvancedRendererConfig::DEFAULT,
        }
    }

    pub(crate) fn for_overlay(object: &ObjectSnapshot, overlay: &ObjectGraphicsOverlay) -> Self {
        if overlay.blit_mode == C4GFXBLIT_PARENT {
            return Self::for_object(object);
        }
        Self {
            mode: overlay.blit_mode,
            modulation: (overlay.color_modulation != 0x00ff_ffff)
                .then_some(overlay.color_modulation),
            fog_modulation: None,
            renderer_config: AdvancedRendererConfig::DEFAULT,
        }
    }

    pub(crate) fn with_fog_modulation(mut self, mut fog: FogModulationSample) -> Self {
        if self.renderer_config.no_box_fades {
            fog = fog.with_flat_provoking_vertex();
        }
        self.fog_modulation = Some(fog);
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DefinitionSprite {
    pub image: ImageData,
    pub actions: HashMap<String, DefinitionActionGraphics>,
    pub color_mask: Option<ColorByOwnerMask>,
    /// DefCore graphics `Scale` of this bitmap's owning definition. Object
    /// geometry remains in definition coordinates; C4Object scales only the
    /// source crop selected from `GetGraphics()` (C4Object.cpp:438-467,
    /// 2639-2670).
    pub graphics_scale: f32,
    /// The def Shape rect (DefCore Offset + Width/Height): idle objects
    /// draw Shape.Wdt x Shape.Hgt from the graphics origin
    /// (C4Object::DrawFace, C4Object.cpp:438-460) — never the whole
    /// sprite sheet — and the face is anchored at the shape top-left,
    /// x + Shape.x / y + Shape.y (C4Object::Draw, C4Object.cpp:2231).
    pub shape: Option<DefinitionRect>,
    /// C4Shape::FireTop from DefCore. Construction scaling changes this with
    /// the live shape before the fire facet is drawn (C4Shape.cpp:103-127).
    pub fire_top: i32,
    /// DefCore Rotateable. Required to reconstruct definition-derived live
    /// shape bounds when a sparse/legacy snapshot has no shape sidecar.
    pub rotateable: i32,
    /// DefCore Line. Line objects return through DrawLine before fire/faces.
    pub line: i32,
    /// DefCore StretchGrowth → C4Def::GrowthType (src/C4Def.cpp:387):
    /// Con scales the shape on both axes (C4Shape::Stretch) instead of
    /// height only (C4Shape::Jolt), C4Object.cpp:329-333.
    pub stretch_growth: bool,
    /// DefCore `TopFace`: source facet and object-relative target rectangle.
    pub top_face: Option<DefinitionTargetRect>,
    /// DefCore `Picture` rect in RAW definition units, exactly as
    /// `C4GraphicsOverlay::UpdateFacet` stores it for MODE_IngamePicture and
    /// MODE_Picture (src/C4DefGraphics.cpp:660-664). It must stay unscaled:
    /// `C4Facet::DrawT` applies the definition `Scale` to the SOURCE crop only
    /// (src/C4Facet.cpp:74-79), while the rect doubles as the fZoomToShape
    /// denominator and the destination extent. A missing or zero-size DefCore
    /// entry already arrives as `(0, 0, Shape.Wdt, Shape.Hgt)`
    /// (src/C4Def.cpp:222-224).
    pub picture: Option<DefinitionRect>,
}

/// Source facet selected from one particle definition's `Graphics.png`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleFacet {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl ParticleFacet {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub(crate) fn phase(self, x: i32, y: i32) -> SourceRect {
        SourceRect::new(
            self.x.saturating_add(self.width.saturating_mul(x)),
            self.y.saturating_add(self.height.saturating_mul(y)),
            self.width,
            self.height,
        )
    }
}

/// Immutable render half of a loaded native particle definition.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticleRenderDefinition {
    pub image: ImageData,
    pub facet: ParticleFacet,
    pub length: i32,
    /// Native `C4ParticleDef::Aspect`: facet width divided by facet height.
    pub aspect: f32,
    pub core: ParticleDefCore,
    pub draw_proc: ParticleDrawProc,
}

pub type ParticleSprite = ParticleRenderDefinition;

/// Definition geometry read by `C4Object`'s developer overlays. Keeping this
/// beside, rather than inside, bitmap sprites preserves the distinction
/// between live DefCore geometry and `SetGraphics` image selection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DefinitionDebugGeometry {
    pub name: Option<String>,
    pub entrance: Option<DefinitionRect>,
    pub collection: Option<DefinitionRect>,
    pub solid_mask: Option<DefinitionTargetRect>,
}

/// Process-local `C4GraphicsSystem::Show*` flags. The debug-draw renderers own
/// the consumers; `clonk-app`'s debug-mode key callbacks own the mutations.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DebugDrawFlags {
    pub show_vertices: bool,
    pub show_entrance: bool,
    pub show_action: bool,
    pub show_command: bool,
    pub show_pathfinder: bool,
    pub show_solid_mask: bool,
    pub show_net_status: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudGraphics {
    pub player: Option<ImageData>,
    pub flag: Option<ImageData>,
    pub crew: Option<ImageData>,
    pub score: Option<ImageData>,
    pub wealth: Option<ImageData>,
    pub rank: Option<ImageData>,
    pub captain: Option<ImageData>,
    pub fire: Option<ImageData>,
    pub menu: Option<ImageData>,
    pub upper_board: Option<ImageData>,
    pub logo: Option<ImageData>,
    pub construction: Option<ImageData>,
    pub energy: Option<ImageData>,
    pub magic: Option<ImageData>,
    pub arrow: Option<ImageData>,
    pub exit: Option<ImageData>,
    pub hand: Option<ImageData>,
    pub build: Option<ImageData>,
    pub energy_bars: Option<ImageData>,
    pub select_mark: Option<ImageData>,
    /// Control.png — `sfcControl` with the `fctKeyboard` cell at (0,0,80,36)
    /// (src/C4GraphicsResource.cpp:200-205).
    pub control: Option<ImageData>,
    /// Gamepad.png — four 80px-wide `fctGamepad` phases loaded at their full
    /// image height (src/C4GraphicsResource.cpp:229).
    pub gamepad: Option<ImageData>,
    /// Background.png — `fctBackground`, the message board backdrop tile
    /// (src/C4GraphicsResource.cpp:209, src/C4MessageBoard.cpp:258).
    pub background: Option<ImageData>,
}

#[derive(Clone, Debug)]
pub struct ColorByOwnerMask {
    pub(crate) width: u32,
    pub(crate) height: u32,
    pub(crate) pixels: Arc<[u8]>,
    gpu_base_texture_id: GpuTextureId,
    gpu_overlay_texture_id: GpuTextureId,
    gpu_scalar_layers: Arc<OnceLock<CachedScalarOwnerLayers>>,
}

#[derive(Clone, Debug)]
struct CachedScalarOwnerLayers {
    source_texture_id: GpuTextureId,
    base: Arc<[u8]>,
    overlay: Arc<[u8]>,
}

impl PartialEq for ColorByOwnerMask {
    fn eq(&self, other: &Self) -> bool {
        self.width == other.width && self.height == other.height && self.pixels == other.pixels
    }
}

impl ColorByOwnerMask {
    pub fn new(width: u32, height: u32, pixels: Arc<[u8]>) -> Self {
        Self {
            width,
            height,
            pixels,
            gpu_base_texture_id: GpuTextureId::fresh(),
            gpu_overlay_texture_id: GpuTextureId::fresh(),
            gpu_scalar_layers: Arc::new(OnceLock::new()),
        }
    }

    pub(crate) fn gpu_layer_resources(
        &self,
        source: &ImageData,
    ) -> Option<(GpuTextureResource, GpuTextureResource)> {
        if (self.width, self.height) != (source.width(), source.height()) {
            return None;
        }
        let pixel_count = usize::try_from(u64::from(self.width) * u64::from(self.height)).ok()?;
        let rgba_len = pixel_count.checked_mul(4)?;
        if source.pixels().len() != rgba_len {
            return None;
        }

        if self.pixels.len() == rgba_len {
            return Some((
                source.gpu_texture_resource(),
                GpuTextureResource::immutable_rgba(
                    self.gpu_overlay_texture_id,
                    self.width,
                    self.height,
                    Arc::clone(&self.pixels),
                ),
            ));
        }
        if self.pixels.len() != pixel_count {
            return None;
        }

        let layers = self.gpu_scalar_layers.get_or_init(|| {
            let mut base = source.pixels().to_vec();
            let mut overlay = vec![0; rgba_len];
            for (index, &mask) in self.pixels.iter().enumerate() {
                if mask == 0 {
                    continue;
                }
                let offset = index * 4;
                let alpha = source.pixels()[offset + 3];
                base[offset..offset + 4].fill(0);
                overlay[offset..offset + 4].copy_from_slice(&[mask, mask, mask, alpha]);
            }
            CachedScalarOwnerLayers {
                source_texture_id: source.gpu_texture_id(),
                base: Arc::from(base.into_boxed_slice()),
                overlay: Arc::from(overlay.into_boxed_slice()),
            }
        });
        if layers.source_texture_id != source.gpu_texture_id() {
            return None;
        }
        Some((
            GpuTextureResource::immutable_rgba(
                self.gpu_base_texture_id,
                self.width,
                self.height,
                Arc::clone(&layers.base),
            ),
            GpuTextureResource::immutable_rgba(
                self.gpu_overlay_texture_id,
                self.width,
                self.height,
                Arc::clone(&layers.overlay),
            ),
        ))
    }

    pub(crate) fn value_at(&self, x: u32, y: u32) -> ColorByOwnerSample {
        if x >= self.width || y >= self.height {
            return ColorByOwnerSample::Scalar(0);
        }
        let idx = (y * self.width + x) as usize;
        let pixel_count = u64::from(self.width) * u64::from(self.height);
        if pixel_count.checked_mul(4) == Some(self.pixels.len() as u64) {
            let idx = idx * 4;
            return self
                .pixels
                .get(idx..idx + 4)
                .map(|pixel| {
                    ColorByOwnerSample::Overlay(Color::new(pixel[0], pixel[1], pixel[2], pixel[3]))
                })
                .unwrap_or(ColorByOwnerSample::Scalar(0));
        }
        ColorByOwnerSample::Scalar(self.pixels.get(idx).copied().unwrap_or(0))
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ColorByOwnerSample {
    Scalar(u8),
    Overlay(Color),
}

#[derive(Clone, Copy)]
pub(crate) enum FilteredColorByOwnerSample {
    Scalar(f32),
    Overlay([f32; 4]),
}

#[derive(Clone, Copy)]
pub(crate) enum PreparedSpriteLayer {
    Legacy(Color),
    Shader { rgb: [f32; 3], alpha: f32 },
}

impl PreparedSpriteLayer {
    fn alpha(self) -> f32 {
        match self {
            Self::Legacy(color) => f32::from(color.a),
            Self::Shader { alpha, .. } => alpha,
        }
    }

    pub(crate) fn into_fragment(self) -> PreparedSpriteFragment {
        match self {
            Self::Legacy(color) => PreparedSpriteFragment::Legacy(color),
            Self::Shader { rgb, alpha } => PreparedSpriteFragment::Shader { rgb, alpha },
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum PreparedSpriteFragment {
    /// Exact pre-existing path for an unmodulated main-surface texel.
    Legacy(Color),
    /// StdGL shader output before gamma lookup and framebuffer blending.
    Shader { rgb: [f32; 3], alpha: f32 },
    /// C4Surface owner-color bitmaps are two textures and therefore two
    /// framebuffer passes: unchanged base first, full RGBA overlay second.
    Layers {
        base: PreparedSpriteLayer,
        overlay: PreparedSpriteLayer,
    },
}

impl PreparedSpriteFragment {
    pub(crate) fn alpha(self) -> f32 {
        match self {
            Self::Legacy(color) => f32::from(color.a),
            Self::Shader { alpha, .. } => alpha,
            Self::Layers { base, overlay } => base.alpha().max(overlay.alpha()),
        }
    }

    pub(crate) fn into_layer(self) -> PreparedSpriteLayer {
        match self {
            Self::Legacy(color) => PreparedSpriteLayer::Legacy(color),
            Self::Shader { rgb, alpha } => PreparedSpriteLayer::Shader { rgb, alpha },
            Self::Layers { .. } => unreachable!("nested sprite layers are never prepared"),
        }
    }
}

pub(crate) fn split_c4_color(raw: u32) -> [u8; 4] {
    [
        ((raw >> 16) & 0xff) as u8,
        ((raw >> 8) & 0xff) as u8,
        (raw & 0xff) as u8,
        ((raw >> 24) & 0xff) as u8,
    ]
}
