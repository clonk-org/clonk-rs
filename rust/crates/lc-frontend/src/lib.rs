#![allow(dead_code)]
#![allow(
    clippy::manual_clamp,
    clippy::op_ref,
    clippy::question_mark,
    clippy::too_many_arguments
)]

pub mod clonk_fonts;
pub mod classic_gui;
pub mod hud;
pub mod message_dialog;
pub mod startup_about_dlg;
pub mod startup_netdlg;
pub mod startup_options_dlg;
pub mod startup_plrsel;
pub mod startup_scensel;
#[cfg(test)]
pub(crate) mod test_support;
mod input;
mod startup_about;
mod startup_main_menu;
mod startup_menu;
mod startup_options;

use lc_engine::{
    math::{fixtoi, itofix, C4Fixed},
    object_visible_for_player,
    DefinitionActionGraphics, DefinitionId, DefinitionRect, DefinitionTargetRect, Direction,
    DrawTransform,
    EnvironmentFrame, EnvironmentSettings, FloatVector2, GammaControlState, GraphicsOverlayMode,
    Landscape, ObjectGraphicsOverlay, ObjectId, ObjectSnapshot, ObjectStatus, ParticleSnapshot,
    PlayerState, RgbColor, SimulationSnapshot, SkyFrame, SkySettings,
    SurfaceSnapshot as EngineSurfaceSnapshot, Vector2, WeatherEvent, FULL_CON, OWNER_NONE,
};
#[cfg(test)]
use lc_engine::{
    VIS_ALLIES, VIS_ENEMIES, VIS_GOD, VIS_LAYER_TOGGLE, VIS_LOCAL, VIS_OVERLAY_ONLY, VIS_OWNER,
};
use lc_graphics::{
    Color, PixelFormat, Point as SurfacePoint, Rect as SurfaceRect, Surface,
    SurfaceSnapshot as GraphicsSurfaceSnapshot, TextFont,
};
use lc_gui::{Rect as GuiRect, Size as GuiSize};
use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::convert::TryFrom;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

pub use input::InputDispatcher;
pub use lc_gui::{
    GuiError as StartupMenuError, GuiResult as StartupMenuResult, ImageData, KeyCode,
    Point as GuiPoint, ScenarioEntry, ScenarioKind,
};
pub use clonk_fonts::{expand_hotkey_markup, ClonkFontSet};
pub use hud::{CommandIcon, CommandImage, CommandOverlayIcon};
pub use startup_about::{AboutAction, StartupAboutDialog};
pub use startup_main_menu::{main_menu_layout, MainMenuAction, MainMenuItem, StartupMainMenu};
pub use startup_menu::{ScenarioSummary, StartupMenu, StartupMenuAction};
pub use startup_options::{ControlOptionItem, ControlOptionsAction, ControlOptionsView};

const MIN_VIEWPORT_ZOOM: f32 = 0.125;
const MAX_VIEWPORT_ZOOM: f32 = 4.0;
/// `C4ViewportScrollBorder` (src/C4Constants.h:95).
const VIEWPORT_SCROLL_BORDER: i32 = 40;
/// `Config.General.ScrollSmooth` (src/C4Config.cpp:386). The C++ viewport
/// clamps the configured value to 1..=50 at the point of use.
const DEFAULT_SCROLL_SMOOTH: i32 = 4;
const CAMERA_UNINITIALIZED: i32 = -31_337;
const PICK_TOLERANCE: f32 = 6.0;
/// `MagicPhysicalFactor` (src/C4Object.h:81).
const MAGIC_PHYSICAL_FACTOR: i32 = 1_000;
const MATERIAL_OVERLAY_EXACT: i32 = 1;
const MATERIAL_OVERLAY_HUGE_ZOOM: i32 = 4;
const MATERIAL_OVERLAY_MONOCHROME: i32 = 8;
/// `C4GFXBLIT_ADDITIVE` (src/C4Surface.h:40).
const C4GFXBLIT_ADDITIVE: u32 = 1;
/// `C4GFXBLIT_MOD2`: MOD2 source modulation for the main surface.
const C4GFXBLIT_MOD2: u32 = 2;
/// `C4GFXBLIT_CLRSFC_OWNCLR`: do not fold global ColorMod into owner color.
const C4GFXBLIT_CLRSFC_OWNCLR: u32 = 4;
/// `C4GFXBLIT_CLRSFC_MOD2`: MOD2 source modulation for the owner surface.
const C4GFXBLIT_CLRSFC_MOD2: u32 = 8;
/// `C4GFXBLIT_PARENT` is an exact overlay sentinel, not a combinable flag
/// (src/C4DefGraphics.cpp:762-768).
const C4GFXBLIT_PARENT: u32 = 256;

/// Presentation fields from one C4MaterialCore. Colors and alpha retain the
/// C++ arrays verbatim: three RGB triplets and two sets of three transparency
/// values (`0` opaque, `255` transparent).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialRenderInfo {
    color: [u8; 9],
    alpha: [u8; 6],
    texture_overlay: Option<String>,
    overlay_type: i32,
    density: i32,
    /// Loose-material sprite sheet (`C4MaterialCore::sPXSGfx`). The sheet is
    /// resolved through the same texture map as landscape patterns.
    pxs_gfx: Option<String>,
    /// `C4TargetRect` [x, y, width, height, target-x, target-y].
    pxs_gfx_rect: [i32; 6],
    pxs_gfx_size: i32,
}

impl MaterialRenderInfo {
    pub fn new(
        color: [u8; 9],
        alpha: [u8; 6],
        texture_overlay: Option<String>,
        overlay_type: i32,
        density: i32,
    ) -> Self {
        Self {
            color,
            alpha,
            texture_overlay,
            overlay_type,
            density,
            pxs_gfx: None,
            pxs_gfx_rect: [0; 6],
            pxs_gfx_size: 0,
        }
    }

    pub fn with_pxs_graphics(
        mut self,
        texture: Option<String>,
        rect: [i32; 6],
        size: i32,
    ) -> Self {
        self.pxs_gfx = texture;
        self.pxs_gfx_rect = rect;
        self.pxs_gfx_size = size;
        self
    }
}

#[derive(Clone, Copy)]
struct MaterialPixel {
    red: u8,
    green: u8,
    blue: u8,
    transparency: u8,
}

fn lighten_material_channel(channel: u8) -> u8 {
    if channel & 0x80 != 0 {
        255
    } else {
        channel << 1
    }
}

fn apply_material_pattern(
    pixel: &mut MaterialPixel,
    pattern: &ImageData,
    x: i32,
    y: i32,
    zoom: i32,
    monochrome: bool,
) {
    let width = pattern.width();
    let height = pattern.height();
    if width == 0 || height == 0 {
        return;
    }
    let sample_x = if zoom == 0 { x } else { x / zoom };
    let sample_y = if zoom == 0 { y } else { y / zoom };
    let source = (((sample_y as u32 % height) * width + sample_x as u32 % width) * 4) as usize;
    let data = pattern.pixels();
    if source + 4 > data.len() {
        return;
    }

    let modifiers = if monochrome {
        [data[source + 2]; 3]
    } else {
        [data[source], data[source + 1], data[source + 2]]
    };
    pixel.red = lighten_material_channel(
        ((u16::from(pixel.red) * u16::from(modifiers[0])) >> 8) as u8,
    );
    pixel.green = lighten_material_channel(
        ((u16::from(pixel.green) * u16::from(modifiers[1])) >> 8) as u8,
    );
    pixel.blue = lighten_material_channel(
        ((u16::from(pixel.blue) * u16::from(modifiers[2])) >> 8) as u8,
    );
    let pattern_transparency = 255u8.saturating_sub(data[source + 3]);
    pixel.transparency = pixel
        .transparency
        .saturating_add(pattern_transparency);
}

fn compose_material_pixel(
    material: &MaterialRenderInfo,
    landscape_pixel: u8,
    x: i32,
    y: i32,
    texture: &ImageData,
    overlay: Option<&ImageData>,
) -> Color {
    let mut pixel = MaterialPixel {
        red: material.color[0],
        green: material.color[1],
        blue: material.color[2],
        transparency: material.alpha[if landscape_pixel & 0x80 == 0 { 0 } else { 3 }],
    };
    let primary_zoom = if material.overlay_type & MATERIAL_OVERLAY_HUGE_ZOOM != 0 {
        4
    } else if material.overlay_type & MATERIAL_OVERLAY_EXACT != 0 {
        1
    } else {
        0
    };
    let monochrome = material.overlay_type & MATERIAL_OVERLAY_MONOCHROME != 0;
    apply_material_pattern(
        &mut pixel,
        texture,
        x,
        y,
        primary_zoom,
        monochrome,
    );
    if let Some(overlay) = overlay {
        let overlay_zoom = if material.overlay_type & MATERIAL_OVERLAY_EXACT != 0 {
            1
        } else {
            2
        };
        apply_material_pattern(
            &mut pixel,
            overlay,
            x,
            y,
            overlay_zoom,
            monochrome,
        );
    }
    Color::new(
        pixel.red,
        pixel.green,
        pixel.blue,
        255u8.saturating_sub(pixel.transparency),
    )
}

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

fn sprite_map_key(definition_id: &str, graphics_name: Option<&str>) -> String {
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

const CATEGORY_BACKGROUND_FLAG: i32 = 1 << 20;
const CATEGORY_PARALLAX_FLAG: i32 = 1 << 21;
const CATEGORY_FOREGROUND_FLAG: i32 = 1 << 23;
const CATEGORY_MOUSE_IGNORE_FLAG: i32 = 1 << 24;

#[derive(Clone, Copy)]
enum ObjectRenderPass {
    Background,
    Normal,
    ForegroundNonParallax,
    ForegroundParallax,
}

/// CStdDDraw state established by C4Object::PrepareDrawing or one explicit
/// C4GraphicsOverlay. Modulation retains C4's packed transparency-alpha color.
#[derive(Clone, Copy)]
struct SpriteBlitState {
    mode: u32,
    modulation: Option<u32>,
}

impl SpriteBlitState {
    const fn normal() -> Self {
        Self {
            mode: 0,
            modulation: None,
        }
    }

    fn for_object(object: &ObjectSnapshot) -> Self {
        let mode = object.blit_mode;
        let modulation = (object.color_modulation != 0
            || mode & (C4GFXBLIT_MOD2 | C4GFXBLIT_CLRSFC_MOD2) != 0)
            .then_some(object.color_modulation);
        Self { mode, modulation }
    }

    fn for_overlay(object: &ObjectSnapshot, overlay: &ObjectGraphicsOverlay) -> Self {
        if overlay.blit_mode == C4GFXBLIT_PARENT {
            return Self::for_object(object);
        }
        Self {
            mode: overlay.blit_mode,
            modulation: (overlay.color_modulation != 0x00ff_ffff)
                .then_some(overlay.color_modulation),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct DefinitionSprite {
    pub image: ImageData,
    pub actions: HashMap<String, DefinitionActionGraphics>,
    pub color_mask: Option<ColorByOwnerMask>,
    /// The def Shape rect (DefCore Offset + Width/Height): idle objects
    /// draw Shape.Wdt x Shape.Hgt from the graphics origin
    /// (C4Object::DrawFace, C4Object.cpp:438-460) — never the whole
    /// sprite sheet — and the face is anchored at the shape top-left,
    /// x + Shape.x / y + Shape.y (C4Object::Draw, C4Object.cpp:2231).
    pub shape: Option<DefinitionRect>,
    /// DefCore StretchGrowth → C4Def::GrowthType (src/C4Def.cpp:387):
    /// Con scales the shape on both axes (C4Shape::Stretch) instead of
    /// height only (C4Shape::Jolt), C4Object.cpp:329-333.
    pub stretch_growth: bool,
    /// DefCore `TopFace`: source facet and object-relative target rectangle.
    pub top_face: Option<DefinitionTargetRect>,
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
    /// Background.png — `fctBackground`, the message board backdrop tile
    /// (src/C4GraphicsResource.cpp:209, src/C4MessageBoard.cpp:258).
    pub background: Option<ImageData>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ColorByOwnerMask {
    width: u32,
    height: u32,
    pixels: Arc<[u8]>,
}

impl ColorByOwnerMask {
    pub fn new(width: u32, height: u32, pixels: Arc<[u8]>) -> Self {
        Self {
            width,
            height,
            pixels,
        }
    }

    fn value_at(&self, x: u32, y: u32) -> u8 {
        if x >= self.width || y >= self.height {
            return 0;
        }
        let idx = (y * self.width + x) as usize;
        self.pixels.get(idx).copied().unwrap_or(0)
    }
}

#[derive(Clone, Copy)]
enum PreparedSpriteFragment {
    /// Exact pre-existing path for an unmodulated main-surface texel.
    Legacy(Color),
    /// StdGL shader output before gamma lookup and framebuffer blending.
    Shader { rgb: [f32; 3], alpha: u8 },
}

impl PreparedSpriteFragment {
    fn alpha(self) -> u8 {
        match self {
            Self::Legacy(color) => color.a,
            Self::Shader { alpha, .. } => alpha,
        }
    }
}

fn split_c4_color(raw: u32) -> [u8; 4] {
    [
        ((raw >> 16) & 0xff) as u8,
        ((raw >> 8) & 0xff) as u8,
        (raw & 0xff) as u8,
        ((raw >> 24) & 0xff) as u8,
    ]
}

/// The ClrByOwner tint passed by C4Object::DrawFace/DrawTopFace to
/// C4DefGraphics::GetBitmap (C4Object.cpp:440-477,2617-2670). This is the
/// live object color, which scripts may change independently of its owner.
fn object_color_by_owner_tint(object: &ObjectSnapshot) -> u32 {
    // C4Surface::SetClr substitutes the legacy blue value 0xff for zero
    // (C4Surface.h:110).
    if object.color == 0 { 0xff } else { object.color }
}

/// CPU-side `ModulateClr` used to fold global ColorMod into ClrByOwnerClr
/// before the owner texture reaches the shader (StdDDraw2.cpp:773-777).
fn modulate_c4_colors(dst: u32, src: u32) -> u32 {
    let dst = split_c4_color(dst);
    let src = split_c4_color(src);
    let mul = |a: u8, b: u8| (u32::from(a) * u32::from(b)) >> 8;
    let alpha = (u32::from(dst[3]) + u32::from(src[3]) - mul(dst[3], src[3])).min(255);
    (alpha << 24)
        | (mul(dst[0], src[0]) << 16)
        | (mul(dst[1], src[1]) << 8)
        | mul(dst[2], src[2])
}

fn shader_modulate_fragment(source: Color, modulation: u32, mod2: bool) -> PreparedSpriteFragment {
    let modulation = split_c4_color(modulation);
    if mod2 {
        let channel = |source: u8, modulation: u8| {
            (2.0 * f32::from(source) + 2.0 * f32::from(modulation) - 255.0)
                .clamp(0.0, 255.0)
        };
        PreparedSpriteFragment::Shader {
            rgb: [
                channel(source.r, modulation[0]),
                channel(source.g, modulation[1]),
                channel(source.b, modulation[2]),
            ],
            // LC_MOD2 intentionally leaves texture alpha untouched
            // (StdGL.cpp:1072-1075).
            alpha: source.a,
        }
    } else {
        let channel = |source: u8, modulation: u8| {
            f32::from(source) * f32::from(modulation) / 255.0
        };
        PreparedSpriteFragment::Shader {
            rgb: [
                channel(source.r, modulation[0]),
                channel(source.g, modulation[1]),
                channel(source.b, modulation[2]),
            ],
            // Textures carry normal opacity in Rust, while StdGL adds C4's
            // transparency-alpha modulation to texture transparency.
            alpha: source.a.saturating_sub(modulation[3]),
        }
    }
}

fn prepare_sprite_fragment(
    source: Color,
    owner_mask: Option<u8>,
    owner_color: Option<u32>,
    blit: SpriteBlitState,
) -> PreparedSpriteFragment {
    if let (Some(mask), Some(mut modulation)) =
        (owner_mask.filter(|mask| *mask != 0), owner_color)
    {
        // The mask stores the grey ClrByOwner texture intensity. Its main-sfc
        // pixel was cleared when C4Surface::CreateColorByOwner split the image
        // (C4Surface.cpp:288-312).
        if let Some(global) = blit.modulation {
            if blit.mode & C4GFXBLIT_CLRSFC_OWNCLR == 0 {
                modulation = modulate_c4_colors(modulation, global);
            }
        }
        // PerformBlt explicitly disables MOD2 for a completely black
        // modulation (StdGL.cpp:471-472).
        let mod2 = blit.mode & C4GFXBLIT_CLRSFC_MOD2 != 0 && modulation != 0;
        return shader_modulate_fragment(
            Color::new(mask, mask, mask, source.a),
            modulation,
            mod2,
        );
    }

    if blit.modulation.is_none() && blit.mode & C4GFXBLIT_MOD2 == 0 {
        return PreparedSpriteFragment::Legacy(source);
    }

    let modulation = blit.modulation.unwrap_or(0x00ff_ffff);
    let mod2 = blit.mode & C4GFXBLIT_MOD2 != 0 && modulation != 0;
    shader_modulate_fragment(source, modulation, mod2)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewportPointer {
    pub owner: i32,
    pub world: FloatVector2,
    pub screen: GuiPoint,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SourceRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl SourceRect {
    fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct CameraKey {
    owner: i32,
    /// C4Viewport owns its smoothing state. A focus/ViewCursor change does
    /// not create a new viewport, so the stable per-owner slot is the key.
    slot: usize,
}

#[derive(Debug, Clone, Copy)]
struct CameraState {
    d_view_x: C4Fixed,
    d_view_y: C4Fixed,
    view_x: i32,
    view_y: i32,
    view_width: i32,
    view_height: i32,
}

impl CameraState {
    /// `CreateViewport` calls `CenterPosition` after setting the output size,
    /// while dViewX/Y retain C4Viewport's negative initialization sentinel.
    fn new(world_width: i32, world_height: i32, view_width: i32, view_height: i32) -> Self {
        Self {
            d_view_x: itofix(CAMERA_UNINITIALIZED),
            d_view_y: itofix(CAMERA_UNINITIALIZED),
            view_x: (world_width - view_width) / 2,
            view_y: (world_height - view_height) / 2,
            view_width,
            view_height,
        }
    }

    fn update(
        &mut self,
        center_x: i32,
        center_y: i32,
        view_width: i32,
        view_height: i32,
        world_width: i32,
        world_height: i32,
        scroll_border: i32,
        scroll_smooth: i32,
    ) -> (i32, i32) {
        // SetOutputSize keeps the previous visible center. It adjusts the
        // integer ViewX/Y but deliberately does not rewrite dViewX/Y.
        self.resize_output(view_width, view_height);

        let scroll_range = (view_width / 10).min(view_height / 10);
        let target_x = classic_camera_target_axis(
            self.view_x,
            center_x,
            view_width,
            world_width,
            scroll_range,
            scroll_border,
        );
        let target_y = classic_camera_target_axis(
            self.view_y,
            center_y,
            view_height,
            world_height,
            scroll_range,
            scroll_border,
        );
        let divisor = scroll_smooth.clamp(1, 50);

        // C4Viewport uses the sign of both fixed coordinates as its coupled
        // initialization test. This also means a negative border position
        // takes the snap branch on every graphics pass.
        if self.d_view_x >= 0 && self.d_view_y >= 0 {
            self.d_view_x += (itofix(target_x) - self.d_view_x) / divisor;
            self.d_view_y += (itofix(target_y) - self.d_view_y) / divisor;
            self.view_x = fixtoi(self.d_view_x);
            self.view_y = fixtoi(self.d_view_y);
        } else {
            self.view_x = target_x;
            self.view_y = target_y;
            self.d_view_x = itofix(target_x);
            self.d_view_y = itofix(target_y);
        }

        (self.view_x, self.view_y)
    }

    fn resize_output(&mut self, view_width: i32, view_height: i32) {
        if self.view_width != view_width {
            self.view_x += (self.view_width - view_width) / 2;
            self.view_width = view_width;
        }
        if self.view_height != view_height {
            self.view_y += (self.view_height - view_height) / 2;
            self.view_height = view_height;
        }
    }

    /// No-owner fullscreen viewports are not player-locked. Without an
    /// explicit FreeScroll input they retain their centered position, and
    /// UpdateViewPosition hard-clamps large worlds while centering small ones.
    fn no_owner_position(
        &mut self,
        view_width: i32,
        view_height: i32,
        world_width: i32,
        world_height: i32,
    ) -> (i32, i32) {
        self.resize_output(view_width, view_height);
        if world_width < view_width {
            self.view_x = (world_width - view_width) / 2;
        } else {
            self.view_x = self.view_x.clamp(0, world_width - view_width);
        }
        if world_height < view_height {
            self.view_y = (world_height - view_height) / 2;
        } else {
            self.view_y = self.view_y.clamp(0, world_height - view_height);
        }
        (self.view_x, self.view_y)
    }
}

/// C4Viewport::AdjustPosition's per-axis dead-zone and progressive edge
/// bounds (src/C4Viewport.cpp:1165-1201). Inputs and the result are whole
/// world pixels; the 16.16 filter is applied afterwards.
fn classic_camera_target_axis(
    current_view: i32,
    center: i32,
    view_extent: i32,
    world_extent: i32,
    scroll_range: i32,
    scroll_border: i32,
) -> i32 {
    let mut extra_bound = 0;
    if center < scroll_border {
        extra_bound = (scroll_border - center).min(scroll_border);
    } else if center >= world_extent - scroll_border {
        extra_bound = (center - world_extent).min(0) + scroll_border;
    }
    extra_bound = extra_bound.max((view_extent - world_extent) / 2 + 1);

    let desired = center - view_extent / 2;
    let target = current_view.clamp(desired - scroll_range, desired + scroll_range);
    let min_view = -extra_bound;
    let max_view = world_extent - view_extent + extra_bound;
    if min_view <= max_view {
        target.clamp(min_view, max_view)
    } else {
        // The oversized-world rule above normally prevents an inverted
        // range. Keep the centered C++ fallback for defensive malformed
        // dimensions rather than panicking in `i32::clamp`.
        (world_extent - view_extent) / 2
    }
}

fn scaled_camera_border(border: i32, zoom: f32, output_extent: u32) -> u32 {
    (border.max(0) as f32 * zoom)
        .round()
        .clamp(0.0, output_extent as f32) as u32
}

#[derive(Debug, Clone, Default)]
pub struct CursorAtlas {
    images: Vec<Option<ImageData>>,
}

impl CursorAtlas {
    pub fn new(images: Vec<Option<ImageData>>) -> Self {
        Self { images }
    }

    pub fn empty() -> Self {
        Self { images: Vec::new() }
    }

    pub fn image_for_resolution(&self, width: u32) -> Option<ImageData> {
        if self.images.is_empty() {
            return None;
        }

        const DEFAULT_INDEX: usize = 5;
        const BREAKPOINTS: [u32; 2] = [1280, 800];

        let mut index = DEFAULT_INDEX;
        if width <= BREAKPOINTS[0] {
            for &bp in &BREAKPOINTS {
                if width >= bp {
                    break;
                }
                index += 1;
            }
        }
        if index >= self.images.len() {
            index = self.images.len() - 1;
        }

        let mut candidates = Vec::with_capacity(self.images.len());
        candidates.push(index);
        for offset in 1..self.images.len() {
            if let Some(left) = index.checked_sub(offset) {
                candidates.push(left);
            }
            let right = index + offset;
            if right < self.images.len() {
                candidates.push(right);
            }
        }

        for idx in candidates {
            if let Some(image) = self.images[idx].clone() {
                return Some(image);
            }
        }
        None
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SkyRenderState {
    settings: SkySettings,
    image: Option<ImageData>,
}

impl SkyRenderState {
    pub fn new(settings: SkySettings, image: Option<ImageData>) -> Self {
        Self { settings, image }
    }

    pub fn settings(&self) -> &SkySettings {
        &self.settings
    }

    pub fn image(&self) -> Option<&ImageData> {
        self.image.as_ref()
    }
}

pub struct GraphicsOverlay<'a> {
    /// Debug FRAME/POS/VEL line — drawn only when `debug_hud` is set.
    pub frame_text: &'a str,
    /// Debug ENERGY/DAMAGE/OWNER line — drawn only when `debug_hud` is set.
    pub status_text: &'a str,
    /// Opt-in debug HUD lines (not part of the C++-faithful overlay).
    pub debug_hud: bool,
    pub players: Vec<PlayerOverlay>,
    /// `Game.Time` seconds for the upper board clock
    /// (C4Game::Sec1Timer, src/C4Game.cpp:1737-1741).
    pub game_time_seconds: u64,
    /// The current message board log line (C4MessageBoard LogBuffer tail,
    /// src/C4MessageBoard.cpp:271-303).
    pub message_board_line: Option<String>,
    /// `Config.Graphics.ShowCommands` (src/C4Config.cpp:449) — gates the
    /// per-viewport command rows (src/C4Viewport.cpp:948).
    pub show_commands: bool,
    /// `Config.Graphics.ShowCommandKeys` (src/C4Config.cpp:450) — key names
    /// on the command key caps (src/C4ObjectCom.cpp:942).
    pub show_command_keys: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerOverlay {
    pub owner: i32,
    pub name: String,
    pub wealth: i32,
    pub score: i32,
    pub cursor: Option<ObjectId>,
    pub eliminated: bool,
    pub owner_color: Color,
    /// `C4Player::SelectCount` for the crew display value
    /// (src/C4Viewport.cpp:1320).
    pub select_count: i32,
    /// `C4Player::ShowStartup` — keyboard hint + name until the first
    /// control com (src/C4Player.cpp:1376, src/C4Viewport.cpp:1450).
    pub show_startup: bool,
    /// `C4Player::ShowControl` and `ShowControlPos`, consumed by
    /// `C4Viewport::DrawPlayerControls` (src/C4Viewport.cpp:1394-1441).
    pub show_control: i32,
    pub show_control_position: i32,
    /// Raw `C4Player::LastCom`; `Com2Control` selects the pressed hint.
    pub last_com: u8,
    /// Short `PlrControlKeyName` values in CON_* order.
    pub control_key_labels: Vec<String>,
    pub crew: Vec<CrewOverlay>,
    /// The cursor object's contextual command icons
    /// (C4Object::DrawCommands, src/C4Object.cpp:2940-3098), resolved by
    /// the app; drawn into the viewport command rows when ShowCommands.
    pub commands: Vec<CommandIcon>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CrewOverlay {
    pub object_id: ObjectId,
    /// The crew member's name (`C4ObjectInfo::sName`).
    pub label: String,
    pub energy_fraction: f32,
    /// Raw `C4Object::MagicEnergy` and resolved `GetPhysical()->Magic`.
    /// A non-zero level inserts the optional middle HUD bar
    /// (src/C4Viewport.cpp:934-938; src/C4Object.cpp:2722-2726).
    pub magic_energy: i32,
    pub magic_capacity: i32,
    /// Raw `C4Object::Breath` and resolved `GetPhysical()->Breath`.
    /// C++ draws this bar only while breath is non-zero and below capacity
    /// (src/C4Viewport.cpp:939-943; src/C4Object.cpp:2728-2731).
    pub breath: i32,
    pub breath_capacity: i32,
    pub is_focus: bool,
    pub portrait: Option<ImageData>,
    /// `C4ObjectInfo::Rank` (src/C4ObjectInfo.cpp:330).
    pub rank: i32,
    /// The def's own rank symbols (`pDef->pRankSymbols`,
    /// src/C4ObjectInfo.cpp:334-341); falls back to the global Rank.png.
    pub rank_symbols: Option<ImageData>,
    /// `cursor->Info` presence + `Info->sName`: the red cursor label above
    /// the flashing mark draws only for crew with an object info
    /// (C4Game::DrawCursors, src/C4Game.cpp:1873-1887).
    pub info_name: Option<String>,
    /// `Info->sRankName` for the extra rank line when `Rank > 0`
    /// (src/C4Game.cpp:1877-1881).
    pub rank_name: Option<String>,
    /// The grouped sections of `cursor->Contents.DrawIDList`
    /// (src/C4Viewport.cpp:911-917; src/C4ObjectList.cpp:343-372).
    pub inventory: Vec<InventoryOverlay>,
}

/// Presentation data for one grouped cursor-inventory section. The first
/// object represents the group, matching `C4ObjectListIterator::GetNext`
/// (src/C4ObjectList.cpp:849-903).
#[derive(Clone, Debug, PartialEq)]
pub struct InventoryOverlay {
    pub object_id: ObjectId,
    pub definition_id: DefinitionId,
    pub picture: Option<ImageData>,
    pub count: usize,
}

#[derive(Debug)]
pub struct ViewportInput<'a> {
    pub owner: i32,
    pub center: Vector2,
    pub offset: Vector2,
    pub zoom: f32,
    pub focus: &'a ObjectSnapshot,
}

impl<'a> ViewportInput<'a> {
    pub fn new(owner: i32, center: Vector2, zoom: f32, focus: &'a ObjectSnapshot) -> Self {
        Self {
            owner,
            center,
            offset: Vector2::ZERO,
            zoom,
            focus,
        }
    }

    pub fn with_offset(mut self, offset: Vector2) -> Self {
        self.offset = offset;
        self
    }

    pub fn from_focus(focus: &'a ObjectSnapshot) -> Self {
        Self {
            owner: focus.owner,
            center: Vector2::new(focus.position.x, focus.position.y),
            offset: Vector2::ZERO,
            zoom: 1.0,
            focus,
        }
    }
}

#[derive(Debug, Clone)]
struct ActiveViewport {
    owner: i32,
    focus: ObjectId,
    rect: SurfaceRect,
    content_rect: SurfaceRect,
    viewport_x: f32,
    viewport_y: f32,
    zoom: f32,
}

pub struct GraphicsSystem {
    surface: Surface,
    font: Arc<dyn TextFont>,
    /// The CStdFont-faithful fonts; the HUD's FontRegular when present
    /// (C4GraphicsResource::InitFonts, src/C4GraphicsResource.cpp:144-169).
    clonk_fonts: Option<Arc<ClonkFontSet>>,
    scenario_label_text: String,
    /// Per-player HUD state fed by [`Self::update_overlay`].
    hud_players: Vec<PlayerOverlay>,
    game_time_seconds: u64,
    message_board_line: Option<String>,
    /// `Config.Graphics.ShowCommands` / `ShowCommandKeys`
    /// (src/C4Config.cpp:449-450, default true).
    show_commands: bool,
    show_command_keys: bool,
    /// Debug FRAME/STATUS lines; `None` hides them (default HUD).
    debug_hud_text: Option<(String, String)>,
    viewport_x: f32,
    viewport_y: f32,
    viewport_zoom: f32,
    surface_width: u32,
    surface_height: u32,
    fallback_ground_height: i32,
    world_width: i32,
    world_height: i32,
    object_sprites: Arc<HashMap<String, DefinitionSprite>>,
    cursor_atlas: Arc<CursorAtlas>,
    hud_graphics: Arc<HudGraphics>,
    active_viewports: Vec<ActiveViewport>,
    camera_states: HashMap<CameraKey, CameraState>,
    /// Gamma currently installed in CStdDDraw. A runtime SetGamma mutates the
    /// snapshot controls during the game tick, but C4GraphicsSystem applies
    /// them only after drawing that render pass; a fresh graphics system has
    /// already received InitGame's explicit ApplyGamma.
    active_gamma_control_points: Option<[u32; 3]>,
    /// C4ConfigGeneral::ScrollSmooth. Config plumbing lives above the
    /// frontend; retain the exact C++ default and clamp at use meanwhile.
    scroll_smooth: i32,
    sky: Option<SkyRenderState>,
    /// Material texture pngs by lowercase texture name — the landscape
    /// plane samples them per pixel (C++ builds Surface32 from the same
    /// Material.c4g textures during MapToSurface).
    material_textures: Arc<HashMap<String, ImageData>>,
    /// C4MaterialCore presentation fields by lowercase material name.
    material_render_info: Arc<HashMap<String, MaterialRenderInfo>>,
    /// Cached RGBA render of the landscape plane, keyed by the pixel
    /// grid's revision.
    landscape_cache: Option<(u64, ImageData)>,
}

impl GraphicsSystem {
    pub fn new(
        surface_width: u32,
        surface_height: u32,
        fallback_ground_height: i32,
        scenario_label: &str,
        font: Arc<dyn TextFont>,
        object_sprites: Arc<HashMap<String, DefinitionSprite>>,
        cursor_atlas: Arc<CursorAtlas>,
        hud_graphics: Arc<HudGraphics>,
    ) -> Self {
        let mut surface = Surface::new(
            surface_width,
            surface_height,
            lc_graphics::PixelFormat::Rgba8888,
        );
        surface.fill(Color::opaque(8, 12, 24));

        Self {
            surface,
            font,
            clonk_fonts: None,
            scenario_label_text: scenario_label.to_string(),
            hud_players: Vec::new(),
            game_time_seconds: 0,
            message_board_line: None,
            show_commands: true,
            show_command_keys: true,
            debug_hud_text: None,
            viewport_x: 0.0,
            viewport_y: 0.0,
            viewport_zoom: 1.0,
            surface_width,
            surface_height,
            fallback_ground_height,
            world_width: surface_width as i32,
            world_height: fallback_ground_height.max(surface_height as i32).max(0),
            object_sprites,
            cursor_atlas,
            hud_graphics,
            active_viewports: Vec::new(),
            camera_states: HashMap::new(),
            active_gamma_control_points: None,
            scroll_smooth: DEFAULT_SCROLL_SMOOTH,
            sky: None,
            material_textures: Arc::new(HashMap::new()),
            material_render_info: Arc::new(HashMap::new()),
            landscape_cache: None,
        }
    }

    pub fn set_object_sprites(&mut self, sprites: Arc<HashMap<String, DefinitionSprite>>) {
        self.object_sprites = sprites;
    }

    pub fn set_world_width(&mut self, world_width: i32) {
        self.world_width = world_width.max(self.surface_width as i32);
    }

    pub fn set_world_height(&mut self, world_height: i32) {
        self.world_height = world_height.max(self.surface_height as i32);
    }

    pub fn set_world_dimensions(&mut self, world_width: i32, world_height: i32) {
        self.set_world_width(world_width);
        self.set_world_height(world_height);
    }

    pub fn set_material_textures(&mut self, textures: Arc<HashMap<String, ImageData>>) {
        self.material_textures = textures;
        self.landscape_cache = None;
    }

    pub fn set_material_render_info(
        &mut self,
        render_info: Arc<HashMap<String, MaterialRenderInfo>>,
    ) {
        self.material_render_info = render_info;
        self.landscape_cache = None;
    }

    pub fn set_sky(&mut self, sky: Option<SkyRenderState>) {
        self.sky = sky;
    }

    /// Set `Config.General.ScrollSmooth` for subsequent viewport renders.
    /// C++ stores the raw value and clamps it to 1..=50 in AdjustPosition.
    pub fn set_scroll_smooth(&mut self, scroll_smooth: i32) {
        self.scroll_smooth = scroll_smooth;
    }

    pub fn hud_graphics(&self) -> Arc<HudGraphics> {
        Arc::clone(&self.hud_graphics)
    }

    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    pub fn surface_mut(&mut self) -> &mut Surface {
        &mut self.surface
    }

    /// Output rectangle of the player's active viewport. Viewport-owned GUI
    /// such as C4ObjectMenu aligns inside this area, not the full backbuffer
    /// (C4Viewport::DrawMenu, C4Viewport.cpp:967-1014).
    pub fn viewport_rect(&self, owner: i32) -> Option<SurfaceRect> {
        self.active_viewports
            .iter()
            .find(|viewport| viewport.owner == owner)
            .map(|viewport| viewport.rect)
    }

    pub fn world_to_screen(&self, owner: i32, position: Vector2) -> Option<(f32, f32)> {
        self.active_viewports
            .iter()
            .find(|viewport| viewport.owner == owner)
            .map(|viewport| {
                let screen_x = (position.x as f32 - viewport.viewport_x) * viewport.zoom
                    + viewport.content_rect.x as f32;
                let screen_y = (position.y as f32 - viewport.viewport_y) * viewport.zoom
                    + viewport.content_rect.y as f32;
                (screen_x, screen_y)
            })
    }

    pub fn viewport_point_at(&self, point: GuiPoint) -> Option<ViewportPointer> {
        let viewport = self.viewport_for_point(point)?;
        let zoom = viewport.zoom.max(MIN_VIEWPORT_ZOOM);
        let base_x = viewport.content_rect.x as f32;
        let base_y = viewport.content_rect.y as f32;
        let world_x = (point.x - base_x) / zoom + viewport.viewport_x;
        let world_y = (point.y - base_y) / zoom + viewport.viewport_y;
        Some(ViewportPointer {
            owner: viewport.owner,
            world: FloatVector2::new(world_x, world_y),
            screen: point,
        })
    }

    pub fn crew_at_point(
        &self,
        snapshot: &SimulationSnapshot,
        owner: i32,
        point: GuiPoint,
    ) -> Option<ObjectId> {
        let viewport = self.viewport_for_point(point)?;
        if viewport.owner != owner {
            return None;
        }

        let mut best: Option<(ObjectId, f32)> = None;
        for object in &snapshot.objects {
            if object.owner != owner
                || !object.crew_member
                || !object.status.is_active()
                || !object.alive
                || !Self::object_is_visible(
                    &snapshot.objects,
                    &snapshot.players,
                    object,
                    owner,
                    false,
                )
            {
                continue;
            }
            if let Some(rect) = self.object_screen_rect_for_viewport(object, viewport) {
                if rect_contains(rect, point, PICK_TOLERANCE) {
                    let center_x = rect.x as f32 + rect.width as f32 * 0.5;
                    let center_y = rect.y as f32 + rect.height as f32 * 0.5;
                    let dx = point.x - center_x;
                    let dy = point.y - center_y;
                    let distance_sq = dx * dx + dy * dy;
                    match best {
                        Some((_, best_dist)) if distance_sq >= best_dist => {}
                        _ => best = Some((object.id, distance_sq)),
                    }
                }
            }
        }
        best.map(|(id, _)| id)
    }

    /// Returns the frontmost world object under a viewport pointer using the
    /// same front-to-back order as `C4Game::FindVisObject`: C++ searches
    /// `Objects.First -> Next`, while drawing uses `Last -> Prev`
    /// (`C4Game.cpp:1426-1492`; `C4ObjectList.cpp:387-396`).
    pub fn object_at_point(
        &self,
        snapshot: &SimulationSnapshot,
        owner: i32,
        point: GuiPoint,
    ) -> Option<ObjectId> {
        self.object_at_point_with_ocf(snapshot, owner, point, u32::MAX)
    }

    /// The OCF-filtered form of [`Self::object_at_point`], matching the mask
    /// passed to `C4Game::FindVisObject` by `C4MouseControl::GetTargetObject`.
    /// A nonmatching front object does not hide a matching object behind it
    /// (C4Game.cpp:1426-1492; C4MouseControl.cpp:1318-1325).
    pub fn object_at_point_with_ocf(
        &self,
        snapshot: &SimulationSnapshot,
        owner: i32,
        point: GuiPoint,
        ocf: u32,
    ) -> Option<ObjectId> {
        let viewport = self.viewport_for_point(point)?;
        if viewport.owner != owner {
            return None;
        }

        // Reconstruct the renderer's effective back-to-front list first.
        // A partial sidecar is legal and draw_objects appends omitted objects
        // canonically, so those omitted objects are the frontmost group.
        let mut back_to_front = Vec::with_capacity(snapshot.objects.len());
        let mut seen = HashSet::with_capacity(snapshot.objects.len());
        if !snapshot.render_order.is_empty() {
            for id in &snapshot.render_order {
                if seen.insert(*id) {
                    if let Some(object) = snapshot.object(*id) {
                        back_to_front.push(object);
                    }
                }
            }
        }
        back_to_front.extend(
            snapshot
                .objects
                .iter()
                .filter(|object| seen.insert(object.id)),
        );
        // A valid C++ player with no cursor cannot see a target through this
        // search: FindVisObject rejects every candidate before the shape
        // check, so right-up falls through to select-next.
        let player_cursor = snapshot
            .players
            .iter()
            .find(|player| player.id == owner)
            .map(|player| player.cursor);
        let cursor_object = match player_cursor {
            Some(Some(cursor)) => Some(snapshot.object(cursor)?),
            Some(None) => return None,
            None => snapshot
                .crew_selection
                .get(&owner)
                .and_then(|selection| selection.cursor)
                .and_then(|cursor| snapshot.object(cursor)),
        };
        let cursor_layer = cursor_object.map(|cursor| cursor.layer);

        back_to_front.into_iter().rev().find_map(|object| {
            if object.status != ObjectStatus::Normal
                || object.container.is_some()
                || object.ocf & ocf == 0
                || object.category & CATEGORY_MOUSE_IGNORE_FLAG != 0
                || cursor_layer.is_some_and(|layer| object.layer != layer)
                || !Self::object_is_visible(
                    &snapshot.objects,
                    &snapshot.players,
                    object,
                    owner,
                    false,
                )
            {
                return None;
            }
            self.object_pick_rect_for_viewport(object, viewport)
                .filter(|rect| rect_contains(*rect, point, 0.0))
                .map(|_| object.id)
        })
    }

    fn viewport_for_point(&self, point: GuiPoint) -> Option<&ActiveViewport> {
        self.active_viewports.iter().rev().find(|viewport| {
            let rect = viewport.content_rect;
            let left = rect.x as f32;
            let top = rect.y as f32;
            let right = left + rect.width as f32;
            let bottom = top + rect.height as f32;
            point.x >= left && point.x < right && point.y >= top && point.y < bottom
        })
    }

    /// Stores the HUD state drawn by [`Self::render_frame`] — the Rust
    /// counterpart of the per-frame data reads in `C4Viewport::DrawOverlay`
    /// (src/C4Viewport.cpp:835-882) and `C4UpperBoard::Execute`
    /// (src/C4UpperBoard.cpp:37-44).
    pub fn update_overlay(&mut self, overlay: &GraphicsOverlay<'_>) {
        self.hud_players = overlay.players.clone();
        self.game_time_seconds = overlay.game_time_seconds;
        self.message_board_line = overlay.message_board_line.clone();
        self.show_commands = overlay.show_commands;
        self.show_command_keys = overlay.show_command_keys;
        self.debug_hud_text = overlay
            .debug_hud
            .then(|| (overlay.frame_text.to_string(), overlay.status_text.to_string()));
    }

    /// Installs the CStdFont-faithful HUD fonts (FontRegular et al).
    pub fn set_clonk_fonts(&mut self, fonts: Option<Arc<ClonkFontSet>>) {
        self.clonk_fonts = fonts;
    }

    /// Installs the current scenario controls immediately, matching the
    /// explicit `ApplyGamma` at the end of `C4Game::Init` (C4Game.cpp:490).
    /// Runtime `SetGamma` changes continue through [`Self::render_frame`]'s
    /// draw-then-apply lifecycle.
    pub fn apply_gamma_now(&mut self, gamma: &GammaControlState) {
        self.active_gamma_control_points = Some(gamma.combined_control_points());
    }

    /// Returns the gamma ramp installed while the current frame is drawn.
    /// Callers that append GUI after [`Self::render_frame`] must capture this
    /// before rendering, because `render_frame` latches `pending` for the next
    /// pass at its tail just like `C4GraphicsSystem::Execute`
    /// (`src/C4GraphicsSystem.cpp:167-199`).
    pub fn active_gamma_ramp(&self, pending: &GammaControlState) -> lc_graphics::GammaRamp {
        lc_graphics::GammaRamp::from_control_points(
            self.active_gamma_control_points
                .unwrap_or_else(|| pending.combined_control_points()),
        )
    }

    pub fn render_frame(
        &mut self,
        snapshot: &SimulationSnapshot,
        viewports: &[ViewportInput<'_>],
    ) -> Vec<EngineSurfaceSnapshot> {
        let pending = snapshot.environment.gamma.combined_control_points();
        // C4Game::Init applies the initialization controls before the first
        // render (C4Game.cpp:490). Later SetGamma calls set fSetGamma during
        // simulation and C4GraphicsSystem::Execute applies them only after it
        // has drawn the current pass (C4GraphicsSystem.cpp:195-199).
        let gamma = self.active_gamma_ramp(&snapshot.environment.gamma);
        let snapshots = self.render_frame_with_gamma(snapshot, viewports, Some(&gamma));
        self.active_gamma_control_points = Some(pending);
        snapshots
    }

    /// Internal seam for C++ per-fragment gamma rendering and exact isolated
    /// fragment tests. Public rendering drives its active/pending lifecycle.
    fn render_frame_with_gamma(
        &mut self,
        snapshot: &SimulationSnapshot,
        viewports: &[ViewportInput<'_>],
        gamma: Option<&lc_graphics::GammaRamp>,
    ) -> Vec<EngineSurfaceSnapshot> {
        self.active_viewports.clear();
        if let Some(background) = self.hud_graphics.background.as_ref() {
            tile_image_on_surface(&mut self.surface, background, 0, 0, gamma);
        } else {
            self.surface.fill(Color::opaque(8, 12, 24));
        }

        let owner_colors = Self::collect_owner_colors(snapshot);
        self.render_viewports(snapshot, viewports, &owner_colors, gamma);

        self.draw_hud(snapshot.frame, gamma);

        self.collect_sprite_atlas(snapshot)
    }

    fn render_viewports(
        &mut self,
        snapshot: &SimulationSnapshot,
        viewports: &[ViewportInput<'_>],
        owner_colors: &HashMap<i32, Color>,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        if viewports.is_empty() {
            if let Some(object) = snapshot.objects.first() {
                let default = ViewportInput::from_focus(object);
                self.render_viewport(
                    snapshot,
                    &default,
                    0,
                    SurfaceRect::new(0, 0, self.surface_width, self.surface_height),
                    owner_colors,
                    gamma,
                );
            }
            return;
        }

        let layout = self.layout_viewports(viewports.len());
        let mut owner_slots = HashMap::<i32, usize>::new();
        for (input, rect) in viewports.iter().zip(layout.into_iter()) {
            let slot = owner_slots.entry(input.owner).or_default();
            let camera_slot = *slot;
            *slot += 1;
            self.render_viewport(snapshot, input, camera_slot, rect, owner_colors, gamma);
        }
    }

    fn render_viewport(
        &mut self,
        snapshot: &SimulationSnapshot,
        input: &ViewportInput<'_>,
        camera_slot: usize,
        rect: SurfaceRect,
        owner_colors: &HashMap<i32, Color>,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }

        let saved_surface_width = self.surface_width;
        let saved_surface_height = self.surface_height;
        let saved_viewport_x = self.viewport_x;
        let saved_viewport_y = self.viewport_y;
        let saved_viewport_zoom = self.viewport_zoom;
        let saved_world_width = self.world_width;
        let saved_world_height = self.world_height;

        self.surface_width = rect.width;
        self.surface_height = rect.height;
        self.update_world_dimensions(snapshot.landscape.as_ref());

        let rect = self.centered_viewport_rect(rect);
        let format = self.surface.format();
        let mut viewport_surface = Surface::new(rect.width, rect.height, format);
        if let Some(background) = self.hud_graphics.background.as_ref() {
            tile_image_on_surface(&mut viewport_surface, background, rect.x, rect.y, gamma);
        } else {
            viewport_surface.fill(Color::opaque(0, 0, 0));
        }
        self.surface_width = rect.width;
        self.surface_height = rect.height;

        let zoom = input.zoom.clamp(MIN_VIEWPORT_ZOOM, MAX_VIEWPORT_ZOOM);
        let world_width = self.world_width.max(1);
        let world_height = self.world_height.max(1);
        // C4Application::SetResolution converts physical output to the
        // logical viewport with ceilf(physical/scale) before SetOutputSize.
        let view_width = ((rect.width as f32 / zoom).ceil() as i32).max(1);
        let view_height = ((rect.height as f32 / zoom).ceil() as i32).max(1);

        let key = CameraKey {
            owner: input.owner,
            slot: camera_slot,
        };

        let state = self.camera_states.entry(key).or_insert_with(|| {
            CameraState::new(world_width, world_height, view_width, view_height)
        });
        let (view_x, view_y) = if input.owner == OWNER_NONE {
            state.no_owner_position(view_width, view_height, world_width, world_height)
        } else {
            state.update(
                input.center.x,
                input.center.y,
                view_width,
                view_height,
                world_width,
                world_height,
                VIEWPORT_SCROLL_BORDER,
                self.scroll_smooth,
            )
        };
        let view_x = view_x.saturating_add(input.offset.x);
        let view_y = view_y.saturating_add(input.offset.y);
        // C4Viewport keeps the full ViewWdt/Hgt and clips landscape drawing
        // around any out-of-map portion. Preserve the existing Rust
        // letterbox representation by turning those portions into tiled
        // margins and drawing only the in-world content surface.
        let border_left = (-view_x).max(0).min(view_width);
        let border_top = (-view_y).max(0).min(view_height);
        let border_right = (view_width - world_width + view_x)
            .max(0)
            .min(view_width - border_left);
        let border_bottom = (view_height - world_height + view_y)
            .max(0)
            .min(view_height - border_top);

        let offset_x = scaled_camera_border(border_left, zoom, rect.width) as i32;
        let offset_y = scaled_camera_border(border_top, zoom, rect.height) as i32;
        let right_pixels = scaled_camera_border(border_right, zoom, rect.width);
        let bottom_pixels = scaled_camera_border(border_bottom, zoom, rect.height);
        let content_width = rect
            .width
            .saturating_sub(offset_x as u32)
            .saturating_sub(right_pixels)
            .max(1);
        let content_height = rect
            .height
            .saturating_sub(offset_y as u32)
            .saturating_sub(bottom_pixels)
            .max(1);
        let origin_x = (view_x + border_left) as f32;
        let origin_y = (view_y + border_top) as f32;

        self.viewport_x = origin_x;
        self.viewport_y = origin_y;
        self.viewport_zoom = zoom;

        self.surface_width = content_width;
        self.surface_height = content_height;

        let content_surface = Surface::new(content_width.max(1), content_height.max(1), format);
        let main_surface = std::mem::replace(&mut self.surface, content_surface);

        let environment = &snapshot.environment;
        let events = &snapshot.weather_events;
        let lighting = Self::lighting_factor(environment.settings.time_of_day);

        self.draw_sky(snapshot.sky.as_ref(), environment, events, lighting, gamma);
        // C4D_Background objects live in Game.BackObjects and draw between
        // sky and landscape (C4Viewport.cpp:1051-1063).
        self.draw_objects(
            &snapshot.objects,
            &snapshot.render_order,
            &snapshot.players,
            input.owner,
            lighting,
            owner_colors,
            ObjectRenderPass::Background,
            gamma,
        );
        let textured_landscape = self.draw_ground(
            environment.ambient_temperature,
            snapshot.landscape.as_ref(),
            lighting,
            gamma,
        );
        // C4Landscape::Draw presents the material-colored Surface32 once and
        // supplies a separate alpha-only liquid-animation mask to
        // BlitLandscape (C4Landscape.cpp:261-270,2599-2616). The scalar
        // repaint below predates the raster renderer and remains only for
        // column-only fixture worlds that have no Surface8 equivalent.
        if !textured_landscape {
            self.draw_liquids(
                environment.ambient_temperature,
                snapshot.landscape.as_ref(),
                lighting,
                gamma,
            );
        }
        // C4Viewport draws sync-relevant C4PXS after the landscape and before
        // objects. Weather precipitation reaches this same path after the
        // simulation creates rain/snow PXS; there is no procedural viewport
        // rain layer (C4Viewport.cpp:1056-1078; C4PXS.cpp:242-307).
        self.draw_pxs(&snapshot.particles, lighting, gamma);
        self.draw_objects(
            &snapshot.objects,
            &snapshot.render_order,
            &snapshot.players,
            input.owner,
            lighting,
            owner_colors,
            ObjectRenderPass::Normal,
            gamma,
        );
        self.draw_objects(
            &snapshot.objects,
            &snapshot.render_order,
            &snapshot.players,
            input.owner,
            lighting,
            owner_colors,
            ObjectRenderPass::ForegroundNonParallax,
            gamma,
        );
        // C4Object::Draw attaches no energy/magic bars to world objects —
        // energy presentation lives in the HUD corner (DrawCursorInfo,
        // src/C4Viewport.cpp:920-945). The world-space fctEnergy bolt only
        // blinks (`Tick35 > 12`) over NeedEnergy structures
        // (src/C4Object.cpp:2505-2510); NeedEnergy is not modeled in the
        // Rust engine yet, so nothing is drawn here.
        let highlight_ids = Self::collect_highlight_ids(snapshot, input.owner, input.focus.id);
        self.draw_selection_marks(
            snapshot,
            &highlight_ids,
            input.owner,
            origin_x,
            origin_y,
            zoom,
            gamma,
        );
        self.draw_player_cursors(snapshot, input.owner, origin_x, origin_y, zoom, gamma);
        self.draw_objects(
            &snapshot.objects,
            &snapshot.render_order,
            &snapshot.players,
            input.owner,
            lighting,
            owner_colors,
            ObjectRenderPass::ForegroundParallax,
            gamma,
        );

        let content_surface = std::mem::replace(&mut self.surface, main_surface);

        self.surface_width = saved_surface_width;
        self.surface_height = saved_surface_height;
        self.viewport_x = saved_viewport_x;
        self.viewport_y = saved_viewport_y;
        self.viewport_zoom = saved_viewport_zoom;
        self.world_width = saved_world_width;
        self.world_height = saved_world_height;

        blit_surface(&mut viewport_surface, &content_surface, offset_x, offset_y);
        blit_surface(&mut self.surface, &viewport_surface, rect.x, rect.y);

        self.active_viewports.push(ActiveViewport {
            owner: input.owner,
            focus: input.focus.id,
            rect,
            content_rect: SurfaceRect::new(
                rect.x + offset_x,
                rect.y + offset_y,
                content_width,
                content_height,
            ),
            viewport_x: origin_x,
            viewport_y: origin_y,
            zoom,
        });
    }

    /// `C4GraphicsSystem::RecalculateViewports` caps fullscreen viewport
    /// output to the landscape plus the two scroll borders and centers the
    /// result inside its layout cell (src/C4GraphicsSystem.cpp:384-396).
    fn centered_viewport_rect(&self, area: SurfaceRect) -> SurfaceRect {
        let border = VIEWPORT_SCROLL_BORDER.saturating_mul(2);
        let max_width = self.world_width.max(1).saturating_add(border) as u32;
        let max_height = self.world_height.max(1).saturating_add(border) as u32;
        let width = area.width.min(max_width);
        let height = area.height.min(max_height);
        SurfaceRect::new(
            area.x + area.width.saturating_sub(width) as i32 / 2,
            area.y + area.height.saturating_sub(height) as i32 / 2,
            width,
            height,
        )
    }

    fn collect_highlight_ids(
        snapshot: &SimulationSnapshot,
        owner: i32,
        focus: ObjectId,
    ) -> HashSet<ObjectId> {
        let mut highlights: HashSet<ObjectId> = HashSet::new();
        highlights.insert(focus);
        if let Some(selection) = snapshot.crew_selection.get(&owner) {
            if let Some(cursor) = selection.cursor {
                highlights.insert(cursor);
            }
            highlights.extend(selection.selected.iter().copied());
        }
        if let Some(state) = snapshot.players.iter().find(|state| state.id == owner) {
            if let Some(cursor) = state.cursor {
                highlights.insert(cursor);
            }
        }
        for player in &snapshot.hud.players {
            if player.owner == owner {
                if let Some(focus_id) = player.focus {
                    highlights.insert(focus_id);
                }
            }
        }
        highlights
    }

    /// `C4Object::DrawSelectMark` (src/C4Object.cpp:3839-3857): the four
    /// PHASES of fctSelectMark (square cells of sheet height) sit at the
    /// shape corners offset by -2. Gated on the owning player's SelectFlash
    /// (src/C4Object.cpp:2497-2502).
    fn draw_selection_marks(
        &mut self,
        snapshot: &SimulationSnapshot,
        highlights: &HashSet<ObjectId>,
        owner: i32,
        origin_x: f32,
        origin_y: f32,
        zoom: f32,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        let Some(image) = self.hud_graphics.select_mark.clone() else {
            return;
        };
        // `Game.Players.Get(Owner)->SelectFlash` (src/C4Object.cpp:2501);
        // fixture snapshots without player entries keep the marks visible.
        if snapshot
            .players
            .iter()
            .find(|player| player.id == owner)
            .map(|player| player.control.select_flash <= 0)
            .unwrap_or(false)
        {
            return;
        }
        let cell = image.height() as i32;
        let surface_width = self.surface_width as f32;
        let surface_height = self.surface_height as f32;
        let margin = (cell as f32).max(16.0);
        for id in highlights {
            let Some(object) = snapshot.object(*id) else {
                continue;
            };
            let screen_x = (object.position.x as f32 - origin_x) * zoom;
            let screen_y = (object.position.y as f32 - origin_y) * zoom;
            if screen_x < -margin
                || screen_x > surface_width + margin
                || screen_y < -margin
                || screen_y > surface_height + margin
            {
                continue;
            }

            let shape = self
                .object_sprites
                .get(&sprite_map_key(&object.definition_id, None))
                .map(Self::sprite_def_shape)
                .filter(|shape| shape.width > 0 && shape.height > 0)
                .unwrap_or_else(|| DefinitionRect::new(-6, -6, 12, 12));
            // cox/coy = x + Shape.x - 2 (src/C4Object.cpp:3850-3856).
            let cox = screen_x + (shape.x as f32) * zoom - 2.0;
            let coy = screen_y + (shape.y as f32) * zoom - 2.0;
            let shape_width = shape.width as f32 * zoom;
            let shape_height = shape.height as f32 * zoom;
            let corners = [
                (cox, coy, 0),
                (cox + shape_width, coy, 1),
                (cox, coy + shape_height, 2),
                (cox + shape_width, coy + shape_height, 3),
            ];
            for (px, py, phase) in corners {
                let source = SourceRect::new(phase * cell, 0, cell, cell);
                if !Self::source_within_image(&image, &source) {
                    continue;
                }
                let rect = GuiRect::from_origin_size(
                    GuiPoint::new(px, py),
                    GuiSize::new(cell as f32, cell as f32),
                );
                draw_image_region(
                    &mut self.surface,
                    &rect,
                    &image,
                    None,
                    &source,
                    false,
                    None,
                    SpriteBlitState::normal(),
                    gamma,
                );
            }
        }
    }

    fn layout_viewports(&self, count: usize) -> Vec<SurfaceRect> {
        if count == 0 {
            return Vec::new();
        }

        // Viewport area between the upper board and the message board
        // (C4GraphicsSystem::RecalculateViewports,
        // src/C4GraphicsSystem.cpp:343-348).
        let chrome = self.hud_chrome_active();
        let mut overlay_height = if chrome {
            hud::UPPER_BOARD_HEIGHT.clamp(0, self.surface_height as i32)
        } else {
            0
        };
        let board_height = if chrome {
            self.message_board_height()
                .clamp(0, self.surface_height as i32)
        } else {
            0
        };
        let mut available_height = (self.surface_height as i32)
            .saturating_sub(overlay_height)
            .saturating_sub(board_height);
        if available_height <= 0 {
            // Surface too small to host the overlay and a viewport. Give the
            // entire surface to the viewport and suppress the overlay instead
            // of producing a zero-height viewport that won't render anything.
            overlay_height = 0;
            available_height = self.surface_height as i32;
        }
        if available_height <= 0 {
            return vec![SurfaceRect::new(
                0,
                overlay_height,
                self.surface_width,
                available_height.max(0) as u32,
            )];
        }

        // C4GraphicsSystem::RecalculateViewports uses floor(sqrt(count)) rows.
        // Any remainder adds one column to the first rows; pixel remainders
        // from the integer cell divisions stay available for the background.
        let rows = ((count as f32).sqrt() as usize).max(1);
        let base_columns = count / rows;
        let longer_rows = count % rows;
        let available_width = self.surface_width;
        let row_height = available_height as u32 / rows as u32;
        let mut rects = Vec::with_capacity(count);
        for row in 0..rows {
            let columns = base_columns + usize::from(row < longer_rows);
            let column_width = available_width / columns as u32;
            for col in 0..columns {
                // Graphics.SplitscreenDividers defaults to enabled. C++ takes
                // four pixels only from non-last cells, leaving no outer inset.
                let divider_width = if col + 1 < columns { 4 } else { 0 };
                let divider_height = if row + 1 < rows { 4 } else { 0 };
                rects.push(SurfaceRect::new(
                    (col as u32 * column_width) as i32,
                    overlay_height + (row as i32 * row_height as i32),
                    column_width.saturating_sub(divider_width),
                    row_height.saturating_sub(divider_height),
                ));
            }
        }

        rects
    }

    pub fn ground_height_at(&self, landscape: Option<&Landscape>, x: i32) -> i32 {
        let clamped_x = if self.world_width > 0 {
            x.clamp(0, self.world_width.saturating_sub(1))
        } else {
            x
        };
        self.surface_height_at(landscape, clamped_x)
            .unwrap_or(self.fallback_ground_height)
    }

    fn update_world_dimensions(&mut self, landscape: Option<&Landscape>) {
        if let Some(landscape) = landscape {
            let width = landscape.width() as i32;
            if width > 0 {
                self.world_width = width;
            }

            self.world_height = landscape.estimated_height().max(1);
        } else {
            if self.world_width <= 0 {
                self.world_width = self.surface_width as i32;
            }
            if self.world_height <= 0 {
                self.world_height = self
                    .fallback_ground_height
                    .max(self.surface_height as i32)
                    .max(1);
            }
        }
    }

    fn draw_sky(
        &mut self,
        frame: Option<&SkyFrame>,
        environment: &EnvironmentFrame,
        events: &[WeatherEvent],
        lighting: f32,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        if let Some(state) = self.sky.clone() {
            self.render_configured_sky(&state, frame, events, lighting, gamma);
        } else {
            let base = environment
                .sky_color
                .map(|color| Color::opaque(color.r, color.g, color.b))
                .unwrap_or_else(|| {
                    Self::sky_color_for_temperature(environment.ambient_temperature)
                });
            let tinted = Self::apply_lighting(base, lighting);
            self.surface.fill(
                gamma.map_or(tinted, |gamma| gamma_encode_fragment(tinted, gamma)),
            );
        }
    }

    fn render_configured_sky(
        &mut self,
        state: &SkyRenderState,
        frame: Option<&SkyFrame>,
        _events: &[WeatherEvent],
        lighting: f32,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        let settings = frame
            .map(|frame| &frame.settings)
            .unwrap_or(&state.settings);

        if let Some(color) = settings.back_color {
            let base = Self::bgr_to_color(color);
            let tinted = Self::apply_lighting(base, lighting);
            self.surface.fill(
                gamma.map_or(tinted, |gamma| gamma_encode_fragment(tinted, gamma)),
            );
        } else if !settings.has_surface {
            self.fill_sky_gradient(settings, lighting, gamma);
        } else {
            self.surface.fill(Color::opaque(0, 0, 0));
        }

        if settings.has_surface {
            if let Some(image) = state.image() {
                self.tile_sky_image(image, settings, frame, lighting, gamma);
            } else {
                self.fill_sky_gradient(settings, lighting, gamma);
            }
        } else if settings.back_color.is_none() {
            self.fill_sky_gradient(settings, lighting, gamma);
        }
    }

    fn fill_sky_gradient(
        &mut self,
        settings: &SkySettings,
        lighting: f32,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        // C4Sky::Draw without a surface fades from GetSkyFadeClr(TargetY)
        // to GetSkyFadeClr(TargetY+Hgt) (C4Sky.cpp:219-225): the fade spans
        // the landscape height in world coordinates, offset by the
        // viewport origin — not merely the visible window.
        let zoom = if self.viewport_zoom > 0.0 {
            self.viewport_zoom
        } else {
            1.0
        };
        let view_top = self.viewport_y.round() as i32;
        let view_bottom = (self.viewport_y + self.surface_height as f32 / zoom).round() as i32;
        let top = Self::sky_fade_color(settings, view_top, self.world_height);
        let bottom = Self::sky_fade_color(settings, view_bottom, self.world_height);
        let top = Color::opaque(top.r, top.g, top.b);
        let bottom = Color::opaque(bottom.r, bottom.g, bottom.b);
        self.fill_vertical_gradient(top, bottom, lighting, gamma);
    }

    /// C4Sky::GetSkyFadeClr (C4Sky.cpp:230-236): integer fade between
    /// FadeClr1 (world top) and FadeClr2 across the landscape height —
    /// iPos2 = iY*256/GBackHgt, channel = (c1*iPos1 + c2*iPos2) >> 8.
    /// C++ never sees out-of-landscape Y (the viewport is clamped); the
    /// clamp here keeps stray coordinates from wrapping the fixed-point mix.
    fn sky_fade_color(settings: &SkySettings, world_y: i32, world_height: i32) -> RgbColor {
        let height = world_height.max(1);
        let pos2 = (world_y * 256 / height).clamp(0, 256);
        let pos1 = 256 - pos2;
        let channel =
            |c1: u8, c2: u8| ((i32::from(c1) * pos1 + i32::from(c2) * pos2) >> 8).clamp(0, 255) as u8;
        RgbColor::new(
            channel(settings.fade_top.r, settings.fade_bottom.r),
            channel(settings.fade_top.g, settings.fade_bottom.g),
            channel(settings.fade_top.b, settings.fade_bottom.b),
        )
    }

    fn fill_vertical_gradient(
        &mut self,
        top: Color,
        bottom: Color,
        lighting: f32,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        if self.surface_width == 0 || self.surface_height == 0 {
            return;
        }
        let height = self.surface_height.saturating_sub(1).max(1);
        for y in 0..self.surface_height {
            let t = y as f32 / height as f32;
            let blended = Self::lerp_color(top, bottom, t);
            let tinted = Self::apply_lighting(blended, lighting);
            let tinted = gamma.map_or(tinted, |gamma| gamma_encode_fragment(tinted, gamma));
            for x in 0..self.surface_width {
                let _ = self.surface.set_pixel(x, y, tinted);
            }
        }
    }

    fn tile_sky_image(
        &mut self,
        image: &ImageData,
        settings: &SkySettings,
        frame: Option<&SkyFrame>,
        lighting: f32,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        let width = image.width();
        let height = image.height();
        if width == 0 || height == 0 {
            return;
        }
        let width_f = width as f32;
        let height_f = height as f32;
        let runtime_x = frame.map(|frame| frame.offset_x).unwrap_or(0.0);
        let runtime_y = frame.map(|frame| frame.offset_y).unwrap_or(0.0);
        let parallax_x = if settings.parallax_x == 0 {
            10
        } else {
            settings.parallax_x
        };
        let parallax_y = if settings.parallax_y == 0 {
            10
        } else {
            settings.parallax_y
        };
        let source_x = (self.viewport_x * 10.0 / parallax_x as f32) - runtime_x;
        let source_y = (self.viewport_y * 10.0 / parallax_y as f32) - runtime_y;
        let offset_x = Self::normalize_offset(source_x, width_f);
        let offset_y = Self::normalize_offset(source_y, height_f);
        let modulation = settings.modulation;

        let mut y = -offset_y;
        while y < self.surface_height as f32 {
            let mut x = -offset_x;
            while x < self.surface_width as f32 {
                self.blit_sky_tile(
                    image,
                    x.round() as i32,
                    y.round() as i32,
                    modulation,
                    lighting,
                    gamma,
                );
                x += width_f;
            }
            y += height_f;
        }
    }

    fn blit_sky_tile(
        &mut self,
        image: &ImageData,
        dest_x: i32,
        dest_y: i32,
        modulation: Option<u32>,
        lighting: f32,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        let width = image.width();
        let height = image.height();
        let pixels = image.pixels();
        for y in 0..height {
            let target_y = dest_y + y as i32;
            if target_y < 0 || target_y >= self.surface_height as i32 {
                continue;
            }
            for x in 0..width {
                let target_x = dest_x + x as i32;
                if target_x < 0 || target_x >= self.surface_width as i32 {
                    continue;
                }
                let idx = ((y * width + x) * 4) as usize;
                if idx + 3 >= pixels.len() {
                    continue;
                }
                let mut color = Color::new(
                    pixels[idx],
                    pixels[idx + 1],
                    pixels[idx + 2],
                    pixels[idx + 3],
                );
                if color.a == 0 {
                    continue;
                }
                if let Some(modulation) = modulation {
                    color = Self::apply_modulation(color, modulation);
                }
                color = color.modulate(lighting);
                if let Some(gamma) = gamma {
                    let background = self
                        .surface
                        .get_pixel(target_x as u32, target_y as u32)
                        .unwrap_or_default();
                    let blended = gamma_blend_fragment_over(color, background, gamma);
                    let _ = self
                        .surface
                        .set_pixel(target_x as u32, target_y as u32, blended);
                } else if color.a == 255 {
                    let _ = self
                        .surface
                        .set_pixel(target_x as u32, target_y as u32, color);
                } else {
                    let background = self
                        .surface
                        .get_pixel(target_x as u32, target_y as u32)
                        .unwrap_or_default();
                    let blended = blend_color_over(color, background);
                    let _ = self
                        .surface
                        .set_pixel(target_x as u32, target_y as u32, blended);
                }
            }
        }
    }

    fn normalize_offset(offset: f32, dimension: f32) -> f32 {
        if dimension <= 0.0 {
            return 0.0;
        }
        let mut wrapped = offset % dimension;
        if wrapped < 0.0 {
            wrapped += dimension;
        }
        wrapped
    }

    fn lerp_color(a: Color, b: Color, t: f32) -> Color {
        let clamped = t.clamp(0.0, 1.0);
        let lerp_channel = |start: u8, end: u8| -> u8 {
            let start = start as f32;
            let end = end as f32;
            (start + (end - start) * clamped).round().clamp(0.0, 255.0) as u8
        };
        Color::new(
            lerp_channel(a.r, b.r),
            lerp_channel(a.g, b.g),
            lerp_channel(a.b, b.b),
            255,
        )
    }

    fn bgr_to_color(value: u32) -> Color {
        let r = ((value >> 16) & 0xff) as u8;
        let g = ((value >> 8) & 0xff) as u8;
        let b = (value & 0xff) as u8;
        Color::opaque(r, g, b)
    }

    fn apply_modulation(color: Color, modulation: u32) -> Color {
        let mod_r = ((modulation >> 16) & 0xff) as u8;
        let mod_g = ((modulation >> 8) & 0xff) as u8;
        let mod_b = (modulation & 0xff) as u8;
        let r = ((color.r as u16 * mod_r as u16) / 255) as u8;
        let g = ((color.g as u16 * mod_g as u16) / 255) as u8;
        let b = ((color.b as u16 * mod_b as u16) / 255) as u8;
        Color::new(r, g, b, color.a)
    }

    fn draw_pxs(
        &mut self,
        particles: &[ParticleSnapshot],
        lighting: f32,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        // C4PXSSystem::Draw is deliberately two-pass: every old-style
        // pixel/velocity line first, then every material sprite. Thus a
        // graphical PXS overlays every old-style PXS regardless of slot
        // order (C4PXS.cpp:248-307).
        for particle in particles {
            if !self.pxs_visible(particle) {
                continue;
            }
            let Some(material_name) = particle
                .definition_id
                .strip_prefix("material/pxs/")
                .map(str::to_ascii_lowercase)
            else {
                continue;
            };
            let Some(material) = self.material_render_info.get(&material_name) else {
                continue;
            };
            if self.pxs_graphics(material).is_some() {
                continue;
            }
            let material = material.clone();
            self.draw_old_style_pxs(particle, &material, lighting, gamma);
        }

        let mut compacted_slot = 0u32;
        for particle in particles {
            let Some(material_name) = particle
                .definition_id
                .strip_prefix("material/pxs/")
                .map(str::to_ascii_lowercase)
            else {
                continue;
            };
            let fallback_slot = compacted_slot;
            compacted_slot = compacted_slot.wrapping_add(1);
            if !self.pxs_visible(particle) {
                continue;
            }
            let Some(material) = self.material_render_info.get(&material_name).cloned() else {
                continue;
            };
            let Some((texture, rect)) = self
                .pxs_graphics(&material)
                .map(|(texture, rect)| (texture.clone(), rect))
            else {
                continue;
            };
            let slot = particle.pxs_slot.unwrap_or(fallback_slot) as usize % 500;
            self.draw_graphical_pxs(particle, &material, &texture, rect, slot, lighting, gamma);
        }
    }

    fn pxs_visible(&self, particle: &ParticleSnapshot) -> bool {
        // VisibleRect is the world target rectangle enlarged by 20 and tests
        // the CURRENT fixtoi position before either pass. It intentionally
        // does not draw a long velocity line merely because that line crosses
        // the viewport (C4PXS.cpp:245-259,283-288).
        let [x, y, _, _] = Self::pxs_fixed(particle);
        let x = lc_engine::math::fixtoi(x);
        let y = lc_engine::math::fixtoi(y);
        let zoom = self.viewport_zoom.max(f32::EPSILON);
        let left = self.viewport_x.floor() as i32 - 20;
        let top = self.viewport_y.floor() as i32 - 20;
        let width = (self.surface.width() as f32 / zoom).ceil() as i32 + 40;
        let height = (self.surface.height() as f32 / zoom).ceil() as i32 + 40;
        x >= left && x < left + width && y >= top && y < top + height
    }

    fn pxs_graphics(
        &self,
        material: &MaterialRenderInfo,
    ) -> Option<(&ImageData, [i32; 6])> {
        let rect = material.pxs_gfx_rect;
        if rect[2] <= 0 || rect[3] <= 0 || material.pxs_gfx_size <= 0 {
            return None;
        }
        material
            .pxs_gfx
            .as_deref()
            .and_then(|name| self.material_textures.get(&name.to_ascii_lowercase()))
            .map(|texture| (texture, rect))
    }

    fn pxs_fixed(particle: &ParticleSnapshot) -> [lc_engine::math::C4Fixed; 4] {
        particle.pxs_fixed.map_or_else(
            || {
                [
                    lc_engine::math::ftofix(particle.position.x),
                    lc_engine::math::ftofix(particle.position.y),
                    lc_engine::math::ftofix(particle.velocity.x),
                    lc_engine::math::ftofix(particle.velocity.y),
                ]
            },
            |raw| raw.map(lc_engine::math::C4Fixed::from_raw),
        )
    }

    fn draw_old_style_pxs(
        &mut self,
        particle: &ParticleSnapshot,
        material: &MaterialRenderInfo,
        lighting: f32,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        let [x, y, xdir, ydir] = Self::pxs_fixed(particle);
        let moving = lc_engine::math::fixtoi(xdir) != 0 || lc_engine::math::fixtoi(ydir) != 0;
        let mut transparency = i32::from(material.alpha[0]);
        if moving {
            let len = lc_engine::math::fixtoi(xdir.abs() + ydir.abs()).max(1);
            transparency = transparency.max(195 - (195 - transparency) / len);
        }
        let color = Color::new(
            material.color[0],
            material.color[1],
            material.color[2],
            255u8.saturating_sub(transparency as u8),
        )
        .modulate(lighting);
        let screen = |wx: f32, wy: f32| {
            (
                (wx - self.viewport_x) * self.viewport_zoom,
                (wy - self.viewport_y) * self.viewport_zoom,
            )
        };
        let end = screen(x.to_float(), y.to_float());
        if moving {
            let start = screen((x - xdir).to_float(), (y - ydir).to_float());
            draw_pxs_line(&mut self.surface, start, end, color, gamma);
        } else {
            draw_pxs_pixel(
                &mut self.surface,
                end.0.round() as i32,
                end.1.round() as i32,
                color,
                gamma,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_graphical_pxs(
        &mut self,
        particle: &ParticleSnapshot,
        material: &MaterialRenderInfo,
        texture: &ImageData,
        rect: [i32; 6],
        slot: usize,
        lighting: f32,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        let [x, y, _, _] = Self::pxs_fixed(particle);
        let facet_width = rect[2];
        let facet_height = rect[3];
        let phases_x = texture.width() as i32 / facet_width;
        let phases_y = texture.height() as i32 / facet_height;
        if phases_x <= 0 || phases_y <= 0 {
            self.draw_old_style_pxs(particle, material, lighting, gamma);
            return;
        }
        let phase_count = (phases_x * phases_y).max(1) as usize;
        let z = 1
            + (((slot / phase_count) ^ 341) % material.pxs_gfx_size as usize) as i32;
        let phase_x = (slot % phases_x as usize) as i32;
        let phase_y = ((slot / phases_x as usize) % phases_y as usize) as i32;
        let world_x = lc_engine::math::fixtoi(x) + z * rect[4] / facet_width;
        let world_y = lc_engine::math::fixtoi(y) + z * rect[5] / facet_width;
        let draw_height = z * facet_height / facet_width;
        if draw_height <= 0 {
            return;
        }
        let target = GuiRect::from_origin_size(
            GuiPoint::new(
                (world_x as f32 - self.viewport_x) * self.viewport_zoom,
                (world_y as f32 - self.viewport_y) * self.viewport_zoom,
            ),
            GuiSize::new(
                z as f32 * self.viewport_zoom,
                draw_height as f32 * self.viewport_zoom,
            ),
        );
        let source = SourceRect::new(
            rect[0] + facet_width * phase_x,
            rect[1] + facet_height * phase_y,
            facet_width,
            facet_height,
        );
        let facet_third = (facet_width / 3).max(1);
        // C++ stores transparency in the high byte. The signed expression
        // intentionally narrows to that byte after its <=255 cap
        // (C4PXS.cpp:300-304; StdGL.cpp:437-469).
        let modulation_transparency = ((facet_third - z) * 16).min(255) as u8;
        draw_pxs_image_region(
            &mut self.surface,
            &target,
            texture,
            &source,
            modulation_transparency,
            lighting,
            gamma,
        );
    }

    /// Per-pixel landscape rendering from the sim plane: every pixel
    /// byte samples its texmap texture png tiled by WORLD coordinates —
    /// the same composition C4Landscape::MapToSurface bakes into
    /// Surface32. Returns false when no plane/textures exist (legacy
    /// column painter takes over).
    fn draw_ground_textured(
        &mut self,
        landscape: Option<&Landscape>,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) -> bool {
        let Some(grid) = landscape.and_then(|landscape| landscape.pixel_grid()) else {
            return false;
        };
        if self.material_textures.is_empty() || self.material_render_info.is_empty() {
            return false;
        }
        let revision = grid.revision();
        let rebuild = self
            .landscape_cache
            .as_ref()
            .map(|(cached, _)| *cached != revision)
            .unwrap_or(true);
        if rebuild {
            let width = grid.width();
            let height = grid.height();
            let bytes = grid.bytes();
            let textures = grid.texture_names();
            let materials = grid.material_names();
            // Per texmap slot: C4TexMapEntry's primary pattern plus the
            // material's secondary pattern.
            enum Slot<'a> {
                Empty,
                Patterns {
                    material: &'a MaterialRenderInfo,
                    texture: &'a ImageData,
                    overlay: Option<&'a ImageData>,
                },
            }
            let slots: Vec<Slot> = (0..128usize)
                .map(|index| {
                    let Some(material) = materials
                        .get(index)
                        .and_then(|name| name.as_deref())
                        .and_then(|name| {
                            self.material_render_info
                                .get(&name.to_ascii_lowercase())
                        })
                    else {
                        return Slot::Empty;
                    };
                    let resolve_texture = |name: &str| {
                        let name = if (25..50).contains(&material.density)
                            && name.eq_ignore_ascii_case("Smooth")
                        {
                            "liquid".to_string()
                        } else {
                            name.to_ascii_lowercase()
                        };
                        self.material_textures.get(&name)
                    };
                    let Some(texture) = textures
                        .get(index)
                        .and_then(|name| name.as_deref())
                        .and_then(resolve_texture)
                    else {
                        return Slot::Empty;
                    };
                    let overlay_name = material
                        .texture_overlay
                        .as_deref()
                        .filter(|name| {
                            self.material_textures
                                .contains_key(&name.to_ascii_lowercase())
                        })
                        .unwrap_or("Smooth");
                    Slot::Patterns {
                        material,
                        texture,
                        overlay: resolve_texture(overlay_name),
                    }
                })
                .collect();
            let mut pixels = vec![0u8; width as usize * height as usize * 4];
            for y in 0..height as usize {
                for x in 0..width as usize {
                    let byte = bytes[y * width as usize + x];
                    // Pixel zero is sky. C4Landscape::GetClrByTex only
                    // applies material patterns when `pix` is nonzero
                    // (C4Landscape.cpp:2622-2632).
                    if byte == 0 {
                        continue;
                    }
                    let index = (byte & 0x7f) as usize;
                    let out = (y * width as usize + x) * 4;
                    match &slots[index] {
                        Slot::Empty => {}
                        Slot::Patterns {
                            material,
                            texture,
                            overlay,
                        } => {
                            let color = compose_material_pixel(
                                material,
                                byte,
                                x as i32,
                                y as i32,
                                texture,
                                *overlay,
                            );
                            pixels[out..out + 4]
                                .copy_from_slice(&[color.r, color.g, color.b, color.a]);
                        }
                    }
                }
            }
            self.landscape_cache = Some((revision, ImageData::new(width, height, pixels)));
        }
        let Some((_, cache)) = &self.landscape_cache else {
            return false;
        };
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let cache_width = cache.width() as i32;
        let cache_height = cache.height() as i32;
        let cache_pixels = cache.pixels();
        for screen_y in 0..self.surface_height {
            let world_y = (self.viewport_y + (screen_y as f32 + 0.5) / zoom).floor() as i32;
            if world_y < 0 || world_y >= cache_height {
                continue;
            }
            for screen_x in 0..self.surface_width {
                let world_x = (self.viewport_x + (screen_x as f32 + 0.5) / zoom).floor() as i32;
                if world_x < 0 || world_x >= cache_width {
                    continue;
                }
                let src = ((world_y * cache_width + world_x) * 4) as usize;
                if cache_pixels[src + 3] == 0 {
                    continue;
                }
                let color = Color::new(
                    cache_pixels[src],
                    cache_pixels[src + 1],
                    cache_pixels[src + 2],
                    cache_pixels[src + 3],
                );
                if let Some(gamma) = gamma {
                    let destination = self
                        .surface
                        .get_pixel(screen_x, screen_y)
                        .unwrap_or_default();
                    let blended = gamma_blend_fragment_over(color, destination, gamma);
                    let _ = self.surface.set_pixel(screen_x, screen_y, blended);
                } else {
                    let _ = self.surface.blend_pixel(screen_x, screen_y, color);
                }
            }
        }
        true
    }

    fn draw_ground(
        &mut self,
        ambient_temperature: i32,
        landscape: Option<&Landscape>,
        lighting: f32,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) -> bool {
        if self.draw_ground_textured(landscape, gamma) {
            return true;
        }
        let ground_color = Self::apply_lighting(
            Self::ground_color_for_temperature(ambient_temperature),
            lighting,
        );
        let ground_color =
            gamma.map_or(ground_color, |gamma| gamma_encode_fragment(ground_color, gamma));
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let surface_height = self.surface_height as i32;
        let max_world_x = self.world_width.saturating_sub(1).max(0);
        for screen_x in 0..self.surface_width {
            let pixel_center = screen_x as f32 + 0.5;
            let world_x = self.viewport_x + pixel_center / zoom;
            let world_x_index = world_x.floor() as i32;
            let world_x_index = world_x_index.clamp(0, max_world_x);
            let ground_world = self.ground_height_at(landscape, world_x_index);
            let mut ground_screen = ((ground_world as f32 - self.viewport_y) * zoom).round() as i32;
            if ground_screen < 0 {
                ground_screen = 0;
            }
            if ground_screen >= surface_height {
                continue;
            }
            for y in ground_screen..surface_height {
                let _ = self.surface.set_pixel(screen_x, y as u32, ground_color);
            }
        }
        false
    }

    fn draw_liquids(
        &mut self,
        ambient_temperature: i32,
        landscape: Option<&Landscape>,
        lighting: f32,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        let Some(landscape) = landscape else {
            return;
        };
        if landscape.liquids().is_empty() {
            return;
        }

        let base_color = Self::apply_lighting(
            Self::liquid_color_for_temperature(ambient_temperature),
            lighting,
        );

        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let surface_width = self.surface_width as i32;
        let surface_height = self.surface_height as i32;

        for (world_x, column) in landscape.liquids().iter().enumerate() {
            if column.segments().is_empty() {
                continue;
            }

            let screen_x = ((world_x as f32 - self.viewport_x) * zoom).round() as i32;
            if screen_x < 0 || screen_x >= surface_width {
                continue;
            }

            for segment in column.segments() {
                let mut start = ((segment.top as f32 - self.viewport_y) * zoom).round() as i32;
                let mut end = ((segment.bottom as f32 - self.viewport_y) * zoom).round() as i32;
                if start > end {
                    std::mem::swap(&mut start, &mut end);
                }
                if end < 0 || start >= surface_height {
                    continue;
                }
                start = start.max(0);
                end = end.min(surface_height - 1);

                for screen_y in start..=end {
                    let x = screen_x as u32;
                    let y = screen_y as u32;
                    let blended = match (self.surface.get_pixel(x, y), gamma) {
                        (Some(existing), Some(gamma)) => {
                            gamma_blend_fragment_over(base_color, existing, gamma)
                        }
                        (Some(existing), None) => blend_color_over(base_color, existing),
                        (None, Some(gamma)) => gamma_encode_fragment(base_color, gamma),
                        (None, None) => base_color,
                    };
                    let _ = self.surface.set_pixel(x, y, blended);
                }
            }
        }
    }

    /// C4Object::IsVisible (src/C4Object.cpp:5600-5629). This is shared by
    /// rendering and FindVisObject-style mouse picking so hidden HUD helpers,
    /// spell targets, and layer-gated objects cannot leak through either path.
    fn object_is_visible(
        objects: &[ObjectSnapshot],
        players: &[PlayerState],
        object: &ObjectSnapshot,
        for_player: i32,
        as_overlay: bool,
    ) -> bool {
        object_visible_for_player(objects, players, object, for_player, as_overlay)
    }

    /// C4Object::TargetPos / ApplyParallaxity
    /// (src/C4Object.h:377-380; C4Object.cpp:5800-5814). The viewport target
    /// and extent are logical pixels even when the output surface is scaled.
    fn object_target_position(&self, object: &ObjectSnapshot) -> (f32, f32) {
        if object.category & CATEGORY_PARALLAX_FLAG == 0 {
            return (self.viewport_x, self.viewport_y);
        }
        let local = |name| {
            object
                .local_vars
                .get(name)
                .and_then(|value| value.as_c4_int())
                .unwrap_or(0)
        };
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let width = ((self.surface_width as f32 / zoom).ceil() as i32).max(1);
        let height = ((self.surface_height as f32 / zoom).ceil() as i32).max(1);
        let apply = |target: f32, parallax: i32, position: i32, extent: i32| {
            if parallax == 0 && position < 0 {
                -extent
            } else {
                (target as i32).wrapping_mul(parallax) / 100
            }
        };
        (
            apply(
                self.viewport_x,
                local("__local_0"),
                object.position.x,
                width,
            ) as f32,
            apply(
                self.viewport_y,
                local("__local_1"),
                object.position.y,
                height,
            ) as f32,
        )
    }

    fn draw_objects(
        &mut self,
        objects: &[ObjectSnapshot],
        render_order: &[ObjectId],
        players: &[PlayerState],
        for_player: i32,
        lighting: f32,
        owner_colors: &HashMap<i32, Color>,
        pass: ObjectRenderPass,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        // Engine snapshots keep object payloads in canonical ID order, while
        // C4ObjectList draws Last -> Prev in its mutable master-list order
        // (src/C4ObjectList.cpp:387-396). Empty is the legacy snapshot
        // fallback; a partial sidecar appends omitted objects canonically.
        let mut ordered = Vec::with_capacity(objects.len());
        let mut seen = HashSet::with_capacity(objects.len());
        if render_order.is_empty() {
            ordered.extend(objects);
        } else {
            let by_id: HashMap<_, _> = objects.iter().map(|object| (object.id, object)).collect();
            ordered.extend(
                render_order
                    .iter()
                    .filter(|id| seen.insert(**id))
                    .filter_map(|id| by_id.get(id).copied()),
            );
            ordered.extend(objects.iter().filter(|object| seen.insert(object.id)));
        }
        let mut selected = Vec::new();

        for object in ordered {
            if object.status != ObjectStatus::Normal {
                continue;
            }
            if !Self::object_is_visible(objects, players, object, for_player, false) {
                continue;
            }
            // `if (Contained && !eDrawMode) return;` (src/C4Object.cpp:2363):
            // carried objects never draw into the landscape.
            if object.container.is_some() {
                continue;
            }
            match pass {
                ObjectRenderPass::Background => {
                    if object.category & CATEGORY_BACKGROUND_FLAG != 0 {
                        selected.push(object);
                    }
                }
                ObjectRenderPass::Normal => {
                    if object.category & (CATEGORY_BACKGROUND_FLAG | CATEGORY_FOREGROUND_FLAG) == 0
                    {
                        selected.push(object);
                    }
                }
                ObjectRenderPass::ForegroundNonParallax => {
                    if object.category & CATEGORY_FOREGROUND_FLAG != 0
                        && object.category & CATEGORY_PARALLAX_FLAG == 0
                    {
                        selected.push(object);
                    }
                }
                ObjectRenderPass::ForegroundParallax => {
                    if object.category & CATEGORY_FOREGROUND_FLAG != 0
                        && object.category & CATEGORY_PARALLAX_FLAG != 0
                    {
                        selected.push(object);
                    }
                }
            }
        }

        for object in &selected {
            self.paint_object(
                object,
                objects,
                players,
                for_player,
                lighting,
                owner_colors,
                gamma,
            );
        }
        for object in &selected {
            self.paint_object_top_face(
                object,
                SpriteBlitState::for_object(object),
                gamma,
            );
        }
    }

    /// C4ObjectList draws every base before any TopFace
    /// (src/C4ObjectList.cpp:390-396). This first increment implements the
    /// full-construction, upright DefCore TopFace used by the elevator car;
    /// action FacetTopFace and growth scaling are separate parity slices.
    fn paint_object_top_face(
        &mut self,
        object: &ObjectSnapshot,
        blit: SpriteBlitState,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        if object.construction != FULL_CON || object.rotation.rem_euclid(360) != 0 {
            return;
        }
        let (base_definition_id, base_graphics_name) =
            if let Some(base) = object.base_graphics.as_ref() {
                (base.definition.clone(), base.graphics_name.clone())
            } else {
                (object.definition_id.clone(), None)
            };
        let mut sprite = self
            .object_sprites
            .get(&sprite_map_key(
                &base_definition_id,
                base_graphics_name.as_deref(),
            ))
            .cloned();
        if sprite.is_none() && base_graphics_name.is_some() {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(&base_definition_id, None))
                .cloned();
        }
        if sprite.is_none() && base_definition_id != object.definition_id {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(&object.definition_id, None))
                .cloned();
        }
        let Some(sprite) = sprite else {
            return;
        };
        let Some(top_face) = sprite.top_face else {
            return;
        };
        let shape = Self::sprite_def_shape(&sprite);
        let cox = object.position.x + shape.x;
        let coy = object.position.y + shape.y;
        self.blit_face(
            &sprite,
            SourceRect::new(top_face.x, top_face.y, top_face.width, top_face.height),
            (
                (cox + top_face.target_x) as f32,
                (coy + top_face.target_y) as f32,
                top_face.width as f32,
                top_face.height as f32,
            ),
            (
                cox as f32 + shape.width as f32 / 2.0,
                coy as f32 + shape.height as f32 / 2.0,
            ),
            false,
            Some(object_color_by_owner_tint(object)),
            self.viewport_zoom.max(MIN_VIEWPORT_ZOOM),
            0.0,
            object.draw_transform,
            blit,
            gamma,
        );
    }

    fn paint_object(
        &mut self,
        object: &ObjectSnapshot,
        objects: &[ObjectSnapshot],
        players: &[PlayerState],
        for_player: i32,
        lighting: f32,
        _owner_colors: &HashMap<i32, Color>,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let content_width = self.surface_width as f32;
        let content_height = self.surface_height as f32;
        let color = object_color(object).modulate(lighting);
        let owner_color = Some(object_color_by_owner_tint(object));
        let rotation_degrees = (object.rotation.rem_euclid(360)) as f32;

        let screen_x = (object.position.x as f32 - self.viewport_x) * zoom;
        let screen_y = (object.position.y as f32 - self.viewport_y) * zoom;

        let base_transform = object.draw_transform;
        let (base_definition_id, base_graphics_name) =
            if let Some(base) = object.base_graphics.as_ref() {
                (base.definition.clone(), base.graphics_name.clone())
            } else {
                (object.definition_id.clone(), None)
            };
        let mut sprite = self
            .object_sprites
            .get(&sprite_map_key(
                &base_definition_id,
                base_graphics_name.as_deref(),
            ))
            .cloned();
        if sprite.is_none() && base_graphics_name.is_some() {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(&base_definition_id, None))
                .cloned();
        }
        if sprite.is_none() && base_definition_id != object.definition_id {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(&object.definition_id, None))
                .cloned();
        }
        if let Some(sprite) = sprite {
            self.draw_object_face(
                object,
                objects,
                &sprite,
                owner_color,
                zoom,
                rotation_degrees,
                base_transform,
                SpriteBlitState::for_object(object),
                gamma,
            );
            self.draw_object_overlays(
                object,
                objects,
                players,
                for_player,
                owner_color,
                screen_x,
                screen_y,
                zoom,
                rotation_degrees,
                base_transform,
                gamma,
            );
            return;
        }

        if screen_x < -10.0
            || screen_y < -10.0
            || screen_x > content_width + 10.0
            || screen_y > content_height + 10.0
        {
            return;
        }

        // No sprite available: debug fallbacks only (C++ objects always
        // have a graphics facet, so these paths have no oracle) — the
        // vertex polygon, then a plain dot.
        if object.vertices.len() >= 3 {
            let mut points = Vec::with_capacity(object.vertices.len());
            let mut min_x = f32::MAX;
            let mut max_x = f32::MIN;
            let mut min_y = f32::MAX;
            let mut max_y = f32::MIN;
            for vertex in &object.vertices {
                let world_x = (object.position.x + vertex.x) as f32;
                let world_y = (object.position.y + vertex.y) as f32;
                let x = (world_x - self.viewport_x) * zoom;
                let y = (world_y - self.viewport_y) * zoom;
                points.push((x.round() as i32, y.round() as i32));
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }

            if max_x >= -zoom
                && min_x <= content_width + zoom
                && max_y >= -zoom
                && min_y <= content_height + zoom
                && fill_polygon(&mut self.surface, &points, color)
            {
                return;
            }
        }

        let size = (6.0 * zoom).max(3.0);
        let rect = GuiRect::from_origin_size(
            GuiPoint::new(
                (screen_x - size / 2.0).max(0.0),
                (screen_y - size / 2.0).max(0.0),
            ),
            GuiSize::new(size, size),
        );
        fill_rect(&mut self.surface, &rect, color);
        self.draw_object_overlays(
            object,
            objects,
            players,
            for_player,
            owner_color,
            screen_x,
            screen_y,
            zoom,
            rotation_degrees,
            base_transform,
            gamma,
        );
    }

    /// C4Shape con-scaling for drawing (C4Object::UpdateShape,
    /// src/C4Object.cpp:325-333): GrowthType stretches x/y/Wdt/Hgt
    /// (C4Shape::Stretch, src/C4Shape.cpp:103-116), otherwise only
    /// y/Hgt shrink (C4Shape::Jolt, src/C4Shape.cpp:119-128).
    fn con_scaled_shape(shape: DefinitionRect, con: i32, stretch_growth: bool) -> DefinitionRect {
        if con == FULL_CON {
            return shape;
        }
        let percent = con * 100 / FULL_CON;
        let mut scaled = shape;
        if stretch_growth {
            scaled.x = scaled.x * percent / 100;
            scaled.width = scaled.width * percent / 100;
        }
        scaled.y = scaled.y * percent / 100;
        scaled.height = scaled.height * percent / 100;
        scaled
    }

    /// The def Shape rect used for drawing; loader sprites without a def
    /// shape fall back to the whole image centered on the position.
    fn sprite_def_shape(sprite: &DefinitionSprite) -> DefinitionRect {
        sprite
            .shape
            .filter(|shape| shape.width > 0 && shape.height > 0)
            .unwrap_or_else(|| {
                let width = sprite.image.width() as i32;
                let height = sprite.image.height() as i32;
                DefinitionRect::new(-width / 2, -height / 2, width, height)
            })
    }

    /// C4Object::Draw facet selection (src/C4Object.cpp:2388-2468):
    /// idle draws the base face only; active actions draw the optional
    /// FacetBase face plus the action facet — an active action with
    /// neither draws nothing (src/C4Object.cpp:2402).
    fn draw_object_face(
        &mut self,
        object: &ObjectSnapshot,
        objects: &[ObjectSnapshot],
        sprite: &DefinitionSprite,
        owner_color: Option<u32>,
        zoom: f32,
        rotation_degrees: f32,
        transform: Option<DrawTransform>,
        blit: SpriteBlitState,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        let con = object.construction.clamp(0, FULL_CON);
        let def_shape = Self::sprite_def_shape(sprite);
        let inst_shape = Self::con_scaled_shape(def_shape, con, sprite.stretch_growth);
        let Some(graphics) = sprite.actions.get(object.action.name.as_str()) else {
            // Idle: BaseFace only, phase (0,0) (src/C4Object.cpp:2388-2392).
            self.draw_base_face(
                object,
                sprite,
                con,
                def_shape,
                inst_shape,
                0,
                0,
                false,
                owner_color,
                zoom,
                rotation_degrees,
                transform,
                blit,
                gamma,
            );
            return;
        };
        let (draw_dir, flipped) = Self::resolve_draw_direction(graphics, object.direction);
        // FacetBase face underneath, phase (0, DrawDir)
        // (src/C4Object.cpp:2397-2399).
        if graphics.facet_base {
            self.draw_base_face(
                object,
                sprite,
                con,
                def_shape,
                inst_shape,
                0,
                draw_dir,
                flipped,
                owner_color,
                zoom,
                rotation_degrees,
                transform,
                blit,
                gamma,
            );
        }
        let Some(facet) = &graphics.facet else {
            return;
        };
        if facet.width <= 0 || facet.height <= 0 {
            return;
        }
        let cox = (object.position.x + inst_shape.x) as f32;
        let coy = (object.position.y + inst_shape.y) as f32;
        // FacetTargetStretch bypasses action phase/direction and object
        // transforms: DrawX scales the declared source from FacetY exactly
        // to Target->y + Target->Shape.y (src/C4Object.cpp:2426-2438).
        if graphics.facet_target_stretch {
            let Some(target) = object
                .action
                .target
                .and_then(|target| objects.iter().find(|object| object.id == target))
            else {
                return;
            };
            let Some(target_sprite) = self
                .object_sprites
                .get(&sprite_map_key(&target.definition_id, None))
            else {
                return;
            };
            let target_shape = Self::con_scaled_shape(
                Self::sprite_def_shape(target_sprite),
                target.construction.clamp(0, FULL_CON),
                target_sprite.stretch_growth,
            );
            let dest_y = coy + facet.target_y as f32;
            let dest_height = (target.position.y + target_shape.y) as f32 - dest_y;
            self.blit_face(
                sprite,
                SourceRect::new(facet.x, facet.y, facet.width, facet.height),
                (
                    cox + facet.target_x as f32,
                    dest_y,
                    facet.width as f32,
                    dest_height,
                ),
                (
                    cox + inst_shape.width as f32 / 2.0,
                    coy + inst_shape.height as f32 / 2.0,
                ),
                false,
                owner_color,
                zoom,
                0.0,
                None,
                blit,
                gamma,
            );
            return;
        }
        // Drawing phase; Reverse mirrors it (src/C4Object.cpp:2419-2420).
        let length = (graphics.length.unwrap_or(1).max(1) as i32).max(1);
        let mut phase = object.action.phase.rem_euclid(length);
        if graphics.reverse {
            phase = length - 1 - phase;
        }
        let source = SourceRect::new(
            facet.x + facet.width.saturating_mul(phase),
            facet.y + facet.height.saturating_mul(draw_dir),
            facet.width,
            facet.height,
        );
        // Full con: the facet at cox+FacetX/coy+FacetY; growing: the
        // con-scaled shape rect at cox/coy (src/C4Object.cpp:2450-2467).
        let dest = if con == FULL_CON {
            (
                cox + facet.target_x as f32,
                coy + facet.target_y as f32,
                facet.width as f32,
                facet.height as f32,
            )
        } else {
            (cox, coy, inst_shape.width as f32, inst_shape.height as f32)
        };
        self.blit_face(
            sprite,
            source,
            dest,
            (
                cox + inst_shape.width as f32 / 2.0,
                coy + inst_shape.height as f32 / 2.0,
            ),
            flipped,
            owner_color,
            zoom,
            rotation_degrees,
            transform,
            blit,
            gamma,
        );
    }

    /// C4Object::DrawFace (src/C4Object.cpp:438-467): the base face is
    /// the def Shape.Wdt x Shape.Hgt crop at phase (iPhaseX, iPhaseY),
    /// stretched by Con — GrowthType shrinks both axes toward the shape
    /// center, otherwise the width stays and the bottom source slice is
    /// shown (construction display).
    #[allow(clippy::too_many_arguments)]
    fn draw_base_face(
        &mut self,
        object: &ObjectSnapshot,
        sprite: &DefinitionSprite,
        con: i32,
        def_shape: DefinitionRect,
        inst_shape: DefinitionRect,
        phase_x: i32,
        phase_y: i32,
        flipped: bool,
        owner_color: Option<u32>,
        zoom: f32,
        rotation_degrees: f32,
        transform: Option<DrawTransform>,
        blit: SpriteBlitState,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        let swdt = def_shape.width;
        let shgt = def_shape.height;
        let fx = swdt * phase_x;
        let mut fy = shgt * phase_y;
        let fwdt = swdt;
        let mut fhgt = shgt;

        let cox = object.position.x + inst_shape.x;
        let coy = object.position.y + inst_shape.y;

        // Grow-type display (src/C4Object.cpp:448-451).
        let mut tx = (cox + (inst_shape.width - swdt * con / FULL_CON) / 2) as f32;
        let ty = (coy + (inst_shape.height - shgt * con / FULL_CON) / 2) as f32;
        let mut twdt = (swdt * con / FULL_CON) as f32;
        let thgt = (shgt * con / FULL_CON) as f32;

        // Construction-type display (src/C4Object.cpp:453-460).
        if !sprite.stretch_growth {
            tx = cox as f32 + (inst_shape.width - swdt) as f32 / 2.0;
            twdt = swdt as f32;
            fy += shgt * (FULL_CON - con).max(0) / FULL_CON;
            fhgt = (shgt * con / FULL_CON).min(shgt);
        }

        self.blit_face(
            sprite,
            SourceRect::new(fx, fy, fwdt, fhgt),
            (tx, ty, twdt, thgt),
            (
                cox as f32 + inst_shape.width as f32 / 2.0,
                coy as f32 + inst_shape.height as f32 / 2.0,
            ),
            flipped,
            owner_color,
            zoom,
            rotation_degrees,
            transform,
            blit,
            gamma,
        );
    }

    /// Blit one object face: clamps the source to the sheet (ActMap
    /// facets may nominally exceed it — Tree1 Still is 73x73 on a
    /// 71px-tall Graphics.png; GL clamps), mirrors flipped faces around
    /// the shape center (C4DrawTransform flipdir, C4Object::UpdateFlipDir
    /// src/C4Object.cpp:415-418, applied at src/C4Object.cpp:2458),
    /// applies the script draw transform at the shape center
    /// (SetTransformAt, src/C4Object.cpp:2431) and rotates around it
    /// (src/C4Object.cpp:483-488, 2428-2435).
    #[allow(clippy::too_many_arguments)]
    fn blit_face(
        &mut self,
        sprite: &DefinitionSprite,
        source: SourceRect,
        dest: (f32, f32, f32, f32),
        shape_center: (f32, f32),
        flipped: bool,
        owner_color: Option<u32>,
        zoom: f32,
        rotation_degrees: f32,
        transform: Option<DrawTransform>,
        blit: SpriteBlitState,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        let (mut dest_x, mut dest_y, mut dest_w, mut dest_h) = dest;
        if dest_w <= 0.0 || dest_h <= 0.0 || source.width <= 0 || source.height <= 0 {
            return;
        }
        let image_w = sprite.image.width() as i32;
        let image_h = sprite.image.height() as i32;
        if source.x < 0 || source.y < 0 {
            return;
        }
        let clamped_w = source.width.min(image_w - source.x);
        let clamped_h = source.height.min(image_h - source.y);
        if clamped_w <= 0 || clamped_h <= 0 {
            return;
        }
        dest_w *= clamped_w as f32 / source.width as f32;
        dest_h *= clamped_h as f32 / source.height as f32;
        let source = SourceRect::new(source.x, source.y, clamped_w, clamped_h);

        let mut flip = flipped;
        if flip {
            dest_x = 2.0 * shape_center.0 - (dest_x + dest_w);
        }
        if let Some(transform) = transform {
            let scale_x = if transform.scale_x.abs() > f32::EPSILON {
                transform.scale_x
            } else {
                1.0
            };
            let scale_y = if transform.scale_y.abs() > f32::EPSILON {
                transform.scale_y
            } else {
                1.0
            };
            let x0 = shape_center.0 + (dest_x - shape_center.0) * scale_x + transform.offset_x;
            let x1 =
                shape_center.0 + (dest_x + dest_w - shape_center.0) * scale_x + transform.offset_x;
            let y0 = shape_center.1 + (dest_y - shape_center.1) * scale_y + transform.offset_y;
            let y1 =
                shape_center.1 + (dest_y + dest_h - shape_center.1) * scale_y + transform.offset_y;
            dest_x = x0.min(x1);
            dest_y = y0.min(y1);
            dest_w = (x1 - x0).abs();
            dest_h = (y1 - y0).abs();
            if scale_x < 0.0 {
                flip = !flip;
            }
        }

        if dest_w <= 0.0 || dest_h <= 0.0 {
            return;
        }

        let viewport_x = self.viewport_x;
        let viewport_y = self.viewport_y;
        if rotation_degrees.abs() <= f32::EPSILON {
            let rect = GuiRect::from_origin_size(
                GuiPoint::new((dest_x - viewport_x) * zoom, (dest_y - viewport_y) * zoom),
                GuiSize::new(dest_w * zoom, dest_h * zoom),
            );
            draw_image_region(
                &mut self.surface,
                &rect,
                &sprite.image,
                sprite.color_mask.as_ref(),
                &source,
                flip,
                owner_color,
                blit,
                gamma,
            );
        } else {
            // The dest rect center orbits the shape center
            // (src/C4Object.cpp:483-488).
            let angle = rotation_degrees.to_radians();
            let (sin, cos) = angle.sin_cos();
            let rel_x = dest_x + dest_w / 2.0 - shape_center.0;
            let rel_y = dest_y + dest_h / 2.0 - shape_center.1;
            let center_x = shape_center.0 + rel_x * cos - rel_y * sin;
            let center_y = shape_center.1 + rel_x * sin + rel_y * cos;
            draw_image_region_rotated(
                &mut self.surface,
                (center_x - viewport_x) * zoom,
                (center_y - viewport_y) * zoom,
                dest_w * zoom,
                dest_h * zoom,
                &sprite.image,
                sprite.color_mask.as_ref(),
                &source,
                flip,
                owner_color,
                rotation_degrees,
                blit,
                gamma,
            );
        }
    }

    fn draw_action_graphic(
        &mut self,
        sprite: &DefinitionSprite,
        action_name: &str,
        phase: i32,
        direction: Direction,
        owner_color: Option<u32>,
        screen_x: f32,
        screen_y: f32,
        zoom: f32,
        rotation_degrees: f32,
        transform: Option<DrawTransform>,
        blit: SpriteBlitState,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) -> bool {
        let Some(graphics) = sprite.actions.get(action_name) else {
            return false;
        };
        let Some(facet) = &graphics.facet else {
            return false;
        };

        if facet.width <= 0 || facet.height <= 0 {
            return false;
        }

        let frame_count = graphics.length.unwrap_or(1).max(1);
        let frame_count_i32 = if frame_count > i32::MAX as u32 {
            i32::MAX
        } else {
            frame_count as i32
        };
        if frame_count_i32 <= 0 {
            return false;
        }

        let frame_index = if graphics.reverse && frame_count_i32 > 1 {
            let cycle = frame_count_i32.saturating_mul(2).saturating_sub(2);
            if cycle <= 0 {
                0
            } else {
                let cycle_i64 = i64::from(cycle);
                let phase_i64 = i64::from(phase);
                let pos = ((phase_i64 % cycle_i64) + cycle_i64) % cycle_i64;
                let pos_i32 = pos as i32;
                if pos_i32 >= frame_count_i32 {
                    cycle - pos_i32
                } else {
                    pos_i32
                }
            }
        } else {
            phase.rem_euclid(frame_count_i32)
        };

        let (draw_dir, flipped) = Self::resolve_draw_direction(graphics, direction);

        let source_rect = SourceRect::new(
            facet.x + facet.width.saturating_mul(frame_index),
            facet.y + facet.height.saturating_mul(draw_dir),
            facet.width,
            facet.height,
        );

        if !Self::source_within_image(&sprite.image, &source_rect) {
            return false;
        }

        let mut scale_x = 1.0f32;
        let mut scale_y = 1.0f32;
        let mut offset_x = 0.0f32;
        let mut offset_y = 0.0f32;
        let mut transform_flipped = false;

        if let Some(transform) = transform {
            if (transform.scale_x).abs() > f32::EPSILON {
                scale_x = transform.scale_x;
            }
            if (transform.scale_y).abs() > f32::EPSILON {
                scale_y = transform.scale_y;
            }
            offset_x = transform.offset_x;
            offset_y = transform.offset_y;
        }

        let final_screen_x = screen_x + offset_x * zoom;
        let final_screen_y = screen_y + offset_y * zoom;

        if scale_x < 0.0 {
            transform_flipped = !transform_flipped;
            scale_x = -scale_x;
        }
        if scale_y < 0.0 {
            scale_y = -scale_y;
        }

        let dest_width = facet.width as f32 * zoom * scale_x;
        let dest_height = facet.height as f32 * zoom * scale_y;
        if dest_width <= 0.0 || dest_height <= 0.0 {
            return false;
        }

        let final_flipped = flipped ^ transform_flipped;

        if rotation_degrees.abs() <= f32::EPSILON {
            let dest_rect = GuiRect::from_origin_size(
                GuiPoint::new(
                    final_screen_x - dest_width / 2.0,
                    final_screen_y - dest_height / 2.0,
                ),
                GuiSize::new(dest_width, dest_height),
            );
            draw_image_region(
                &mut self.surface,
                &dest_rect,
                &sprite.image,
                sprite.color_mask.as_ref(),
                &source_rect,
                final_flipped,
                owner_color,
                blit,
                gamma,
            );
        } else {
            draw_image_region_rotated(
                &mut self.surface,
                final_screen_x,
                final_screen_y,
                dest_width,
                dest_height,
                &sprite.image,
                sprite.color_mask.as_ref(),
                &source_rect,
                final_flipped,
                owner_color,
                rotation_degrees,
                blit,
                gamma,
            );
        }
        true
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_object_overlays(
        &mut self,
        object: &ObjectSnapshot,
        objects: &[ObjectSnapshot],
        players: &[PlayerState],
        for_player: i32,
        owner_color: Option<u32>,
        screen_x: f32,
        screen_y: f32,
        zoom: f32,
        rotation_degrees: f32,
        base_transform: Option<DrawTransform>,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        let mut object_ancestry = HashSet::from([object.id]);
        self.draw_object_overlays_inner(
            object,
            objects,
            players,
            for_player,
            owner_color,
            screen_x,
            screen_y,
            zoom,
            rotation_degrees,
            base_transform,
            gamma,
            &mut object_ancestry,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_object_overlays_inner(
        &mut self,
        object: &ObjectSnapshot,
        objects: &[ObjectSnapshot],
        players: &[PlayerState],
        for_player: i32,
        owner_color: Option<u32>,
        screen_x: f32,
        screen_y: f32,
        zoom: f32,
        rotation_degrees: f32,
        base_transform: Option<DrawTransform>,
        gamma: Option<&lc_graphics::GammaRamp>,
        object_ancestry: &mut HashSet<ObjectId>,
    ) {
        if object.graphics_overlays.is_empty() {
            return;
        }
        for overlay in &object.graphics_overlays {
            match overlay.mode {
                GraphicsOverlayMode::Action | GraphicsOverlayMode::Base => {
                    // Parent is a sentinel tested by equality in C++; any
                    // ordinary mode, including combinations that carry bit 1,
                    // remains local.
                    let blit = SpriteBlitState::for_overlay(object, overlay);
                    let combined_transform = match (base_transform, overlay.transform) {
                        (Some(base), Some(local)) => Some(base.combined(local)),
                        (Some(base), None) => Some(base),
                        (None, Some(local)) => Some(local),
                        (None, None) => None,
                    };
                    if overlay.mode == GraphicsOverlayMode::Action {
                        self.draw_overlay_action(
                            object,
                            overlay,
                            owner_color,
                            screen_x,
                            screen_y,
                            zoom,
                            rotation_degrees,
                            combined_transform,
                            blit,
                            gamma,
                        );
                    } else {
                        self.draw_overlay_base(
                            object,
                            overlay,
                            owner_color,
                            screen_x,
                            screen_y,
                            zoom,
                            rotation_degrees,
                            combined_transform,
                            blit,
                            gamma,
                        );
                    }
                }
                GraphicsOverlayMode::Object => self.draw_overlay_object(
                    object,
                    overlay,
                    objects,
                    players,
                    for_player,
                    zoom,
                    gamma,
                    object_ancestry,
                ),
                _ => {}
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_overlay_object(
        &mut self,
        host: &ObjectSnapshot,
        overlay: &ObjectGraphicsOverlay,
        objects: &[ObjectSnapshot],
        players: &[PlayerState],
        for_player: i32,
        zoom: f32,
        gamma: Option<&lc_graphics::GammaRamp>,
        object_ancestry: &mut HashSet<ObjectId>,
    ) {
        let Some(target) = overlay
            .overlay_object
            .and_then(|id| objects.iter().find(|object| object.id == id))
        else {
            return;
        };
        // C4GraphicsOverlay::IsValid rejects missing/deleted targets and
        // overlay-object recursion. Imported snapshots can still be malformed,
        // so keep the draw walk bounded as well (C4DefGraphics.cpp:692-706).
        if target.status == ObjectStatus::Deleted
            || !Self::object_is_visible(objects, players, target, for_player, true)
            || !object_ancestry.insert(target.id)
        {
            return;
        }

        let saved_viewport_x = self.viewport_x;
        let saved_viewport_y = self.viewport_y;
        let offset_x = overlay.transform.map_or(0, |transform| transform.offset_x as i32);
        let offset_y = overlay.transform.map_or(0, |transform| transform.offset_y as i32);
        // C++ mutates cgo.TargetX/Y rather than the object's position. Keeping
        // the simulation coordinates intact is important for stretched action
        // facets that inspect their action target while the referenced object
        // is painted at the host's output position.
        let (host_target_x, host_target_y) = self.object_target_position(host);
        self.viewport_x = host_target_x - host.position.x as f32 + target.position.x as f32
            - offset_x as f32;
        self.viewport_y = host_target_y - host.position.y as f32 + target.position.y as f32
            - offset_y as f32;

        let (base_definition_id, base_graphics_name) =
            if let Some(base) = target.base_graphics.as_ref() {
                (base.definition.clone(), base.graphics_name.clone())
            } else {
                (target.definition_id.clone(), None)
            };
        let mut sprite = self
            .object_sprites
            .get(&sprite_map_key(
                &base_definition_id,
                base_graphics_name.as_deref(),
            ))
            .cloned();
        if sprite.is_none() && base_graphics_name.is_some() {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(&base_definition_id, None))
                .cloned();
        }
        if sprite.is_none() && base_definition_id != target.definition_id {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(&target.definition_id, None))
                .cloned();
        }

        if let Some(sprite) = sprite {
            // MODE_Object's exact Parent sentinel inherits the referenced
            // object's state, not the host object's state
            // (C4DefGraphics.cpp:761-768).
            let blit = if overlay.blit_mode == C4GFXBLIT_PARENT {
                SpriteBlitState::for_object(target)
            } else {
                SpriteBlitState::for_overlay(host, overlay)
            };
            let owner_color = Some(object_color_by_owner_tint(target));
            let rotation_degrees = (target.rotation.rem_euclid(360)) as f32;
            self.draw_object_face(
                target,
                objects,
                &sprite,
                owner_color,
                zoom,
                rotation_degrees,
                target.draw_transform,
                blit,
                gamma,
            );
            let screen_x = (target.position.x as f32 - self.viewport_x) * zoom;
            let screen_y = (target.position.y as f32 - self.viewport_y) * zoom;
            self.draw_object_overlays_inner(
                target,
                objects,
                players,
                for_player,
                owner_color,
                screen_x,
                screen_y,
                zoom,
                rotation_degrees,
                target.draw_transform,
                gamma,
                object_ancestry,
            );
            self.paint_object_top_face(target, blit, gamma);
        }

        self.viewport_x = saved_viewport_x;
        self.viewport_y = saved_viewport_y;
        object_ancestry.remove(&target.id);
    }

    fn draw_overlay_action(
        &mut self,
        object: &ObjectSnapshot,
        overlay: &ObjectGraphicsOverlay,
        owner_color: Option<u32>,
        screen_x: f32,
        screen_y: f32,
        zoom: f32,
        rotation_degrees: f32,
        transform: Option<DrawTransform>,
        blit: SpriteBlitState,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        let definition_id = overlay
            .definition
            .as_deref()
            .unwrap_or(&object.definition_id);
        let graphics_name = overlay.graphics_name.as_deref();
        let mut sprite = self
            .object_sprites
            .get(&sprite_map_key(definition_id, graphics_name))
            .cloned();
        if sprite.is_none() && graphics_name.is_some() {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(definition_id, None))
                .cloned();
        }
        if sprite.is_none() && definition_id != &object.definition_id {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(&object.definition_id, None))
                .cloned();
        }
        let Some(sprite) = sprite else {
            return;
        };
        let action_name = overlay
            .action
            .as_deref()
            .unwrap_or(object.action.name.as_str());
        let phase = if overlay.phase != 0 {
            overlay.phase
        } else {
            object.action.phase
        };
        let _ = self.draw_action_graphic(
            &sprite,
            action_name,
            phase,
            object.direction,
            owner_color,
            screen_x,
            screen_y,
            zoom,
            rotation_degrees,
            transform,
            blit,
            gamma,
        );
    }

    fn draw_overlay_base(
        &mut self,
        object: &ObjectSnapshot,
        overlay: &ObjectGraphicsOverlay,
        owner_color: Option<u32>,
        screen_x: f32,
        screen_y: f32,
        zoom: f32,
        rotation_degrees: f32,
        transform: Option<DrawTransform>,
        blit: SpriteBlitState,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        let definition_id = overlay
            .definition
            .as_deref()
            .unwrap_or(&object.definition_id);
        let graphics_name = overlay.graphics_name.as_deref();
        let mut sprite = self
            .object_sprites
            .get(&sprite_map_key(definition_id, graphics_name))
            .cloned();
        if sprite.is_none() && graphics_name.is_some() {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(definition_id, None))
                .cloned();
        }
        if sprite.is_none() && definition_id != &object.definition_id {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(&object.definition_id, None))
                .cloned();
        }
        let Some(sprite) = sprite else {
            return;
        };
        let sprite_width = (sprite.image.width() as f32 * zoom).max(1.0);
        let sprite_height = (sprite.image.height() as f32 * zoom).max(1.0);
        let source_rect = SourceRect::new(
            0,
            0,
            sprite.image.width() as i32,
            sprite.image.height() as i32,
        );
        if !Self::source_within_image(&sprite.image, &source_rect) {
            return;
        }

        let mut scale_x = 1.0f32;
        let mut scale_y = 1.0f32;
        let mut offset_x = 0.0f32;
        let mut offset_y = 0.0f32;
        let mut flip_x = false;

        if let Some(transform) = transform {
            if (transform.scale_x).abs() > f32::EPSILON {
                scale_x = transform.scale_x;
            }
            if (transform.scale_y).abs() > f32::EPSILON {
                scale_y = transform.scale_y;
            }
            offset_x = transform.offset_x;
            offset_y = transform.offset_y;
        }

        let final_screen_x = screen_x + offset_x * zoom;
        let final_screen_y = screen_y + offset_y * zoom;
        if scale_x < 0.0 {
            flip_x = !flip_x;
            scale_x = -scale_x;
        }
        if scale_y < 0.0 {
            scale_y = -scale_y;
        }

        let dest_width = sprite_width * scale_x;
        let dest_height = sprite_height * scale_y;
        if dest_width <= 0.0 || dest_height <= 0.0 {
            return;
        }

        if rotation_degrees.abs() <= f32::EPSILON {
            let rect = GuiRect::from_origin_size(
                GuiPoint::new(
                    final_screen_x - dest_width / 2.0,
                    final_screen_y - dest_height / 2.0,
                ),
                GuiSize::new(dest_width, dest_height),
            );
            draw_image_region(
                &mut self.surface,
                &rect,
                &sprite.image,
                sprite.color_mask.as_ref(),
                &source_rect,
                flip_x,
                owner_color,
                blit,
                gamma,
            );
        } else {
            draw_image_region_rotated(
                &mut self.surface,
                final_screen_x,
                final_screen_y,
                dest_width,
                dest_height,
                &sprite.image,
                sprite.color_mask.as_ref(),
                &source_rect,
                flip_x,
                owner_color,
                rotation_degrees,
                blit,
                gamma,
            );
        }
    }

    fn resolve_draw_direction(
        graphics: &DefinitionActionGraphics,
        direction: Direction,
    ) -> (i32, bool) {
        let direction = direction.to_script_value();
        if let Some(flip_dir) = graphics.flip_dir {
            let flip_dir = flip_dir.min(i32::MAX as u32) as i32;
            if flip_dir > 0 && direction >= flip_dir {
                return (
                    flip_dir
                        .saturating_mul(2)
                        .saturating_sub(1)
                        .saturating_sub(direction),
                    true,
                );
            }
        }
        (direction, false)
    }

    fn source_within_image(image: &ImageData, rect: &SourceRect) -> bool {
        let width = image.width() as i32;
        let height = image.height() as i32;
        if width <= 0 || height <= 0 {
            return false;
        }
        rect.x >= 0
            && rect.y >= 0
            && rect.width > 0
            && rect.height > 0
            && rect.x + rect.width <= width
            && rect.y + rect.height <= height
    }

    /// `C4Game::DrawCursors` (src/C4Game.cpp:1852-1874): while a player's
    /// CursorFlash/SelectFlash timer runs, the fctCursor mark — the 35th
    /// square cell of the mouse-cursor sheet, phase +1 when the crew is
    /// contained (src/C4GraphicsResource.cpp:328-336, src/C4Game.cpp:1868)
    /// — is drawn centered above the cursor clonk's def shape.
    fn draw_player_cursors(
        &mut self,
        snapshot: &SimulationSnapshot,
        owner: i32,
        origin_x: f32,
        origin_y: f32,
        zoom: f32,
        gamma: Option<&lc_graphics::GammaRamp>,
    ) {
        let Some(image) = self.cursor_atlas.image_for_resolution(self.surface_width) else {
            return;
        };

        let player = snapshot.players.iter().find(|player| player.id == owner);
        // `if (pPlr->CursorFlash || pPlr->SelectFlash)` (src/C4Game.cpp:1863).
        if player
            .map(|player| player.control.cursor_flash <= 0 && player.control.select_flash <= 0)
            .unwrap_or(false)
        {
            return;
        }
        let cursor_id = player.and_then(|player| player.cursor).or_else(|| {
            snapshot
                .crew_selection
                .get(&owner)
                .and_then(|selection| selection.cursor)
        });
        let Some(cursor_id) = cursor_id else {
            return;
        };
        let Some(object) = snapshot.object(cursor_id) else {
            return;
        };

        // fctCursor: cell size = sheet height; phase 1 while contained.
        let cell = image.height() as i32;
        let phase = i32::from(object.container.is_some());
        let source = SourceRect::new((35 + phase) * cell, 0, cell, cell);
        if !Self::source_within_image(&image, &source) {
            return;
        }

        let content_width = self.surface_width as f32;
        let content_height = self.surface_height as f32;
        let screen_x = (object.position.x as f32 - origin_x) * zoom;
        let screen_y = (object.position.y as f32 - origin_y) * zoom;
        let margin = 16.0;
        if screen_x < -margin
            || screen_y < -margin
            || screen_x > content_width + margin
            || screen_y > content_height + margin
        {
            return;
        }

        // `coy - cursor->Def->Shape.Hgt / 2 - fctCursor.Hgt`
        // (src/C4Game.cpp:1872): offset by the def shape height, not the
        // sprite-sheet image height.
        let (cursor_definition_id, cursor_graphics_name) =
            if let Some(base) = object.base_graphics.as_ref() {
                (base.definition.clone(), base.graphics_name.clone())
            } else {
                (object.definition_id.clone(), None)
            };
        let shape_height = {
            let mut sprite = self.object_sprites.get(&sprite_map_key(
                &cursor_definition_id,
                cursor_graphics_name.as_deref(),
            ));
            if sprite.is_none() && cursor_graphics_name.is_some() {
                sprite = self
                    .object_sprites
                    .get(&sprite_map_key(&cursor_definition_id, None));
            }
            if sprite.is_none() && cursor_definition_id != object.definition_id {
                sprite = self
                    .object_sprites
                    .get(&sprite_map_key(&object.definition_id, None));
            }
            sprite
                .map(|sprite| (Self::sprite_def_shape(sprite).height as f32 * zoom).max(1.0))
                .unwrap_or(12.0 * zoom)
        };
        let cursor_size = cell as f32;

        let mark_top = screen_y - shape_height / 2.0 - cursor_size;
        let rect = GuiRect::from_origin_size(
            GuiPoint::new(screen_x - cursor_size / 2.0, mark_top),
            GuiSize::new(cursor_size, cursor_size),
        );
        draw_image_region(
            &mut self.surface,
            &rect,
            &image,
            None,
            &source,
            false,
            None,
            SpriteBlitState::normal(),
            gamma,
        );

        // Cursor name label (src/C4Game.cpp:1873-1887): with cursor->Info,
        // the crew name — prefixed by a `sRankName` line when Rank > 0 —
        // is drawn in FontRegular, red 0xffff0000, centered above the mark
        // (`coy - Shape.Hgt/2 - fctCursor.Hgt - 2 - texthgt`). TextOut
        // splits the C++ "rank|name" on '|' into stacked centered lines
        // (src/StdDDraw2.cpp:1039).
        let label = self
            .hud_players
            .iter()
            .find(|player| player.owner == owner)
            .and_then(|player| player.crew.iter().find(|crew| crew.object_id == cursor_id))
            .and_then(|crew| {
                crew.info_name
                    .as_ref()
                    .map(|name| (name.clone(), crew.rank, crew.rank_name.clone()))
            });
        if let Some((name, rank, rank_name)) = label {
            let font = hud::HudFont::from_set(self.clonk_fonts.as_deref(), self.font.as_ref());
            let line_height = font.line_height();
            // `texthgt = GetLineHeight(); if (Rank > 0) texthgt += texthgt`
            // (src/C4Game.cpp:1876-1880).
            let lines: Vec<String> = rank_name
                .filter(|_| rank > 0)
                .map(|rank_name| vec![rank_name, name.clone()])
                .unwrap_or_else(|| vec![name]);
            let text_height = line_height * lines.len() as i32;
            let text_x = screen_x.round() as i32;
            let mut text_y = mark_top.round() as i32 - 2 - text_height;
            for line in &lines {
                font.draw_with_gamma(
                    &mut self.surface,
                    text_x,
                    text_y,
                    line,
                    Color::opaque(0xff, 0x00, 0x00),
                    lc_graphics::clonk_font::TextAlign::Center,
                    gamma,
                );
                text_y += line_height;
            }
        }
    }

    /// `Game.GraphicsResource.FontRegular` for HUD text.
    fn hud_font(&self) -> hud::HudFont<'_> {
        hud::HudFont::from_set(self.clonk_fonts.as_deref(), self.font.as_ref())
    }

    /// The bottom border the message board strip occupies
    /// (`MessageBoard.Output.Hgt` = one FontRegular line,
    /// src/C4MessageBoard.cpp:73-76,228 / C4GraphicsSystem.cpp:346).
    fn message_board_height(&self) -> i32 {
        self.hud_font().line_height()
    }

    /// Whether the fullscreen chrome (upper board + message board) is
    /// active. C++ only sets the boards up when their Graphics.c4g facets
    /// loaded (`C4UpperBoard::Init` bails without `fctUpperBoard.Surface`,
    /// src/C4UpperBoard.cpp:114-118); asset-less test setups render bare
    /// viewports.
    fn hud_chrome_active(&self) -> bool {
        self.hud_graphics.upper_board.is_some()
    }

    /// The pixels the upper board texture actually covers —
    /// `Output.Hgt = max(C4UpperBoardHeight, fctUpperBoard.Hgt)`
    /// (src/C4UpperBoard.cpp:117-120).
    fn upper_board_pixel_height(&self) -> i32 {
        self.hud_graphics
            .upper_board
            .as_ref()
            .map(|image| (image.height() as i32).max(hud::UPPER_BOARD_HEIGHT))
            .unwrap_or(hud::UPPER_BOARD_HEIGHT)
    }

    /// The fullscreen overlay in the C4GraphicsSystem::Execute order:
    /// per-viewport player HUD, then message board, then upper board
    /// (src/C4GraphicsSystem.cpp:352-365).
    fn draw_hud(&mut self, frame: u64, gamma: Option<&lc_graphics::GammaRamp>) {
        // Per-viewport player info (C4Viewport::DrawOverlay,
        // src/C4Viewport.cpp:835-848).
        let viewports = self.active_viewports.clone();
        for viewport in &viewports {
            let Some(player) = self
                .hud_players
                .iter()
                .find(|player| player.owner == viewport.owner)
            else {
                continue;
            };
            let player = player.clone();
            let rect = viewport.rect;
            let font = hud::HudFont::from_set(self.clonk_fonts.as_deref(), self.font.as_ref());

            // Cursor info: C++ draws nothing without a cursor crew member
            // (src/C4Viewport.cpp:891-897) — the faithful "no crew"
            // presentation is an empty corner.
            let cursor_crew = player
                .cursor
                .and_then(|id| player.crew.iter().find(|crew| crew.object_id == id))
                .or_else(|| player.crew.iter().find(|crew| crew.is_focus))
                .or_else(|| player.crew.first());
            if let Some(crew) = cursor_crew {
                hud::draw_cursor_info_with_gamma(
                    &mut self.surface,
                    &font,
                    &self.hud_graphics,
                    rect,
                    &crew.label,
                    crew.rank,
                    crew.portrait.as_ref(),
                    crew.rank_symbols.as_ref(),
                    gamma,
                );
                hud::draw_inventory_with_gamma(
                    &mut self.surface,
                    &font,
                    rect,
                    &crew.inventory,
                    gamma,
                );
                hud::draw_energy_bar_with_gamma(
                    &mut self.surface,
                    &self.hud_graphics,
                    rect,
                    crew.energy_fraction,
                    gamma,
                );
                let mut bar_slot = 1;
                if crew.magic_energy != 0 {
                    hud::draw_level_bar_with_gamma(
                        &mut self.surface,
                        &self.hud_graphics,
                        rect,
                        hud::HudBarKind::Magic,
                        bar_slot,
                        crew.magic_energy / MAGIC_PHYSICAL_FACTOR,
                        crew.magic_capacity / MAGIC_PHYSICAL_FACTOR,
                        gamma,
                    );
                    bar_slot += 1;
                }
                if crew.breath != 0 && crew.breath < crew.breath_capacity {
                    hud::draw_level_bar_with_gamma(
                        &mut self.surface,
                        &self.hud_graphics,
                        rect,
                        hud::HudBarKind::Breath,
                        bar_slot,
                        crew.breath,
                        crew.breath_capacity,
                        gamma,
                    );
                }
            }

            // Command rows (src/C4Viewport.cpp:947-961), gated on
            // Config.Graphics.ShowCommands; 23px key caps pick FontTiny
            // (`cgo.Hgt <= C4MN_SymbolSize`, src/C4ObjectCom.cpp:940).
            if self.show_commands && !player.commands.is_empty() {
                let tiny = self
                    .clonk_fonts
                    .as_deref()
                    .map(|set| hud::HudFont::Clonk(&set.mini))
                    .unwrap_or(hud::HudFont::Fallback(self.font.as_ref()));
                hud::draw_commands_with_gamma(
                    &mut self.surface,
                    &tiny,
                    &self.hud_graphics,
                    rect,
                    &player.commands,
                    self.show_command_keys,
                    gamma,
                );
            }

            hud::draw_player_fixed_items_with_gamma(
                &mut self.surface,
                &font,
                &self.hud_graphics,
                rect,
                player.wealth,
                player.score,
                player.select_count,
                player.crew.len() as i32,
                player.owner_color,
                gamma,
            );

            let tiny = self
                .clonk_fonts
                .as_deref()
                .map(|set| hud::HudFont::Clonk(&set.mini))
                .unwrap_or(hud::HudFont::Fallback(self.font.as_ref()));
            hud::draw_player_controls_with_gamma(
                &mut self.surface,
                &font,
                &tiny,
                &self.hud_graphics,
                rect,
                player.show_control,
                player.show_control_position,
                player.last_com,
                &player.control_key_labels,
                frame,
                gamma,
            );

            if player.show_startup {
                hud::draw_player_startup_with_gamma(
                    &mut self.surface,
                    &font,
                    &self.hud_graphics,
                    rect,
                    &player.name,
                    player.owner_color,
                    gamma,
                );
            }
        }

        let font = hud::HudFont::from_set(self.clonk_fonts.as_deref(), self.font.as_ref());
        if self.hud_chrome_active() {
            hud::draw_message_board_with_gamma(
                &mut self.surface,
                &font,
                &self.hud_graphics,
                self.message_board_line.as_deref(),
                gamma,
            );
            hud::draw_upper_board_with_gamma(
                &mut self.surface,
                &font,
                &self.hud_graphics,
                &self.scenario_label_text,
                self.game_time_seconds,
                gamma,
            );
        }

        // Opt-in debug lines (replaces the old debug bar; off by default).
        if let Some((frame_text, status_text)) = self.debug_hud_text.clone() {
            let line_height = font.line_height();
            let base_y = if self.hud_chrome_active() {
                self.upper_board_pixel_height() + 2
            } else {
                2
            };
            font.draw_with_gamma(
                &mut self.surface,
                hud::SYMBOL_BORDER,
                base_y,
                &frame_text,
                Color::opaque(255, 255, 255),
                lc_graphics::clonk_font::TextAlign::Left,
                gamma,
            );
            font.draw_with_gamma(
                &mut self.surface,
                hud::SYMBOL_BORDER,
                base_y + line_height,
                &status_text,
                Color::opaque(255, 255, 255),
                lc_graphics::clonk_font::TextAlign::Left,
                gamma,
            );
        }
    }

    #[cfg(test)]
    pub fn viewport(&self) -> (i32, i32) {
        self.active_viewports
            .first()
            .map(|viewport| {
                let zoom = viewport.zoom.max(MIN_VIEWPORT_ZOOM);
                let offset_x = viewport.content_rect.x as f32 / zoom;
                let offset_y = viewport.content_rect.y as f32 / zoom;
                let adjusted_x = (viewport.viewport_x - offset_x).max(0.0);
                let adjusted_y = (viewport.viewport_y - offset_y).max(0.0);
                (adjusted_x.round() as i32, adjusted_y.round() as i32)
            })
            .unwrap_or((
                self.viewport_x.round() as i32,
                self.viewport_y.round() as i32,
            ))
    }

    fn surface_height_at(&self, landscape: Option<&Landscape>, x: i32) -> Option<i32> {
        landscape.and_then(|landscape| landscape.surface_height(x))
    }

    fn lighting_factor(time_of_day: u16) -> f32 {
        // C++ CR has no ambient day/night dimming for standard scenarios
        // (C4Weather adjusts the SKY gamma only) — an unset time-of-day
        // must render at full brightness, not as midnight. The cycle
        // stays for sandbox worlds that drive the clock.
        if time_of_day == 0 {
            return 1.0;
        }
        let cycle = EnvironmentSettings::TIME_CYCLE as f32;
        if cycle <= 0.0 {
            return 1.0;
        }
        let half_cycle = cycle / 2.0;
        let time = (time_of_day as u32 % EnvironmentSettings::TIME_CYCLE as u32) as f32;
        let mut distance = (time - half_cycle).abs();
        if distance > half_cycle {
            distance = cycle - distance;
        }
        let normalized = 1.0 - distance / half_cycle;
        let normalized = normalized.clamp(0.0, 1.0);
        let min = 0.35f32;
        let max = 1.0f32;
        min + normalized * (max - min)
    }

    fn apply_lighting(color: Color, lighting: f32) -> Color {
        color.modulate(lighting)
    }

    fn collect_owner_colors(snapshot: &SimulationSnapshot) -> HashMap<i32, Color> {
        let mut colors: HashMap<i32, Color> = HashMap::new();
        for player in &snapshot.players {
            if let Some(rgb) = player.color {
                colors.insert(player.id, Color::opaque(rgb.r, rgb.g, rgb.b));
            }
        }

        let mut owners: HashSet<i32> = snapshot.players.iter().map(|state| state.id).collect();
        owners.extend(snapshot.known_crew_owners.iter().copied());
        owners.extend(snapshot.eliminated_crew_owners.iter().copied());
        for object in &snapshot.objects {
            if object.owner != OWNER_NONE {
                owners.insert(object.owner);
            }
        }

        for owner in owners {
            if owner == OWNER_NONE {
                continue;
            }
            colors
                .entry(owner)
                .or_insert_with(|| default_owner_color(owner));
        }

        colors
    }

    fn collect_sprite_atlas(&self, snapshot: &SimulationSnapshot) -> Vec<EngineSurfaceSnapshot> {
        let mut atlas = Vec::with_capacity(
            2 + snapshot
                .objects
                .len()
                .saturating_add(self.active_viewports.len()),
        );

        let full_snapshot = self.surface.snapshot();
        atlas.push(Self::make_engine_surface(
            "back_buffer".to_string(),
            full_snapshot,
        ));

        for (index, viewport) in self.active_viewports.iter().enumerate() {
            if let Some(region) = self.surface.snapshot_region(viewport.rect) {
                let owner_label = if viewport.owner < 0 {
                    "none".to_string()
                } else {
                    viewport.owner.to_string()
                };
                let label = format!("viewport#{index}:player={owner_label}");
                atlas.push(Self::make_engine_surface(label, region));
            }
        }

        let overlay_height = self
            .upper_board_pixel_height()
            .clamp(0, self.surface_height as i32) as u32;
        if overlay_height > 0 {
            if let Some(snapshot) = self.surface.snapshot_region(SurfaceRect::new(
                0,
                0,
                self.surface_width,
                overlay_height,
            )) {
                atlas.push(Self::make_engine_surface(
                    "upper_board".to_string(),
                    snapshot,
                ));
            }
        }

        for viewport in &self.active_viewports {
            if let Some(object) = snapshot.object(viewport.focus) {
                if let Some(rect) = self.object_screen_rect_for_viewport(object, viewport) {
                    if let Some(snap) = self.surface.snapshot_region(rect) {
                        let label =
                            format!("focus#{}:player={}", object.id.as_u64(), viewport.owner);
                        atlas.push(Self::make_engine_surface(label, snap));
                    }
                }
            }
        }

        for object in &snapshot.objects {
            if let Some(rect) = self
                .active_viewports
                .iter()
                .find_map(|viewport| self.object_screen_rect_for_viewport(object, viewport))
            {
                if let Some(snap) = self.surface.snapshot_region(rect) {
                    let label =
                        format!("object#{}:def={}", object.id.as_u64(), object.definition_id);
                    atlas.push(Self::make_engine_surface(label, snap));
                }
            }
        }

        atlas
    }

    fn make_engine_surface(
        label: String,
        snapshot: GraphicsSurfaceSnapshot,
    ) -> EngineSurfaceSnapshot {
        let width = i32::try_from(snapshot.width()).unwrap_or(i32::MAX);
        let height = i32::try_from(snapshot.height()).unwrap_or(i32::MAX);
        EngineSurfaceSnapshot {
            label,
            width,
            height,
            hash: u64::from(snapshot.checksum()),
        }
    }

    fn object_screen_rect_for_viewport(
        &self,
        object: &ObjectSnapshot,
        viewport: &ActiveViewport,
    ) -> Option<SurfaceRect> {
        if !object.status.is_active() || !object.alive {
            return None;
        }

        let base_x = viewport.content_rect.x as f32;
        let base_y = viewport.content_rect.y as f32;
        let zoom = viewport.zoom.max(MIN_VIEWPORT_ZOOM);

        if object.vertices.is_empty() {
            let screen_x = (object.position.x as f32 - viewport.viewport_x) * zoom + base_x;
            let screen_y = (object.position.y as f32 - viewport.viewport_y) * zoom + base_y;
            let size = (6.0 * zoom).max(3.0);
            let half = size / 2.0;
            let rect = SurfaceRect::new(
                (screen_x - half).floor() as i32,
                (screen_y - half).floor() as i32,
                size.ceil() as u32,
                size.ceil() as u32,
            );
            return rect.intersection(viewport.rect);
        }

        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;
        for vertex in &object.vertices {
            let world_x = (object.position.x + vertex.x) as f32;
            let world_y = (object.position.y + vertex.y) as f32;
            let x = (world_x - viewport.viewport_x) * zoom + base_x;
            let y = (world_y - viewport.viewport_y) * zoom + base_y;
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }

        if !(min_x.is_finite() && max_x.is_finite() && min_y.is_finite() && max_y.is_finite()) {
            return None;
        }
        if min_x > max_x || min_y > max_y {
            return None;
        }

        let padding = (2.0 * zoom).max(1.0);
        let left = (min_x - padding).floor() as i32;
        let top = (min_y - padding).floor() as i32;
        let right = (max_x + padding).ceil() as i32;
        let bottom = (max_y + padding).ceil() as i32;

        if right < left || bottom < top {
            return None;
        }

        let width = (right - left + 1).max(1) as u32;
        let height = (bottom - top + 1).max(1) as u32;
        SurfaceRect::new(left, top, width, height).intersection(viewport.rect)
    }

    /// C4Game::FindVisObject point searches the current C4Shape rectangle,
    /// including structures and carryables whose `Alive` flag is false
    /// (C4Game.cpp:1469-1476). This intentionally differs from the
    /// crew-focused atlas/selection bounds above.
    fn object_pick_rect_for_viewport(
        &self,
        object: &ObjectSnapshot,
        viewport: &ActiveViewport,
    ) -> Option<SurfaceRect> {
        let base_x = viewport.content_rect.x as f32;
        let base_y = viewport.content_rect.y as f32;
        let zoom = viewport.zoom.max(MIN_VIEWPORT_ZOOM);
        let shape = self
            .object_sprites
            .get(&sprite_map_key(&object.definition_id, None))
            .map(|sprite| {
                Self::con_scaled_shape(
                    Self::sprite_def_shape(sprite),
                    object.construction.clamp(0, FULL_CON),
                    sprite.stretch_growth,
                )
            })
            .filter(|shape| shape.width > 0 && shape.height > 0);

        let (world_left, world_top, world_right, world_bottom) = if let Some(shape) = shape {
            (
                object.position.x + shape.x,
                object.position.y + shape.y,
                object.position.x + shape.x + shape.width,
                object.position.y + shape.y + shape.height,
            )
        } else if object.vertices.is_empty() {
            (
                object.position.x - 3,
                object.position.y - 3,
                object.position.x + 3,
                object.position.y + 3,
            )
        } else {
            let min_x = object.vertices.iter().map(|vertex| vertex.x).min()?;
            let max_x = object.vertices.iter().map(|vertex| vertex.x).max()?;
            let min_y = object.vertices.iter().map(|vertex| vertex.y).min()?;
            let max_y = object.vertices.iter().map(|vertex| vertex.y).max()?;
            (
                object.position.x + min_x,
                object.position.y + min_y,
                object.position.x + max_x + 1,
                object.position.y + max_y + 1,
            )
        };

        let left = ((world_left as f32 - viewport.viewport_x) * zoom + base_x).floor() as i32;
        let top = ((world_top as f32 - viewport.viewport_y) * zoom + base_y).floor() as i32;
        let right = ((world_right as f32 - viewport.viewport_x) * zoom + base_x).ceil() as i32;
        let bottom = ((world_bottom as f32 - viewport.viewport_y) * zoom + base_y).ceil() as i32;
        let width = (right - left).max(1) as u32;
        let height = (bottom - top).max(1) as u32;
        SurfaceRect::new(left, top, width, height).intersection(viewport.rect)
    }

    fn sky_color_for_temperature(temperature: i32) -> Color {
        let factor = Self::temperature_factor(temperature);
        let cold = (10, 16, 32);
        let warm = (84, 52, 16);
        Color::opaque(
            Self::blend_channel(cold.0, warm.0, factor),
            Self::blend_channel(cold.1, warm.1, factor),
            Self::blend_channel(cold.2, warm.2, factor),
        )
    }

    fn ground_color_for_temperature(temperature: i32) -> Color {
        let factor = Self::temperature_factor(temperature);
        let cold = (28, 84, 44);
        let warm = (108, 90, 32);
        Color::opaque(
            Self::blend_channel(cold.0, warm.0, factor),
            Self::blend_channel(cold.1, warm.1, factor),
            Self::blend_channel(cold.2, warm.2, factor),
        )
    }

    fn liquid_color_for_temperature(temperature: i32) -> Color {
        let factor = Self::temperature_factor(temperature);
        let cold = (36, 112, 200);
        let warm = (48, 132, 160);
        Color::new(
            Self::blend_channel(cold.0, warm.0, factor),
            Self::blend_channel(cold.1, warm.1, factor),
            Self::blend_channel(cold.2, warm.2, factor),
            192,
        )
    }

    fn temperature_factor(temperature: i32) -> f32 {
        let clamped = temperature.clamp(-50, 50);
        (clamped as f32 + 50.0) / 100.0
    }

    fn blend_channel(cold: u8, warm: u8, factor: f32) -> u8 {
        let factor = factor.clamp(0.0, 1.0);
        let cold = cold as f32;
        let warm = warm as f32;
        let value = cold + (warm - cold) * factor;
        value.round().clamp(0.0, 255.0) as u8
    }
}

fn tile_image_on_surface(
    surface: &mut Surface,
    image: &ImageData,
    origin_x: i32,
    origin_y: i32,
    gamma: Option<&lc_graphics::GammaRamp>,
) {
    let image_width = image.width() as usize;
    let image_height = image.height() as usize;
    let surface_width = surface.width() as usize;
    let surface_height = surface.height() as usize;
    if image_width == 0 || image_height == 0 || surface_width == 0 || surface_height == 0 {
        return;
    }
    let source = image.pixels();
    let source_stride = image_width.saturating_mul(4);
    let destination_stride = surface.stride();
    if source.len() < source_stride.saturating_mul(image_height)
        || destination_stride < surface_width.saturating_mul(4)
    {
        return;
    }
    let start_x = origin_x.rem_euclid(image_width as i32) as usize;
    let destination = surface.pixels_mut();
    for y in 0..surface_height {
        let source_y = (origin_y + y as i32).rem_euclid(image_height as i32) as usize;
        let source_row = &source[source_y * source_stride..(source_y + 1) * source_stride];
        let destination_row = &mut destination
            [y * destination_stride..y * destination_stride + surface_width * 4];
        let mut destination_x = 0;
        let mut source_x = start_x;
        while destination_x < surface_width {
            let copy_width = (image_width - source_x).min(surface_width - destination_x);
            let source_start = source_x * 4;
            let destination_start = destination_x * 4;
            if let Some(gamma) = gamma {
                for pixel in 0..copy_width {
                    let source = source_start + pixel * 4;
                    let destination = destination_start + pixel * 4;
                    let encoded = gamma_encode_fragment(
                        Color::new(
                            source_row[source],
                            source_row[source + 1],
                            source_row[source + 2],
                            source_row[source + 3],
                        ),
                        gamma,
                    );
                    destination_row[destination..destination + 4]
                        .copy_from_slice(&[encoded.r, encoded.g, encoded.b, encoded.a]);
                }
            } else {
                destination_row[destination_start..destination_start + copy_width * 4]
                    .copy_from_slice(&source_row[source_start..source_start + copy_width * 4]);
            }
            destination_x += copy_width;
            source_x = 0;
        }
    }
}

fn blit_surface(dst: &mut Surface, src: &Surface, offset_x: i32, offset_y: i32) {
    if src.width() == 0 || src.height() == 0 {
        return;
    }
    if dst.format() != src.format() {
        return;
    }
    if offset_x >= dst.width() as i32 || offset_y >= dst.height() as i32 {
        return;
    }

    let start_x = offset_x.max(0) as u32;
    let start_y = offset_y.max(0) as u32;
    if start_x >= dst.width() || start_y >= dst.height() {
        return;
    }

    let max_width = dst.width().saturating_sub(start_x);
    let max_height = dst.height().saturating_sub(start_y);
    let copy_width = src.width().min(max_width);
    let copy_height = src.height().min(max_height);
    if copy_width == 0 || copy_height == 0 {
        return;
    }

    let dst_stride = dst.stride();
    let src_stride = src.stride();
    if src.width() == 0 {
        return;
    }
    let bpp = src_stride / src.width() as usize;
    if bpp == 0 {
        return;
    }

    let dst_pixels = dst.pixels_mut();
    let src_pixels = src.pixels();

    for row in 0..copy_height {
        let dst_row = (start_y + row) as usize;
        let dst_offset = dst_row
            .saturating_mul(dst_stride)
            .saturating_add(start_x as usize * bpp);
        let src_offset = row as usize * src_stride;
        let len = copy_width as usize * bpp;
        if dst_offset + len > dst_pixels.len() || src_offset + len > src_pixels.len() {
            break;
        }
        dst_pixels[dst_offset..dst_offset + len]
            .copy_from_slice(&src_pixels[src_offset..src_offset + len]);
    }
}

fn object_color(object: &ObjectSnapshot) -> Color {
    let mut hasher = DefaultHasher::new();
    object.definition_id.hash(&mut hasher);
    let hash = hasher.finish();

    let channel = |shift: u32| -> u8 {
        let component = ((hash >> shift) & 0x7F) as u8;
        component.saturating_add(64)
    };

    let base_r = channel(0);
    let base_g = channel(8);
    let base_b = channel(16);
    let low_tint = (200u8, 64u8, 64u8);
    let energy = object.energy.clamp(0, 100) as f32 / 100.0;
    let mix_channel = |base: u8, low: u8| -> u8 {
        let value = (low as f32 * (1.0 - energy) + base as f32 * energy)
            .round()
            .clamp(0.0, 255.0);
        value as u8
    };

    let mut r = mix_channel(base_r, low_tint.0);
    let mut g = mix_channel(base_g, low_tint.1);
    let mut b = mix_channel(base_b, low_tint.2);

    if !object.alive || !object.status.is_active() {
        let fade = 0.45f32;
        r = (r as f32 * fade).round() as u8;
        g = (g as f32 * fade).round() as u8;
        b = (b as f32 * fade).round() as u8;
    }

    Color::opaque(r, g, b)
}

fn draw_pxs_pixel(
    surface: &mut Surface,
    x: i32,
    y: i32,
    color: Color,
    gamma: Option<&lc_graphics::GammaRamp>,
) {
    if x < 0 || y < 0 || x >= surface.width() as i32 || y >= surface.height() as i32 {
        return;
    }
    let background = surface.get_pixel(x as u32, y as u32).unwrap_or_default();
    let blended = gamma.map_or_else(
        || blend_color_over(color, background),
        |gamma| gamma_blend_fragment_over(color, background, gamma),
    );
    let _ = surface.set_pixel(x as u32, y as u32, blended);
}

fn draw_pxs_line(
    surface: &mut Surface,
    start: (f32, f32),
    end: (f32, f32),
    color: Color,
    gamma: Option<&lc_graphics::GammaRamp>,
) {
    // Integer raster counterpart of CStdGL::DrawLineDw's GL_LINES call. Its
    // vertices are shifted by 0.5, and GL's diamond-exit rule makes the
    // segment half-open at its final endpoint (StdGL.cpp:893-933).
    let Some((start, end)) = clip_pxs_line(surface, start, end) else {
        return;
    };
    let (mut x0, mut y0) = (start.0.round() as i32, start.1.round() as i32);
    let (x1, y1) = (end.0.round() as i32, end.1.round() as i32);
    if x0 == x1 && y0 == y1 {
        draw_pxs_pixel(surface, x0, y0, color, gamma);
        return;
    }
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut error = dx + dy;
    while x0 != x1 || y0 != y1 {
        draw_pxs_pixel(surface, x0, y0, color, gamma);
        let doubled = error * 2;
        if doubled >= dy {
            error += dy;
            x0 += sx;
        }
        if doubled <= dx {
            error += dx;
            y0 += sy;
        }
    }
}

fn clip_pxs_line(
    surface: &Surface,
    start: (f32, f32),
    end: (f32, f32),
) -> Option<((f32, f32), (f32, f32))> {
    if surface.width() == 0 || surface.height() == 0 {
        return None;
    }
    // GL clips C4PXS velocity lines to the render target. Liang-Barsky keeps
    // a malicious/extreme xdir from turning the CPU raster into a multi-
    // billion-step loop while preserving the in-target segment.
    let (x0, y0) = (f64::from(start.0), f64::from(start.1));
    let (dx, dy) = (f64::from(end.0) - x0, f64::from(end.1) - y0);
    if !(x0.is_finite() && y0.is_finite() && dx.is_finite() && dy.is_finite()) {
        return None;
    }
    let mut enter = 0.0f64;
    let mut exit = 1.0f64;
    let bounds = [
        (-dx, x0),
        (dx, f64::from(surface.width() - 1) - x0),
        (-dy, y0),
        (dy, f64::from(surface.height() - 1) - y0),
    ];
    for (direction, distance) in bounds {
        if direction.abs() <= f64::EPSILON {
            if distance < 0.0 {
                return None;
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
            return None;
        }
    }
    Some((
        ((x0 + enter * dx) as f32, (y0 + enter * dy) as f32),
        ((x0 + exit * dx) as f32, (y0 + exit * dy) as f32),
    ))
}

fn draw_pxs_image_region(
    surface: &mut Surface,
    target: &GuiRect,
    image: &ImageData,
    source: &SourceRect,
    modulation_transparency: u8,
    lighting: f32,
    gamma: Option<&lc_graphics::GammaRamp>,
) {
    if target.size.width <= 0.0
        || target.size.height <= 0.0
        || source.width <= 0
        || source.height <= 0
    {
        return;
    }
    if image.width() == 0 || image.height() == 0 {
        return;
    }
    let scale_x = target.size.width / source.width as f32;
    let scale_y = target.size.height / source.height as f32;
    let right = target.origin.x + target.size.width;
    let bottom = target.origin.y + target.size.height;
    let first_x = (target.origin.x - 0.5).ceil() as i32;
    let first_y = (target.origin.y - 0.5).ceil() as i32;
    let tile_size = cpp_tex_size(image.width(), image.height()) as i32;
    for y in first_y.max(0)..surface.height() as i32 {
        let pixel_y = y as f32 + 0.5;
        if pixel_y >= bottom {
            break;
        }
        let source_edge_y = source.y as f32 + (pixel_y - target.origin.y) / scale_y;
        let tile_y = (source_edge_y.floor() as i32).div_euclid(tile_size) * tile_size;
        let sample_y = source_edge_y - 0.5 - tile_y as f32;
        for x in first_x.max(0)..surface.width() as i32 {
            let pixel_x = x as f32 + 0.5;
            if pixel_x >= right {
                break;
            }
            let source_edge_x = source.x as f32 + (pixel_x - target.origin.x) / scale_x;
            let tile_x = (source_edge_x.floor() as i32).div_euclid(tile_size) * tile_size;
            let sample_x = source_edge_x - 0.5 - tile_x as f32;
            let sample = bilinear_sample_tile(
                image,
                tile_x,
                tile_y,
                tile_size,
                sample_x,
                sample_y,
            );
            // PerformBlt's shader and fixed-function paths ADD the filtered
            // texture and modulation transparency, then clamp. Keep filtered
            // alpha in float until the final framebuffer store
            // (StdGL.cpp:490-503,1070-1079,1320-1324).
            let texture_transparency = 255.0 - sample[3].clamp(0.0, 255.0);
            let transparency = (texture_transparency + f32::from(modulation_transparency))
                .min(255.0);
            let opacity = 255.0 - transparency;
            if opacity <= 0.0 {
                continue;
            }
            let alpha = opacity / 255.0;
            let background = surface.get_pixel(x as u32, y as u32).unwrap_or_default();
            let blend = |channel, source: f32, destination: u8| -> u8 {
                let source = (source * lighting).clamp(0.0, 255.0);
                store_channel(
                    sample_channel(gamma, channel, source) * alpha
                        + f32::from(destination) * (1.0 - alpha),
                )
            };
            let color = Color::new(
                blend(
                    lc_graphics::gamma::GammaChannel::Red,
                    sample[0],
                    background.r,
                ),
                blend(
                    lc_graphics::gamma::GammaChannel::Green,
                    sample[1],
                    background.g,
                ),
                blend(
                    lc_graphics::gamma::GammaChannel::Blue,
                    sample[2],
                    background.b,
                ),
                255,
            );
            let _ = surface.set_pixel(x as u32, y as u32, color);
        }
    }
}

fn blend_color_over(source: Color, dest: Color) -> Color {
    let alpha = source.a as u16;
    if alpha == 0 {
        return dest;
    }
    if alpha == 255 {
        return source;
    }
    let inv_alpha = 255u16 - alpha;
    let blend_channel =
        |src: u8, dst: u8| -> u8 { ((src as u16 * alpha + dst as u16 * inv_alpha) / 255) as u8 };

    Color::new(
        blend_channel(source.r, dest.r),
        blend_channel(source.g, dest.g),
        blend_channel(source.b, dest.b),
        (alpha + (dest.a as u16 * inv_alpha) / 255).min(255) as u8,
    )
}

fn fill_polygon(surface: &mut Surface, points: &[(i32, i32)], color: Color) -> bool {
    if points.len() < 3 {
        return false;
    }

    let width = surface.width() as i32;
    let height = surface.height() as i32;
    if width <= 0 || height <= 0 {
        return false;
    }

    let mut min_y = i32::MAX;
    let mut max_y = i32::MIN;
    for &(_, y) in points {
        min_y = min_y.min(y);
        max_y = max_y.max(y);
    }

    if min_y > max_y || min_y >= height || max_y < 0 {
        return false;
    }

    let start_y = min_y.max(0);
    let end_y = max_y.min(height - 1);
    if start_y > end_y {
        return false;
    }

    let mut intersections = Vec::with_capacity(points.len());
    let mut drawn = false;

    for y in start_y..=end_y {
        intersections.clear();
        let y_f = y as f64;
        for i in 0..points.len() {
            let (x1, y1) = points[i];
            let (x2, y2) = points[(i + 1) % points.len()];
            let y1_f = y1 as f64;
            let y2_f = y2 as f64;
            if ((y1_f <= y_f) && (y2_f > y_f)) || ((y2_f <= y_f) && (y1_f > y_f)) {
                let x1_f = x1 as f64;
                let x2_f = x2 as f64;
                let x = x1_f + (y_f - y1_f) * (x2_f - x1_f) / (y2_f - y1_f);
                intersections.push(x);
            }
        }

        if intersections.len() < 2 {
            continue;
        }

        intersections.sort_by(|a, b| a.partial_cmp(b).unwrap());

        let mut iter = intersections.iter();
        while let Some(&start) = iter.next() {
            if let Some(&end) = iter.next() {
                let mut x_start = start.ceil() as i32;
                let mut x_end = end.floor() as i32;
                if x_end < x_start {
                    continue;
                }
                if x_end < 0 || x_start >= width {
                    continue;
                }
                if x_start < 0 {
                    x_start = 0;
                }
                if x_end >= width {
                    x_end = width - 1;
                }
                for x in x_start..=x_end {
                    let _ = surface.set_pixel(x as u32, y as u32, color);
                    drawn = true;
                }
            }
        }
    }

    drawn
}

pub(crate) fn fill_rect(surface: &mut Surface, rect: &GuiRect, color: Color) {
    let x0 = rect.origin.x.floor() as i32;
    let y0 = rect.origin.y.floor() as i32;
    let x1 = (rect.origin.x + rect.size.width).ceil() as i32;
    let y1 = (rect.origin.y + rect.size.height).ceil() as i32;

    let x0 = x0.clamp(0, surface.width() as i32);
    let y0 = y0.clamp(0, surface.height() as i32);
    let x1 = x1.clamp(0, surface.width() as i32);
    let y1 = y1.clamp(0, surface.height() as i32);

    for y in y0..y1 {
        for x in x0..x1 {
            let _ = surface.set_pixel(x as u32, y as u32, color);
        }
    }
}

fn draw_image_region(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    mask: Option<&ColorByOwnerMask>,
    source: &SourceRect,
    flip_x: bool,
    owner_color: Option<u32>,
    blit: SpriteBlitState,
    gamma: Option<&lc_graphics::GammaRamp>,
) {
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return;
    }

    if source.width <= 0 || source.height <= 0 {
        return;
    }

    let dest_width = rect.size.width.max(1.0).round() as u32;
    let dest_height = rect.size.height.max(1.0).round() as u32;
    if dest_width == 0 || dest_height == 0 || image.width() == 0 || image.height() == 0 {
        return;
    }

    let dest_x = rect.origin.x.round() as i32;
    let dest_y = rect.origin.y.round() as i32;

    let bounds = surface.bounds();
    let image_width = image.width() as i32;
    let image_height = image.height() as i32;
    let pixels = image.pixels();

    for dy in 0..dest_height {
        let target_y = dest_y + dy as i32;
        if target_y < bounds.y || target_y >= bounds.y + bounds.height as i32 {
            continue;
        }

        let src_y = ((dy as f32 / dest_height as f32) * source.height as f32)
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
            let source = prepare_sprite_fragment(color, owner_mask, owner_color, blit);
            if source.alpha() == 0 {
                continue;
            }
            let background = surface
                .get_pixel(target_x as u32, target_y as u32)
                .unwrap_or_default();
            let blended = composite_sprite_fragment(source, background, blit, gamma);

            let _ = surface.set_pixel(target_x as u32, target_y as u32, blended);
        }
    }
}

fn draw_image_region_rotated(
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
    gamma: Option<&lc_graphics::GammaRamp>,
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

    let bounds = surface.bounds();
    if bounds.width == 0 || bounds.height == 0 {
        return;
    }

    let half_w = dest_width / 2.0;
    let half_h = dest_height / 2.0;
    let angle_rad = rotation_degrees.to_radians();
    let cos_theta = angle_rad.cos();
    let sin_theta = angle_rad.sin();

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
            let source = prepare_sprite_fragment(color, owner_mask, owner_color, blit);
            if source.alpha() == 0 {
                continue;
            }
            let background = surface.get_pixel(x as u32, y as u32).unwrap_or_default();
            let blended = composite_sprite_fragment(source, background, blit, gamma);
            let _ = surface.set_pixel(x as u32, y as u32, blended);
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
    gamma: Option<&lc_graphics::GammaRamp>,
) {
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

            let background = surface
                .get_pixel(target_x as u32, target_y as u32)
                .unwrap_or_default();
            let blended = match gamma {
                Some(gamma) if color.a == 255 => gamma_encode_fragment(color, gamma),
                Some(gamma) => gamma_blend_fragment_over(color, background, gamma),
                None if color.a == 255 => color,
                None => blend_colors(color, background),
            };

            let _ = surface.set_pixel(target_x as u32, target_y as u32, blended);
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
    gamma: Option<&lc_graphics::GammaRamp>,
) {
    let pixels = image.pixels();
    let (iw, ih) = (image.width(), image.height());
    let src_w = src_w.min(iw.saturating_sub(src_x));
    let src_h = src_h.min(ih.saturating_sub(src_y));
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
            let blended = if rgba[3] == 255 {
                Color::new(
                    store_channel(sample_channel(
                        gamma,
                        lc_graphics::gamma::GammaChannel::Red,
                        f32::from(rgba[0]),
                    )),
                    store_channel(sample_channel(
                        gamma,
                        lc_graphics::gamma::GammaChannel::Green,
                        f32::from(rgba[1]),
                    )),
                    store_channel(sample_channel(
                        gamma,
                        lc_graphics::gamma::GammaChannel::Blue,
                        f32::from(rgba[2]),
                    )),
                    255,
                )
            } else if gamma.is_none() {
                let background = surface.get_pixel(tx as u32, ty as u32).unwrap_or_default();
                blend_colors(Color::new(rgba[0], rgba[1], rgba[2], rgba[3]), background)
            } else {
                let dst = surface.get_pixel(tx as u32, ty as u32).unwrap_or_default();
                let af = f32::from(rgba[3]) / 255.0;
                let blend = |channel, src: u8, dst: u8| -> u8 {
                    store_channel(
                        sample_channel(gamma, channel, f32::from(src)) * af
                            + f32::from(dst) * (1.0 - af),
                    )
                };
                Color::new(
                    blend(lc_graphics::gamma::GammaChannel::Red, rgba[0], dst.r),
                    blend(
                        lc_graphics::gamma::GammaChannel::Green,
                        rgba[1],
                        dst.g,
                    ),
                    blend(
                        lc_graphics::gamma::GammaChannel::Blue,
                        rgba[2],
                        dst.b,
                    ),
                    255,
                )
            };
            let _ = surface.set_pixel(tx as u32, ty as u32, blended);
        }
    }
}

/// The GL texture tile size the C++ engine picks for an image: the next
/// power of two of min(W, H), capped at the 4096 max texture size
/// (C4Surface::CreateTextures, C4Surface.cpp:166-189). Images larger than
/// the tile in either dimension are split across multiple textures, which
/// produces visible filtering seams at tile boundaries — faithfully
/// reproduced here.
fn cpp_tex_size(width: u32, height: u32) -> u32 {
    let need = width.min(height).max(1);
    let mut n = 1u32;
    while (1 << n) < need {
        n += 1;
    }
    (1u32 << n).min(4096)
}

/// Bilinearly samples one `tile_size` texture tile of `image` at GL_LINEAR
/// coordinates `(u_rel, v_rel)` relative to the tile origin
/// `(tile_x, tile_y)` in image texels.
///
/// The engine sets GL_CLAMP_TO_EDGE at texture creation
/// (C4Surface.cpp:1102-1103), so the two taps clamp to the tile's edge
/// texels; texels inside the tile but outside the image are padding.
fn bilinear_sample_tile(
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
            return [0.0; 4]; // tile padding
        }
        let idx = ((y as u32 * image.width() + x as u32) * 4) as usize;
        pixels
            .get(idx..idx + 4)
            .map(|p| [p[0] as f32, p[1] as f32, p[2] as f32, p[3] as f32])
            .unwrap_or([0.0; 4])
    };
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

/// How a stretched blit combines with the framebuffer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BilinearBlend {
    /// `glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA)`.
    AlphaOver,
    /// `glBlendFunc(GL_SRC_ALPHA, GL_ONE)` (StdGL.cpp:908, additive blits).
    Additive,
}

/// Stretches `image` into `rect` like `CStdDDraw::Blit` (StdDDraw2.cpp:
/// 637-786): one quad per power-of-two texture tile, GL_LINEAR sampling with
/// GL_REPEAT wrap per tile, the blit shader's gamma lookup on the fragment
/// color, and float blending rounded once on store.
fn draw_image_bilinear_impl(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    gamma: Option<&lc_graphics::GammaRamp>,
    blend_mode: BilinearBlend,
) {
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 || image.width() == 0 || image.height() == 0
    {
        return;
    }
    let (fw, fh) = (image.width() as f32, image.height() as f32);
    let (tx, ty) = (rect.origin.x, rect.origin.y);
    let scale_x = rect.size.width / fw;
    let scale_y = rect.size.height / fh;
    let ts = cpp_tex_size(image.width(), image.height()) as i32;
    let tiles_x = (image.width() as i32 - 1) / ts + 1;
    let tiles_y = (image.height() as i32 - 1) / ts + 1;

    for tile_iy in 0..tiles_y {
        for tile_ix in 0..tiles_x {
            let (blit_x, blit_y) = (tile_ix * ts, tile_iy * ts);
            // Source range of this tile in image texels (fx = fy = 0 here).
            let s_left = blit_x as f32;
            let s_top = blit_y as f32;
            let s_right = ((blit_x + ts) as f32).min(fw);
            let s_bottom = ((blit_y + ts) as f32).min(fh);
            // Destination quad (tTexBlt* in StdDDraw2.cpp:738-741).
            let t_left = s_left * scale_x + tx;
            let t_top = s_top * scale_y + ty;
            let t_right = s_right * scale_x + tx;
            let t_bottom = s_bottom * scale_y + ty;
            // Pixels whose centers fall inside the quad.
            let px0 = (t_left - 0.5).ceil() as i32;
            let py0 = (t_top - 0.5).ceil() as i32;
            for py in py0.max(0)..surface.height() as i32 {
                if (py as f32 + 0.5) >= t_bottom {
                    break;
                }
                for px in px0.max(0)..surface.width() as i32 {
                    if (px as f32 + 0.5) >= t_right {
                        break;
                    }
                    let u_rel = (px as f32 + 0.5 - tx) / scale_x - 0.5 - blit_x as f32;
                    let v_rel = (py as f32 + 0.5 - ty) / scale_y - 0.5 - blit_y as f32;
                    let s = bilinear_sample_tile(image, blit_x, blit_y, ts, u_rel, v_rel);
                    if s[3] <= 0.0 {
                        continue;
                    }
                    let af = (s[3] / 255.0).clamp(0.0, 1.0);
                    let dst = surface.get_pixel(px as u32, py as u32).unwrap_or_default();
                    let out = match blend_mode {
                        BilinearBlend::AlphaOver => {
                            let blend = |channel, src: f32, dst: u8| -> u8 {
                                store_channel(
                                    sample_channel(gamma, channel, src) * af
                                        + f32::from(dst) * (1.0 - af),
                                )
                            };
                            Color::new(
                                blend(lc_graphics::gamma::GammaChannel::Red, s[0], dst.r),
                                blend(lc_graphics::gamma::GammaChannel::Green, s[1], dst.g),
                                blend(lc_graphics::gamma::GammaChannel::Blue, s[2], dst.b),
                                255,
                            )
                        }
                        BilinearBlend::Additive => {
                            let add = |channel, src: f32, dst: u8| -> u8 {
                                store_channel(
                                    f32::from(dst) + sample_channel(gamma, channel, src) * af,
                                )
                            };
                            Color::new(
                                add(lc_graphics::gamma::GammaChannel::Red, s[0], dst.r),
                                add(lc_graphics::gamma::GammaChannel::Green, s[1], dst.g),
                                add(lc_graphics::gamma::GammaChannel::Blue, s[2], dst.b),
                                dst.a,
                            )
                        }
                    };
                    let _ = surface.set_pixel(px as u32, py as u32, out);
                }
            }
        }
    }
}

/// Samples a filtered colour channel the way the C++ blit shader does. The
/// normalized R16 result stays in float for blending and is rounded only on
/// framebuffer store (StdGL.cpp:908,1082-1086,1246-1255).
fn sample_channel(
    gamma: Option<&lc_graphics::GammaRamp>,
    channel: lc_graphics::gamma::GammaChannel,
    x: f32,
) -> f32 {
    gamma
        .map(|ramp| ramp.sample_channel_float(channel, x))
        .unwrap_or_else(|| x.clamp(0.0, 255.0))
}

fn store_channel(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

/// Applies the C++ shader's independent normalized R16 lookups to one source
/// fragment. Alpha bypasses gamma unchanged (StdGL.cpp:1081-1087).
pub fn gamma_encode_fragment(color: Color, gamma: &lc_graphics::GammaRamp) -> Color {
    Color::new(
        store_channel(sample_channel(
            Some(gamma),
            lc_graphics::gamma::GammaChannel::Red,
            f32::from(color.r),
        )),
        store_channel(sample_channel(
            Some(gamma),
            lc_graphics::gamma::GammaChannel::Green,
            f32::from(color.g),
        )),
        store_channel(sample_channel(
            Some(gamma),
            lc_graphics::gamma::GammaChannel::Blue,
            f32::from(color.b),
        )),
        color.a,
    )
}

/// Gamma-samples the source in float, then performs source-alpha blending and
/// rounds once on framebuffer store. A post-composite gamma pass is observably
/// different (StdGL.cpp:908,1081-1087,1246-1263).
pub fn gamma_blend_fragment_over(
    source: Color,
    destination: Color,
    gamma: &lc_graphics::GammaRamp,
) -> Color {
    if source.a == 0 {
        return destination;
    }
    let alpha = f32::from(source.a) / 255.0;
    let blend = |channel, source: u8, destination: u8| {
        store_channel(
            sample_channel(Some(gamma), channel, f32::from(source)) * alpha
                + f32::from(destination) * (1.0 - alpha),
        )
    };
    Color::new(
        blend(
            lc_graphics::gamma::GammaChannel::Red,
            source.r,
            destination.r,
        ),
        blend(
            lc_graphics::gamma::GammaChannel::Green,
            source.g,
            destination.g,
        ),
        blend(
            lc_graphics::gamma::GammaChannel::Blue,
            source.b,
            destination.b,
        ),
        blend_color_over(source, destination).a,
    )
}

/// Draws a solid or translucent GUI rectangle through the active fragment
/// gamma lookup before alpha blending, matching `DrawBoxDw`/the GL shader.
pub fn draw_color_rect(
    surface: &mut Surface,
    rect: SurfaceRect,
    color: Color,
    gamma: Option<&lc_graphics::GammaRamp>,
) {
    let Some(clipped) = rect.intersection(surface.bounds()) else {
        return;
    };
    for y in clipped.y..clipped.y + clipped.height as i32 {
        for x in clipped.x..clipped.x + clipped.width as i32 {
            let destination = surface.get_pixel(x as u32, y as u32).unwrap_or_default();
            let output = match gamma {
                Some(gamma) if color.a == 255 => gamma_encode_fragment(color, gamma),
                Some(gamma) => gamma_blend_fragment_over(color, destination, gamma),
                None if color.a == 255 => color,
                None => blend_colors(color, destination),
            };
            let _ = surface.set_pixel(x as u32, y as u32, output);
        }
    }
}

/// Draws fallback `TextFont` glyphs through the same source-fragment gamma
/// path as `CStdDDraw::TextOut`. The temporary mask preserves glyph coverage
/// so gamma is applied before, rather than after, alpha blending.
#[allow(clippy::too_many_arguments)]
pub fn draw_text_with_gamma(
    font: &dyn TextFont,
    surface: &mut Surface,
    x: f32,
    y: f32,
    text: &str,
    size: f32,
    color: Color,
    gamma: Option<&lc_graphics::GammaRamp>,
) {
    let Some(gamma) = gamma else {
        font.draw_text(surface, x, y, text, size, color);
        return;
    };
    let metrics = font.measure_text(text, size);
    let padding = (size * 0.25).ceil() as i32 + 2;
    let mask_width = (metrics.width.ceil() as i32 + 2 * padding).max(1) as u32;
    let mask_height = (metrics.height.ceil() as i32 + 2 * padding).max(1) as u32;
    let target_x = x.floor() as i32 - padding;
    let target_y = y.floor() as i32 - padding;
    let mut mask = Surface::new(mask_width, mask_height, surface.format());
    font.draw_text(
        &mut mask,
        x - target_x as f32,
        y - target_y as f32,
        text,
        size,
        Color::new(255, 255, 255, color.a),
    );
    for mask_y in 0..mask.height() {
        let pixel_y = target_y + mask_y as i32;
        if pixel_y < 0 || pixel_y >= surface.height() as i32 {
            continue;
        }
        for mask_x in 0..mask.width() {
            let pixel_x = target_x + mask_x as i32;
            if pixel_x < 0 || pixel_x >= surface.width() as i32 {
                continue;
            }
            let Some(coverage) = mask.get_pixel(mask_x, mask_y).map(|pixel| pixel.a) else {
                continue;
            };
            if coverage == 0 {
                continue;
            }
            let destination = surface
                .get_pixel(pixel_x as u32, pixel_y as u32)
                .unwrap_or_default();
            let source = Color::new(color.r, color.g, color.b, coverage);
            let output = gamma_blend_fragment_over(source, destination, gamma);
            let _ = surface.set_pixel(pixel_x as u32, pixel_y as u32, output);
        }
    }
}

/// Textured `C4GFXBLIT_ADDITIVE`: owner/source modulation and the optional
/// R16 lookup have already selected the source fragment, then StdGL combines
/// it as `destination + source * source_alpha` and preserves framebuffer
/// alpha. C++ stores texture alpha as transparency and therefore spells the
/// equivalent source factor `GL_ONE_MINUS_SRC_ALPHA` (src/StdGL.cpp:1320-1324).
fn blend_fragment_additive(
    source: Color,
    destination: Color,
    gamma: Option<&lc_graphics::GammaRamp>,
) -> Color {
    if source.a == 0 {
        return destination;
    }
    let alpha = f32::from(source.a) / 255.0;
    let add = |channel, source: u8, destination: u8| {
        store_channel(
            f32::from(destination)
                + sample_channel(gamma, channel, f32::from(source)) * alpha,
        )
    };
    Color::new(
        add(
            lc_graphics::gamma::GammaChannel::Red,
            source.r,
            destination.r,
        ),
        add(
            lc_graphics::gamma::GammaChannel::Green,
            source.g,
            destination.g,
        ),
        add(
            lc_graphics::gamma::GammaChannel::Blue,
            source.b,
            destination.b,
        ),
        destination.a,
    )
}

fn composite_sprite_fragment(
    source: PreparedSpriteFragment,
    destination: Color,
    blit: SpriteBlitState,
    gamma: Option<&lc_graphics::GammaRamp>,
) -> Color {
    if let PreparedSpriteFragment::Legacy(source) = source {
        if blit.mode & C4GFXBLIT_ADDITIVE != 0 {
            return blend_fragment_additive(source, destination, gamma);
        }
        return match (source.a, gamma) {
            (255, Some(gamma)) => gamma_encode_fragment(source, gamma),
            (255, None) => source,
            (_, Some(gamma)) => gamma_blend_fragment_over(source, destination, gamma),
            (_, None) => blend_colors(source, destination),
        };
    }

    let PreparedSpriteFragment::Shader { rgb, alpha } = source else {
        unreachable!();
    };
    if alpha == 0 {
        return destination;
    }
    let alpha_factor = f32::from(alpha) / 255.0;
    let channel = |gamma_channel, source: f32, destination: u8| {
        let source = sample_channel(gamma, gamma_channel, source);
        if blit.mode & C4GFXBLIT_ADDITIVE != 0 {
            store_channel(f32::from(destination) + source * alpha_factor)
        } else {
            store_channel(
                source * alpha_factor + f32::from(destination) * (1.0 - alpha_factor),
            )
        }
    };
    Color::new(
        channel(
            lc_graphics::gamma::GammaChannel::Red,
            rgb[0],
            destination.r,
        ),
        channel(
            lc_graphics::gamma::GammaChannel::Green,
            rgb[1],
            destination.g,
        ),
        channel(
            lc_graphics::gamma::GammaChannel::Blue,
            rgb[2],
            destination.b,
        ),
        if blit.mode & C4GFXBLIT_ADDITIVE != 0 {
            destination.a
        } else {
            blend_color_over(Color::new(0, 0, 0, alpha), destination).a
        },
    )
}

/// Stretches `image` into `rect` with GL_LINEAR-equivalent bilinear sampling
/// (tiled textures, GL_REPEAT) and normal alpha-over blending. `gamma`
/// mirrors the per-fragment gamma lookup of the C++ blit shader.
pub fn draw_image_bilinear(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    gamma: Option<&lc_graphics::GammaRamp>,
) {
    draw_image_bilinear_impl(surface, rect, image, gamma, BilinearBlend::AlphaOver);
}

/// Stretches `image` into `rect` with bilinear sampling and additive blending
/// (`dst + src*alpha`, StdGL.cpp:908 `glBlendFunc(GL_SRC_ALPHA, GL_ONE)`), as
/// used for the GUI button focus highlight (C4GuiButton.cpp:94-98).
pub fn draw_image_bilinear_additive(
    surface: &mut Surface,
    rect: &GuiRect,
    image: &ImageData,
    gamma: Option<&lc_graphics::GammaRamp>,
) {
    draw_image_bilinear_impl(surface, rect, image, gamma, BilinearBlend::Additive);
}

fn blend_colors(foreground: Color, background: Color) -> Color {
    if foreground.a == 0 {
        return background;
    }
    if foreground.a == 255 {
        return foreground;
    }

    let alpha = foreground.a as u16;
    let inv_alpha = 255u16 - alpha;
    let blend_channel =
        |fg: u8, bg: u8| -> u8 { ((fg as u16 * alpha + bg as u16 * inv_alpha) / 255) as u8 };
    let blended_alpha = alpha + (background.a as u16 * inv_alpha) / 255;

    Color::new(
        blend_channel(foreground.r, background.r),
        blend_channel(foreground.g, background.g),
        blend_channel(foreground.b, background.b),
        blended_alpha.min(255) as u8,
    )
}

pub(crate) fn draw_text(
    surface: &mut Surface,
    rect: &GuiRect,
    text: &str,
    color: Color,
    font_size: f32,
    padding: f32,
    font: &dyn TextFont,
) {
    let origin_x = rect.origin.x + padding;
    let origin_y = rect.origin.y + padding;
    font.draw_text(surface, origin_x, origin_y, text, font_size.max(1.0), color);
}

fn rect_contains(rect: SurfaceRect, point: GuiPoint, tolerance: f32) -> bool {
    let left = rect.x as f32 - tolerance;
    let top = rect.y as f32 - tolerance;
    let right = rect.x as f32 + rect.width as f32 + tolerance;
    let bottom = rect.y as f32 + rect.height as f32 + tolerance;
    point.x >= left && point.x < right && point.y >= top && point.y < bottom
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_engine::scenario::{load_system_scripts, LegacyDefinitionResolver};
    use lc_engine::{
        CommandStackSnapshot, Engine, EnvironmentFrame, JoinPlayerConfig, Landscape,
        LiquidSegment, MaterialId, ObjectId, ObjectUpdate, ObjectVertex, PlayerState, RgbColor,
        Scenario, ScenarioError, SpawnConfig, Vector2,
    };
    use lc_graphics::{BitmapFont, PixelFormat};
    use lc_resources::{Group, MaterialLibrary};
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn test_font() -> Arc<dyn TextFont> {
        Arc::new(BitmapFont::new())
    }

    fn gray(v: u8) -> Color {
        Color::new(v, v, v, 255)
    }

    #[test]
    fn renderer_resolves_raw_draw_dir_and_flip_dir_rows() {
        // C4Object::UpdateFlipDir keeps raw Action.Dir and computes
        // DrawDir=2*FlipDir-1-Dir for the mirrored half
        // (C4Object.cpp:404-430).
        let banner = DefinitionActionGraphics {
            directions: 14,
            flip_dir: Some(7),
            ..DefinitionActionGraphics::default()
        };
        for (raw, expected) in [(13, (0, true)), (7, (6, true))] {
            let direction = Direction::from_script_value(raw);
            assert_eq!(GraphicsSystem::resolve_draw_direction(&banner, direction), expected);
        }

        let flag = DefinitionActionGraphics {
            directions: 9,
            ..DefinitionActionGraphics::default()
        };
        let direction = Direction::from_script_value(4);
        assert_eq!(
            GraphicsSystem::resolve_draw_direction(&flag, direction),
            (4, false)
        );
    }

    #[test]
    fn draw_image_strip_copies_subregion_one_to_one() {
        // 4x1 source: columns 0..4 are 10,20,30,40 gray.
        let pixels: Vec<u8> = [10u8, 20, 30, 40]
            .iter()
            .flat_map(|v| [*v, *v, *v, 255])
            .collect();
        let image = ImageData::new(4, 1, pixels);
        let mut surface = Surface::new(2, 1, PixelFormat::Rgba8888);
        draw_image_strip(&mut surface, 0, 0, &image, 2, 0, 2, 1, None);
        assert_eq!(surface.get_pixel(0, 0), Some(gray(30)));
        assert_eq!(surface.get_pixel(1, 0), Some(gray(40)));
    }

    #[test]
    fn draw_image_strip_gamma_uses_independent_rgb_tables() {
        // The blit shader samples three independent R16 gamma textures after
        // texture modulation (StdGL.cpp:1068-1087,1246-1263).
        let image = ImageData::new(1, 1, vec![0, 0, 0, 255]);
        let gamma = lc_graphics::GammaRamp::from_control_points([
            0x102030, 0x405060, 0x708090,
        ]);
        let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);

        draw_image_strip(&mut surface, 0, 0, &image, 0, 0, 1, 1, Some(&gamma));

        assert_eq!(
            surface.get_pixel(0, 0),
            Some(Color::new(17, 33, 49, 255))
        );
    }

    #[test]
    fn draw_image_bilinear_gamma_samples_r16_before_alpha_blending() {
        // Gamma lookup precedes fixed-function source-alpha blending; the
        // normalized R16 sample stays in float until framebuffer storage
        // (StdGL.cpp:908,1081-1087,1246-1255).
        let image = ImageData::new(1, 1, vec![64, 128, 192, 128]);
        let gamma = lc_graphics::GammaRamp::from_control_points([
            0x000000, 0x646464, 0xc8c8c8,
        ]);
        let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);
        surface
            .set_pixel(0, 0, Color::opaque(200, 200, 200))
            .unwrap();

        draw_image_bilinear(
            &mut surface,
            &GuiRect::new(0.0, 0.0, 1.0, 1.0),
            &image,
            Some(&gamma),
        );

        assert_eq!(
            surface.get_pixel(0, 0),
            Some(Color::new(125, 150, 175, 255))
        );
    }

    #[test]
    fn draw_image_bilinear_matches_gl_linear_sampling() {
        // 2x1 black|white stretched to 4x1: GL_LINEAR samples at texel centres
        // (i+0.5)*sw/dw - 0.5 with GL_CLAMP_TO_EDGE (C4Surface.cpp:1102):
        // 0, 64, 191, 255.
        let pixels: Vec<u8> = [0u8, 255].iter().flat_map(|v| [*v, *v, *v, 255]).collect();
        let image = ImageData::new(2, 1, pixels);
        let mut surface = Surface::new(4, 1, PixelFormat::Rgba8888);
        draw_image_bilinear(&mut surface, &GuiRect::new(0.0, 0.0, 4.0, 1.0), &image, None);
        assert_eq!(surface.get_pixel(0, 0), Some(gray(0)));
        assert_eq!(surface.get_pixel(1, 0), Some(gray(64)));
        assert_eq!(surface.get_pixel(2, 0), Some(gray(191)));
        assert_eq!(surface.get_pixel(3, 0), Some(gray(255)));
    }

    #[test]
    fn draw_image_bilinear_additive_adds_weighted_source() {
        // Additive blit per StdGL.cpp:908 glBlendFunc(GL_SRC_ALPHA, GL_ONE):
        // dst = min(dst + src*a/255, 255).
        let pixels: Vec<u8> = vec![100, 100, 100, 128];
        let image = ImageData::new(1, 1, pixels);
        let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);
        surface.set_pixel(0, 0, gray(200)).unwrap();
        draw_image_bilinear_additive(&mut surface, &GuiRect::new(0.0, 0.0, 1.0, 1.0), &image, None);
        // 200 + round(100*128/255) = 200 + 50 = 250
        assert_eq!(surface.get_pixel(0, 0), Some(Color::new(250, 250, 250, 255)));
    }

    fn empty_sprites() -> Arc<HashMap<String, DefinitionSprite>> {
        Arc::new(HashMap::new())
    }

    fn empty_cursor_atlas() -> Arc<CursorAtlas> {
        Arc::new(CursorAtlas::empty())
    }

    fn empty_hud_graphics() -> Arc<HudGraphics> {
        Arc::new(HudGraphics::default())
    }

    struct RepositoryContentResolver {
        root: PathBuf,
    }

    impl LegacyDefinitionResolver for RepositoryContentResolver {
        fn resolve_definition_groups(
            &self,
            _scenario: &Group,
            identifier: &str,
        ) -> Result<Vec<Group>, ScenarioError> {
            Group::open(self.root.join(identifier.replace('\\', "/")))
                .map(|group| vec![group])
                .map_err(ScenarioError::Resources)
        }
    }

    /// Loads an installed tutorial through the same definition/material/system
    /// prerequisites as the app. These tests deliberately render real engine
    /// snapshots and real Graphics.png facets rather than reconstructed test
    /// sprites.
    fn load_repository_tutorial(number: u8) -> Engine {
        let repository = test_support::repo_root();
        let content = repository.join("content");
        let scenario_path = content.join(format!("Tutorial.c4f/Tutorial{number:02}.c4s"));
        let scenario = Scenario::load_from_path_with(
            &scenario_path,
            &RepositoryContentResolver {
                root: content.clone(),
            },
        )
        .unwrap_or_else(|error| panic!("scenario `{}` loads: {error}", scenario_path.display()));

        let material_group = Group::open(content.join("Material.c4g"))
            .expect("installed Material.c4g opens");
        let materials =
            MaterialLibrary::from_group(&material_group).expect("installed materials load");
        let system_group =
            Group::open(repository.join("planet/System.c4g")).expect("System.c4g opens");
        let system_scripts = load_system_scripts(&system_group).expect("system scripts load");

        let mut engine = Engine::with_seed(0);
        engine.configure_materials_from_library(&materials);
        engine.install_global_scripts(&system_scripts);
        engine.set_standard_names(
            system_group
                .read_file("Names.txt")
                .ok()
                .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
        );
        scenario
            .apply(&mut engine)
            .unwrap_or_else(|error| panic!("scenario `{}` applies: {error}", scenario_path.display()));
        engine
    }

    fn join_repository_player(engine: &mut Engine, name: &str) -> i32 {
        engine
            .join_player(JoinPlayerConfig {
                name: name.to_string(),
                player_info_id: 0,
                score: 0,
                total_playing_time: 0,
                team: None,
                color_dw: 0xff_00_00,
                pref_color: 0,
                pref_position: 0,
                crew: Vec::new(),
                control_style: false,
                auto_context_menu: false,
                startup_player_count: 1,
            })
            .expect("repository tutorial player joins")
            .number
    }

    fn real_elevator_sprites(engine: &Engine) -> Arc<HashMap<String, DefinitionSprite>> {
        let mut sprites = HashMap::new();
        for definition_id in ["ELEV", "ELEC"] {
            let image = engine
                .definition_sprite_image(definition_id, None)
                .unwrap_or_else(|| panic!("{definition_id} has its real Graphics.png"));
            let width = image.width();
            let height = image.height();
            let color_mask = image
                .color_mask()
                .map(|mask| ColorByOwnerMask::new(width, height, mask));
            sprites.insert(
                sprite_map_key(definition_id, None),
                DefinitionSprite {
                    image: ImageData::from_arc(width, height, image.into_pixels()),
                    actions: engine
                        .definition_action_graphics(definition_id)
                        .unwrap_or_default(),
                    color_mask,
                    shape: engine.definition_shape_rect(definition_id),
                    stretch_growth: engine.definition_stretch_growth(definition_id),
                    top_face: engine.definition_top_face(definition_id),
                },
            );
        }
        Arc::new(sprites)
    }

    fn assert_surface_pixels_eq(actual: &Surface, expected: &Surface, context: &str) {
        assert_eq!(actual.width(), expected.width(), "{context}: width");
        assert_eq!(actual.height(), expected.height(), "{context}: height");
        if let Some((index, (actual_pixel, expected_pixel))) = actual
            .pixels()
            .chunks_exact(4)
            .zip(expected.pixels().chunks_exact(4))
            .enumerate()
            .find(|(_, (actual, expected))| actual != expected)
        {
            let x = index % actual.width() as usize;
            let y = index / actual.width() as usize;
            panic!(
                "{context}: first mismatch at ({x}, {y}): actual={actual_pixel:?}, expected={expected_pixel:?}"
            );
        }
    }

    fn make_snapshot() -> SimulationSnapshot {
        SimulationSnapshot {
            frame: 0,
            game_time: 0,
            game_over: false,
            round_results: Default::default(),
            physics: None,
            objects: vec![ObjectSnapshot {
                id: ObjectId::new(1),
                definition_id: "TestObject".to_string(),
                custom_name: None,
                position: Vector2::new(100, 100),
                velocity: Vector2::ZERO,
                rotation: 0,
                energy: 100,
                need_energy: false,
                construction: lc_engine::FULL_CON,
                damage: 0,
                magic_energy: 0,
                magic_capacity: 0,
                action: Default::default(),
                direction: Default::default(),
                command_direction: Default::default(),
                action_procedure: None,
                effects: Vec::new(),
                vertices: Vec::new(),
                contact_density: 50,
                own_vertices: None,
                container: None,
                layer: None,
                visibility: 0,
                blit_mode: 0,
                color: 0,
                color_modulation: 0,
                picture_rect: Default::default(),
                contents: Vec::new(),
                components: HashMap::new(),
                component_order: Vec::new(),
                status: Default::default(),
                owner: 0,
                controller: 0,
                category: lc_engine::DEFAULT_CATEGORY,
                crew_member: true,
                selected: false,
                alive: true,
                base_graphics: None,
                graphics_overlays: Vec::new(),
                draw_transform: None,
                command_queue: Vec::new(),
                command_stack: CommandStackSnapshot::default(),
                local_vars: HashMap::new(),
                in_liquid: false,
                mobile: false,
                ocf: 0,
            timer: 0,
                own_mass: 0,
                on_fire: false,
                fire_phase: 0,
                fire_caused_by: -1,
                info_physical: None,
                temporary_physical: None,
                physical_changes: Vec::new(),
                breath: 0,
                last_energy_loss_cause: -1,
                base: -1,
                fixed_position: None,
                fixed_velocity: None,
                rotation_velocity: None,
                fixed_rotation: None,
            }],
            render_order: Vec::new(),
            environment: EnvironmentFrame::default(),
            sky: None,
            weather_events: Vec::new(),
            global_effects: Vec::new(),
            script_globals: Default::default(),
            particles: Vec::new(),
            players: Vec::new(),
            crew_selection: Default::default(),
            crew_roles: Default::default(),
            known_crew_owners: Vec::new(),
            eliminated_crew_owners: Vec::new(),
            landscape: Some(Landscape::flat(256, 120)),
            rng: lc_engine::LcgRng::seed_from_u64(0),
            surfaces: Vec::new(),
            hud: Default::default(),
            controls: Vec::new(),
            network_packets: Vec::new(),
            definition_categories: Default::default(),
            transfer_zones: Vec::new(),
            menu_requests: Vec::new(),
            audio: Vec::new(),
        }
    }

    fn standard_gamma_color(color: Color) -> Color {
        gamma_encode_fragment(color, &lc_graphics::GammaRamp::standard())
    }

    #[test]
    fn public_gamma_render_defers_runtime_change_until_after_current_pass() {
        // A runtime SetGamma marks fSetGamma during simulation, but
        // C4GraphicsSystem::Execute draws all viewports first and calls
        // ApplyGamma only at its tail (C4GraphicsSystem.cpp:160-199).
        let snapshot = make_snapshot();
        let mut changed = snapshot.clone();
        changed
            .environment
            .gamma
            .set_ramp(0, [0x102030, 0x405060, 0x708090]);
        let viewports = [ViewportInput::from_focus(&snapshot.objects[0])];
        let make_graphics = || {
            GraphicsSystem::new(
                320,
                180,
                150,
                "Gamma Seam",
                test_font(),
                empty_sprites(),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            )
        };
        let mut public = make_graphics();
        public.render_frame(&snapshot, &viewports);
        public.render_frame(&changed, &viewports);
        let before_apply = public.surface().pixels().to_vec();
        public.render_frame(&changed, &viewports);
        let after_apply = public.surface().pixels().to_vec();

        let standard = lc_graphics::GammaRamp::from_control_points(
            snapshot.environment.gamma.combined_control_points(),
        );
        let changed_ramp = lc_graphics::GammaRamp::from_control_points(
            changed.environment.gamma.combined_control_points(),
        );
        let mut standard_render = make_graphics();
        standard_render.render_frame_with_gamma(&changed, &viewports, Some(&standard));
        let mut changed_render = make_graphics();
        changed_render.render_frame_with_gamma(&changed, &viewports, Some(&changed_ramp));

        assert_eq!(before_apply, standard_render.surface().pixels());
        assert_eq!(after_apply, changed_render.surface().pixels());
        assert_ne!(before_apply, after_apply);
    }

    #[test]
    fn tiled_viewport_background_gamma_encodes_raw_translucent_texels() {
        // The back-buffer and small-world border use fctBackground through
        // BlitSurfaceTile (C4GraphicsSystem.cpp:290; C4Viewport.cpp:1033-1036).
        let image = ImageData::new(1, 1, vec![64, 128, 192, 128]);
        let gamma = lc_graphics::GammaRamp::from_control_points([
            0x000000, 0x646464, 0xc8c8c8,
        ]);
        let render = |gamma: Option<&lc_graphics::GammaRamp>| {
            let mut surface = Surface::new(3, 2, PixelFormat::Rgba8888);
            surface.fill(Color::opaque(7, 11, 13));
            tile_image_on_surface(&mut surface, &image, 0, 0, gamma);
            surface.pixels().to_vec()
        };

        assert!(render(Some(&gamma))
            .chunks_exact(4)
            .all(|pixel| pixel == [50, 100, 150, 128]));
        assert!(render(None)
            .chunks_exact(4)
            .all(|pixel| pixel == [64, 128, 192, 128]));
    }

    #[test]
    fn gamma_render_seam_encodes_sky_channels_independently() {
        // C4Sky::Draw emits its solid/fade colours through DrawBoxDw/Fade;
        // DummyShader samples three independent gamma textures before output
        // (C4Sky.cpp:206-225; StdGL.cpp:1185-1200).
        let mut graphics = GraphicsSystem::new(
            1,
            1,
            1,
            "Gamma Sky",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let environment = EnvironmentFrame {
            sky_color: Some(RgbColor::new(0, 0, 0)),
            ..EnvironmentFrame::default()
        };
        let gamma = lc_graphics::GammaRamp::from_control_points([
            0x102030, 0x405060, 0x708090,
        ]);

        graphics.draw_sky(None, &environment, &[], 1.0, Some(&gamma));

        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(Color::new(17, 33, 49, 255))
        );
    }

    #[test]
    fn gamma_render_seam_encodes_tutorial_six_sky_gradient() {
        // DrawBoxFade interpolation is gamma sampled per fragment before the
        // framebuffer store (C4Sky.cpp:219-225; StdGL.cpp:846-889,1193-1200).
        let mut graphics = GraphicsSystem::new(
            1,
            1,
            1,
            "Gamma Gradient",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let gamma = lc_graphics::GammaRamp::from_control_points([
            0x000000, 0x646464, 0xc8c8c8,
        ]);

        graphics.fill_vertical_gradient(
            Color::opaque(64, 128, 192),
            Color::opaque(64, 128, 192),
            1.0,
            Some(&gamma),
        );

        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(Color::new(50, 100, 150, 255))
        );
    }

    #[test]
    fn gamma_render_seam_encodes_sky_image_before_alpha_blending() {
        // C4Sky::Draw sends its tiled surface through BlitSurfaceTile2, whose
        // shader gamma-samples the source before blending (C4Sky.cpp:210-218;
        // StdGL.cpp:1068-1087).
        let mut graphics = GraphicsSystem::new(
            1,
            1,
            1,
            "Gamma Sky Image",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics
            .surface_mut()
            .set_pixel(0, 0, Color::opaque(200, 200, 200))
            .expect("background pixel");
        let image = ImageData::new(1, 1, vec![64, 128, 192, 128]);
        let gamma = lc_graphics::GammaRamp::from_control_points([
            0x000000, 0x646464, 0xc8c8c8,
        ]);

        graphics.blit_sky_tile(&image, 0, 0, None, 1.0, Some(&gamma));

        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(Color::new(125, 150, 175, 255))
        );
    }

    #[test]
    fn gamma_render_seam_encodes_fallback_landscape_fragments() {
        // The fallback painter stands in for the same landscape presentation
        // shader. Even black is sampled through MinGamma, yielding one rather
        // than a raw zero (StdGL.cpp:1139-1148; StdDDraw2.cpp:237-271).
        let mut graphics = GraphicsSystem::new(
            1,
            1,
            0,
            "Gamma Fallback Ground",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let gamma = lc_graphics::GammaRamp::from_control_points([
            0x000000, 0x646464, 0xc8c8c8,
        ]);

        assert!(!graphics.draw_ground(0, None, 0.0, Some(&gamma)));

        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(Color::new(1, 1, 1, 255))
        );
    }

    #[test]
    fn color_by_owner_uses_object_color_instead_of_owner_lookup() {
        // C4Object::DrawFace passes the live C4Object::Color to GetBitmap
        // (C4Object.cpp:440-477). This may differ from the current player
        // color after SetColorDw, and unowned FISH explicitly sets white in
        // Birth (Objects.c4d/Animals.c4d/Fish.c4d/Script.c:233-240).
        let sprite = DefinitionSprite {
            // CreateColorByOwner clears owner-only base pixels to black.
            image: ImageData::new(1, 1, vec![0, 0, 0, 255]),
            actions: HashMap::new(),
            color_mask: Some(ColorByOwnerMask::new(1, 1, Arc::from([128]))),
            shape: Some(DefinitionRect::new(0, 0, 1, 1)),
            stretch_growth: false,
            top_face: None,
        };
        let mut recolored = make_snapshot().objects.remove(0);
        recolored.definition_id = "ObjectColor".to_string();
        recolored.position = Vector2::new(1, 1);
        recolored.owner = 7;
        recolored.color = 0x00ff_0000;
        recolored.crew_member = false;

        let mut fish = recolored.clone();
        fish.id = ObjectId::new(2);
        fish.position = Vector2::new(2, 1);
        fish.owner = OWNER_NONE;
        fish.color = 0x00ff_ffff;

        let mut graphics = GraphicsSystem::new(
            4,
            3,
            3,
            "Object ColorByOwner",
            test_font(),
            Arc::new(HashMap::from([(
                sprite_map_key("ObjectColor", None),
                sprite,
            )])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        graphics.draw_objects(
            &[recolored, fish],
            &[],
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::from([(7, Color::opaque(0, 0, 255))]),
            ObjectRenderPass::Normal,
            None,
        );

        assert_eq!(
            graphics.surface().get_pixel(1, 1),
            Some(Color::opaque(128, 0, 0)),
            "SetColorDw red must win over the owner's blue player color"
        );
        assert_eq!(
            graphics.surface().get_pixel(2, 1),
            Some(Color::opaque(128, 128, 128)),
            "an unowned white FISH must not expose its cleared black base"
        );
    }

    #[test]
    fn color_by_owner_preserves_packed_transparency_with_ownclr() {
        // C4Surface::ClrByOwnerClr is passed to PerformBlt as the full packed
        // C4 color. Its high byte is transparency, and bit 4 keeps this raw
        // object color instead of folding in global ColorMod
        // (StdDDraw2.cpp:773-777). ReleaseClonk uses this exact combination.
        let sprite = DefinitionSprite {
            image: ImageData::new(1, 1, vec![0, 0, 0, 255]),
            actions: HashMap::new(),
            color_mask: Some(ColorByOwnerMask::new(1, 1, Arc::from([255]))),
            shape: Some(DefinitionRect::new(0, 0, 1, 1)),
            stretch_growth: false,
            top_face: None,
        };
        let mut object = make_snapshot().objects.remove(0);
        object.definition_id = "OwnerTransparency".to_string();
        object.position = Vector2::new(1, 1);
        object.color = 0x80ff_0000;
        object.color_modulation = 0x0000_ff00;
        object.blit_mode = C4GFXBLIT_CLRSFC_OWNCLR;
        object.crew_member = false;

        let mut graphics = GraphicsSystem::new(
            3,
            3,
            3,
            "Packed owner transparency",
            test_font(),
            Arc::new(HashMap::from([(
                sprite_map_key("OwnerTransparency", None),
                sprite,
            )])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(0, 0, 200));
        graphics.draw_objects(
            &[object],
            &[],
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        assert_eq!(
            graphics.surface().get_pixel(1, 1),
            Some(Color::opaque(127, 0, 100)),
            "0x80 transparency must blend raw red over blue; OWNCLR must ignore green ColorMod"
        );
    }

    #[test]
    fn object_additive_bit_covers_base_action_and_top_face_after_gamma() {
        // C4Object::Draw brackets its base/action facet with PrepareDrawing
        // (C4Object.cpp:2410-2416,2498-2499), and DrawTopFace brackets the
        // separate top pass the same way (C4Object.cpp:2648-2672). Bit 1 is
        // additive even alongside C4GFXBLIT_CUSTOM (C4Surface.h:38-49).
        let source = Color::new(64, 128, 192, 128);
        let mut sprites = HashMap::new();
        sprites.insert(
            sprite_map_key("BaseAdd", None),
            DefinitionSprite {
                image: ImageData::new(1, 1, vec![source.r, source.g, source.b, source.a]),
                actions: HashMap::new(),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                stretch_growth: false,
                top_face: None,
            },
        );
        sprites.insert(
            sprite_map_key("ActionAdd", None),
            DefinitionSprite {
                image: ImageData::new(1, 1, vec![source.r, source.g, source.b, source.a]),
                actions: HashMap::from([(
                    "Active".to_string(),
                    DefinitionActionGraphics {
                        facet: Some(lc_engine::DefinitionActionFacet {
                            x: 0,
                            y: 0,
                            width: 1,
                            height: 1,
                            target_x: 0,
                            target_y: 0,
                        }),
                        length: Some(1),
                        ..DefinitionActionGraphics::default()
                    },
                )]),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                stretch_growth: false,
                top_face: None,
            },
        );
        sprites.insert(
            sprite_map_key("TopAdd", None),
            DefinitionSprite {
                image: ImageData::new(
                    2,
                    1,
                    vec![0, 0, 0, 0, source.r, source.g, source.b, source.a],
                ),
                actions: HashMap::new(),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                stretch_growth: false,
                top_face: Some(DefinitionTargetRect::new(1, 0, 1, 1, 0, 0)),
            },
        );

        let template = make_snapshot().objects.remove(0);
        let make_object = |id, definition_id: &str, x, action: &str, blit_mode| {
            let mut object = template.clone();
            object.id = ObjectId::new(id);
            object.definition_id = definition_id.to_string();
            object.position = Vector2::new(x, 1);
            object.action = lc_engine::ActionState::new(action);
            object.blit_mode = blit_mode;
            object.crew_member = false;
            object
        };
        let objects = vec![
            make_object(1, "BaseAdd", 1, "Idle", 129),
            make_object(2, "ActionAdd", 3, "Active", 129),
            make_object(3, "TopAdd", 5, "Idle", 129),
            make_object(4, "BaseAdd", 7, "Idle", 0),
        ];
        let gamma = lc_graphics::GammaRamp::from_control_points([
            0x000000, 0x646464, 0xc8c8c8,
        ]);
        let mut graphics = GraphicsSystem::new(
            9,
            3,
            3,
            "Object additive",
            test_font(),
            Arc::new(sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(200, 200, 200));

        graphics.draw_objects(
            &objects,
            &[],
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            Some(&gamma),
        );

        let additive = Some(Color::opaque(225, 250, 255));
        for (label, x) in [("base", 1), ("action", 3), ("top", 5)] {
            assert_eq!(graphics.surface().get_pixel(x, 1), additive, "{label}");
        }
        assert_eq!(
            graphics.surface().get_pixel(7, 1),
            Some(Color::opaque(125, 150, 175)),
            "normal mode must remain source-alpha over"
        );
    }

    #[test]
    fn graphics_overlay_additive_and_exact_parent_mode_preserve_owner_modulation() {
        // C4GraphicsOverlay::Draw uses its own mode, except exact
        // C4GFXBLIT_PARENT, which calls the parent object's PrepareDrawing
        // (C4DefGraphics.cpp:753-768,824-831). ColorByOwner modulation happens
        // before the selected framebuffer blend (StdDDraw2.cpp:769-777).
        let mut object = make_snapshot().objects.remove(0);
        object.position = Vector2::new(1, 1);
        object.blit_mode = 1;
        let source = Color::new(10, 20, 30, 128);
        let owner = 0x0064_788c;
        let sprite = DefinitionSprite {
            image: ImageData::new(1, 1, vec![source.r, source.g, source.b, source.a]),
            actions: HashMap::new(),
            color_mask: Some(ColorByOwnerMask::new(1, 1, Arc::from([255]))),
            shape: Some(DefinitionRect::new(0, 0, 1, 1)),
            stretch_growth: false,
            top_face: None,
        };
        let render = |object_mode, overlay_mode| {
            let mut object = object.clone();
            object.blit_mode = object_mode;
            object.graphics_overlays = vec![ObjectGraphicsOverlay::new(
                1,
                GraphicsOverlayMode::Base,
            )
            .with_definition(Some("OverlayAdd".to_string()))
            .with_blit_mode(overlay_mode)];
            let mut graphics = GraphicsSystem::new(
                3,
                3,
                3,
                "Overlay additive",
                test_font(),
                Arc::new(HashMap::from([(
                    sprite_map_key("OverlayAdd", None),
                    sprite.clone(),
                )])),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.surface_mut().fill(Color::opaque(20, 30, 40));
            graphics.draw_object_overlays(
                &object,
                &[],
                &[],
                OWNER_NONE,
                Some(owner),
                1.0,
                1.0,
                1.0,
                0.0,
                None,
                None,
            );
            graphics.surface().get_pixel(1, 1)
        };

        let additive = Some(Color::opaque(70, 90, 110));
        assert_eq!(render(0, 1), additive, "explicit overlay additive");
        assert_eq!(render(1, 256), additive, "exact parent inheritance");
        assert_eq!(
            render(1, 0),
            Some(Color::opaque(60, 75, 90)),
            "explicit normal overlay must override an additive parent"
        );
    }

    #[test]
    fn object_overlay_draws_contained_overlay_only_target_at_host_offset() {
        // MODE_Object rewrites the viewport target so the referenced object is
        // drawn at the host position, using only the overlay transform's
        // truncated translation. It invokes both Draw and DrawTopFace with
        // ODM_Overlay, so containment is ignored and VIS_OverlayOnly is
        // evaluated with fAsOverlay=true (C4DefGraphics.cpp:753-789;
        // C4Object.cpp:2237-2258,2502-2505,2572-2580,2631-2633,5600-5608).
        let mut template = make_snapshot().objects.remove(0);
        template.crew_member = false;

        let mut host = template.clone();
        host.id = ObjectId::new(1);
        host.definition_id = "OverlayHost".to_string();
        host.position = Vector2::new(3, 3);
        host.blit_mode = 0;
        host.draw_transform = Some(DrawTransform::from_components(4.0, 4.0, 5.0, 5.0));
        host.graphics_overlays = vec![
            ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Object)
                .with_blit_mode(C4GFXBLIT_PARENT)
                .with_overlay_object(Some(ObjectId::new(2)))
                .with_transform(Some(DrawTransform::from_components(3.0, 2.0, 2.9, -1.9))),
        ];

        let mut target = template;
        target.id = ObjectId::new(2);
        target.definition_id = "OverlayTarget".to_string();
        target.position = Vector2::new(10, 6);
        target.container = Some(host.id);
        target.owner = 4;
        target.visibility = lc_engine::VIS_OVERLAY_ONLY | lc_engine::VIS_OWNER;
        target.blit_mode = C4GFXBLIT_ADDITIVE;
        target.graphics_overlays.clear();

        let sprites = Arc::new(HashMap::from([
            (
                sprite_map_key("OverlayHost", None),
                DefinitionSprite {
                    image: ImageData::new(1, 1, vec![0, 0, 0, 0]),
                    actions: HashMap::new(),
                    color_mask: None,
                    shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                    stretch_growth: false,
                    top_face: None,
                },
            ),
            (
                sprite_map_key("OverlayTarget", None),
                DefinitionSprite {
                    image: ImageData::new(2, 1, vec![40, 0, 0, 255, 0, 40, 0, 255]),
                    actions: HashMap::new(),
                    color_mask: None,
                    shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                    stretch_growth: false,
                    top_face: Some(DefinitionTargetRect::new(1, 0, 1, 1, 1, 0)),
                },
            ),
        ]));
        let render = |for_player| {
            let mut graphics = GraphicsSystem::new(
                12,
                8,
                8,
                "Object overlay",
                test_font(),
                Arc::clone(&sprites),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.surface_mut().fill(Color::opaque(10, 10, 10));
            graphics.draw_objects(
                &[host.clone(), target.clone()],
                &[],
                &[],
                for_player,
                1.0,
                &HashMap::new(),
                ObjectRenderPass::Normal,
                None,
            );
            graphics.surface().clone()
        };

        let visible = render(4);
        assert_eq!(visible.get_pixel(5, 2), Some(Color::opaque(50, 10, 10)));
        assert_eq!(visible.get_pixel(6, 2), Some(Color::opaque(10, 50, 10)));
        assert_eq!(visible.get_pixel(7, 2), Some(Color::opaque(10, 10, 10)));
        assert_eq!(visible.get_pixel(10, 6), Some(Color::opaque(10, 10, 10)));

        let hidden = render(5);
        assert_eq!(hidden.get_pixel(5, 2), Some(Color::opaque(10, 10, 10)));
        assert_eq!(hidden.get_pixel(6, 2), Some(Color::opaque(10, 10, 10)));

        let render_parallax = |host: ObjectSnapshot,
                               target: ObjectSnapshot,
                               viewport: Vector2,
                               width: u32,
                               height: u32| {
            let mut graphics = GraphicsSystem::new(
                width,
                height,
                height as i32,
                "Parallax object overlay",
                test_font(),
                Arc::clone(&sprites),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.viewport_x = viewport.x as f32;
            graphics.viewport_y = viewport.y as f32;
            graphics.surface_mut().fill(Color::opaque(10, 10, 10));
            graphics.draw_objects(
                &[host, target],
                &[],
                &[],
                4,
                1.0,
                &HashMap::new(),
                ObjectRenderPass::Normal,
                None,
            );
            graphics.surface().clone()
        };

        let int_value = |value| {
            serde_json::from_value(serde_json::json!({ "Int": value }))
                .expect("deserialize C4Script integer")
        };
        let mut percentage_host = host.clone();
        percentage_host.position = Vector2::new(50, 30);
        percentage_host.category |= CATEGORY_PARALLAX_FLAG;
        percentage_host
            .local_vars
            .insert("__local_0".to_string(), int_value(50));
        percentage_host
            .local_vars
            .insert("__local_1".to_string(), int_value(50));
        let mut percentage_target = target.clone();
        percentage_target.position = Vector2::new(70, 40);
        let percentage = render_parallax(
            percentage_host,
            percentage_target,
            Vector2::new(20, 20),
            80,
            50,
        );
        assert_eq!(
            percentage.get_pixel(42, 19),
            Some(Color::opaque(50, 10, 10)),
            "Local(0/1)=50 scales viewport TargetX/Y before MODE_Object anchoring"
        );

        let mut hud_host = host.clone();
        hud_host.position = Vector2::new(-10, -5);
        hud_host.category |= CATEGORY_PARALLAX_FLAG;
        hud_host
            .local_vars
            .insert("__local_0".to_string(), int_value(0));
        hud_host
            .local_vars
            .insert("__local_1".to_string(), int_value(0));
        let hud = render_parallax(
            hud_host,
            target,
            Vector2::new(20, 20),
            80,
            50,
        );
        assert_eq!(
            hud.get_pixel(72, 44),
            Some(Color::opaque(50, 10, 10)),
            "zero parallax plus negative host coordinates anchors from right/bottom"
        );
    }

    #[test]
    fn shipped_star_definition_uses_additive_action_graphics() {
        // The real STAR definition declares BlitMode=1 and its Appear action
        // uses ten 3x3 frames. Phase four's opaque centre is grey 184.
        let definition = crate::test_support::repo_root()
            .join("content/Objects.c4d/Environment.c4d/Stars.c4d/Star.c4d");
        let def_core = std::fs::read_to_string(definition.join("DefCore.txt"))
            .expect("read shipped STAR DefCore");
        assert!(def_core.lines().any(|line| line.trim() == "BlitMode=1"));
        let rgba = image::open(definition.join("Graphics.png"))
            .expect("decode shipped STAR graphics")
            .into_rgba8();
        let (width, height) = rgba.dimensions();
        let sprite = DefinitionSprite {
            image: ImageData::new(width, height, rgba.into_raw()),
            actions: HashMap::from([(
                "Appear".to_string(),
                DefinitionActionGraphics {
                    facet: Some(lc_engine::DefinitionActionFacet {
                        x: 0,
                        y: 0,
                        width: 3,
                        height: 3,
                        target_x: 0,
                        target_y: 0,
                    }),
                    length: Some(10),
                    ..DefinitionActionGraphics::default()
                },
            )]),
            color_mask: None,
            shape: Some(DefinitionRect::new(-2, -2, 4, 4)),
            stretch_growth: false,
            top_face: None,
        };
        let mut star = make_snapshot().objects.remove(0);
        star.definition_id = "STAR".to_string();
        star.position = Vector2::new(5, 5);
        star.action = lc_engine::ActionState::new("Appear");
        star.action.phase = 4;
        star.blit_mode = 1;
        star.crew_member = false;
        let mut graphics = GraphicsSystem::new(
            10,
            10,
            10,
            "Shipped STAR additive",
            test_font(),
            Arc::new(HashMap::from([(sprite_map_key("STAR", None), sprite)])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(50, 60, 70));

        graphics.draw_objects(
            &[star],
            &[],
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        assert_eq!(
            graphics.surface().get_pixel(4, 4),
            Some(Color::opaque(234, 244, 254))
        );
    }

    #[test]
    fn object_mod2_modulates_base_action_top_and_rotated_faces() {
        // Object ColorMod is activated around both C4Object::Draw passes
        // (C4Object.cpp:2410-2499,2648-2672). Bit 2 selects BlitShaderMod2
        // for the main surface (StdDDraw2.cpp:768-770; StdGL.cpp:1072-1079).
        let source = Color::new(64, 128, 192, 128);
        let plain_sprite = |width, height, shape| DefinitionSprite {
            image: ImageData::new(
                width,
                height,
                (0..width * height)
                    .flat_map(|_| [source.r, source.g, source.b, source.a])
                    .collect(),
            ),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(shape),
            stretch_growth: false,
            top_face: None,
        };
        let mut sprites = HashMap::from([(
            sprite_map_key("BaseMod2", None),
            plain_sprite(1, 1, DefinitionRect::new(0, 0, 1, 1)),
        )]);
        let mut action = plain_sprite(1, 1, DefinitionRect::new(0, 0, 1, 1));
        action.actions.insert(
            "Active".to_string(),
            DefinitionActionGraphics {
                facet: Some(lc_engine::DefinitionActionFacet {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                    target_x: 0,
                    target_y: 0,
                }),
                length: Some(1),
                ..DefinitionActionGraphics::default()
            },
        );
        sprites.insert(sprite_map_key("ActionMod2", None), action);
        sprites.insert(
            sprite_map_key("TopMod2", None),
            DefinitionSprite {
                image: ImageData::new(
                    2,
                    1,
                    vec![0, 0, 0, 0, source.r, source.g, source.b, source.a],
                ),
                actions: HashMap::new(),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                stretch_growth: false,
                top_face: Some(DefinitionTargetRect::new(1, 0, 1, 1, 0, 0)),
            },
        );
        sprites.insert(
            sprite_map_key("RotatedMod2", None),
            plain_sprite(3, 3, DefinitionRect::new(-1, -1, 3, 3)),
        );

        let template = make_snapshot().objects.remove(0);
        let make_object = |id, definition: &str, position, action: &str, rotation| {
            let mut object = template.clone();
            object.id = ObjectId::new(id);
            object.definition_id = definition.to_string();
            object.position = position;
            object.action = lc_engine::ActionState::new(action);
            object.rotation = rotation;
            object.blit_mode = 2;
            object.color_modulation = 0x0020_4080;
            object.crew_member = false;
            object
        };
        let objects = vec![
            make_object(1, "BaseMod2", Vector2::new(1, 2), "Idle", 0),
            make_object(2, "ActionMod2", Vector2::new(3, 2), "Active", 0),
            make_object(3, "TopMod2", Vector2::new(5, 2), "Idle", 0),
            make_object(4, "RotatedMod2", Vector2::new(8, 2), "Idle", 45),
        ];
        let mut graphics = GraphicsSystem::new(
            11,
            5,
            5,
            "Object MOD2 routes",
            test_font(),
            Arc::new(sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(200, 200, 200));

        graphics.draw_objects(
            &objects,
            &[],
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        // Shader MOD2 source: clamp(2*src + 2*mod - 255) = (0,129,255),
        // then ordinary source-alpha over the framebuffer.
        let expected = Some(Color::opaque(100, 164, 228));
        for (route, x) in [("base", 1), ("action", 3), ("top", 5), ("rotated", 8)] {
            assert_eq!(graphics.surface().get_pixel(x, 2), expected, "{route}");
        }
    }

    #[test]
    fn object_mod2_black_reset_and_additive_gamma_precedence_match_stdgl() {
        // PerformBlt resets MOD2 when the active modulation is all black
        // (StdGL.cpp:442-472), yielding a normal black silhouette. Additive
        // remains an independent framebuffer blend bit (StdGL.cpp:1320-1324).
        let mut sprites = HashMap::new();
        sprites.insert(
            sprite_map_key("BlackMod2", None),
            DefinitionSprite {
                image: ImageData::new(1, 1, vec![200, 200, 200, 255]),
                actions: HashMap::new(),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                stretch_growth: false,
                top_face: None,
            },
        );
        sprites.insert(
            sprite_map_key("AddMod2", None),
            DefinitionSprite {
                image: ImageData::new(1, 1, vec![64, 128, 192, 128]),
                actions: HashMap::new(),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                stretch_growth: false,
                top_face: None,
            },
        );
        let template = make_snapshot().objects.remove(0);
        let mut black = template.clone();
        black.definition_id = "BlackMod2".to_string();
        black.position = Vector2::new(1, 1);
        black.blit_mode = 2;
        black.color_modulation = 0;
        black.crew_member = false;
        let mut combined = template;
        combined.id = ObjectId::new(2);
        combined.definition_id = "AddMod2".to_string();
        combined.position = Vector2::new(3, 1);
        combined.blit_mode = 1 | 2 | 128;
        combined.color_modulation = 0x0020_4080;
        combined.crew_member = false;
        let gamma = lc_graphics::GammaRamp::from_control_points([
            0x000000, 0x646464, 0xc8c8c8,
        ]);
        let mut graphics = GraphicsSystem::new(
            5,
            3,
            3,
            "MOD2 precedence",
            test_font(),
            Arc::new(sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(100, 100, 100));

        graphics.draw_objects(
            &[black, combined],
            &[],
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            Some(&gamma),
        );

        assert_eq!(
            graphics.surface().get_pixel(1, 1),
            Some(gamma_encode_fragment(Color::opaque(0, 0, 0), &gamma))
        );
        let modulated = [0.0, 129.0, 255.0];
        let alpha = 128.0 / 255.0;
        let expected = Color::opaque(
            store_channel(
                100.0
                    + sample_channel(
                        Some(&gamma),
                        lc_graphics::gamma::GammaChannel::Red,
                        modulated[0],
                    ) * alpha,
            ),
            store_channel(
                100.0
                    + sample_channel(
                        Some(&gamma),
                        lc_graphics::gamma::GammaChannel::Green,
                        modulated[1],
                    ) * alpha,
            ),
            store_channel(
                100.0
                    + sample_channel(
                        Some(&gamma),
                        lc_graphics::gamma::GammaChannel::Blue,
                        modulated[2],
                    ) * alpha,
            ),
        );
        assert_eq!(graphics.surface().get_pixel(3, 1), Some(expected));
    }

    #[test]
    fn color_by_owner_bits_four_and_eight_have_distinct_source_modulation() {
        // Base and owner surfaces are separate C++ passes. Bit 4 keeps the
        // owner's raw color independent of global ColorMod; bit 8 selects
        // MOD2 only for the grey owner surface (StdDDraw2.cpp:768-778).
        let sprite = DefinitionSprite {
            image: ImageData::new(1, 1, vec![255, 255, 255, 255]),
            actions: HashMap::new(),
            color_mask: Some(ColorByOwnerMask::new(1, 1, Arc::from([64]))),
            shape: Some(DefinitionRect::new(0, 0, 1, 1)),
            stretch_growth: false,
            top_face: None,
        };
        let owner = Color::opaque(64, 128, 192);
        let render = |mode| {
            let mut object = make_snapshot().objects.remove(0);
            object.definition_id = "OwnerModes".to_string();
            object.position = Vector2::new(1, 1);
            object.blit_mode = mode;
            object.color = 0x0040_80c0;
            object.color_modulation = 0x0080_4020;
            object.crew_member = false;
            let mut graphics = GraphicsSystem::new(
                3,
                3,
                3,
                "Owner modulation modes",
                test_font(),
                Arc::new(HashMap::from([(
                    sprite_map_key("OwnerModes", None),
                    sprite.clone(),
                )])),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.surface_mut().fill(Color::opaque(9, 11, 13));
            graphics.draw_objects(
                &[object],
                &[],
                &[],
                OWNER_NONE,
                1.0,
                &HashMap::from([(0, owner)]),
                ObjectRenderPass::Normal,
                None,
            );
            graphics.surface().get_pixel(1, 1)
        };

        // owner ⊗ global is (32,32,24) by C++'s >>8 combine. The owner
        // texture is grey 64. Bit 8's normalized shader formula is
        // clamp(2*grey + 2*mod - 255), proving it is not bit-2 aliasing.
        assert_eq!(render(0), Some(Color::opaque(8, 8, 6)));
        assert_eq!(render(4), Some(Color::opaque(16, 32, 48)));
        assert_eq!(render(8), Some(Color::opaque(0, 0, 0)));
        assert_eq!(render(4 | 8), Some(Color::opaque(1, 129, 255)));
    }

    #[test]
    fn overlay_mod2_uses_local_modulation_or_exact_parent_state() {
        // Explicit overlays activate modulation only when their color differs
        // from 0x00ffffff (C4DefGraphics.cpp:762-768). Thus mode 2 + default
        // white is MOD2-to-white, while explicit black triggers the PerformBlt
        // black reset. Exact parent mode inherits both mode and ColorMod.
        let sprite = DefinitionSprite {
            image: ImageData::new(3, 3, [64, 128, 192, 255].repeat(9)),
            actions: HashMap::new(),
            color_mask: None,
            shape: Some(DefinitionRect::new(-1, -1, 3, 3)),
            stretch_growth: false,
            top_face: None,
        };
        let render = |overlay_mode, overlay_modulation, rotation| {
            let mut object = make_snapshot().objects.remove(0);
            object.position = Vector2::new(2, 2);
            object.blit_mode = 2;
            object.color_modulation = 0x0020_4080;
            let mut overlay = ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Base)
                .with_definition(Some("OverlayMod2".to_string()))
                .with_blit_mode(overlay_mode);
            overlay.color_modulation = overlay_modulation;
            object.graphics_overlays = vec![overlay];
            let mut graphics = GraphicsSystem::new(
                5,
                5,
                5,
                "Overlay MOD2",
                test_font(),
                Arc::new(HashMap::from([(
                    sprite_map_key("OverlayMod2", None),
                    sprite.clone(),
                )])),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.surface_mut().fill(Color::opaque(9, 11, 13));
            graphics.draw_object_overlays(
                &object,
                &[],
                &[],
                OWNER_NONE,
                None,
                2.0,
                2.0,
                1.0,
                rotation,
                None,
                None,
            );
            graphics.surface().get_pixel(2, 2)
        };

        assert_eq!(render(2, 0x00ff_ffff, 0.0), Some(Color::opaque(255, 255, 255)));
        assert_eq!(render(2, 0, 0.0), Some(Color::opaque(0, 0, 0)));
        assert_eq!(render(256, 0x00ff_ffff, 45.0), Some(Color::opaque(0, 129, 255)));
        assert_eq!(render(0, 0x0020_4080, 0.0), Some(Color::opaque(8, 32, 96)));
    }

    #[test]
    fn shipped_firelump_uses_mod2_color_modulation() {
        // FRBL declares BlitMode=2; Existing() continuously assigns
        // SetClrModulation(RGB(iR,iG,64)). Use one real sheet texel from its
        // base face to pin shipped MOD2 behavior.
        let definition = crate::test_support::repo_root()
            .join("content/Fantasy.c4d/Magic.c4d/Firelump.c4d/Fball.c4d");
        let def_core = std::fs::read_to_string(definition.join("DefCore.txt"))
            .expect("read shipped FRBL DefCore");
        assert!(def_core.lines().any(|line| line.trim() == "BlitMode=2"));
        let script = std::fs::read(definition.join("Script.c")).expect("read shipped FRBL Script");
        assert!(script
            .windows(b"SetClrModulation(RGB(iR,iG,64))".len())
            .any(|window| window == b"SetClrModulation(RGB(iR,iG,64))"));
        let rgba = image::open(definition.join("Graphics.png"))
            .expect("decode shipped FRBL graphics")
            .into_rgba8();
        let (width, height) = rgba.dimensions();
        let sprite = DefinitionSprite {
            image: ImageData::new(width, height, rgba.into_raw()),
            actions: HashMap::from([(
                "Exist".to_string(),
                DefinitionActionGraphics {
                    facet_base: true,
                    ..DefinitionActionGraphics::default()
                },
            )]),
            color_mask: None,
            shape: Some(DefinitionRect::new(-5, -5, 10, 10)),
            stretch_growth: false,
            top_face: None,
        };
        let mut firelump = make_snapshot().objects.remove(0);
        firelump.definition_id = "FRBL".to_string();
        firelump.position = Vector2::new(10, 10);
        firelump.action = lc_engine::ActionState::new("Exist");
        firelump.blit_mode = 2;
        firelump.color_modulation = 0x0018_2040;
        firelump.crew_member = false;
        let mut graphics = GraphicsSystem::new(
            20,
            20,
            20,
            "Shipped FRBL MOD2",
            test_font(),
            Arc::new(HashMap::from([(sprite_map_key("FRBL", None), sprite)])),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(50, 60, 70));

        graphics.draw_objects(
            &[firelump],
            &[],
            &[],
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            None,
        );

        // Graphics.png (6,0) = (255,140,0,179). MOD2 with (24,32,64)
        // produces (255,89,0), then alpha-over gives this framebuffer value.
        assert_eq!(graphics.surface().get_pixel(11, 5), Some(Color::opaque(194, 80, 21)));
    }

    #[test]
    fn object_and_old_style_pxs_gamma_sample_independent_r16_channels() {
        let gamma = lc_graphics::GammaRamp::from_control_points([
            0x102030, 0x405060, 0x708090,
        ]);
        let snapshot = make_snapshot();
        let mut graphics = GraphicsSystem::new(
            128,
            128,
            128,
            "Gamma Object/PXS",
            test_font(),
            solid_sprite(
                "TestObject",
                1,
                1,
                Color::opaque(0, 0, 0),
                Some(DefinitionRect::new(0, 0, 1, 1)),
                false,
            ),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(200, 200, 200));
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "rain".to_string(),
            MaterialRenderInfo::new([0; 9], [0; 6], None, 0, 25),
        )])));

        graphics.draw_pxs(
            &[pxs_particle("rain", [96 << 16, 100 << 16, 0, 0], 0)],
            1.0,
            Some(&gamma),
        );
        graphics.draw_objects(
            &snapshot.objects,
            &snapshot.render_order,
            &snapshot.players,
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            ObjectRenderPass::Normal,
            Some(&gamma),
        );

        let encoded = Some(Color::new(17, 33, 49, 255));
        assert_eq!(graphics.surface().get_pixel(96, 100), encoded);
        assert_eq!(graphics.surface().get_pixel(100, 100), encoded);
    }

    #[test]
    fn rotated_base_overlay_gamma_samples_before_translucent_blending() {
        let mut object = make_snapshot().objects.remove(0);
        let mut overlay = ObjectGraphicsOverlay::new(1, GraphicsOverlayMode::Base);
        overlay.definition = Some("Overlay".to_string());
        object.graphics_overlays.push(overlay);
        let mut graphics = GraphicsSystem::new(
            9,
            9,
            9,
            "Gamma Rotated Overlay",
            test_font(),
            solid_sprite(
                "Overlay",
                3,
                3,
                Color::new(64, 128, 192, 128),
                Some(DefinitionRect::new(-1, -1, 3, 3)),
                false,
            ),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(200, 200, 200));
        let gamma = lc_graphics::GammaRamp::from_control_points([
            0x000000, 0x646464, 0xc8c8c8,
        ]);

        graphics.draw_object_overlays(
            &object,
            &[],
            &[],
            OWNER_NONE,
            None,
            4.0,
            4.0,
            1.0,
            45.0,
            None,
            Some(&gamma),
        );

        assert_eq!(
            graphics.surface().get_pixel(4, 4),
            Some(Color::new(125, 150, 175, 255))
        );
    }

    #[test]
    fn graphical_pxs_gamma_samples_filtered_rgb_before_translucent_blending() {
        let mut surface = Surface::new(1, 1, PixelFormat::Rgba8888);
        surface.fill(Color::opaque(200, 200, 200));
        let image = ImageData::new(1, 1, vec![64, 128, 192, 128]);
        let gamma = lc_graphics::GammaRamp::from_control_points([
            0x000000, 0x646464, 0xc8c8c8,
        ]);

        draw_pxs_image_region(
            &mut surface,
            &GuiRect::new(0.0, 0.0, 1.0, 1.0),
            &image,
            &SourceRect::new(0, 0, 1, 1),
            0,
            1.0,
            Some(&gamma),
        );

        assert_eq!(
            surface.get_pixel(0, 0),
            Some(Color::new(125, 150, 175, 255))
        );
    }

    #[test]
    fn tutorial_seven_acid_rain_pxs_uses_its_green_gamma_ramp() {
        // Tutorial07 Script.c:12 and AcidRain.c4m:3: the opaque old-style
        // PXS fragment (200,250,200) is sampled by the scenario's green-heavy
        // ramp before it replaces the framebuffer pixel.
        let mut graphics = GraphicsSystem::new(
            4,
            4,
            4,
            "Tutorial 07 Acid Rain",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let background = Color::opaque(7, 11, 13);
        graphics.surface_mut().fill(background);
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "acidrain".to_string(),
            MaterialRenderInfo::new(
                [200, 250, 200, 200, 250, 200, 200, 250, 200],
                [0; 6],
                None,
                0,
                25,
            ),
        )])));
        let gamma = lc_graphics::GammaRamp::from_control_points([
            0x000000, 0x648064, 0xc8ffc8,
        ]);

        graphics.draw_pxs(
            &[pxs_particle("acidrain", [2 << 16, 2 << 16, 0, 0], 0)],
            1.0,
            Some(&gamma),
        );

        assert_eq!(
            graphics.surface().get_pixel(2, 2),
            Some(Color::opaque(157, 250, 157))
        );
        assert_eq!(graphics.surface().get_pixel(1, 2), Some(background));
    }

    #[test]
    fn real_tutorial_seven_apply_gamma_now_replaces_reused_menu_gamma() {
        // Tutorial07 Initialize installs this ramp before the first game
        // render (Tutorial07.c4s/Script.c:12; C4Game.cpp:490), and its shipped
        // AcidRain material supplies opaque old-style PXS colour 200,250,200
        // (AcidRain.c4m:3). C4PXS::Draw emits that fragment through the active
        // shader gamma textures (C4PXS.cpp:242-277; StdGL.cpp:1082-1087).
        let tutorial = crate::test_support::repo_root().join("content/Tutorial.c4f/Tutorial07.c4s");
        let script = std::fs::read_to_string(tutorial.join("Script.c"))
            .expect("read shipped Tutorial07 Script.c");
        let gamma_values = script
            .lines()
            .find(|line| line.contains("SetGamma("))
            .expect("shipped Tutorial07 sets gamma")
            .split(|character: char| !character.is_ascii_digit())
            .filter(|value| !value.is_empty())
            .map(|value| value.parse::<u32>().expect("Tutorial07 gamma channel"))
            .collect::<Vec<_>>();
        assert_eq!(gamma_values.len(), 9);
        let rgb = |offset: usize| {
            (gamma_values[offset] << 16)
                | (gamma_values[offset + 1] << 8)
                | gamma_values[offset + 2]
        };

        let material = std::fs::read_to_string(tutorial.join("Material.c4g/AcidRain.c4m"))
            .expect("read shipped Tutorial07 AcidRain material");
        let material_color = material
            .lines()
            .find_map(|line| line.strip_prefix("Color="))
            .expect("shipped AcidRain material color")
            .split(',')
            .map(|value| value.parse::<u8>().expect("AcidRain color channel"))
            .collect::<Vec<_>>()
            .try_into()
            .expect("AcidRain has three RGB triplets");

        let mut snapshot = make_snapshot();
        snapshot
            .environment
            .gamma
            .set_ramp(0, [rgb(0), rgb(3), rgb(6)]);
        snapshot
            .particles
            .push(pxs_particle("acidrain", [100 << 16, 60 << 16, 0, 0], 0));
        let mut graphics = GraphicsSystem::new(
            120,
            100,
            100,
            "Tutorial 07 Acid Rain",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "acidrain".to_string(),
            MaterialRenderInfo::new(material_color, [0; 6], None, 0, 25),
        )])));

        let menu_snapshot = make_snapshot();
        graphics.render_frame(
            &menu_snapshot,
            &[ViewportInput::from_focus(&menu_snapshot.objects[0])],
        );
        graphics.apply_gamma_now(&snapshot.environment.gamma);
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::from_focus(&snapshot.objects[0])],
        );
        let (x, y) = graphics
            .world_to_screen(0, Vector2::new(100, 60))
            .expect("acid-rain point is in the Tutorial07 viewport");

        assert_eq!(
            graphics
                .surface()
                .get_pixel(x.round() as u32, y.round() as u32),
            Some(Color::opaque(157, 250, 157)),
        );
    }

    #[test]
    fn real_tutorial_seven_acid_rain_matches_cpp_animated_pxs_sequence() {
        // Tutorial07 fixes rain at 77 and wind at 50 and selects AcidRain
        // (Scenario.txt:70-75). FXP1's Process action calls Precipitation
        // every two frames and that callback inserts three PXS at strength 77
        // (Precipitation.c4d/ActMap.txt:1-8; Script.c:5-17). ExecObjects runs
        // before PXS.Execute, so each new triplet moves on its creation frame
        // (C4Game.cpp:808-835; C4PXS.cpp:218-239). AcidRain has no PXSGfx:
        // C++ therefore draws a gamma-shaded, half-open velocity line from
        // x-xdir/y-ydir to x/y (C4PXS.cpp:242-277; StdGL.cpp:893-933,
        // 1082-1087). Pin ten actual frames, not merely spawn counts.
        let mut engine = load_repository_tutorial(7);
        let material_group = Group::open(
            crate::test_support::repo_root()
                .join("content/Tutorial.c4f/Tutorial07.c4s/Material.c4g"),
        )
        .expect("Tutorial07 Material.c4g opens");
        let materials = MaterialLibrary::from_group(&material_group)
            .expect("Tutorial07 materials load");
        let acid = materials.get("AcidRain").expect("AcidRain material");
        let color: [u8; 9] = acid
            .int_list("Color")
            .expect("AcidRain color")
            .into_iter()
            .map(|value| value as u8)
            .collect::<Vec<_>>()
            .try_into()
            .expect("AcidRain has three RGB triplets");
        assert!(acid.value("PXSGfx").is_none());
        assert_eq!(color, [200, 250, 200, 200, 250, 200, 200, 250, 200]);
        assert_eq!(acid.int("Density"), Some(25));
        let material = MaterialRenderInfo::new(
            color,
            [0; 6],
            acid.value("TextureOverlay").map(ToOwned::to_owned),
            acid.int("OverlayType").unwrap_or(0),
            acid.int("Density").unwrap_or(0),
        );

        // The burned TRB1/TRB2 trees include TREE's Construction callback but
        // have no Initialize action. C++ SetAction therefore returns false and
        // skips TREE's SetDir(Random(2)), which is observable in this synced
        // precipitation ledger (Tree.c4d/Script.c:20-30;
        // C4Object.cpp:4218-4234).
        let expected_frames = [
            (0, 0xdbdc_9dc5, 0, None),
            (3, 0x0bad_19e8, 21, Some((62, 1, 217, 7))),
            (3, 0x6c8a_4955, 24, Some((63, 8, 217, 15))),
            (6, 0x31b1_5c65, 42, Some((63, 1, 218, 22))),
            (6, 0xddd7_6c00, 45, Some((64, 8, 219, 29))),
            (9, 0xaecd_41a0, 63, Some((65, 1, 220, 36))),
            (9, 0x378c_d1f5, 68, Some((66, 8, 221, 43))),
            (12, 0xdeb1_9f59, 84, Some((67, 1, 222, 50))),
            (12, 0x5669_9b94, 87, Some((68, 8, 224, 57))),
            (15, 0x5549_b99d, 106, Some((69, 1, 225, 64))),
        ];
        let expected_first_particle = [
            [4_102_875, 552_373, 39_643, 486_837],
            [4_151_777, 1_033_657, 48_902, 481_284],
            [4_208_380, 1_504_401, 56_603, 470_744],
            [4_270_101, 1_973_587, 61_721, 469_186],
            [4_334_985, 2_436_830, 64_884, 463_243],
            [4_400_655, 2_899_856, 65_670, 463_026],
            [4_471_515, 3_362_274, 70_860, 462_418],
            [4_549_622, 3_824_705, 78_107, 462_431],
            [4_631_974, 4_279_441, 82_352, 454_736],
        ];

        for (frame_index, &(expected_count, checksum, changed_count, expected_bounds)) in
            expected_frames.iter().enumerate()
        {
            let snapshot = engine.tick().expect("Tutorial07 weather frame");
            let pxs = snapshot
                .particles
                .iter()
                .filter(|particle| particle.definition_id == "material/pxs/acidrain")
                .collect::<Vec<_>>();
            assert_eq!(pxs.len(), expected_count, "frame {} PXS cadence", snapshot.frame);
            assert_eq!(
                pxs.iter().map(|particle| particle.pxs_slot).collect::<Vec<_>>(),
                (0..expected_count as u32).map(Some).collect::<Vec<_>>(),
                "frame {} preserves C4PXS slot order",
                snapshot.frame,
            );
            if let Some(first) = pxs.first() {
                assert_eq!(
                    first.pxs_fixed,
                    Some(expected_first_particle[frame_index - 1]),
                    "frame {} first AcidRain PXS trajectory",
                    snapshot.frame,
                );
            }
            let mut graphics = GraphicsSystem::new(
                1024,
                256,
                256,
                "Tutorial 07 Acid Rain",
                test_font(),
                empty_sprites(),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.surface_mut().fill(Color::opaque(7, 11, 13));
            graphics.set_material_render_info(Arc::new(HashMap::from([(
                "acidrain".to_string(),
                material.clone(),
            )])));
            let gamma_points = snapshot.environment.gamma.combined_control_points();
            assert_eq!(gamma_points, [0x000000, 0x648064, 0xc8ffc8]);
            let gamma = lc_graphics::GammaRamp::from_control_points(gamma_points);
            graphics.draw_pxs(
                &snapshot.particles,
                GraphicsSystem::lighting_factor(snapshot.environment.settings.time_of_day),
                Some(&gamma),
            );
            let background = Color::opaque(7, 11, 13);
            let changed = (0..graphics.surface().height())
                .flat_map(|y| (0..graphics.surface().width()).map(move |x| (x, y)))
                .filter(|&(x, y)| graphics.surface().get_pixel(x, y) != Some(background))
                .collect::<Vec<_>>();
            let bounds = changed.iter().fold(None, |bounds, &(x, y)| {
                Some(bounds.map_or(
                    (x, y, x, y),
                    |(x0, y0, x1, y1): (u32, u32, u32, u32)| {
                        (x0.min(x), y0.min(y), x1.max(x), y1.max(y))
                    },
                ))
            });
            assert_eq!(
                graphics.surface().snapshot().checksum(),
                checksum,
                "frame {} rendered PXS streaks",
                snapshot.frame,
            );
            assert_eq!(
                (changed.len(), bounds),
                (changed_count, expected_bounds),
                "frame {} rendered PXS coverage",
                snapshot.frame,
            );
        }
    }

    #[test]
    fn real_tutorial_seven_acid_landscape_matches_cpp_material_color() {
        // The shipped slot is Acid-Smooth, but C4TexMapEntry changes every
        // liquid <mat>-Smooth primary pattern to Liquid. Acid's own Liquid
        // overlay is then sampled at zoom two (C4Texture.cpp:68-99;
        // C4Material.cpp:349-377), after which Tutorial07's three-channel
        // gamma ramp is applied by the landscape shader (StdGL.cpp:1130-1148).
        // The optional liquid-animation tint is off by C++ default
        // (C4Config.cpp:451).
        let mut engine = load_repository_tutorial(7);
        let snapshot = engine.tick().expect("Tutorial07 first frame");
        let grid = snapshot
            .landscape
            .as_ref()
            .and_then(Landscape::pixel_grid)
            .expect("Tutorial07 pixel landscape");

        let local_material_group = Group::open(
            crate::test_support::repo_root()
                .join("content/Tutorial.c4f/Tutorial07.c4s/Material.c4g"),
        )
        .expect("Tutorial07 Material.c4g opens");
        let texmap_source = local_material_group
            .read_file("Texmap.txt")
            .expect("Tutorial07 Texmap.txt reads");
        let texmap = lc_resources::texmap::TextureMap::parse(&String::from_utf8_lossy(
            &texmap_source,
        ));
        let acid_slot = texmap.entry(22).expect("Tutorial07 Acid texmap slot");
        assert_eq!(
            (acid_slot.material.as_str(), acid_slot.texture.as_str()),
            ("Acid", "Smooth"),
        );
        assert_eq!(grid.material_names()[22].as_deref(), Some("Acid"));
        assert_eq!(grid.texture_names()[22].as_deref(), Some("Liquid"));

        // C4Landscape::ApplyLighting shades material edges from Placement
        // (C4Landscape.cpp:2534-2588). This real 16x16 interior plus its
        // complete x/y comparison neighbourhood is Acid (Placement=10), so
        // C++ applies neither edge lightening nor darkening here.
        let acid_at = |x: i32, y: i32| {
            grid.byte_at(x, y)
                .and_then(|byte| grid.material_names().get((byte & 0x7f) as usize))
                .and_then(|name| name.as_deref())
                .is_some_and(|name| name.eq_ignore_ascii_case("Acid"))
        };
        assert!((0..16).all(|dy| {
            (0..16).all(|dx| {
                let x = 196 + dx;
                let y = 349 + dy;
                acid_at(x - 1, y)
                    && acid_at(x + 1, y)
                    && (-9..=8).all(|offset| acid_at(x, y + offset))
            })
        }));

        let global_material_group = Group::open(
            crate::test_support::repo_root().join("content/Material.c4g"),
        )
        .expect("installed Material.c4g opens");
        let global_materials = MaterialLibrary::from_group(&global_material_group)
            .expect("installed materials load");
        let acid = global_materials.get("Acid").expect("Acid material");
        let mut color = [0u8; 9];
        for (target, source) in color
            .iter_mut()
            .zip(acid.int_list("Color").expect("Acid Color"))
        {
            *target = source as u8;
        }
        assert_eq!(color, [0, 190, 0, 0, 200, 0, 0, 210, 0]);
        assert_eq!(acid.value("TextureOverlay"), Some("Liquid"));
        assert_eq!(
            (acid.int("Density"), acid.int("Placement")),
            (Some(25), Some(10)),
        );
        let resource = lc_resources::graphics::GraphicsResource::from_group(global_material_group)
            .expect("installed material graphics index");
        let liquid = resource.load_image("LIQUID.png").expect("Liquid texture");
        assert_eq!((liquid.width(), liquid.height()), (128, 128));
        let liquid = ImageData::new(liquid.width(), liquid.height(), liquid.pixels().to_vec());
        let material = MaterialRenderInfo::new(
            color,
            [0; 6],
            acid.value("TextureOverlay").map(ToOwned::to_owned),
            acid.int("OverlayType").unwrap_or(0),
            acid.int("Density").unwrap_or(0),
        );
        let mut graphics = GraphicsSystem::new(
            16,
            16,
            16,
            "Tutorial 07 Acid",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.viewport_x = 196.0;
        graphics.viewport_y = 349.0;
        graphics.surface_mut().fill(Color::opaque(7, 11, 13));
        graphics.set_material_textures(Arc::new(HashMap::from([(
            "liquid".to_string(),
            liquid,
        )])));
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "acid".to_string(),
            material,
        )])));
        let gamma_points = snapshot.environment.gamma.combined_control_points();
        assert_eq!(gamma_points, [0x000000, 0x648064, 0xc8ffc8]);
        assert_eq!(snapshot.environment.settings.time_of_day, 0);
        assert_eq!(GraphicsSystem::lighting_factor(0), 1.0);
        let gamma = lc_graphics::GammaRamp::from_control_points(gamma_points);
        assert!(graphics.draw_ground_textured(snapshot.landscape.as_ref(), Some(&gamma)));

        assert_eq!(
            [(0, 0), (4, 0), (15, 0), (0, 8), (8, 8), (15, 15)]
                .map(|(x, y)| graphics.surface().get_pixel(x, y)),
            [
                Some(Color::opaque(1, 192, 1)),
                Some(Color::opaque(1, 188, 1)),
                Some(Color::opaque(1, 190, 1)),
                Some(Color::opaque(1, 190, 1)),
                Some(Color::opaque(1, 192, 1)),
                Some(Color::opaque(1, 182, 1)),
            ],
        );
        assert_eq!(graphics.surface().snapshot().checksum(), 0x03df_cb2d);
    }

    #[test]
    fn viewport_point_at_maps_screen_to_world() {
        let snapshot = make_snapshot();
        let focus = &snapshot.objects[0];
        let mut graphics = GraphicsSystem::new(
            320,
            180,
            150,
            "Viewport Test",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (screen_x, screen_y) = graphics
            .world_to_screen(focus.owner, focus.position)
            .expect("screen coordinates available");
        let pointer = graphics
            .viewport_point_at(GuiPoint::new(screen_x, screen_y))
            .expect("viewport pointer available");
        assert_eq!(pointer.owner, focus.owner);
        assert!(
            (pointer.world.x - focus.position.x as f32).abs() < 0.5,
            "expected world x close to focus, got {}",
            pointer.world.x
        );
        assert!(
            (pointer.world.y - focus.position.y as f32).abs() < 0.5,
            "expected world y close to focus, got {}",
            pointer.world.y
        );
    }

    #[test]
    fn crew_at_point_returns_local_crew() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].owner = 1;
        let focus = &snapshot.objects[0];

        let mut graphics = GraphicsSystem::new(
            320,
            180,
            150,
            "Crew Pick",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (screen_x, screen_y) = graphics
            .world_to_screen(1, focus.position)
            .expect("screen coordinates available");
        let point = GuiPoint::new(screen_x, screen_y);

        let picked = graphics.crew_at_point(&snapshot, 1, point);
        assert_eq!(picked, Some(focus.id));
        assert_eq!(
            graphics.crew_at_point(&snapshot, 2, point),
            None,
            "other owners should not pick crew"
        );
    }

    #[test]
    fn object_at_point_uses_cpp_front_to_back_order() {
        // C4Game::FindVisObject walks Objects.First -> Next, the reverse of
        // C4ObjectList::Draw's Last -> Prev order. Ownership does not filter
        // context targets; MouseIgnore and contained objects do.
        let mut snapshot = make_snapshot();
        snapshot.objects[0].owner = 1;
        snapshot.objects[0].ocf = 1;
        let back_id = snapshot.objects[0].id;
        let mut front = snapshot.objects[0].clone();
        front.id = ObjectId::new(2);
        front.owner = 2;
        let front_id = front.id;
        snapshot.objects.push(front);
        snapshot.render_order = vec![back_id, front_id];

        let focus = snapshot.objects[0].clone();
        let mut graphics = GraphicsSystem::new(
            320,
            180,
            150,
            "Object Pick",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.render_frame(&snapshot, &[ViewportInput::from_focus(&focus)]);
        let (screen_x, screen_y) = graphics
            .world_to_screen(1, focus.position)
            .expect("screen coordinates available");
        let point = GuiPoint::new(screen_x, screen_y);

        assert_eq!(graphics.object_at_point(&snapshot, 1, point), Some(front_id));

        snapshot.objects[1].visibility = lc_engine::VIS_NONE;
        assert_eq!(
            graphics.object_at_point(&snapshot, 1, point),
            Some(back_id),
            "FindVisObject must skip a VIS_None front object"
        );
        snapshot.objects[1].visibility = lc_engine::VIS_ALL;

        snapshot.objects[0].ocf = lc_engine::ocf::CONTAINER;
        assert_eq!(
            graphics.object_at_point_with_ocf(
                &snapshot,
                1,
                point,
                lc_engine::ocf::CONTAINER,
            ),
            Some(back_id),
            "an OCF-filtered search skips a nonmatching front object"
        );
        snapshot.objects[0].ocf = 1;

        snapshot.objects[1].alive = false;
        assert_eq!(
            graphics.object_at_point(&snapshot, 1, point),
            Some(front_id),
            "structures and items are context targets despite Alive=false"
        );

        snapshot.objects[1].category |= CATEGORY_MOUSE_IGNORE_FLAG;
        assert_eq!(graphics.object_at_point(&snapshot, 1, point), Some(back_id));

        snapshot.objects[0].container = Some(front_id);
        assert_eq!(graphics.object_at_point(&snapshot, 1, point), None);

        snapshot.objects[0].container = None;
        snapshot.objects[1].category &= !CATEGORY_MOUSE_IGNORE_FLAG;
        snapshot.players = vec![PlayerState {
            id: 1,
            cursor: None,
            ..PlayerState::default()
        }];
        assert_eq!(
            graphics.object_at_point(&snapshot, 1, point),
            None,
            "a valid player without a cursor must fall through to select-next"
        );
    }

    #[test]
    fn object_visibility_matches_cpp_masks_layers_and_local_bits() {
        let mut snapshot = make_snapshot();
        let object = &mut snapshot.objects[0];
        object.owner = 1;
        snapshot.players = vec![
            PlayerState {
                id: 1,
                ..PlayerState::default()
            },
            PlayerState {
                id: 2,
                hostility: vec![1],
                ..PlayerState::default()
            },
            PlayerState {
                id: 3,
                ..PlayerState::default()
            },
        ];

        snapshot.objects[0].visibility = VIS_OWNER;
        assert!(GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            1,
            false,
        ));
        assert!(!GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            2,
            false,
        ));

        snapshot.objects[0].visibility = VIS_ALLIES | VIS_ENEMIES;
        assert!(GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            2,
            false,
        ));
        assert!(GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            3,
            false,
        ));
        assert!(!GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            1,
            false,
        ));

        snapshot.objects[0].visibility = VIS_LOCAL;
        snapshot.objects[0]
            .local_vars
            .insert(
                "__local_0".into(),
                serde_json::from_value(serde_json::json!({"Int": 1 << 3}))
                    .expect("numbered Local value"),
            );
        assert!(GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            3,
            false,
        ));
        assert!(!GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            2,
            false,
        ));

        snapshot.objects[0].visibility = VIS_GOD;
        assert!(GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            OWNER_NONE,
            false,
        ));
        snapshot.objects[0].visibility = VIS_OVERLAY_ONLY;
        assert!(!GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            1,
            false,
        ));
        assert!(GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            1,
            true,
        ));

        let mut layer = snapshot.objects[0].clone();
        layer.id = ObjectId::new(2);
        layer.layer = None;
        layer.visibility = VIS_OWNER | VIS_LAYER_TOGGLE;
        snapshot.objects[0].visibility = 0;
        snapshot.objects[0].layer = Some(layer.id);
        snapshot.objects.push(layer);
        assert!(!GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            1,
            false,
        ));
        assert!(GraphicsSystem::object_is_visible(
            &snapshot.objects,
            &snapshot.players,
            &snapshot.objects[0],
            3,
            false,
        ));
    }

    #[test]
    fn graphics_system_draws_ground() {
        let snapshot = make_snapshot();
        let focus = &snapshot.objects[0];
        let mut graphics = GraphicsSystem::new(
            320,
            180,
            150,
            "Test Scenario",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.set_world_width(256);

        let viewports = vec![ViewportInput::from_focus(focus)];
        let atlas = graphics.render_frame(&snapshot, &viewports);
        assert!(!atlas.is_empty());

        let ground = graphics.surface().get_pixel(0, 179).unwrap();
        assert_ne!(ground, Color::opaque(8, 12, 24));
    }

    fn pxs_particle(
        material: &str,
        fixed: [i32; 4],
        slot: u32,
    ) -> lc_engine::ParticleSnapshot {
        lc_engine::ParticleSnapshot {
            definition_id: format!("material/pxs/{material}"),
            position: FloatVector2::new(
                fixed[0] as f32 / 65_536.0,
                fixed[1] as f32 / 65_536.0,
            ),
            velocity: FloatVector2::new(
                fixed[2] as f32 / 65_536.0,
                fixed[3] as f32 / 65_536.0,
            ),
            life: 0,
            parameter_a: 0.0,
            parameter_b: 0,
            layer: lc_engine::ParticleLayer::Global,
            pxs_fixed: Some(fixed),
            pxs_slot: Some(slot),
        }
    }

    #[test]
    fn old_style_pxs_draws_cpp_velocity_line_with_alpha() {
        // C4PXSSystem::Draw uses the material palette color and turns moving
        // pixels into x-xdir/y-ydir velocity lines. Its Clonk transparency is
        // max(alpha, 195-(195-alpha)/fixtoi(|xdir|+|ydir|))
        // (C4PXS.cpp:242-275).
        let mut graphics = GraphicsSystem::new(
            12,
            12,
            12,
            "PXS",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "rain".to_string(),
            MaterialRenderInfo::new([200, 100, 50, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 25),
        )])));
        let particle = pxs_particle("rain", [8 << 16, 8 << 16, 2 << 16, 0], 3);

        graphics.draw_pxs(std::slice::from_ref(&particle), 1.0, None);

        assert_eq!(
            graphics.surface().get_pixel(6, 8),
            Some(Color::opaque(123, 61, 30)),
            "two-pixel velocity has C++ transparency 98 (opacity 157)"
        );
        assert_eq!(graphics.surface().get_pixel(7, 8), Some(Color::opaque(123, 61, 30)));
        assert_eq!(
            graphics.surface().get_pixel(8, 8),
            Some(Color::opaque(0, 0, 0)),
            "GL_LINES applies the diamond-exit rule and omits the final endpoint",
        );
    }

    #[test]
    fn offscreen_pxs_endpoint_culls_crossing_velocity_line() {
        // The enlarged VisibleRect checks fixtoi(x,y) before drawing the
        // x-xdir velocity line (C4PXS.cpp:245-275). This endpoint is far
        // outside that rect even though its 100px line crosses the surface.
        let mut graphics = GraphicsSystem::new(
            12,
            12,
            12,
            "PXS",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(1, 2, 3));
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "rain".to_string(),
            MaterialRenderInfo::new([255, 255, 255, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 25),
        )])));
        let particle = pxs_particle("rain", [100 << 16, 6 << 16, 100 << 16, 0], 3);

        graphics.draw_pxs(std::slice::from_ref(&particle), 1.0, None);

        assert!(graphics
            .surface()
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel == [1, 2, 3, 255]));
    }

    #[test]
    fn graphical_pxs_uses_saved_slot_phase_and_falls_back_without_texture() {
        // The graphical pass derives phase and z from cnt2, the slot WITHIN
        // the 500-entry chunk, then applies PXSGfxRt offsets and size
        // (C4PXS.cpp:280-307). A missing PXSGfx texture stays in the first,
        // old-style pass (C4Material.cpp:382-385; C4PXS.cpp:257-260).
        let mut graphics = GraphicsSystem::new(
            16,
            16,
            16,
            "PXS",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        let mut snow_pixels = vec![0; 12 * 6 * 4];
        for y in 0..6usize {
            for x in 0..12usize {
                let index = (y * 12 + x) * 4;
                if x < 6 {
                    snow_pixels[index + 2] = 255;
                } else {
                    snow_pixels[index] = 255;
                }
                snow_pixels[index + 3] = 128;
            }
        }
        graphics.set_material_textures(Arc::new(HashMap::from([(
            "snow".to_string(),
            ImageData::new(12, 6, snow_pixels),
        )])));
        graphics.set_material_render_info(Arc::new(HashMap::from([
            (
                "snow".to_string(),
                MaterialRenderInfo::new([0, 255, 0, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 25)
                    .with_pxs_graphics(Some("Snow".to_string()), [0, 0, 6, 6, 6, 0], 1),
            ),
            (
                "ash".to_string(),
                MaterialRenderInfo::new([90, 80, 70, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 25)
                    .with_pxs_graphics(Some("Missing".to_string()), [0, 0, 2, 2, 0, 0], 3),
            ),
        ])));
        let graphical = pxs_particle("snow", [4 << 16, 4 << 16, 0, 0], 507);
        let fallback = pxs_particle("ash", [10 << 16, 10 << 16, 0, 0], 1);

        graphics.draw_pxs(&[graphical, fallback], 1.0, None);

        // 507 % 500 = 7: phase x=1; z=1; tx=6 shifts x by one. Texture
        // transparency 127 plus modulation 16 gives 143, i.e. source opacity
        // 112 over black (PerformBlt alpha addition).
        assert_eq!(graphics.surface().get_pixel(5, 4), Some(Color::opaque(112, 0, 0)));
        assert_eq!(graphics.surface().get_pixel(10, 10), Some(Color::opaque(90, 80, 70)));
    }

    #[test]
    fn graphical_pxs_uses_gl_linear_filtering_across_its_source_facet() {
        // C4Facet::DrawX supplies a 2x4 source facet and a non-exact 4x4
        // target, which enables GL_LINEAR (C4Facet.cpp:296-303;
        // StdDDraw2.cpp:663-669; StdGL.cpp:527-531). Internal facet edges are
        // not sampler boundaries, so the first/last columns blend adjacent
        // sheet texels too.
        let columns = [
            Color::opaque(255, 0, 0),
            Color::opaque(0, 0, 0),
            Color::opaque(255, 255, 255),
            Color::opaque(0, 0, 255),
        ];
        let pixels = (0..4)
            .flat_map(|_| columns)
            .flat_map(|color| [color.r, color.g, color.b, color.a])
            .collect();
        let image = ImageData::new(4, 4, pixels);
        let mut surface = Surface::new(4, 4, PixelFormat::Rgba8888);
        surface.fill(Color::opaque(0, 0, 0));

        draw_pxs_image_region(
            &mut surface,
            &GuiRect::new(0.0, 0.0, 4.0, 4.0),
            &image,
            &SourceRect::new(1, 0, 2, 4),
            0,
            1.0,
            None,
        );

        assert_eq!(surface.get_pixel(0, 1), Some(Color::opaque(64, 0, 0)));
        assert_eq!(surface.get_pixel(1, 1), Some(Color::opaque(64, 64, 64)));
        assert_eq!(surface.get_pixel(2, 1), Some(Color::opaque(191, 191, 191)));
        assert_eq!(surface.get_pixel(3, 1), Some(Color::opaque(191, 191, 255)));
    }

    #[test]
    fn scalar_precipitation_without_real_pxs_does_not_paint_the_viewport() {
        // C4Viewport has no synthetic precipitation pass: weather launches
        // the FXP1 precipitation object, whose callback inserts real material
        // into the simulation (C4Viewport.cpp:1056-1078; C4Weather.cpp:48-58,
        // 205-214). A scalar alone must not alter otherwise identical pixels.
        let render = |snapshot: &SimulationSnapshot| {
            let mut graphics = GraphicsSystem::new(
                80,
                60,
                60,
                "Weather",
                test_font(),
                empty_sprites(),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.render_frame(snapshot, &[ViewportInput::from_focus(&snapshot.objects[0])]);
            graphics.surface().pixels().to_vec()
        };
        let dry = make_snapshot();
        let mut scalar_only = dry.clone();
        scalar_only.environment.precipitation = 80;
        scalar_only.environment.settings = scalar_only
            .environment
            .settings
            .with_precipitation(80)
            .with_precipitation_strength(80);

        assert_eq!(render(&scalar_only), render(&dry));
    }

    #[test]
    fn weather_lightning_event_does_not_synthesize_an_early_flash() {
        // C4Weather::LaunchLightning only creates FXL1 and calls Activate
        // (C4Weather.cpp:158-168). Activate accumulates enlightenment, but
        // SetGamma is deferred until FXL1's Advance callback executes on the
        // next object phase (Effects/Lightning/Script.c:16-31, 72-92), because
        // Weather.Execute runs after ExecObjects (C4Game.cpp:811-835). The
        // launch-frame presentation therefore must not add a separate flash.
        let render = |snapshot: &SimulationSnapshot| {
            let mut graphics = GraphicsSystem::new(
                80,
                60,
                60,
                "Weather lightning",
                test_font(),
                empty_sprites(),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.set_sky(Some(SkyRenderState::new(
                SkySettings {
                    fade_top: RgbColor::new(24, 48, 96),
                    fade_bottom: RgbColor::new(96, 128, 192),
                    ..Default::default()
                },
                None,
            )));
            graphics.render_frame(snapshot, &[ViewportInput::from_focus(&snapshot.objects[0])]);
            graphics.surface().pixels().to_vec()
        };
        let clear = make_snapshot();
        let mut launched = clear.clone();
        launched
            .weather_events
            .push(WeatherEvent::Lightning { position: 40 });

        assert_eq!(render(&launched), render(&clear));
    }

    #[test]
    fn render_frame_places_pxs_between_landscape_and_objects() {
        // C4Viewport::Draw orders Landscape -> PXS -> Objects
        // (C4Viewport.cpp:1056-1073). The red 1x1 object must therefore cover
        // the blue old-style PXS at the same world position.
        let mut snapshot = make_snapshot();
        snapshot.particles.push(pxs_particle(
            "rain",
            [100 << 16, 100 << 16, 0, 0],
            0,
        ));
        let sprites = solid_sprite(
            "TestObject",
            1,
            1,
            Color::opaque(240, 0, 0),
            Some(DefinitionRect::new(0, 0, 1, 1)),
            false,
        );
        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "PXS order",
            test_font(),
            Arc::clone(&sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "rain".to_string(),
            MaterialRenderInfo::new([0, 0, 240, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 25),
        )])));
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::from_focus(&snapshot.objects[0])],
        );
        let (screen_x, screen_y) = graphics
            .world_to_screen(0, snapshot.objects[0].position)
            .expect("active viewport");

        assert_eq!(
            graphics
                .surface()
                .get_pixel(screen_x.round() as u32, screen_y.round() as u32),
            Some(standard_gamma_color(Color::opaque(240, 0, 0))),
        );

        let mut background_snapshot = snapshot.clone();
        background_snapshot.objects[0].category |= CATEGORY_BACKGROUND_FLAG;
        let mut background_graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "PXS background order",
            test_font(),
            sprites,
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        background_graphics.set_material_render_info(Arc::new(HashMap::from([(
            "rain".to_string(),
            MaterialRenderInfo::new([0, 0, 240, 0, 0, 0, 0, 0, 0], [0; 6], None, 0, 25),
        )])));
        background_graphics.render_frame(
            &background_snapshot,
            &[ViewportInput::from_focus(&background_snapshot.objects[0])],
        );
        let (screen_x, screen_y) = background_graphics
            .world_to_screen(0, background_snapshot.objects[0].position)
            .expect("active viewport");
        let lighting = GraphicsSystem::lighting_factor(
            background_snapshot.environment.settings.time_of_day,
        );

        assert_eq!(
            background_graphics
                .surface()
                .get_pixel(screen_x.round() as u32, screen_y.round() as u32),
            Some(standard_gamma_color(
                Color::opaque(0, 0, 240).modulate(lighting),
            )),
            "PXS must cover C4D_Background objects drawn before landscape",
        );
    }

    #[test]
    fn foreground_parallax_split_straddles_cursor_marks_like_cpp() {
        // ForeObjects.DrawIfCategory(... C4D_Parallax, true) draws the
        // non-parallax foreground before Game.DrawCursors; the false pass
        // draws parallax/custom-GUI objects afterwards
        // (C4Viewport.cpp:1080-1103; C4ObjectList.cpp:400-409).
        let render = |category: i32| {
            let mut snapshot = make_snapshot();
            snapshot.objects[0].position = Vector2::new(40, 40);
            snapshot.objects[0].owner = 1;
            snapshot.objects[0].category = category;
            snapshot.landscape = Some(Landscape::flat(128, 80));
            snapshot.players.push(PlayerState {
                id: 1,
                cursor: Some(snapshot.objects[0].id),
                control: lc_engine::PlayerControlState {
                    select_flash: 30,
                    ..Default::default()
                },
                ..PlayerState::default()
            });
            let sprites = solid_sprite(
                "TestObject",
                12,
                12,
                Color::opaque(220, 0, 0),
                Some(DefinitionRect::new(-6, -6, 12, 12)),
                false,
            );
            let hud = HudGraphics {
                select_mark: Some(ImageData::new(
                    20,
                    5,
                    (0..100).flat_map(|_| [0, 220, 0, 255]).collect(),
                )),
                ..Default::default()
            };
            let mut graphics = GraphicsSystem::new(
                80,
                60,
                60,
                "Foreground order",
                test_font(),
                sprites,
                empty_cursor_atlas(),
                Arc::new(hud),
            );
            graphics.render_frame(
                &snapshot,
                &[ViewportInput::from_focus(&snapshot.objects[0])],
            );
            let (viewport_x, viewport_y) = graphics.viewport();
            let x = snapshot.objects[0].position.x - viewport_x - 6;
            let y = snapshot.objects[0].position.y - viewport_y - 6;
            graphics.surface().get_pixel(x as u32, y as u32)
        };

        assert_eq!(
            render(lc_engine::DEFAULT_CATEGORY | CATEGORY_FOREGROUND_FLAG),
            Some(standard_gamma_color(Color::opaque(0, 220, 0))),
            "cursor mark covers ordinary foreground",
        );
        assert_eq!(
            render(
                lc_engine::DEFAULT_CATEGORY
                    | CATEGORY_FOREGROUND_FLAG
                    | CATEGORY_PARALLAX_FLAG,
            ),
            Some(standard_gamma_color(Color::opaque(220, 0, 0))),
            "custom-GUI/parallax foreground covers cursor mark",
        );
    }

    #[test]
    fn overlay_state_feeds_the_hud_render() {
        let snapshot = make_snapshot();
        let focus = &snapshot.objects[0];
        let mut graphics = GraphicsSystem::new(
            320,
            180,
            150,
            "Test Scenario",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.update_overlay(&GraphicsOverlay {
            frame_text: "FRAME",
            status_text: "STATUS",
            debug_hud: false,
            players: Vec::new(),
            game_time_seconds: 61,
            message_board_line: Some("Player join: Test".to_string()),
            show_commands: true,
            show_command_keys: true,
        });
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);
        // Rendering with overlay state must not panic; without an
        // UpperBoard texture the chrome stays off (C4UpperBoard::Init,
        // src/C4UpperBoard.cpp:114-118) and the viewport spans the surface.
    }

    #[test]
    fn graphics_system_draws_player_control_hints_from_overlay() {
        // DrawOverlay reaches DrawPlayerInfo -> DrawPlayerControls after the
        // world pass (src/C4Viewport.cpp:835-848,1324-1327), so the selected
        // Control.png key cap must overwrite the viewport pixel.
        let snapshot = make_snapshot();
        let focus = &snapshot.objects[0];
        let width = 320u32;
        let height = 164u32;
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        for y in 100..164 {
            for x in 0..64 {
                let index = ((y * width + x) * 4) as usize;
                pixels[index..index + 4].copy_from_slice(&[10, 10, 200, 255]);
            }
        }
        let hud_graphics = Arc::new(HudGraphics {
            control: Some(ImageData::new(width, height, pixels)),
            ..HudGraphics::default()
        });
        let mut graphics = GraphicsSystem::new(
            320,
            180,
            150,
            "Control Hint",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            hud_graphics,
        );
        graphics.update_overlay(&GraphicsOverlay {
            frame_text: "",
            status_text: "",
            debug_hud: false,
            players: vec![PlayerOverlay {
                owner: 0,
                name: "Player".to_string(),
                wealth: 0,
                score: 0,
                cursor: None,
                eliminated: false,
                owner_color: Color::opaque(0, 100, 200),
                select_count: 0,
                show_startup: false,
                show_control: 1,
                show_control_position: 0,
                last_com: 5,
                control_key_labels: Vec::new(),
                crew: Vec::new(),
                commands: Vec::new(),
            }],
            game_time_seconds: 0,
            message_board_line: None,
            show_commands: true,
            show_command_keys: true,
        });

        graphics.render_frame(&snapshot, &[ViewportInput::from_focus(focus)]);
        // size=min(320/3,7*180/24)=52, default origin=(134,15).
        assert_eq!(
            graphics.surface().get_pixel(135, 16),
            Some(Color::opaque(10, 10, 200))
        );
    }

    #[test]
    fn chrome_layout_reserves_upper_board_and_message_board_strips() {
        // C4GraphicsSystem::RecalculateViewports: the viewport area sits
        // between the 50px upper board and the one-line message board
        // (src/C4GraphicsSystem.cpp:343-348, src/C4Constants.h:77).
        let snapshot = make_snapshot();
        let focus = &snapshot.objects[0];
        let board = ImageData::new(4, 55, vec![120; 4 * 55 * 4]);
        let hud_graphics = Arc::new(HudGraphics {
            upper_board: Some(board),
            ..HudGraphics::default()
        });
        let mut graphics = GraphicsSystem::new(
            320,
            240,
            150,
            "Chrome",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            hud_graphics,
        );
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);
        let rect = graphics.active_viewports[0].rect;
        assert_eq!(rect.y, hud::UPPER_BOARD_HEIGHT);
        let board_height = graphics.message_board_height();
        assert_eq!(
            rect.height as i32,
            240 - hud::UPPER_BOARD_HEIGHT - board_height
        );
    }

    fn viewport_layout(width: u32, height: u32, count: usize) -> Vec<SurfaceRect> {
        GraphicsSystem::new(
            width,
            height,
            height as i32,
            "Viewport layout",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        )
        .layout_viewports(count)
    }

    #[test]
    fn splitscreen_layout_matches_cpp_for_two_players() {
        assert_eq!(
            viewport_layout(800, 600, 2),
            vec![
                SurfaceRect::new(0, 0, 396, 600),
                SurfaceRect::new(400, 0, 400, 600),
            ]
        );
    }

    #[test]
    fn splitscreen_layout_matches_cpp_for_three_players() {
        assert_eq!(
            viewport_layout(800, 600, 3),
            vec![
                SurfaceRect::new(0, 0, 262, 600),
                SurfaceRect::new(266, 0, 262, 600),
                SurfaceRect::new(532, 0, 266, 600),
            ]
        );
    }

    #[test]
    fn splitscreen_layout_matches_cpp_for_four_players() {
        assert_eq!(
            viewport_layout(800, 600, 4),
            vec![
                SurfaceRect::new(0, 0, 396, 296),
                SurfaceRect::new(400, 0, 400, 296),
                SurfaceRect::new(0, 300, 396, 300),
                SurfaceRect::new(400, 300, 400, 300),
            ]
        );
    }

    #[test]
    fn splitscreen_layout_leaves_cpp_integer_division_remainder_unassigned() {
        assert_eq!(
            viewport_layout(801, 601, 4),
            vec![
                SurfaceRect::new(0, 0, 396, 296),
                SurfaceRect::new(400, 0, 400, 296),
                SurfaceRect::new(0, 300, 396, 300),
                SurfaceRect::new(400, 300, 400, 300),
            ]
        );
    }

    #[test]
    fn sprite_atlas_captures_back_buffer_and_object() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].vertices = vec![
            ObjectVertex::new(-4, -4),
            ObjectVertex::new(4, -4),
            ObjectVertex::new(4, 4),
            ObjectVertex::new(-4, 4),
        ];
        let focus = &snapshot.objects[0];
        let mut graphics = GraphicsSystem::new(
            120,
            80,
            60,
            "Atlas Scenario",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );

        let viewports = vec![ViewportInput::from_focus(focus)];
        let atlas = graphics.render_frame(&snapshot, &viewports);

        assert!(atlas.iter().any(|entry| entry.label == "back_buffer"));
        let object_label = format!("object#{}:def={}", focus.id.as_u64(), focus.definition_id);
        assert!(
            atlas.iter().any(|entry| entry.label == object_label),
            "expected atlas entry for {object_label}, got labels: {:?}",
            atlas.iter().map(|entry| &entry.label).collect::<Vec<_>>()
        );
    }

    #[test]
    fn viewport_tracks_focus_vertically() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(100, 260);
        snapshot.landscape = Some(Landscape::flat(256, 280));
        let mut graphics = GraphicsSystem::new(
            320,
            180,
            150,
            "Test Scenario",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (_, viewport_y) = graphics.viewport();
        assert!(viewport_y > 0);
    }

    fn initialized_camera(view_x: i32, view_y: i32, width: i32, height: i32) -> CameraState {
        CameraState {
            d_view_x: itofix(view_x),
            d_view_y: itofix(view_y),
            view_x,
            view_y,
            view_width: width,
            view_height: height,
        }
    }

    #[test]
    fn camera_smoothing_uses_cpp_fixed_divisor_four_sequence() {
        // C4Viewport.cpp:1203-1206 retains the 16.16 residue and projects
        // each graphics pass with fixtoi. A 0 -> 100 target therefore does
        // not follow the old f32 alpha-0.2 sequence (20,36,48.8).
        let mut camera = initialized_camera(0, 0, 100, 1);
        let mut visible = Vec::new();
        for _ in 0..3 {
            visible.push(
                camera
                    .update(150, 0, 100, 1, 1_000, 1, VIEWPORT_SCROLL_BORDER, 4)
                    .0,
            );
        }
        assert_eq!(visible, vec![25, 44, 58]);
    }

    #[test]
    fn camera_smoothing_has_no_small_or_jump_snap_thresholds() {
        let mut one_pixel = initialized_camera(0, 0, 100, 1);
        let mut one_pixel_visible = Vec::new();
        for _ in 0..3 {
            one_pixel_visible.push(
                one_pixel
                    .update(51, 0, 100, 1, 1_000, 1, VIEWPORT_SCROLL_BORDER, 4)
                    .0,
            );
        }
        assert_eq!(one_pixel_visible, vec![0, 0, 1]);

        let mut jump = initialized_camera(0, 0, 100, 1);
        assert_eq!(
            jump.update(450, 0, 100, 1, 1_000, 1, VIEWPORT_SCROLL_BORDER, 4)
                .0,
            100,
            "a 400px target delta is quartered rather than snapped"
        );
    }

    #[test]
    fn camera_scroll_smooth_is_clamped_like_cpp_config() {
        let mut zero = initialized_camera(0, 0, 100, 1);
        assert_eq!(
            zero.update(150, 0, 100, 1, 1_000, 1, VIEWPORT_SCROLL_BORDER, 0)
                .0,
            100,
            "ScrollSmooth=0 clamps to divisor one"
        );

        let mut huge = initialized_camera(0, 0, 100, 1);
        assert_eq!(
            huge.update(150, 0, 100, 1, 1_000, 1, VIEWPORT_SCROLL_BORDER, 500)
                .0,
            2,
            "ScrollSmooth values above 50 clamp to divisor 50"
        );
    }

    #[test]
    fn camera_dead_zone_delays_slow_elevator_follow_per_render() {
        // With a 100x80 viewport the shared range is 8px. The focus can move
        // eight pixels without changing the target. At nine pixels the target
        // advances to 451, whose fixed projection remains 450 for two more
        // graphics passes before rounding to 451 on the third.
        let mut camera = initialized_camera(450, 460, 100, 80);
        assert_eq!(
            camera
                .update(508, 500, 100, 80, 1_000, 1_000, VIEWPORT_SCROLL_BORDER, 4)
                .0,
            450
        );
        let repeated = (0..3)
            .map(|_| {
                camera
                    .update(509, 500, 100, 80, 1_000, 1_000, VIEWPORT_SCROLL_BORDER, 4)
                    .0
            })
            .collect::<Vec<_>>();
        assert_eq!(repeated, vec![450, 450, 451]);
    }

    #[test]
    fn camera_edge_bounds_progress_through_the_cpp_scroll_border() {
        let first_view = |center_x| {
            let mut camera = CameraState::new(500, 500, 100, 80);
            camera
                .update(
                    center_x,
                    250,
                    100,
                    80,
                    500,
                    500,
                    VIEWPORT_SCROLL_BORDER,
                    DEFAULT_SCROLL_SMOOTH,
                )
                .0
        };
        assert_eq!(first_view(0), -40);
        assert_eq!(first_view(20), -20);
        assert_eq!(first_view(40), 0);

        // The negative dViewX makes C++ take the coupled initialization
        // branch on the next pass, snapping both axes to their new targets.
        let mut camera = CameraState::new(500, 500, 100, 80);
        assert_eq!(
            camera
                .update(0, 250, 100, 80, 500, 500, VIEWPORT_SCROLL_BORDER, 4)
                .0,
            -40
        );
        assert_eq!(
            camera
                .update(100, 250, 100, 80, 500, 500, VIEWPORT_SCROLL_BORDER, 4)
                .0,
            42
        );
    }

    fn camera_world_snapshot() -> SimulationSnapshot {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(500, 500);
        snapshot.landscape = Some(Landscape::flat(1_000, 1_000));
        snapshot
    }

    #[test]
    fn camera_state_survives_focus_changes_in_the_same_viewport_slot() {
        let mut snapshot = camera_world_snapshot();
        let mut second = snapshot.objects[0].clone();
        second.id = ObjectId::new(2);
        second.position = Vector2::new(900, 500);
        snapshot.objects.push(second);
        let mut graphics = GraphicsSystem::new(
            100,
            80,
            80,
            "Camera focus",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );

        graphics.render_frame(
            &snapshot,
            &[ViewportInput::new(
                0,
                Vector2::new(500, 500),
                1.0,
                &snapshot.objects[0],
            )],
        );
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::new(
                0,
                Vector2::new(900, 500),
                1.0,
                &snapshot.objects[1],
            )],
        );

        let camera = graphics
            .camera_states
            .get(&CameraKey { owner: 0, slot: 0 })
            .expect("stable viewport camera");
        assert_eq!(camera.view_x, 548);
        assert_eq!(graphics.active_viewports[0].viewport_x, 548.0);
    }

    #[test]
    fn script_view_offset_applies_after_camera_smoothing() {
        // C4Viewport::Execute computes/smooths dViewX/Y first, then adds
        // ViewOffsX/Y only to the rendered ViewX/Y (C4Viewport.cpp:1183-1214).
        // Earthquake shake must therefore move the current frame instantly
        // without feeding the displacement back into the smooth camera.
        let snapshot = camera_world_snapshot();
        let mut graphics = GraphicsSystem::new(
            100,
            80,
            80,
            "Script view offset",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let base = ViewportInput::new(
            0,
            Vector2::new(500, 500),
            1.0,
            &snapshot.objects[0],
        );
        graphics.render_frame(&snapshot, &[base]);
        let camera_before = *graphics
            .camera_states
            .get(&CameraKey { owner: 0, slot: 0 })
            .expect("camera state after baseline render");

        let shaken = ViewportInput::new(
            0,
            Vector2::new(500, 500),
            1.0,
            &snapshot.objects[0],
        )
        .with_offset(Vector2::new(7, -4));
        graphics.render_frame(&snapshot, &[shaken]);

        let camera_after = graphics
            .camera_states
            .get(&CameraKey { owner: 0, slot: 0 })
            .expect("camera state after shaken render");
        assert_eq!(camera_after.view_x, camera_before.view_x);
        assert_eq!(camera_after.view_y, camera_before.view_y);
        assert_eq!(
            graphics.active_viewports[0].viewport_x,
            camera_before.view_x as f32 + 7.0
        );
        assert_eq!(
            graphics.active_viewports[0].viewport_y,
            camera_before.view_y as f32 - 4.0
        );
    }

    #[test]
    fn camera_state_survives_a_render_where_the_viewport_is_absent() {
        let mut snapshot = camera_world_snapshot();
        let mut second = snapshot.objects[0].clone();
        second.id = ObjectId::new(2);
        second.position = Vector2::new(900, 500);
        snapshot.objects.push(second);
        let mut graphics = GraphicsSystem::new(
            100,
            80,
            80,
            "Camera absence",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::new(
                0,
                Vector2::new(500, 500),
                1.0,
                &snapshot.objects[0],
            )],
        );

        let mut absent = snapshot.clone();
        absent.objects.clear();
        graphics.render_frame(&absent, &[]);

        graphics.render_frame(
            &snapshot,
            &[ViewportInput::new(
                0,
                Vector2::new(900, 500),
                1.0,
                &snapshot.objects[1],
            )],
        );
        assert_eq!(
            graphics
                .camera_states
                .get(&CameraKey { owner: 0, slot: 0 })
                .expect("camera retained across missed draw")
                .view_x,
            548
        );
    }

    #[test]
    fn camera_edge_border_remains_tiled_outside_world_content() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(0, 250);
        snapshot.landscape = Some(Landscape::flat(500, 500));
        let background_color = Color::opaque(73, 41, 19);
        let background = ImageData::new(
            1,
            1,
            vec![
                background_color.r,
                background_color.g,
                background_color.b,
                background_color.a,
            ],
        );
        let mut graphics = GraphicsSystem::new(
            100,
            80,
            80,
            "Camera border",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            Arc::new(HudGraphics {
                background: Some(background),
                ..HudGraphics::default()
            }),
        );
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::from_focus(&snapshot.objects[0])],
        );

        let camera = graphics
            .camera_states
            .get(&CameraKey { owner: 0, slot: 0 })
            .expect("camera state");
        assert_eq!(camera.view_x, -40);
        assert_eq!(graphics.active_viewports[0].content_rect.x, 40);
        assert_eq!(graphics.active_viewports[0].content_rect.width, 60);
        assert_eq!(graphics.surface().get_pixel(0, 0), Some(background_color));
        assert_ne!(
            graphics.surface().get_pixel(40, 0),
            Some(background_color),
            "in-world sky starts after the tiled border"
        );
    }

    #[test]
    fn no_owner_viewport_stays_centered_without_free_scroll_input() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].owner = OWNER_NONE;
        snapshot.objects[0].position = Vector2::new(0, 0);
        snapshot.landscape = Some(Landscape::flat(500, 500));
        let mut graphics = GraphicsSystem::new(
            100,
            80,
            80,
            "Observer",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::from_focus(&snapshot.objects[0])],
        );
        let camera = graphics
            .camera_states
            .get(&CameraKey {
                owner: OWNER_NONE,
                slot: 0,
            })
            .expect("no-owner camera");
        assert_eq!((camera.view_x, camera.view_y), (200, 210));
    }

    #[test]
    fn viewport_zoom_uses_cpp_ceil_extent_without_resetting_fixed_state() {
        let snapshot = camera_world_snapshot();
        let mut graphics = GraphicsSystem::new(
            100,
            80,
            80,
            "Camera scale",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.render_frame(
            &snapshot,
            &[ViewportInput::new(
                0,
                Vector2::new(500, 500),
                1.5,
                &snapshot.objects[0],
            )],
        );
        let camera = graphics
            .camera_states
            .get(&CameraKey { owner: 0, slot: 0 })
            .expect("scaled camera");
        assert_eq!((camera.view_width, camera.view_height), (67, 54));
        assert_ne!(camera.d_view_x, itofix(CAMERA_UNINITIALIZED));
    }

    #[test]
    fn viewport_clamps_to_world_height() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(100, 30);
        snapshot.landscape = Some(Landscape::flat(256, 200));
        let mut graphics = GraphicsSystem::new(
            320,
            180,
            150,
            "Test Scenario",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);
        let (_, top_view) = graphics.viewport();
        assert_eq!(top_view, 0);

        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(100, 360);
        snapshot.landscape = Some(Landscape::flat(256, 360));
        let mut graphics = GraphicsSystem::new(
            320,
            180,
            150,
            "Test Scenario",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);
        let (_, bottom_view) = graphics.viewport();
        assert_eq!(
            bottom_view,
            360 - 180 + VIEWPORT_SCROLL_BORDER,
            "a focus at the raw map bottom exposes C++'s 40px scroll border"
        );
    }

    #[test]
    fn viewport_uses_the_landscape_world_height_below_the_surface_depth() {
        // `GBackHgt` is the authoritative viewport bound; it is not inferred
        // from the deepest solid column (C4Viewport.cpp:1160-1209).
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(320, 240);
        let mut landscape = Landscape::flat(640, 300);
        landscape.set_world_height(480);
        snapshot.landscape = Some(landscape);
        let mut graphics = GraphicsSystem::new(
            640,
            480,
            300,
            "Tutorial",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );

        let viewports = vec![ViewportInput::from_focus(&snapshot.objects[0])];
        graphics.render_frame(&snapshot, &viewports);

        assert_eq!(graphics.active_viewports[0].content_rect.height, 480);
    }

    #[test]
    fn small_world_viewport_is_centered_with_tiled_scroll_borders() {
        // Fullscreen viewports are capped to GBackWdt/Hgt plus the two
        // 40-pixel scroll borders (C4GraphicsSystem.cpp:384-396). Areas
        // outside the viewport and its landscape borders tile Background.png
        // (C4GraphicsSystem.cpp:285-290; C4Viewport.cpp:1030-1041).
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(320, 240);
        let surface = (0..640)
            .map(|x| if x % 2 == 0 { 479 } else { 480 })
            .collect();
        let mut landscape = Landscape::new(640, surface).expect("valid landscape surface");
        landscape.set_world_height(480);
        snapshot.landscape = Some(landscape);
        let background_pattern = [
            Color::opaque(73, 41, 19),
            Color::opaque(19, 73, 41),
            Color::opaque(41, 19, 73),
            Color::opaque(101, 83, 59),
        ];
        let background = ImageData::new(
            2,
            2,
            background_pattern
                .iter()
                .flat_map(|color| [color.r, color.g, color.b, color.a])
                .collect(),
        );
        let hud_graphics = Arc::new(HudGraphics {
            background: Some(background),
            ..HudGraphics::default()
        });
        let mut graphics = GraphicsSystem::new(
            1_000,
            800,
            300,
            "Tutorial",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            hud_graphics,
        );

        let viewports = vec![ViewportInput::from_focus(&snapshot.objects[0])];
        graphics.render_frame(&snapshot, &viewports);

        let viewport = &graphics.active_viewports[0];
        assert_eq!(
            (
                viewport.rect.x,
                viewport.rect.y,
                viewport.rect.width,
                viewport.rect.height
            ),
            (140, 120, 720, 560)
        );
        assert_eq!(
            (
                viewport.content_rect.x,
                viewport.content_rect.y,
                viewport.content_rect.width,
                viewport.content_rect.height
            ),
            (180, 160, 640, 480)
        );

        let pattern_at = |x: u32, y: u32| background_pattern[((y % 2) * 2 + x % 2) as usize];
        assert_eq!(graphics.surface().get_pixel(0, 0), Some(pattern_at(0, 0)));
        assert_eq!(
            graphics.surface().get_pixel(140, 120),
            Some(pattern_at(140, 120))
        );

        let last_content_y =
            (viewport.content_rect.y + viewport.content_rect.height as i32 - 1) as u32;
        let first_border_y = (viewport.content_rect.y + viewport.content_rect.height as i32) as u32;
        let terrain_x = (viewport.content_rect.x + 10) as u32;
        let sky_x = terrain_x + 1;
        let terrain_bottom = graphics
            .surface()
            .get_pixel(terrain_x, last_content_y)
            .expect("terrain bottom pixel");
        let sky_bottom = graphics
            .surface()
            .get_pixel(sky_x, last_content_y)
            .expect("sky bottom pixel");
        assert_ne!(terrain_bottom, sky_bottom, "bottom row must be nonuniform");

        for x in [terrain_x, sky_x] {
            let border = graphics
                .surface()
                .get_pixel(x, first_border_y)
                .expect("first border pixel below content");
            let last_content = graphics
                .surface()
                .get_pixel(x, last_content_y)
                .expect("last content pixel");
            assert_eq!(border, pattern_at(x, first_border_y));
            assert_ne!(border, last_content, "terrain edge must not be extended");
        }
    }

    #[test]
    fn object_color_reflects_energy_level() {
        let snapshot = make_snapshot();
        let mut energized = snapshot.objects[0].clone();
        energized.energy = 100;
        let high = object_color(&energized);

        let mut depleted = energized.clone();
        depleted.energy = 0;
        let low = object_color(&depleted);

        assert_ne!(high, low);
        let high_sum = u16::from(high.r) + u16::from(high.g) + u16::from(high.b);
        let low_sum = u16::from(low.r) + u16::from(low.g) + u16::from(low.b);
        assert!(high_sum > low_sum);
    }

    #[test]
    fn fill_polygon_paints_triangle() {
        let mut surface = Surface::new(32, 32, PixelFormat::Rgba8888);
        let color = Color::opaque(48, 64, 96);
        let triangle = [(4, 4), (24, 6), (10, 24)];

        let painted = fill_polygon(&mut surface, &triangle, color);
        assert!(painted);
        assert_eq!(surface.get_pixel(12, 12), Some(color));
    }

    #[test]
    fn render_frame_draws_object_vertices() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].vertices = vec![
            ObjectVertex::new(-6, -6),
            ObjectVertex::new(6, -6),
            ObjectVertex::new(6, 6),
            ObjectVertex::new(-6, 6),
        ];
        snapshot.landscape = Some(Landscape::flat(128, 80));

        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Polygon Scenario",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let lighting = GraphicsSystem::lighting_factor(snapshot.environment.settings.time_of_day);
        let expected = GraphicsSystem::apply_lighting(object_color(&snapshot.objects[0]), lighting);
        let (viewport_x, viewport_y) = graphics.viewport();
        let screen_x = snapshot.objects[0].position.x - viewport_x;
        let screen_y = snapshot.objects[0].position.y - viewport_y;
        assert!(screen_x >= 0 && screen_x < graphics.surface().width() as i32);
        assert!(screen_y >= 0 && screen_y < graphics.surface().height() as i32);
        let pixel = graphics
            .surface()
            .get_pixel(screen_x as u32, screen_y as u32);
        assert_eq!(pixel, Some(expected));
    }

    #[test]
    fn contained_objects_are_not_drawn_in_the_world() {
        // `if (Contained && !eDrawMode) return;` (src/C4Object.cpp:2363):
        // carried items (e.g. the Mage's starting FLAG) never blit into the
        // landscape — they only appear in HUD inventory/menus.
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].vertices = vec![
            ObjectVertex::new(-6, -6),
            ObjectVertex::new(6, -6),
            ObjectVertex::new(6, 6),
            ObjectVertex::new(-6, 6),
        ];
        snapshot.objects[0].container = Some(ObjectId::new(999));
        snapshot.landscape = Some(Landscape::flat(128, 80));

        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Contained Scenario",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let lighting = GraphicsSystem::lighting_factor(snapshot.environment.settings.time_of_day);
        let filled = GraphicsSystem::apply_lighting(object_color(&snapshot.objects[0]), lighting);
        let (viewport_x, viewport_y) = graphics.viewport();
        let screen_x = snapshot.objects[0].position.x - viewport_x;
        let screen_y = snapshot.objects[0].position.y - viewport_y;
        let pixel = graphics
            .surface()
            .get_pixel(screen_x as u32, screen_y as u32);
        assert_ne!(
            pixel,
            Some(filled),
            "contained object must not paint its debug polygon"
        );
    }

    fn solid_sprite(
        definition_id: &str,
        width: u32,
        height: u32,
        color: Color,
        shape: Option<DefinitionRect>,
        stretch_growth: bool,
    ) -> Arc<HashMap<String, DefinitionSprite>> {
        let pixels: Vec<u8> = (0..width * height)
            .flat_map(|_| [color.r, color.g, color.b, color.a])
            .collect();
        let mut sprites = HashMap::new();
        sprites.insert(
            sprite_map_key(definition_id, None),
            DefinitionSprite {
                image: ImageData::new(width, height, pixels),
                actions: HashMap::new(),
                color_mask: None,
                shape,
                stretch_growth,
                top_face: None,
            },
        );
        Arc::new(sprites)
    }

    #[test]
    fn definition_top_faces_draw_after_every_object_base_like_cpp() {
        // C4ObjectList::Draw performs one complete base pass and only then a
        // complete TopFace pass (src/C4ObjectList.cpp:390-396). Thus A's
        // TopFace must cover the later overlapping base of B.
        let mut snapshot = make_snapshot();
        snapshot.objects[0].container = Some(ObjectId::new(99));
        let mut top_object = snapshot.objects[0].clone();
        top_object.id = ObjectId::new(2);
        top_object.definition_id = "TopObject".to_string();
        top_object.position = Vector2::new(105, 100);
        top_object.container = None;
        top_object.crew_member = false;
        top_object.action = lc_engine::ActionState::new("Active");
        let mut base_object = top_object.clone();
        base_object.id = ObjectId::new(3);
        base_object.definition_id = "BaseObject".to_string();
        base_object.action = Default::default();
        snapshot.objects.extend([top_object, base_object]);
        snapshot.landscape = Some(Landscape::flat(160, 140));

        let green = Color::opaque(0, 200, 0);
        let blue = Color::opaque(0, 0, 200);
        let mut sprites = HashMap::new();
        sprites.insert(
            sprite_map_key("TopObject", None),
            DefinitionSprite {
                image: ImageData::new(2, 1, vec![0, 0, 0, 0, 0, 200, 0, 255]),
                actions: HashMap::from([(
                    "Active".to_string(),
                    DefinitionActionGraphics::default(),
                )]),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                stretch_growth: false,
                top_face: Some(DefinitionTargetRect::new(1, 0, 1, 1, 0, 0)),
            },
        );
        sprites.insert(
            sprite_map_key("BaseObject", None),
            DefinitionSprite {
                image: ImageData::new(1, 1, vec![0, 0, 200, 255]),
                actions: HashMap::new(),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                stretch_growth: false,
                top_face: None,
            },
        );

        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "TopFace pass",
            test_font(),
            Arc::new(sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.render_frame(&snapshot, &[ViewportInput::from_focus(&snapshot.objects[0])]);
        let (viewport_x, viewport_y) = graphics.viewport();
        let x = (105 - viewport_x) as u32;
        let y = (100 - viewport_y) as u32;
        assert_eq!(
            graphics.surface().get_pixel(x, y),
            Some(standard_gamma_color(green))
        );
        assert_ne!(
            graphics.surface().get_pixel(x, y),
            Some(standard_gamma_color(blue))
        );
    }

    #[test]
    fn snapshot_order_keeps_elevator_case_over_base_when_y_sort_conflicts() {
        // C4ObjectList draws both its base and TopFace passes Last -> Prev
        // without positional sorting (src/C4ObjectList.cpp:387-396).
        // ELEV explicitly orders ELEC over itself (Elevator/Script.c:12-14),
        // which must still hold after the carriage rises above the base.
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(40, 50);
        snapshot.objects[0].container = Some(ObjectId::new(99));

        let elevator_id = ObjectId::new(3);
        let case_id = ObjectId::new(2);
        let mut elevator = snapshot.objects[0].clone();
        elevator.id = elevator_id;
        elevator.definition_id = "ELEV".to_string();
        elevator.position = Vector2::new(64, 50);
        elevator.container = None;
        elevator.crew_member = false;
        elevator.action = lc_engine::ActionState::new("Active");

        let mut case = elevator.clone();
        case.id = case_id;
        case.definition_id = "ELEC".to_string();
        case.position.y = 40;
        // Object payloads remain canonical by ID; the sidecar is C++'s
        // Last->Prev draw order: ELEV, then ELEC.
        snapshot.objects.extend([case, elevator]);
        snapshot.render_order = vec![ObjectId::new(1), elevator_id, case_id];
        snapshot.landscape = Some(Landscape::flat(128, 100));

        let red = Color::opaque(200, 0, 0);
        let green = Color::opaque(0, 200, 0);
        let mut sprites = HashMap::new();
        sprites.insert(
            sprite_map_key("ELEV", None),
            DefinitionSprite {
                image: ImageData::new(1, 1, vec![red.r, red.g, red.b, red.a]),
                actions: HashMap::from([(
                    "Active".to_string(),
                    DefinitionActionGraphics::default(),
                )]),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                stretch_growth: false,
                top_face: Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 0)),
            },
        );
        sprites.insert(
            sprite_map_key("ELEC", None),
            DefinitionSprite {
                image: ImageData::new(1, 1, vec![green.r, green.g, green.b, green.a]),
                actions: HashMap::from([(
                    "Active".to_string(),
                    DefinitionActionGraphics::default(),
                )]),
                color_mask: None,
                shape: Some(DefinitionRect::new(0, 0, 1, 1)),
                stretch_growth: false,
                top_face: Some(DefinitionTargetRect::new(0, 0, 1, 1, 0, 10)),
            },
        );

        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Elevator object order",
            test_font(),
            Arc::new(sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let viewports = vec![ViewportInput::from_focus(&snapshot.objects[0])];
        graphics.render_frame(&snapshot, &viewports);

        let (viewport_x, viewport_y) = graphics.viewport();
        assert_eq!(
            graphics
                .surface()
                .get_pixel((64 - viewport_x) as u32, (50 - viewport_y) as u32),
            Some(standard_gamma_color(green)),
            "SetObjectOrder keeps the raised ELEC TopFace over ELEV"
        );
    }

    #[test]
    fn sprite_takes_precedence_over_vertex_polygon() {
        // C4Object::Draw never renders shape vertices as geometry — an
        // object with a graphics facet always blits it (src/C4Object.cpp:
        // 2388-2392 idle DrawFace); the polygon is only our debug fallback
        // for sprite-less objects.
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].vertices = vec![
            ObjectVertex::new(-6, -6),
            ObjectVertex::new(6, -6),
            ObjectVertex::new(6, 6),
            ObjectVertex::new(-6, 6),
        ];
        snapshot.landscape = Some(Landscape::flat(128, 80));

        let green = Color::opaque(0, 200, 0);
        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Sprite Precedence",
            test_font(),
            solid_sprite(
                "TestObject",
                8,
                8,
                green,
                Some(DefinitionRect::new(-4, -4, 8, 8)),
                false,
            ),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (viewport_x, viewport_y) = graphics.viewport();
        let screen_x = (snapshot.objects[0].position.x - viewport_x) as u32;
        let screen_y = (snapshot.objects[0].position.y - viewport_y) as u32;
        assert_eq!(
            graphics.surface().get_pixel(screen_x, screen_y),
            Some(standard_gamma_color(green)),
            "expected the sprite pixel, not the vertex-polygon fill"
        );
    }

    #[test]
    fn idle_face_is_anchored_at_shape_top_left() {
        // C4Object::Draw anchors the face at the shape top-left:
        // cox = x + Shape.x, coy = y + Shape.y (src/C4Object.cpp:2231),
        // and DrawFace blits Shape.Wdt x Shape.Hgt there
        // (src/C4Object.cpp:438-451) — never centered on the position.
        let mut snapshot = make_snapshot();
        // Keep the focus on a sprite-less dummy so its selection marks /
        // energy bar do not overdraw the probed object.
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].definition_id = "FocusDummy".to_string();
        let mut subject = snapshot.objects[0].clone();
        subject.id = ObjectId::new(2);
        subject.definition_id = "TestObject".to_string();
        subject.position = Vector2::new(64, 40);
        subject.crew_member = false;
        snapshot.objects.push(subject);
        snapshot.landscape = Some(Landscape::flat(128, 80));

        let green = Color::opaque(0, 200, 0);
        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Idle Anchor",
            test_font(),
            solid_sprite(
                "TestObject",
                8,
                8,
                green,
                Some(DefinitionRect::new(-8, -8, 8, 8)),
                false,
            ),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (viewport_x, viewport_y) = graphics.viewport();
        let sx = 64 - viewport_x;
        let sy = 40 - viewport_y;
        // The face covers world [56,64) x [32,40).
        assert_eq!(
            graphics.surface().get_pixel((sx - 5) as u32, (sy - 5) as u32),
            Some(standard_gamma_color(green)),
            "expected the face inside the shape rect"
        );
        assert_ne!(
            graphics.surface().get_pixel((sx + 1) as u32, (sy + 1) as u32),
            Some(standard_gamma_color(green)),
            "face must not extend past the shape rect (centered draw would)"
        );
        assert_ne!(
            graphics.surface().get_pixel((sx - 9) as u32, (sy - 9) as u32),
            Some(standard_gamma_color(green)),
            "face must start at the shape top-left"
        );
    }

    #[test]
    fn growing_face_shrinks_toward_the_scaled_shape_rect() {
        // GrowthType con display (src/C4Object.cpp:448-451): the target
        // is swdt*Con/FullCon x shgt*Con/FullCon centered in the
        // con-scaled shape rect (C4Shape::Stretch scales Offset too,
        // src/C4Shape.cpp:105-109) — not centered on the position.
        let mut snapshot = make_snapshot();
        // Keep the focus on a sprite-less dummy so its selection marks /
        // energy bar do not overdraw the probed object.
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].definition_id = "FocusDummy".to_string();
        let mut subject = snapshot.objects[0].clone();
        subject.id = ObjectId::new(2);
        subject.definition_id = "TestObject".to_string();
        subject.position = Vector2::new(64, 40);
        subject.crew_member = false;
        subject.construction = FULL_CON / 2;
        snapshot.objects.push(subject);
        snapshot.landscape = Some(Landscape::flat(128, 80));

        let green = Color::opaque(0, 200, 0);
        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Growth Anchor",
            test_font(),
            solid_sprite(
                "TestObject",
                8,
                16,
                green,
                Some(DefinitionRect::new(0, -16, 8, 16)),
                true,
            ),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (viewport_x, viewport_y) = graphics.viewport();
        let sx = 64 - viewport_x;
        let sy = 40 - viewport_y;
        // Con 50%: inst shape (0,-8,4,8), target 4x8 at world [64,68) x [32,40).
        // (Probe the lower face rows: the GUI overlay text occupies the
        // top rows of the tiny test surface.)
        assert_eq!(
            graphics.surface().get_pixel((sx + 1) as u32, (sy - 2) as u32),
            Some(standard_gamma_color(green)),
            "expected the half-grown face inside the con-scaled shape"
        );
        assert_ne!(
            graphics.surface().get_pixel((sx - 2) as u32, (sy - 2) as u32),
            Some(standard_gamma_color(green)),
            "half-grown face must not spill left of the scaled shape"
        );
        assert_ne!(
            graphics.surface().get_pixel((sx + 5) as u32, (sy - 2) as u32),
            Some(standard_gamma_color(green)),
            "half-grown face must be half-width"
        );
    }

    #[test]
    fn base_graphics_variant_selects_the_named_sprite() {
        // SetGraphics swaps GetGraphics() to a named C4AdditionalDefGraphics
        // (src/C4DefGraphics.cpp, C4Object::SetGraphics); the snapshot
        // carries the variant on ObjectBaseGraphics and the renderer must
        // blit that sheet, not the default one.
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].definition_id = "FocusDummy".to_string();
        let mut subject = snapshot.objects[0].clone();
        subject.id = ObjectId::new(2);
        subject.definition_id = "TestObject".to_string();
        subject.position = Vector2::new(64, 40);
        subject.crew_member = false;
        subject.base_graphics = Some(lc_engine::ObjectBaseGraphics {
            definition: "TestObject".to_string(),
            graphics_name: Some("2".to_string()),
            blit_mode: 0,
        });
        snapshot.objects.push(subject);
        snapshot.landscape = Some(Landscape::flat(128, 80));

        let red = Color::opaque(200, 0, 0);
        let green = Color::opaque(0, 200, 0);
        let shape = Some(DefinitionRect::new(-4, -4, 8, 8));
        let mut sprites = HashMap::new();
        sprites.extend(
            solid_sprite("TestObject", 8, 8, red, shape, false)
                .as_ref()
                .clone(),
        );
        sprites.insert(
            sprite_map_key("TestObject", Some("2")),
            solid_sprite("TestObject", 8, 8, green, shape, false)
                .as_ref()
                .clone()
                .remove(&sprite_map_key("TestObject", None))
                .expect("variant sprite"),
        );

        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Variant",
            test_font(),
            Arc::new(sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (viewport_x, viewport_y) = graphics.viewport();
        let sx = 64 - viewport_x;
        let sy = 40 - viewport_y;
        assert_eq!(
            graphics.surface().get_pixel((sx + 1) as u32, (sy + 1) as u32),
            Some(standard_gamma_color(green)),
            "expected the '2' graphics variant, not the default sheet"
        );
    }

    #[test]
    fn action_facet_is_anchored_at_shape_plus_facet_target() {
        // Regular action facet at full con: drawn facet-sized at
        // cox + Action.FacetX / coy + Action.FacetY (src/C4Object.cpp:
        // 2453-2459), sourcing Facet x/y from the sheet.
        let mut snapshot = make_snapshot();
        // Keep the focus on a sprite-less dummy so its selection marks /
        // energy bar do not overdraw the probed object.
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].definition_id = "FocusDummy".to_string();
        let mut subject = snapshot.objects[0].clone();
        subject.id = ObjectId::new(2);
        subject.definition_id = "TestObject".to_string();
        subject.position = Vector2::new(64, 40);
        subject.crew_member = false;
        subject.action = lc_engine::ActionState::new("Still");
        snapshot.objects.push(subject);
        snapshot.landscape = Some(Landscape::flat(128, 80));

        // 16x8 sheet: left half red, right half green.
        let red = Color::opaque(200, 0, 0);
        let green = Color::opaque(0, 200, 0);
        let mut pixels = Vec::new();
        for _y in 0..8 {
            for x in 0..16 {
                let color = if x < 8 { red } else { green };
                pixels.extend_from_slice(&[color.r, color.g, color.b, color.a]);
            }
        }
        let mut actions = HashMap::new();
        actions.insert(
            "Still".to_string(),
            DefinitionActionGraphics {
                facet: Some(lc_engine::DefinitionActionFacet {
                    x: 8,
                    y: 0,
                    width: 8,
                    height: 8,
                    target_x: 2,
                    target_y: 4,
                }),
                directions: 1,
                flip_dir: None,
                reverse: false,
                facet_base: false,
                facet_top_face: false,
                facet_target_stretch: false,
                length: Some(1),
            },
        );
        let mut sprites = HashMap::new();
        sprites.insert(
            sprite_map_key("TestObject", None),
            DefinitionSprite {
                image: ImageData::new(16, 8, pixels),
                actions,
                color_mask: None,
                shape: Some(DefinitionRect::new(-4, -4, 8, 8)),
                stretch_growth: false,
                top_face: None,
            },
        );

        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Facet Anchor",
            test_font(),
            Arc::new(sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (viewport_x, viewport_y) = graphics.viewport();
        let sx = 64 - viewport_x;
        let sy = 40 - viewport_y;
        // cox = 64-4, coy = 40-4; facet dest [62,70) x [40,48) with the
        // GREEN sheet half (Facet x=8).
        assert_eq!(
            graphics.surface().get_pixel((sx + 1) as u32, (sy + 3) as u32),
            Some(standard_gamma_color(green)),
            "expected the facet at cox+FacetX/coy+FacetY sourcing Facet x/y"
        );
        assert_ne!(
            graphics.surface().get_pixel((sx - 3) as u32, (sy - 3) as u32),
            Some(standard_gamma_color(green)),
            "facet must not be centered on the position"
        );
    }

    #[test]
    fn action_facet_target_stretches_exactly_to_target_shape_top() {
        // C4Object::Draw stretches FacetTargetStretch from
        // coy + Action.FacetY through, but not beyond,
        // (Target->y + Target->Shape.y) (src/C4Object.cpp:2426-2438).
        // C4Facet::DrawX scales the declared 2x4 source into that rectangle
        // (src/C4Facet.cpp:296-303).
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(40, 60);
        snapshot.objects[0].definition_id = "FocusDummy".to_string();

        let case_id = ObjectId::new(3);
        let mut elevator = snapshot.objects[0].clone();
        elevator.id = ObjectId::new(2);
        elevator.definition_id = "ELEV".to_string();
        elevator.position = Vector2::new(64, 60);
        elevator.crew_member = false;
        elevator.action = lc_engine::ActionState::new("LiftCase");
        elevator.action.target = Some(case_id);

        let mut case = snapshot.objects[0].clone();
        case.id = case_id;
        case.definition_id = "ELEC".to_string();
        case.position = Vector2::new(64, 95);
        case.crew_member = false;
        case.container = Some(ObjectId::new(99));
        snapshot.objects.extend([elevator, case]);
        snapshot.landscape = Some(Landscape::flat(128, 120));

        let green = Color::opaque(0, 200, 0);
        let mut elevator_pixels = vec![0; 60 * 9 * 4];
        for y in 5..9 {
            for x in 58..60 {
                let offset = (y * 60 + x) * 4;
                elevator_pixels[offset..offset + 4]
                    .copy_from_slice(&[green.r, green.g, green.b, green.a]);
            }
        }
        let lift_case = DefinitionActionGraphics {
            facet: Some(lc_engine::DefinitionActionFacet {
                x: 58,
                y: 5,
                width: 2,
                height: 4,
                target_x: 13,
                target_y: 13,
            }),
            directions: 1,
            facet_base: false,
            facet_target_stretch: true,
            length: Some(1),
            ..DefinitionActionGraphics::default()
        };
        let mut sprites = HashMap::new();
        sprites.insert(
            sprite_map_key("ELEV", None),
            DefinitionSprite {
                image: ImageData::new(60, 9, elevator_pixels),
                actions: HashMap::from([("LiftCase".to_string(), lift_case)]),
                color_mask: None,
                shape: Some(DefinitionRect::new(-14, -28, 28, 56)),
                stretch_growth: false,
                top_face: None,
            },
        );
        sprites.insert(
            sprite_map_key("ELEC", None),
            DefinitionSprite {
                image: ImageData::new(24, 26, vec![0; 24 * 26 * 4]),
                actions: HashMap::new(),
                color_mask: None,
                shape: Some(DefinitionRect::new(-12, -13, 24, 26)),
                stretch_growth: false,
                top_face: None,
            },
        );

        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Facet target stretch",
            test_font(),
            Arc::new(sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let viewports = vec![ViewportInput::from_focus(&snapshot.objects[0])];
        graphics.render_frame(&snapshot, &viewports);

        let (viewport_x, viewport_y) = graphics.viewport();
        // ELEV: cox=64-14, coy=60-28. Facet target starts at (63,45).
        // ELEC target Shape.y=-13, so the exclusive bottom is 95-13=82.
        let cable_x = (63 - viewport_x) as u32;
        let cable_top = (45 - viewport_y) as u32;
        let cable_last = (81 - viewport_y) as u32;
        let cable_bottom = (82 - viewport_y) as u32;
        assert_eq!(
            graphics.surface().get_pixel(cable_x, cable_top),
            Some(standard_gamma_color(green))
        );
        assert_eq!(
            graphics.surface().get_pixel(cable_x, cable_last),
            Some(standard_gamma_color(green))
        );
        assert_eq!(
            graphics.surface().get_pixel(cable_x + 1, cable_last),
            Some(standard_gamma_color(green))
        );
        assert_ne!(
            graphics.surface().get_pixel(cable_x + 2, cable_last),
            Some(standard_gamma_color(green)),
            "the stretched target keeps the source facet's two-pixel width"
        );
        assert_ne!(
            graphics.surface().get_pixel(cable_x, cable_bottom),
            Some(standard_gamma_color(green)),
            "FacetTargetStretch must stop at the target shape's top edge"
        );
    }

    #[test]
    fn real_tutorial_elevator_facets_construction_and_live_frame_delta_match_cpp() {
        // Real Tutorial05 starts ELEV at Con=80 with no case
        // (Tutorial05/Script.c:30-34). C4Object::DrawFace exposes the bottom
        // Con slice for construction graphics (src/C4Object.cpp:440-475), and
        // UpdateFace installs a non-growth TopFace only at full con
        // (src/C4Object.cpp:357-376).
        let mut tutorial05 = load_repository_tutorial(5);
        join_repository_player(&mut tutorial05, "real Tutorial05 elevator render");
        let partial_snapshot = tutorial05.snapshot();
        let partial = partial_snapshot
            .objects
            .iter()
            .find(|object| object.definition_id == "ELEV")
            .expect("Tutorial05 creates its partial ELEV");
        assert_eq!(partial.construction, 80_000);
        assert!(
            partial_snapshot
                .objects
                .iter()
                .all(|object| object.definition_id != "ELEC"),
            "ELEV Initialize creates ELEC only after completion"
        );

        let partial_sprites = real_elevator_sprites(&tutorial05);
        let real_elev = partial_sprites
            .get(&sprite_map_key("ELEV", None))
            .expect("real ELEV sprite");
        assert_eq!(real_elev.image.width(), 84);
        assert_eq!(real_elev.image.height(), 56);
        assert_eq!(
            real_elev.shape,
            Some(DefinitionRect::new(-14, -28, 28, 56))
        );
        assert_eq!(
            real_elev.top_face,
            Some(DefinitionTargetRect::new(28, 0, 28, 56, 0, 0))
        );
        assert!(!real_elev.stretch_growth);
        assert!(real_elev.color_mask.is_none());

        let partial_origin = Vector2::new(partial.position.x - 48, partial.position.y - 56);
        let mut partial_graphics = GraphicsSystem::new(
            96,
            112,
            112,
            "real Tutorial05 partial ELEV",
            test_font(),
            Arc::clone(&partial_sprites),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        partial_graphics.surface_mut().fill(Color::opaque(0, 0, 0));
        partial_graphics.viewport_x = partial_origin.x as f32;
        partial_graphics.viewport_y = partial_origin.y as f32;
        partial_graphics.paint_object(
            partial,
            &partial_snapshot.objects,
            &partial_snapshot.players,
            OWNER_NONE,
            1.0,
            &HashMap::new(),
            None,
        );

        let mut partial_expected = Surface::new(96, 112, PixelFormat::Rgba8888);
        partial_expected.fill(Color::opaque(0, 0, 0));
        // C++ construction display: source y=56*(100-80)/100=11,
        // source/destination h=56*80/100=44; Jolt makes Shape.y=-22.
        draw_image_region(
            &mut partial_expected,
            &GuiRect::new(
                (partial.position.x - 14 - partial_origin.x) as f32,
                (partial.position.y - 22 - partial_origin.y) as f32,
                28.0,
                44.0,
            ),
            &real_elev.image,
            None,
            &SourceRect::new(0, 11, 28, 44),
            false,
            None,
            SpriteBlitState::normal(),
            None,
        );
        assert_surface_pixels_eq(
            partial_graphics.surface(),
            &partial_expected,
            "real Tutorial05 ELEV must render the exact C++ eighty-percent construction slice"
        );
        let before_top_face = partial_graphics.surface().clone();
        partial_graphics.paint_object_top_face(
            partial,
            SpriteBlitState::for_object(partial),
            None,
        );
        assert_surface_pixels_eq(
            partial_graphics.surface(),
            &before_top_face,
            "an incomplete ELEV must not draw its full-con TopFace"
        );

        // Tutorial06 supplies the same real definitions and builds ELEV to
        // completion. Spawn one through the real ELEV Initialize callback so
        // SetAction("LiftCase", pCase) and SetObjectOrder run exactly as in
        // Elevator/Script.c:10-15.
        let mut tutorial06 = load_repository_tutorial(6);
        let elevator_id = tutorial06
            .spawn_object(
                SpawnConfig::new("ELEV").with_position(Vector2::new(332, 148)),
            )
            .expect("real Tutorial06 ELEV spawns");
        let first_snapshot = tutorial06.snapshot();
        let elevator = first_snapshot
            .object(elevator_id)
            .expect("spawned ELEV is present");
        assert_eq!(elevator.construction, FULL_CON);
        assert_eq!(elevator.action.name, "LiftCase");
        let case_id = elevator.action.target.expect("LiftCase targets real ELEC");
        let first_case = first_snapshot.object(case_id).expect("ELEV creates ELEC");
        assert_eq!(first_case.definition_id, "ELEC");
        assert_eq!(
            first_case.action.name, "Wait",
            "ELEC Initialize selects its facet-less active action"
        );
        // CreateObject(ELEC, 0, +27) supplies the requested construction
        // bottom. Initial DoCon then keeps that bottom fixed while changing
        // ELEC's zero-con shape to its full 26px shape, moving its center up
        // by Shape.Hgt+Shape.y=13 (src/C4Object.cpp:1428-1496).
        assert_eq!(
            first_case.position,
            Vector2::new(elevator.position.x, elevator.position.y + 14)
        );

        let sprites = real_elevator_sprites(&tutorial06);
        let elev_sprite = sprites
            .get(&sprite_map_key("ELEV", None))
            .expect("real Tutorial06 ELEV sprite");
        let lift_case = elev_sprite
            .actions
            .get("LiftCase")
            .expect("real ELEV LiftCase ActMap entry");
        let cable_facet = lift_case.facet.as_ref().expect("LiftCase cable facet");
        assert_eq!(
            (
                cable_facet.x,
                cable_facet.y,
                cable_facet.width,
                cable_facet.height,
                cable_facet.target_x,
                cable_facet.target_y,
            ),
            (58, 5, 2, 4, 13, 0),
            "the five-value C4TargetRect defaults FacetY to zero (src/C4Rect.cpp:80-84)"
        );
        assert!(lift_case.facet_base);
        assert!(lift_case.facet_target_stretch);

        let case_sprite = sprites
            .get(&sprite_map_key("ELEC", None))
            .expect("real Tutorial06 ELEC sprite");
        assert_eq!(case_sprite.image.width(), 24);
        assert_eq!(case_sprite.image.height(), 28);
        assert_eq!(
            case_sprite.shape,
            Some(DefinitionRect::new(-12, -13, 24, 26))
        );
        assert_eq!(
            case_sprite.top_face,
            Some(DefinitionTargetRect::new(0, 0, 24, 26, 0, 0))
        );
        assert!(case_sprite.color_mask.is_none());

        let origin = Vector2::new(elevator.position.x - 48, elevator.position.y - 48);
        let render_elevator_base_and_cable = |snapshot: &SimulationSnapshot| {
            let elevator = snapshot.object(elevator_id).expect("ELEV remains live");
            let mut graphics = GraphicsSystem::new(
                96,
                128,
                128,
                "real Tutorial06 ELEV cable",
                test_font(),
                Arc::clone(&sprites),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.surface_mut().fill(Color::opaque(0, 0, 0));
            graphics.viewport_x = origin.x as f32;
            graphics.viewport_y = origin.y as f32;
            graphics.paint_object(
                elevator,
                &snapshot.objects,
                &snapshot.players,
                OWNER_NONE,
                1.0,
                &HashMap::new(),
                None,
            );
            graphics.surface().clone()
        };
        let expected_elevator_base_and_cable = |snapshot: &SimulationSnapshot| {
            let elevator = snapshot.object(elevator_id).expect("ELEV remains live");
            let case = snapshot.object(case_id).expect("ELEC remains live");
            let mut expected = Surface::new(96, 128, PixelFormat::Rgba8888);
            expected.fill(Color::opaque(0, 0, 0));
            draw_image_region(
                &mut expected,
                &GuiRect::new(
                    (elevator.position.x - 14 - origin.x) as f32,
                    (elevator.position.y - 28 - origin.y) as f32,
                    28.0,
                    56.0,
                ),
                &elev_sprite.image,
                None,
                &SourceRect::new(0, 0, 28, 56),
                false,
                None,
                SpriteBlitState::normal(),
                None,
            );
            // C4Object::Draw computes the live target every draw:
            // height=(Target.y+Target.Shape.y)-(y+Shape.y+FacetY)
            // (src/C4Object.cpp:2426-2438), then DrawX stretches the declared
            // 2x4 source (src/C4Facet.cpp:296-303).
            let cable_top = elevator.position.y - 28 + cable_facet.target_y;
            let case_top = case.position.y - 13;
            draw_image_region(
                &mut expected,
                &GuiRect::new(
                    (elevator.position.x - 14 + cable_facet.target_x - origin.x) as f32,
                    (cable_top - origin.y) as f32,
                    cable_facet.width as f32,
                    (case_top - cable_top) as f32,
                ),
                &elev_sprite.image,
                None,
                &SourceRect::new(
                    cable_facet.x,
                    cable_facet.y,
                    cable_facet.width,
                    cable_facet.height,
                ),
                false,
                None,
                SpriteBlitState::normal(),
                None,
            );
            expected
        };
        let render_case = |snapshot: &SimulationSnapshot| {
            let case = snapshot.object(case_id).expect("ELEC remains live");
            let mut graphics = GraphicsSystem::new(
                96,
                128,
                128,
                "real Tutorial06 ELEC carriage",
                test_font(),
                Arc::clone(&sprites),
                empty_cursor_atlas(),
                empty_hud_graphics(),
            );
            graphics.surface_mut().fill(Color::opaque(0, 0, 0));
            graphics.viewport_x = origin.x as f32;
            graphics.viewport_y = origin.y as f32;
            graphics.paint_object(
                case,
                &snapshot.objects,
                &snapshot.players,
                OWNER_NONE,
                1.0,
                &HashMap::new(),
                None,
            );
            graphics.paint_object_top_face(case, SpriteBlitState::for_object(case), None);
            graphics.surface().clone()
        };
        let expected_case = |snapshot: &SimulationSnapshot| {
            let case = snapshot.object(case_id).expect("ELEC remains live");
            let mut expected = Surface::new(96, 128, PixelFormat::Rgba8888);
            expected.fill(Color::opaque(0, 0, 0));
            // Wait is active but has neither FacetBase nor Facet, so the
            // C4Object::Draw base pass draws nothing (src/C4Object.cpp:
            // 2419-2496). The full carriage is this one DrawTopFace blit in
            // the second object-list pass (src/C4ObjectList.cpp:387-396;
            // src/C4Object.cpp:2617-2670).
            draw_image_region(
                &mut expected,
                &GuiRect::new(
                    (case.position.x - 12 - origin.x) as f32,
                    (case.position.y - 13 - origin.y) as f32,
                    24.0,
                    26.0,
                ),
                &case_sprite.image,
                None,
                &SourceRect::new(0, 0, 24, 26),
                false,
                None,
                SpriteBlitState::normal(),
                None,
            );
            expected
        };

        let first_cable = render_elevator_base_and_cable(&first_snapshot);
        assert_surface_pixels_eq(
            &first_cable,
            &expected_elevator_base_and_cable(&first_snapshot),
            "real LiftCase must use the shipped cable facet and live ELEC top"
        );
        let first_carriage = render_case(&first_snapshot);
        assert_surface_pixels_eq(
            &first_carriage,
            &expected_case(&first_snapshot),
            "real ELEC full-con TopFace must render the carriage"
        );
        assert!(
            first_carriage
                .pixels()
                .chunks_exact(4)
                .any(|pixel| pixel != [0, 0, 0, 255]),
            "regression guard: a missing ELEC carriage must fail visibly"
        );

        let moved_position = Vector2::new(first_case.position.x, first_case.position.y + 1);
        tutorial06
            .apply_object_update(case_id, ObjectUpdate::new().with_position(moved_position))
            .expect("move real ELEC by one live simulation pixel");
        let second_snapshot = tutorial06.tick().expect("next Tutorial06 frame");
        assert_eq!(second_snapshot.frame, first_snapshot.frame + 1);
        let second_case = second_snapshot.object(case_id).expect("moved ELEC survives");
        assert_eq!(second_case.position, moved_position);

        let second_cable = render_elevator_base_and_cable(&second_snapshot);
        assert_surface_pixels_eq(
            &second_cable,
            &expected_elevator_base_and_cable(&second_snapshot),
            "the cable endpoint must follow the live case by one frame pixel"
        );
        let second_carriage = render_case(&second_snapshot);
        assert_surface_pixels_eq(
            &second_carriage,
            &expected_case(&second_snapshot),
            "the rendered carriage must follow the live case without lag or quantization"
        );
        assert!(
            first_carriage.pixels() != second_carriage.pixels(),
            "consecutive snapshots one pixel apart must produce distinct carriage placement"
        );
    }

    #[test]
    fn render_frame_draws_player_cursor() {
        // C4Game::DrawCursors (src/C4Game.cpp:1852-1874): while CursorFlash
        // or SelectFlash runs, ONE cell of the mouse-cursor sheet is drawn
        // above the cursor clonk — fctCursor is the 35th square cell (cell
        // size = sheet height, C4GraphicsResource::ApplyCursorGfx,
        // src/C4GraphicsResource.cpp:328-336), NOT the whole sheet.
        let mut snapshot = make_snapshot();
        snapshot.objects[0].owner = 1;
        let object_id = snapshot.objects[0].id;
        snapshot.players.push(PlayerState {
            id: 1,
            cursor: Some(object_id),
            control: lc_engine::PlayerControlState {
                cursor_flash: 30,
                ..Default::default()
            },
            ..PlayerState::default()
        });

        // 40-cell sheet, 4px cells: cell 35 magenta-ish, everything else green.
        let cell = 4u32;
        let mut cursor_pixels = Vec::new();
        for _y in 0..cell {
            for x in 0..40 * cell {
                if (35 * cell..36 * cell).contains(&x) {
                    cursor_pixels.extend_from_slice(&[123, 45, 210, 255]);
                } else {
                    cursor_pixels.extend_from_slice(&[0, 200, 0, 255]);
                }
            }
        }
        let cursor_pixels = Arc::from(cursor_pixels.into_boxed_slice());
        let cursor_image = ImageData::from_arc(40 * cell, cell, cursor_pixels);
        let mut cursor_entries = vec![None; 8];
        cursor_entries[5] = Some(cursor_image);
        let cursor_atlas = Arc::new(CursorAtlas::new(cursor_entries));

        let mut graphics = GraphicsSystem::new(
            320,
            180,
            150,
            "Cursor Scenario",
            test_font(),
            empty_sprites(),
            cursor_atlas,
            empty_hud_graphics(),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let cell_color = [123u8, 45, 210, 255];
        let other_cells = [0u8, 200, 0, 255];
        let mut found = false;
        let mut leaked = false;
        for chunk in graphics.surface().pixels().chunks_exact(4) {
            if chunk == cell_color {
                found = true;
            }
            if chunk == other_cells {
                leaked = true;
            }
        }
        assert!(found, "expected the fctCursor cell above the cursor crew");
        assert!(!leaked, "other sheet cells must not be drawn");
    }

    /// Cursor + flash + a 40-cell atlas sheet so the mark (cell 35) draws.
    fn cursor_label_fixture(info_name: Option<&str>) -> (SimulationSnapshot, GraphicsSystem) {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].owner = 1;
        snapshot.objects[0].position = Vector2::new(160, 90);
        let object_id = snapshot.objects[0].id;
        snapshot.players.push(PlayerState {
            id: 1,
            cursor: Some(object_id),
            control: lc_engine::PlayerControlState {
                cursor_flash: 30,
                ..Default::default()
            },
            ..PlayerState::default()
        });

        let cell = 4u32;
        let pixels: Vec<u8> = (0..40 * cell * cell)
            .flat_map(|_| [0u8, 200, 0, 255])
            .collect();
        let cursor_image = ImageData::new(40 * cell, cell, pixels);
        let mut cursor_entries = vec![None; 8];
        cursor_entries[5] = Some(cursor_image);
        let cursor_atlas = Arc::new(CursorAtlas::new(cursor_entries));

        let mut graphics = GraphicsSystem::new(
            320,
            180,
            150,
            "Cursor Label Scenario",
            test_font(),
            empty_sprites(),
            cursor_atlas,
            empty_hud_graphics(),
        );
        let players = vec![PlayerOverlay {
            owner: 1,
            name: "P1".to_string(),
            wealth: 0,
            score: 0,
            cursor: Some(object_id),
            eliminated: false,
            owner_color: Color::opaque(0, 100, 200),
            select_count: 1,
            show_startup: false,
            show_control: 0,
            show_control_position: 0,
            last_com: 0,
            control_key_labels: Vec::new(),
            crew: vec![CrewOverlay {
                object_id,
                label: "Joe".to_string(),
                energy_fraction: 1.0,
                magic_energy: 0,
                magic_capacity: 0,
                breath: 0,
                breath_capacity: 0,
                is_focus: true,
                portrait: None,
                rank: 0,
                rank_symbols: None,
                info_name: info_name.map(str::to_string),
                rank_name: None,
                inventory: Vec::new(),
            }],
            commands: Vec::new(),
        }];
        graphics.update_overlay(&GraphicsOverlay {
            frame_text: "",
            status_text: "",
            debug_hud: false,
            players,
            game_time_seconds: 0,
            message_board_line: None,
            show_commands: true,
            show_command_keys: true,
        });
        (snapshot, graphics)
    }

    fn count_red_text_pixels(graphics: &GraphicsSystem) -> usize {
        let red = standard_gamma_color(Color::opaque(255, 0, 0));
        graphics
            .surface()
            .pixels()
            .chunks_exact(4)
            .filter(|chunk| *chunk == [red.r, red.g, red.b, red.a])
            .count()
    }

    #[test]
    fn cursor_name_label_drawn_in_red_above_cursor_mark() {
        // C4Game::DrawCursors (src/C4Game.cpp:1873-1887): with cursor->Info,
        // the crew name is drawn in FontRegular, color 0xffff0000, centered
        // above the flashing cursor mark.
        let (snapshot, mut graphics) = cursor_label_fixture(Some("Joe"));
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);
        assert!(
            count_red_text_pixels(&graphics) > 0,
            "expected red 0xffff0000 name text above the cursor mark"
        );
    }

    #[test]
    fn cursor_name_label_needs_object_info() {
        // `if (cursor->Info)` (src/C4Game.cpp:1873): no info, no label.
        let (snapshot, mut graphics) = cursor_label_fixture(None);
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);
        assert_eq!(
            count_red_text_pixels(&graphics),
            0,
            "objects without info draw no cursor label"
        );
    }

    #[test]
    fn cursor_label_rank_line_stacks_above_the_name() {
        // `Rank > 0` doubles texthgt and prefixes the sRankName line
        // (src/C4Game.cpp:1877-1881), so the label block starts one line
        // higher than the rank-0 name-only label.
        let min_red_y = |graphics: &GraphicsSystem| {
            let red = standard_gamma_color(Color::opaque(255, 0, 0));
            graphics
                .surface()
                .pixels()
                .chunks_exact(4)
                .enumerate()
                .filter(|(_, chunk)| *chunk == [red.r, red.g, red.b, red.a])
                .map(|(index, _)| index / graphics.surface().width() as usize)
                .min()
        };

        let (snapshot, mut graphics) = cursor_label_fixture(Some("Joe"));
        let viewports = vec![ViewportInput::from_focus(&snapshot.objects[0])];
        graphics.render_frame(&snapshot, &viewports);
        let name_only_top = min_red_y(&graphics).expect("name label drawn");

        let (snapshot, mut graphics) = cursor_label_fixture(Some("Joe"));
        let mut players = graphics.hud_players.clone();
        players[0].crew[0].rank = 3;
        players[0].crew[0].rank_name = Some("Captain".to_string());
        graphics.update_overlay(&GraphicsOverlay {
            frame_text: "",
            status_text: "",
            debug_hud: false,
            players,
            game_time_seconds: 0,
            message_board_line: None,
            show_commands: true,
            show_command_keys: true,
        });
        let viewports = vec![ViewportInput::from_focus(&snapshot.objects[0])];
        graphics.render_frame(&snapshot, &viewports);
        let ranked_top = min_red_y(&graphics).expect("rank|name label drawn");

        assert!(
            ranked_top < name_only_top,
            "rank line must raise the label block (ranked_top={ranked_top}, name_only_top={name_only_top})"
        );
    }

    #[test]
    fn focused_crew_draws_partial_breath_in_the_next_cpp_bar_slot() {
        // C4Viewport::DrawCursorInfo places Breath after Energy and the
        // optional MagicEnergy bar; C4Object::DrawBreath selects bar_idx=2,
        // i.e. EnergyBars columns 4/5 (src/C4Viewport.cpp:920-943;
        // src/C4Object.cpp:2728-2731; src/C4Facet.cpp:334-387).
        let (mut snapshot, mut graphics) = cursor_label_fixture(Some("Joe"));
        snapshot.objects[0].breath = 50;
        snapshot.objects[0].info_physical = Some(lc_engine::PhysicalInfo {
            breath: 100,
            ..lc_engine::PhysicalInfo::default()
        });
        graphics.hud_players[0].crew[0].breath = 50;
        graphics.hud_players[0].crew[0].breath_capacity = 100;

        // Sentinel 6x3 EnergyBars sheet: every source column has a distinct
        // opaque color, repeated for top/middle/bottom cells.
        let columns = [
            [220, 0, 0, 255],
            [70, 0, 0, 255],
            [0, 220, 0, 255],
            [0, 70, 0, 255],
            [0, 0, 220, 255],
            [0, 0, 70, 255],
        ];
        let pixels = (0..3)
            .flat_map(|_| columns.into_iter().flatten())
            .collect();
        graphics.hud_graphics = Arc::new(HudGraphics {
            energy_bars: Some(ImageData::new(6, 3, pixels)),
            ..HudGraphics::default()
        });

        let focus = &snapshot.objects[0];
        graphics.render_frame(&snapshot, &[ViewportInput::from_focus(focus)]);

        let bar_bottom_y = 180 - hud::SYMBOL_SIZE - hud::SYMBOL_BORDER - 1;
        let energy_x = hud::SYMBOL_BORDER as u32;
        let breath_x = energy_x + 2; // one-pixel bar + C++'s one-pixel gap
        assert_eq!(
            graphics.surface().get_pixel(energy_x, bar_bottom_y as u32),
            Some(standard_gamma_color(Color::opaque(220, 0, 0))),
            "energy remains in bar index 0"
        );
        assert_eq!(
            graphics.surface().get_pixel(breath_x, bar_bottom_y as u32),
            Some(standard_gamma_color(Color::opaque(0, 0, 220))),
            "partial breath uses filled source column 4 immediately after energy"
        );

        graphics.hud_players[0].crew[0].magic_energy = 1_000;
        graphics.hud_players[0].crew[0].magic_capacity = 2_000;
        graphics.render_frame(&snapshot, &[ViewportInput::from_focus(focus)]);
        assert_eq!(
            graphics.surface().get_pixel(breath_x, bar_bottom_y as u32),
            Some(standard_gamma_color(Color::opaque(0, 220, 0))),
            "present magic occupies the middle slot with source column 2"
        );
        assert_eq!(
            graphics
                .surface()
                .get_pixel(breath_x + 2, bar_bottom_y as u32),
            Some(standard_gamma_color(Color::opaque(0, 0, 220))),
            "breath shifts one compact slot right when magic is present"
        );
    }

    #[test]
    fn no_floating_energy_bars_or_bolt_over_crew() {
        // C4Object::Draw (src/C4Object.cpp:2151-2556) draws NO energy or
        // magic bars attached to the object — energy lives in the HUD
        // corner (C4Viewport::DrawCursorInfo, src/C4Viewport.cpp:920-945).
        // The fctEnergy bolt appears world-space only for NeedEnergy
        // structures, blinking on `Tick35 > 12` (src/C4Object.cpp:2505-2510)
        // — never as a persistent crew marker.
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].owner = 1;
        snapshot.objects[0].energy = 70;
        snapshot.objects[0].magic_energy = 30;
        snapshot.objects[0].magic_capacity = 50;
        snapshot.landscape = Some(Landscape::flat(128, 80));
        snapshot.players.push(PlayerState {
            id: 1,
            cursor: Some(snapshot.objects[0].id),
            ..PlayerState::default()
        });

        let bolt = [230u8, 20, 20, 255];
        let bolt_pixels: Vec<u8> = (0..8 * 8).flat_map(|_| bolt).collect();
        let hud = HudGraphics {
            energy: Some(ImageData::new(8, 8, bolt_pixels.clone())),
            magic: Some(ImageData::new(8, 8, bolt_pixels)),
            ..Default::default()
        };

        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "Energy Scenario",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            Arc::new(hud),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let bar_background = Color::new(16, 24, 40, 210);
        for chunk in graphics.surface().pixels().chunks_exact(4) {
            assert_ne!(chunk, bolt, "no floating Energy/Magic bolt icons");
            assert_ne!(
                chunk,
                [
                    bar_background.r,
                    bar_background.g,
                    bar_background.b,
                    bar_background.a
                ],
                "no floating bar backgrounds"
            );
        }
    }

    #[test]
    fn select_marks_draw_four_corner_phases_while_select_flash_runs() {
        // C4Object::DrawSelectMark (src/C4Object.cpp:3839-3857): the four
        // PHASES of fctSelectMark (SelectMark.png, 4 square cells of sheet
        // height) sit at the shape corners offset by -2 — never the whole
        // sheet blitted over the object. Gated on the owner's SelectFlash
        // (src/C4Object.cpp:2497-2502).
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].owner = 1;
        snapshot.landscape = Some(Landscape::flat(128, 80));
        snapshot.players.push(PlayerState {
            id: 1,
            cursor: Some(snapshot.objects[0].id),
            control: lc_engine::PlayerControlState {
                select_flash: 30,
                ..Default::default()
            },
            ..PlayerState::default()
        });

        let corner_colors = [
            [200u8, 10, 10, 255],
            [10, 200, 10, 255],
            [10, 10, 200, 255],
            [200, 200, 10, 255],
        ];
        let mut pixels = Vec::new();
        for _y in 0..5 {
            for x in 0..20 {
                pixels.extend_from_slice(&corner_colors[(x / 5) as usize]);
            }
        }
        let hud = HudGraphics {
            select_mark: Some(ImageData::new(20, 5, pixels)),
            ..Default::default()
        };

        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "SelectMark Scenario",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            Arc::new(hud),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (viewport_x, viewport_y) = graphics.viewport();
        let sx = snapshot.objects[0].position.x - viewport_x;
        let sy = snapshot.objects[0].position.y - viewport_y;
        // Fallback shape (-6,-6,12,12): cox = sx - 6 - 2, corners 12 apart.
        let expected = [
            (sx - 6, sy - 6, corner_colors[0]),
            (sx + 6, sy - 6, corner_colors[1]),
            (sx - 6, sy + 6, corner_colors[2]),
            (sx + 6, sy + 6, corner_colors[3]),
        ];
        for (px, py, color) in expected {
            assert_eq!(
                graphics.surface().get_pixel(px as u32, py as u32),
                Some(Color::new(color[0], color[1], color[2], color[3])),
                "corner phase at ({px}, {py})"
            );
        }
        // The whole-sheet regression put cell colors at the object center.
        let center = graphics.surface().get_pixel(sx as u32, sy as u32);
        assert!(
            corner_colors
                .iter()
                .all(|c| center != Some(Color::new(c[0], c[1], c[2], c[3]))),
            "no sheet cells across the object center"
        );
    }

    #[test]
    fn select_marks_stay_hidden_without_select_flash() {
        // `Game.Players.Get(Owner)->SelectFlash` gate (src/C4Object.cpp:2501).
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(40, 40);
        snapshot.objects[0].owner = 1;
        snapshot.landscape = Some(Landscape::flat(128, 80));
        snapshot.players.push(PlayerState {
            id: 1,
            cursor: Some(snapshot.objects[0].id),
            ..PlayerState::default()
        });

        let mark = [200u8, 10, 10, 255];
        let pixels: Vec<u8> = (0..20 * 5).flat_map(|_| mark).collect();
        let hud = HudGraphics {
            select_mark: Some(ImageData::new(20, 5, pixels)),
            ..Default::default()
        };

        let mut graphics = GraphicsSystem::new(
            80,
            60,
            60,
            "SelectMark Scenario",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            Arc::new(hud),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        assert!(
            graphics
                .surface()
                .pixels()
                .chunks_exact(4)
                .all(|chunk| chunk != mark),
            "no flash → no select marks"
        );
    }

    #[test]
    fn player_cursor_mark_stays_hidden_without_flash() {
        // The `pPlr->CursorFlash || pPlr->SelectFlash` gate
        // (src/C4Game.cpp:1863): expired flash timers draw no mark.
        let mut snapshot = make_snapshot();
        snapshot.objects[0].owner = 1;
        let object_id = snapshot.objects[0].id;
        snapshot.players.push(PlayerState {
            id: 1,
            cursor: Some(object_id),
            ..PlayerState::default()
        });

        let cell = 4u32;
        let pixels: Vec<u8> = (0..40 * cell * cell)
            .flat_map(|_| [123, 45, 210, 255])
            .collect();
        let cursor_image = ImageData::from_arc(40 * cell, cell, Arc::from(pixels.into_boxed_slice()));
        let mut cursor_entries = vec![None; 8];
        cursor_entries[5] = Some(cursor_image);
        let cursor_atlas = Arc::new(CursorAtlas::new(cursor_entries));

        let mut graphics = GraphicsSystem::new(
            320,
            180,
            150,
            "Cursor Scenario",
            test_font(),
            empty_sprites(),
            cursor_atlas,
            empty_hud_graphics(),
        );
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let cell_color = [123u8, 45, 210, 255];
        assert!(
            graphics
                .surface()
                .pixels()
                .chunks_exact(4)
                .all(|chunk| chunk != cell_color),
            "no flash → no cursor mark"
        );
    }

    #[test]
    fn sky_fade_color_matches_c4sky_get_sky_fade_clr() {
        // C4Sky::GetSkyFadeClr (C4Sky.cpp:230-236): integer fade between
        // FadeClr1 (world top) and FadeClr2 across the landscape height:
        // iPos2 = iY*256/GBackHgt, channel = (c1*iPos1 + c2*iPos2) >> 8.
        let settings = SkySettings {
            fade_top: RgbColor::new(28, 64, 152),
            fade_bottom: RgbColor::new(192, 196, 252),
            ..Default::default()
        };

        assert_eq!(
            GraphicsSystem::sky_fade_color(&settings, 0, 400),
            RgbColor::new(28, 64, 152),
            "world top shows FadeClr1"
        );
        assert_eq!(
            GraphicsSystem::sky_fade_color(&settings, 400, 400),
            RgbColor::new(192, 196, 252),
            "world bottom shows FadeClr2"
        );
        // iY=100, GBackHgt=400: iPos2 = 64, iPos1 = 192;
        // r = (28*192 + 192*64) >> 8 = 69, g = (64*192 + 196*64) >> 8 = 97,
        // b = (152*192 + 252*64) >> 8 = 177.
        assert_eq!(
            GraphicsSystem::sky_fade_color(&settings, 100, 400),
            RgbColor::new(69, 97, 177),
        );
    }

    #[test]
    fn sky_gradient_shows_fade_top_at_the_top_of_the_view() {
        // C4Sky::Draw without a surface fades FadeClr1 -> FadeClr2 top to
        // bottom (C4Sky.cpp:219-225 via GetSkyFadeClr, C4Sky.cpp:230-236).
        let mut snapshot = make_snapshot();
        snapshot.environment.settings.time_of_day = 0; // full daylight
        snapshot.landscape = None;
        snapshot.objects[0].position = Vector2::new(60, 40);

        let settings = lc_engine::SkySettings {
            fade_top: RgbColor::new(200, 16, 16),
            fade_bottom: RgbColor::new(16, 16, 200),
            ..Default::default()
        };

        let focus = &snapshot.objects[0];
        let mut graphics = GraphicsSystem::new(
            120,
            80,
            60,
            "Sky Fade",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.set_sky(Some(SkyRenderState::new(settings, None)));
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let top = graphics.surface().get_pixel(0, 0).unwrap();
        assert!(
            top.r > top.b,
            "expected the red fade_top at the top of the view, got {top:?}"
        );
    }

    #[test]
    fn sky_gradient_is_not_pre_tinted_by_the_season_curve() {
        // C4Sky::Draw emits GetSkyFadeClr directly (C4Sky.cpp:219-236).
        // C4Weather's season curve is one global gamma control applied to
        // the completed frame (C4GraphicsSystem.cpp:787-809), so tinting
        // only the sky here would apply it once before the global LUT.
        let mut snapshot = make_snapshot();
        snapshot.environment.settings = EnvironmentSettings::new(0)
            .with_season(0)
            .with_temperature(-20)
            .with_gamma_enabled();
        snapshot.landscape = None;
        let fade = RgbColor::new(100, 120, 140);
        let settings = SkySettings {
            fade_top: fade,
            fade_bottom: fade,
            ..Default::default()
        };

        let focus = &snapshot.objects[0];
        let mut graphics = GraphicsSystem::new(
            120,
            80,
            60,
            "Unmodified Sky Fade",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.set_sky(Some(SkyRenderState::new(settings, None)));
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(Color::opaque(fade.r, fade.g, fade.b))
        );
    }

    #[test]
    fn lighting_darkens_sky_at_night() {
        let mut daytime = make_snapshot();
        daytime.environment.sky_color = Some(RgbColor::new(160, 160, 160));
        daytime.environment.settings.time_of_day = EnvironmentSettings::TIME_CYCLE / 2;

        let focus = &daytime.objects[0];
        let mut day_view = GraphicsSystem::new(
            120,
            80,
            60,
            "Day",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let day_viewports = vec![ViewportInput::from_focus(focus)];
        day_view.render_frame(&daytime, &day_viewports);
        let day_pixel = day_view.surface().get_pixel(0, 0).unwrap();

        let mut nighttime = daytime.clone();
        // 0 means "no day/night cycle" (full daylight); 1 is deepest night.
        nighttime.environment.settings.time_of_day = 1;
        let mut night_view = GraphicsSystem::new(
            120,
            80,
            60,
            "Night",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let night_focus = &nighttime.objects[0];
        let night_viewports = vec![ViewportInput::from_focus(night_focus)];
        night_view.render_frame(&nighttime, &night_viewports);
        let night_pixel = night_view.surface().get_pixel(0, 0).unwrap();

        let base_color = Color::opaque(160, 160, 160);
        let day_factor = GraphicsSystem::lighting_factor(daytime.environment.settings.time_of_day);
        let night_factor =
            GraphicsSystem::lighting_factor(nighttime.environment.settings.time_of_day);
        let expected_day = GraphicsSystem::apply_lighting(base_color, day_factor);
        let expected_night = GraphicsSystem::apply_lighting(base_color, night_factor);

        assert_eq!(day_pixel, expected_day);
        assert_eq!(night_pixel, expected_night);
        assert_ne!(expected_day, expected_night);
    }

    #[test]
    fn lighting_darkens_objects_at_night() {
        let mut daytime = make_snapshot();
        daytime.environment.settings.time_of_day = EnvironmentSettings::TIME_CYCLE / 2;
        daytime.objects[0].position = Vector2::new(150, 140);
        // Keep the probe inside GBackHgt: C4Viewport clips landscape drawing
        // at the borders (C4Viewport.cpp:1035-1041).
        daytime.landscape = Some(Landscape::flat(256, 150));

        let mut day_view = GraphicsSystem::new(
            200,
            150,
            150,
            "Day Object",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let day_focus = &daytime.objects[0];
        let day_viewports = vec![ViewportInput::from_focus(day_focus)];
        day_view.render_frame(&daytime, &day_viewports);
        let (day_viewport_x, day_viewport_y) = day_view.viewport();
        let day_screen_x = (daytime.objects[0].position.x - day_viewport_x) as u32;
        let day_screen_y = (daytime.objects[0].position.y - day_viewport_y) as u32;
        let day_pixel = day_view
            .surface()
            .get_pixel(day_screen_x, day_screen_y)
            .unwrap();

        let mut nighttime = daytime.clone();
        // 0 means "no day/night cycle" (full daylight); 1 is deepest night.
        nighttime.environment.settings.time_of_day = 1;
        let mut night_view = GraphicsSystem::new(
            200,
            150,
            150,
            "Night Object",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let night_focus = &nighttime.objects[0];
        let night_viewports = vec![ViewportInput::from_focus(night_focus)];
        night_view.render_frame(&nighttime, &night_viewports);
        let (night_viewport_x, night_viewport_y) = night_view.viewport();
        let night_screen_x = (nighttime.objects[0].position.x - night_viewport_x) as u32;
        let night_screen_y = (nighttime.objects[0].position.y - night_viewport_y) as u32;
        let night_pixel = night_view
            .surface()
            .get_pixel(night_screen_x, night_screen_y)
            .unwrap();

        let day_factor = GraphicsSystem::lighting_factor(daytime.environment.settings.time_of_day);
        let night_factor =
            GraphicsSystem::lighting_factor(nighttime.environment.settings.time_of_day);
        assert!(night_factor < day_factor);
        let ratio = if day_factor <= 0.0 {
            0.0
        } else {
            night_factor / day_factor
        };
        let expected_night = GraphicsSystem::apply_lighting(day_pixel, ratio);

        assert_eq!(night_pixel, expected_night);
        assert_ne!(day_pixel, night_pixel);
    }

    #[test]
    fn narrow_world_produces_letterbox_content_rect() {
        let mut snapshot = make_snapshot();
        snapshot.landscape = Some(Landscape::flat(40, 40));
        snapshot.objects[0].position = Vector2::new(20, 20);

        let focus = &snapshot.objects[0];
        let mut graphics = GraphicsSystem::new(
            120,
            80,
            40,
            "Letterbox",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let viewports = vec![ViewportInput::new(0, Vector2::new(20, 20), 1.0, focus)];
        graphics.render_frame(&snapshot, &viewports);

        let viewport = graphics
            .active_viewports
            .first()
            .expect("expected active viewport");
        assert!(viewport.content_rect.width < viewport.rect.width);
        assert_eq!(viewport.content_rect.width, 40);

        let left_bar = viewport.content_rect.x - viewport.rect.x;
        let right_bar = (viewport.rect.x + viewport.rect.width as i32)
            - (viewport.content_rect.x + viewport.content_rect.width as i32);
        assert!(left_bar > 0);
        assert!(right_bar > 0);
    }

    #[test]
    fn liquids_overlay_ground_with_blending() {
        let mut snapshot = make_snapshot();
        snapshot.environment.settings.time_of_day = EnvironmentSettings::TIME_CYCLE / 2;
        snapshot.objects[0].position = Vector2::new(40, 50);
        if let Some(landscape) = snapshot.landscape.as_mut() {
            landscape.set_liquid_column(30, vec![LiquidSegment::new(40, 60)]);
        }
        let focus = &snapshot.objects[0];
        let mut graphics = GraphicsSystem::new(
            120,
            80,
            80,
            "Liquid Scenario",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);

        let (viewport_x, viewport_y) = graphics.viewport();
        let screen_x = (30 - viewport_x) as u32;
        let screen_y = (50 - viewport_y) as u32;

        let pixel = graphics
            .surface()
            .get_pixel(screen_x, screen_y)
            .expect("pixel in bounds");

        let lighting = GraphicsSystem::lighting_factor(snapshot.environment.settings.time_of_day);
        let liquid = GraphicsSystem::apply_lighting(
            GraphicsSystem::liquid_color_for_temperature(snapshot.environment.ambient_temperature),
            lighting,
        );
        let sky = GraphicsSystem::apply_lighting(
            snapshot
                .environment
                .sky_color
                .map(|color| Color::opaque(color.r, color.g, color.b))
                .unwrap_or_else(|| {
                    GraphicsSystem::sky_color_for_temperature(
                        snapshot.environment.ambient_temperature,
                    )
                }),
            lighting,
        );
        let expected = blend_color_over(liquid, sky);
        assert_eq!(pixel, expected);
    }

    #[test]
    fn textured_acid_liquid_keeps_its_material_color() {
        // C++ bakes the material's Color and both texture patterns into
        // Surface32 (C4Landscape.cpp:2619-2633). Its liquid pass supplies an
        // alpha-only animation mask (:2599-2616) to BlitLandscape (:261-270);
        // it never replaces Acid's RGB with a generic water color.
        let mut landscape: Landscape = serde_json::from_value(serde_json::json!({
            "width": 1,
            "surface": [1],
            "world_height": 1,
            "pixels": {
                "width": 1,
                "height": 1,
                "bytes": "01",
                "texture_names": [null, "Smooth"],
                "densities": [0, 25],
                "material_names": [null, "Acid"]
            }
        }))
        .expect("pixel landscape");
        landscape.set_liquid_column(
            0,
            vec![LiquidSegment::with_material(
                0,
                0,
                MaterialId::new(1),
            )],
        );

        // Neutral 128-valued patterns preserve Acid's (0,190,0) RGB under
        // CPattern's ModulateClrA + LightenClr composition.
        let textures = HashMap::from([(
            "liquid".to_string(),
            ImageData::new(1, 1, vec![128, 128, 128, 255]),
        )]);
        let materials = HashMap::from([(
            "acid".to_string(),
            MaterialRenderInfo::new(
                [0, 190, 0, 0, 200, 0, 0, 210, 0],
                [0; 6],
                Some("Liquid".to_string()),
                0,
                25,
            ),
        )]);

        let mut snapshot = make_snapshot();
        snapshot.landscape = Some(landscape);
        let mut graphics = GraphicsSystem::new(
            1,
            1,
            1,
            "Acid color",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.set_material_textures(Arc::new(textures));
        graphics.set_material_render_info(Arc::new(materials));

        let viewports = vec![ViewportInput::new(
            0,
            Vector2::ZERO,
            1.0,
            &snapshot.objects[0],
        )];
        graphics.render_frame(&snapshot, &viewports);

        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(standard_gamma_color(Color::opaque(0, 190, 0))),
            "the liquid animation path must preserve Acid's material RGB"
        );
    }

    #[test]
    fn material_patterns_modulate_texmap_then_overlay_at_overlay_zoom() {
        // C4Landscape::GetClrByTex applies the texmap pattern and then the
        // material pattern (C4Landscape.cpp:2619-2633). CPattern samples the
        // material overlay at zoom two and performs ModulateClrA + LightenClr
        // (C4Material.cpp:374-377; StdDDraw2.cpp:187-207).
        let material = MaterialRenderInfo::new(
            [64, 96, 128, 0, 0, 0, 0, 0, 0],
            [10, 0, 0, 0, 0, 0],
            Some("Smooth".to_string()),
            0,
            50,
        );
        let rough = ImageData::new(
            4,
            1,
            vec![
                1, 1, 1, 255, 2, 2, 2, 255, 128, 64, 255, 235, 3, 3, 3, 255,
            ],
        );
        let smooth = ImageData::new(2, 1, vec![4, 4, 4, 255, 64, 128, 32, 245]);

        assert_eq!(
            compose_material_pixel(&material, 1, 2, 0, &rough, Some(&smooth)),
            Color::new(32, 48, 62, 215),
        );
    }

    #[test]
    fn material_ift_bit_selects_the_background_alpha_triplet() {
        // Mat2Pal selects Alpha[0] for a foreground texmap byte and Alpha[3]
        // for the same byte plus IFT (C4Landscape.cpp:2828-2845).
        let material = MaterialRenderInfo::new(
            [100, 110, 120, 0, 0, 0, 0, 0, 0],
            [10, 0, 0, 70, 0, 0],
            None,
            0,
            50,
        );
        let texture = ImageData::new(1, 1, vec![255, 255, 255, 255]);

        let foreground = compose_material_pixel(&material, 1, 0, 0, &texture, None);
        let background = compose_material_pixel(&material, 1 | 0x80, 0, 0, &texture, None);

        assert_eq!(foreground.a, 245);
        assert_eq!(background.a, 185);
        assert_eq!(foreground.r, background.r);
        assert_eq!(foreground.g, background.g);
        assert_eq!(foreground.b, background.b);
    }

    #[test]
    fn material_overlay_flags_control_primary_and_overlay_sampling() {
        // C4TexMapEntry::Init uses HugeZoom=4 for the primary pattern;
        // C4Material::CrossMapMaterials forces the secondary overlay to zoom
        // two unless Exact selects one (C4Texture.cpp:91-102;
        // C4Material.cpp:374-377).
        let primary = ImageData::new(
            4,
            1,
            vec![
                32, 32, 32, 255, 16, 16, 16, 255, 8, 8, 8, 255, 64, 64, 64, 255,
            ],
        );
        let white = ImageData::new(1, 1, vec![255, 255, 255, 255]);
        let overlay = ImageData::new(
            4,
            1,
            vec![
                8, 8, 8, 255, 64, 64, 64, 255, 32, 32, 32, 255, 16, 16, 16, 255,
            ],
        );
        let default = MaterialRenderInfo::new([128; 9], [0; 6], None, 0, 50);
        let huge = MaterialRenderInfo::new(
            [128; 9],
            [0; 6],
            None,
            MATERIAL_OVERLAY_HUGE_ZOOM,
            50,
        );
        let exact = MaterialRenderInfo::new(
            [128; 9],
            [0; 6],
            None,
            MATERIAL_OVERLAY_EXACT,
            50,
        );

        assert_eq!(
            compose_material_pixel(&default, 1, 3, 0, &primary, None).r,
            64,
        );
        assert_eq!(
            compose_material_pixel(&huge, 1, 3, 0, &primary, None).r,
            32,
        );
        assert_eq!(
            compose_material_pixel(&default, 1, 2, 0, &white, Some(&overlay)).r,
            126,
        );
        assert_eq!(
            compose_material_pixel(&exact, 1, 2, 0, &white, Some(&overlay)).r,
            62,
        );
    }

    #[test]
    fn monochrome_material_patterns_use_the_blue_texture_channel() {
        // CPattern::PatternClr passes the low byte of the BGRA dword to
        // ModulateClrMonoA, i.e. the source texture's blue channel
        // (StdDDraw2.cpp:195-205; StdPNGLibpng.cpp:200-223).
        let mut pixel = MaterialPixel {
            red: 64,
            green: 96,
            blue: 128,
            transparency: 0,
        };
        let texture = ImageData::new(1, 1, vec![10, 20, 200, 255]);

        apply_material_pattern(&mut pixel, &texture, 0, 0, 0, true);

        assert_eq!([pixel.red, pixel.green, pixel.blue], [100, 150, 200]);
    }

    #[test]
    fn textured_material_alpha_blends_over_the_rendered_sky() {
        // C4Landscape::GetClrByTex stores the material transparency in
        // Surface32 (C4Landscape.cpp:2619-2633), and BlitLandscape composites
        // that surface over the already-rendered viewport (StdGL.cpp:578-580,
        // 640-664). The Rust cache must not force the material opaque.
        let landscape: Landscape = serde_json::from_value(serde_json::json!({
            "width": 1,
            "surface": [1],
            "world_height": 1,
            "pixels": {
                "width": 1,
                "height": 1,
                "bytes": "01",
                "texture_names": [null, "Rough"],
                "densities": [0, 50],
                "material_names": [null, "Earth"]
            }
        }))
        .expect("pixel landscape");
        let textures = HashMap::from([
            (
                "rough".to_string(),
                ImageData::new(1, 1, vec![255, 255, 255, 255]),
            ),
            (
                "smooth".to_string(),
                ImageData::new(1, 1, vec![255, 255, 255, 255]),
            ),
        ]);
        let materials = HashMap::from([(
            "earth".to_string(),
            MaterialRenderInfo::new(
                [64, 0, 0, 0, 0, 0, 0, 0, 0],
                [127, 0, 0, 0, 0, 0],
                Some("Smooth".to_string()),
                0,
                50,
            ),
        )]);
        let mut graphics = GraphicsSystem::new(
            1,
            1,
            1,
            "Material alpha",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics
            .surface_mut()
            .set_pixel(0, 0, Color::opaque(10, 20, 30))
            .expect("sky pixel");
        graphics.set_material_textures(Arc::new(textures));
        graphics.set_material_render_info(Arc::new(materials));

        assert!(graphics.draw_ground_textured(Some(&landscape), None));
        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(Color::opaque(130, 9, 14)),
        );
    }

    #[test]
    fn textured_landscape_gamma_samples_r16_before_alpha_blending() {
        // BlitLandscape applies the per-channel R16 gamma lookup to its source
        // fragment before fixed-function alpha blending (StdGL.cpp:578-618,
        // 1139-1148,1246-1263).
        let landscape: Landscape = serde_json::from_value(serde_json::json!({
            "width": 1,
            "surface": [1],
            "world_height": 1,
            "pixels": {
                "width": 1,
                "height": 1,
                "bytes": "01",
                "texture_names": [null, "Rough"],
                "densities": [0, 50],
                "material_names": [null, "Earth"]
            }
        }))
        .expect("pixel landscape");
        let revision = landscape
            .pixel_grid()
            .expect("pixel grid")
            .revision();
        let mut graphics = GraphicsSystem::new(
            1,
            1,
            1,
            "Gamma Material",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );
        graphics.set_material_textures(Arc::new(HashMap::from([(
            "rough".to_string(),
            ImageData::new(1, 1, vec![255, 255, 255, 255]),
        )])));
        graphics.set_material_render_info(Arc::new(HashMap::from([(
            "earth".to_string(),
            MaterialRenderInfo::new([255; 9], [0; 6], None, 0, 50),
        )])));
        // Presentation is under test, not cache construction. Keeping the raw
        // cached source unencoded also pins that later gamma changes do not
        // require rebuilding the landscape cache.
        graphics.landscape_cache = Some((
            revision,
            ImageData::new(1, 1, vec![64, 128, 192, 128]),
        ));
        graphics
            .surface_mut()
            .set_pixel(0, 0, Color::opaque(200, 200, 200))
            .expect("sky pixel");
        let gamma = lc_graphics::GammaRamp::from_control_points([
            0x000000, 0x646464, 0xc8c8c8,
        ]);

        assert!(graphics.draw_ground_textured(Some(&landscape), Some(&gamma)));

        assert_eq!(
            graphics.surface().get_pixel(0, 0),
            Some(Color::new(125, 150, 175, 255))
        );
    }
}
