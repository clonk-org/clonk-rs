#![allow(dead_code)]
#![allow(
    clippy::manual_clamp,
    clippy::op_ref,
    clippy::question_mark,
    clippy::too_many_arguments
)]

pub mod clonk_fonts;
pub mod hud;
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
    DefinitionActionGraphics, DefinitionRect, Direction, DrawTransform, EnvironmentFrame,
    EnvironmentSettings, FloatVector2, GraphicsOverlayMode, Landscape, ObjectGraphicsOverlay,
    ObjectId, ObjectSnapshot, ObjectStatus, RgbColor, SimulationSnapshot, SkyFrame, SkySettings,
    SurfaceSnapshot as EngineSurfaceSnapshot, Vector2, WeatherEvent, CATEGORY_SORT_LIMIT, FULL_CON,
    OWNER_NONE,
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
pub use startup_main_menu::{MainMenuAction, MainMenuItem, StartupMainMenu};
pub use startup_menu::{ScenarioSummary, StartupMenu, StartupMenuAction};
pub use startup_options::{ControlOptionItem, ControlOptionsAction, ControlOptionsView};

const MIN_VIEWPORT_ZOOM: f32 = 0.125;
const MAX_VIEWPORT_ZOOM: f32 = 4.0;
const CAMERA_SMOOTHING_ALPHA: f32 = 0.2;
const CAMERA_SNAP_THRESHOLD: f32 = 1.0;
const CAMERA_JUMP_THRESHOLD: f32 = 256.0;
const PICK_TOLERANCE: f32 = 6.0;
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

fn blend_color_by_owner(base: Color, mask_value: u8, owner_color: Color) -> Color {
    let mask = mask_value as u16;
    if mask == 0 {
        return base;
    }
    let inv_mask = 255u16.saturating_sub(mask);
    let mix_channel = |base_channel: u8, owner_channel: u8| -> u8 {
        let tinted = (owner_channel as u16 * mask) / 255;
        let base_contrib = (base_channel as u16 * inv_mask) / 255;
        (tinted + base_contrib).min(255) as u8
    };

    Color::new(
        mix_channel(base.r, owner_color.r),
        mix_channel(base.g, owner_color.g),
        mix_channel(base.b, owner_color.b),
        base.a,
    )
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
    focus: ObjectId,
}

#[derive(Debug, Clone)]
struct CameraState {
    x: f32,
    y: f32,
    zoom: f32,
    initialized: bool,
}

impl CameraState {
    fn new(x: f32, y: f32, zoom: f32) -> Self {
        Self {
            x,
            y,
            zoom,
            initialized: false,
        }
    }

    fn update(
        &mut self,
        target_x: f32,
        target_y: f32,
        zoom: f32,
        min_x: f32,
        max_x: f32,
        min_y: f32,
        max_y: f32,
    ) -> (f32, f32) {
        if !self.initialized || (self.zoom - zoom).abs() > 0.01 {
            self.x = target_x;
            self.y = target_y;
            self.initialized = true;
        } else {
            self.x = smooth_value(self.x, target_x);
            self.y = smooth_value(self.y, target_y);
        }

        self.zoom = zoom;
        self.x = self.x.clamp(min_x, max_x);
        self.y = self.y.clamp(min_y, max_y);
        (self.x, self.y)
    }
}

fn smooth_value(current: f32, target: f32) -> f32 {
    let delta = target - current;
    if delta.abs() <= CAMERA_SNAP_THRESHOLD || delta.abs() >= CAMERA_JUMP_THRESHOLD {
        target
    } else {
        current + delta * CAMERA_SMOOTHING_ALPHA
    }
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
}

#[derive(Debug)]
pub struct ViewportInput<'a> {
    pub owner: i32,
    pub center: Vector2,
    pub zoom: f32,
    pub focus: &'a ObjectSnapshot,
}

impl<'a> ViewportInput<'a> {
    pub fn new(owner: i32, center: Vector2, zoom: f32, focus: &'a ObjectSnapshot) -> Self {
        Self {
            owner,
            center,
            zoom,
            focus,
        }
    }

    pub fn from_focus(focus: &'a ObjectSnapshot) -> Self {
        Self {
            owner: focus.owner,
            center: Vector2::new(focus.position.x, focus.position.y),
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
    sky: Option<SkyRenderState>,
    /// Material texture pngs by lowercase texture name — the landscape
    /// plane samples them per pixel (C++ builds Surface32 from the same
    /// Material.c4g textures during MapToSurface).
    material_textures: Arc<HashMap<String, ImageData>>,
    /// Material base colors (first Color= triplet): the landscape pixel
    /// is base x texture, doubled — CPattern::PatternClr's ModulateClrA +
    /// LightenClr (StdDDraw2.cpp:187-207).
    material_colors: Arc<HashMap<String, [u8; 3]>>,
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
            sky: None,
            material_textures: Arc::new(HashMap::new()),
            material_colors: Arc::new(HashMap::new()),
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

    pub fn set_material_colors(&mut self, colors: Arc<HashMap<String, [u8; 3]>>) {
        self.material_colors = colors;
        self.landscape_cache = None;
    }

    pub fn set_sky(&mut self, sky: Option<SkyRenderState>) {
        self.sky = sky;
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

    pub fn render_frame(
        &mut self,
        snapshot: &SimulationSnapshot,
        viewports: &[ViewportInput<'_>],
    ) -> Vec<EngineSurfaceSnapshot> {
        self.active_viewports.clear();
        self.surface.fill(Color::opaque(8, 12, 24)); // base fill before compositing viewports

        let owner_colors = Self::collect_owner_colors(snapshot);
        let mut used_camera_keys = Vec::new();
        self.render_viewports(snapshot, viewports, &owner_colors, &mut used_camera_keys);
        let used_keys: HashSet<_> = used_camera_keys.into_iter().collect();
        self.camera_states.retain(|key, _| used_keys.contains(key));

        self.draw_hud();

        self.collect_sprite_atlas(snapshot)
    }

    fn render_viewports(
        &mut self,
        snapshot: &SimulationSnapshot,
        viewports: &[ViewportInput<'_>],
        owner_colors: &HashMap<i32, Color>,
        used_camera_keys: &mut Vec<CameraKey>,
    ) {
        if viewports.is_empty() {
            if let Some(object) = snapshot.objects.first() {
                let default = ViewportInput::from_focus(object);
                self.render_viewport(
                    snapshot,
                    &default,
                    SurfaceRect::new(0, 0, self.surface_width, self.surface_height),
                    owner_colors,
                    used_camera_keys,
                );
            }
            return;
        }

        let layout = self.layout_viewports(viewports.len());
        for (input, rect) in viewports.iter().zip(layout.into_iter()) {
            self.render_viewport(snapshot, input, rect, owner_colors, used_camera_keys);
        }
    }

    fn render_viewport(
        &mut self,
        snapshot: &SimulationSnapshot,
        input: &ViewportInput<'_>,
        rect: SurfaceRect,
        owner_colors: &HashMap<i32, Color>,
        used_camera_keys: &mut Vec<CameraKey>,
    ) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }

        let format = self.surface.format();
        let mut viewport_surface = Surface::new(rect.width, rect.height, format);
        viewport_surface.fill(Color::opaque(0, 0, 0));

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

        let zoom = input.zoom.clamp(MIN_VIEWPORT_ZOOM, MAX_VIEWPORT_ZOOM);
        let world_width = if self.world_width > 0 {
            self.world_width as f32
        } else {
            rect.width.max(1) as f32 / zoom
        };
        let world_height = if self.world_height > 0 {
            self.world_height as f32
        } else {
            rect.height.max(1) as f32 / zoom
        };

        let mut visible_world_width = (rect.width as f32 / zoom).max(1.0);
        let mut visible_world_height = (rect.height as f32 / zoom).max(1.0);

        if visible_world_width > world_width && world_width > 0.0 {
            visible_world_width = world_width;
        }
        if visible_world_height > world_height && world_height > 0.0 {
            visible_world_height = world_height;
        }

        let content_width = (visible_world_width * zoom)
            .round()
            .clamp(1.0, rect.width as f32) as u32;
        let content_height = (visible_world_height * zoom)
            .round()
            .clamp(1.0, rect.height as f32) as u32;

        let offset_x = ((rect.width as i32 - content_width as i32) / 2).max(0);
        let offset_y = ((rect.height as i32 - content_height as i32) / 2).max(0);

        let target = input.center;
        let target_x = target.x as f32;
        let target_y = target.y as f32;

        let desired_origin_x = target_x - visible_world_width / 2.0;
        let desired_origin_y = target_y - visible_world_height / 2.0;

        let max_origin_x = (world_width - visible_world_width).max(0.0);
        let max_origin_y = (world_height - visible_world_height).max(0.0);

        let clamped_origin_x = desired_origin_x.clamp(0.0, max_origin_x);
        let clamped_origin_y = desired_origin_y.clamp(0.0, max_origin_y);

        let key = CameraKey {
            owner: input.owner,
            focus: input.focus.id,
        };

        let state = self
            .camera_states
            .entry(key)
            .or_insert_with(|| CameraState::new(clamped_origin_x, clamped_origin_y, zoom));
        let (origin_x, origin_y) = state.update(
            clamped_origin_x,
            clamped_origin_y,
            zoom,
            0.0,
            max_origin_x,
            0.0,
            max_origin_y,
        );
        used_camera_keys.push(key);

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

        self.draw_sky(snapshot.sky.as_ref(), environment, events, lighting);
        self.draw_ground(
            environment.ambient_temperature,
            snapshot.landscape.as_ref(),
            lighting,
        );
        self.draw_liquids(
            environment.ambient_temperature,
            snapshot.landscape.as_ref(),
            lighting,
        );
        self.draw_objects(&snapshot.objects, lighting, owner_colors);
        self.draw_precipitation(
            environment.precipitation,
            environment.ambient_temperature,
            snapshot.frame,
            lighting,
        );
        // C4Object::Draw attaches no energy/magic bars to world objects —
        // energy presentation lives in the HUD corner (DrawCursorInfo,
        // src/C4Viewport.cpp:920-945). The world-space fctEnergy bolt only
        // blinks (`Tick35 > 12`) over NeedEnergy structures
        // (src/C4Object.cpp:2505-2510); NeedEnergy is not modeled in the
        // Rust engine yet, so nothing is drawn here.
        let highlight_ids = Self::collect_highlight_ids(snapshot, input.owner, input.focus.id);
        self.draw_selection_marks(snapshot, &highlight_ids, input.owner, origin_x, origin_y, zoom);
        self.draw_player_cursors(snapshot, input.owner, origin_x, origin_y, zoom);

        let content_surface = std::mem::replace(&mut self.surface, main_surface);

        self.surface_width = saved_surface_width;
        self.surface_height = saved_surface_height;
        self.viewport_x = saved_viewport_x;
        self.viewport_y = saved_viewport_y;
        self.viewport_zoom = saved_viewport_zoom;
        self.world_width = saved_world_width;
        self.world_height = saved_world_height;

        blit_surface(&mut viewport_surface, &content_surface, offset_x, offset_y);
        if content_surface.width() > 0 && content_surface.height() > 0 {
            let content_width = content_surface.width() as i32;
            let content_height = content_surface.height() as i32;
            let offset_x_i32 = offset_x;
            let offset_y_i32 = offset_y;
            for y in 0..viewport_surface.height() as i32 {
                for x in 0..viewport_surface.width() as i32 {
                    let rel_x = x - offset_x_i32;
                    let rel_y = y - offset_y_i32;
                    if rel_x >= 0 && rel_x < content_width && rel_y >= 0 && rel_y < content_height {
                        continue;
                    }
                    let sample_x = rel_x.clamp(0, content_width - 1) as u32;
                    let sample_y = rel_y.clamp(0, content_height - 1) as u32;
                    if let Some(color) = content_surface.get_pixel(sample_x, sample_y) {
                        let _ = viewport_surface.set_pixel(x as u32, y as u32, color);
                    }
                }
            }
        }
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
                draw_image_region(&mut self.surface, &rect, &image, None, &source, false, None);
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

        let columns = match count {
            1 => 1,
            2 => 1,
            _ => (count as f32).sqrt().ceil() as usize,
        }
        .max(1);
        let rows = count.div_ceil(columns).max(1);

        let available_width = self.surface_width;
        let base_width = available_width / columns as u32;
        let leftover_width = available_width % columns as u32;
        let base_height = (available_height as u32) / rows as u32;
        let leftover_height = (available_height as u32) % rows as u32;

        let mut rects = Vec::with_capacity(count);
        let mut viewport_index = 0usize;
        let mut y = overlay_height.max(0) as u32;
        for row in 0..rows {
            let mut x = 0u32;
            let row_height = base_height + if row < leftover_height as usize { 1 } else { 0 };
            for col in 0..columns {
                if viewport_index >= count {
                    break;
                }
                let col_width = base_width + if col < leftover_width as usize { 1 } else { 0 };
                if col_width == 0 || row_height == 0 {
                    rects.push(SurfaceRect::new(x as i32, y as i32, col_width, row_height));
                } else {
                    let margin = if rows == 1 && columns == 1 { 0 } else { 2 };
                    let mut rect_x = x as i32 + margin;
                    let mut rect_y = y as i32 + margin;
                    let mut rect_width = col_width.saturating_sub((margin * 2).max(0) as u32);
                    let mut rect_height = row_height.saturating_sub((margin * 2).max(0) as u32);
                    if rect_width == 0 {
                        rect_width = col_width;
                        rect_x = x as i32;
                    }
                    if rect_height == 0 {
                        rect_height = row_height;
                        rect_y = y as i32;
                    }
                    rects.push(SurfaceRect::new(rect_x, rect_y, rect_width, rect_height));
                }
                x += col_width;
                viewport_index += 1;
            }
            y += row_height;
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

            let mut max_surface_height = landscape
                .surface()
                .iter()
                .copied()
                .max()
                .unwrap_or(self.fallback_ground_height);
            if max_surface_height < self.fallback_ground_height {
                max_surface_height = self.fallback_ground_height;
            }
            if max_surface_height <= 0 {
                max_surface_height = self.surface_height as i32;
            }
            self.world_height = max_surface_height.max(1);
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
    ) {
        if let Some(state) = self.sky.clone() {
            self.render_configured_sky(&state, frame, environment, events, lighting);
        } else {
            let base = environment
                .sky_color
                .map(|color| Color::opaque(color.r, color.g, color.b))
                .unwrap_or_else(|| {
                    Self::sky_color_for_temperature(environment.ambient_temperature)
                });
            let tinted = Self::apply_lighting(base, lighting);
            self.surface.fill(tinted);
        }
    }

    fn render_configured_sky(
        &mut self,
        state: &SkyRenderState,
        frame: Option<&SkyFrame>,
        environment: &EnvironmentFrame,
        events: &[WeatherEvent],
        lighting: f32,
    ) {
        let settings = frame
            .map(|frame| &frame.settings)
            .unwrap_or(&state.settings);

        if let Some(color) = settings.back_color {
            let base = Self::bgr_to_color(color);
            let tinted = Self::apply_lighting(base, lighting);
            self.surface.fill(tinted);
        } else if !settings.has_surface {
            self.fill_sky_gradient(settings, environment, lighting);
        } else {
            self.surface.fill(Color::opaque(0, 0, 0));
        }

        if settings.has_surface {
            if let Some(image) = state.image() {
                self.tile_sky_image(image, settings, frame, lighting);
            } else {
                self.fill_sky_gradient(settings, environment, lighting);
            }
        } else if settings.back_color.is_none() {
            self.fill_sky_gradient(settings, environment, lighting);
        }

        if events
            .iter()
            .any(|event| matches!(event, WeatherEvent::Lightning { .. }))
        {
            self.overlay_lightning_flash();
        }
    }

    fn fill_sky_gradient(
        &mut self,
        settings: &SkySettings,
        environment: &EnvironmentFrame,
        lighting: f32,
    ) {
        let gamma = environment.settings.season_gamma();
        let top_gamma = gamma.map(|(_, _, high)| high);
        let bottom_gamma = gamma.map(|(low, _, _)| low);
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
        let top = Self::mix_color_with_gamma(
            Self::sky_fade_color(settings, view_top, self.world_height),
            top_gamma,
        );
        let bottom = Self::mix_color_with_gamma(
            Self::sky_fade_color(settings, view_bottom, self.world_height),
            bottom_gamma,
        );
        self.fill_vertical_gradient(top, bottom, lighting);
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

    fn fill_vertical_gradient(&mut self, top: Color, bottom: Color, lighting: f32) {
        if self.surface_width == 0 || self.surface_height == 0 {
            return;
        }
        let height = self.surface_height.saturating_sub(1).max(1);
        for y in 0..self.surface_height {
            let t = y as f32 / height as f32;
            let blended = Self::lerp_color(top, bottom, t);
            let tinted = Self::apply_lighting(blended, lighting);
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
                if color.a == 255 {
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

    fn mix_color_with_gamma(base: RgbColor, gamma: Option<RgbColor>) -> Color {
        if let Some(gamma) = gamma {
            let r = ((base.r as u16 + gamma.r as u16) / 2) as u8;
            let g = ((base.g as u16 + gamma.g as u16) / 2) as u8;
            let b = ((base.b as u16 + gamma.b as u16) / 2) as u8;
            Color::opaque(r, g, b)
        } else {
            Color::opaque(base.r, base.g, base.b)
        }
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

    fn overlay_lightning_flash(&mut self) {
        let pixels = self.surface.pixels_mut();
        for chunk in pixels.chunks_exact_mut(4) {
            chunk[0] = chunk[0].saturating_add(96);
            chunk[1] = chunk[1].saturating_add(96);
            chunk[2] = chunk[2].saturating_add(144);
        }
    }

    fn draw_precipitation(
        &mut self,
        precipitation: i32,
        ambient_temperature: i32,
        frame: u64,
        lighting: f32,
    ) {
        if precipitation == 0 {
            return;
        }

        if precipitation > 0 {
            if ambient_temperature <= 0 {
                let intensity = precipitation.clamp(0, 100) as usize;
                let flakes = intensity.saturating_mul(2).max(16);
                for idx in 0..flakes {
                    let offset = frame as usize * 7 + idx * 19;
                    let x = ((idx * 41 + offset) % self.surface_width as usize) as u32;
                    let y = (offset % self.surface_height as usize) as u32;
                    let color = Color::new(236, 236, 252, 220).modulate(lighting);
                    let _ = self.surface.set_pixel(x, y, color);
                    if y + 1 < self.surface_height {
                        let _ = self.surface.set_pixel(x, y + 1, color);
                    }
                }
            } else {
                let intensity = precipitation.clamp(0, 100) as usize;
                let streaks = intensity.saturating_mul(3).max(12);
                for idx in 0..streaks {
                    let offset = frame as usize * 11 + idx * 17;
                    let x = ((idx * 53 + offset) % self.surface_width as usize) as u32;
                    let base_y = (offset % self.surface_height as usize) as i32;
                    for step in 0..4 {
                        let y = base_y - step;
                        if y < 0 {
                            continue;
                        }
                        let color = Color::new(148, 176, 220, 160).modulate(lighting);
                        let _ = self.surface.set_pixel(x, y as u32, color);
                    }
                }
            }
        } else {
            let dryness = precipitation.saturating_neg().clamp(0, 100) as usize;
            let shimmer_count = dryness.saturating_mul(2).max(8);
            for idx in 0..shimmer_count {
                let offset = frame as usize * 5 + idx * 23;
                let x = ((idx * 67 + offset) % self.surface_width as usize) as u32;
                let band = offset % 6;
                let y = self.surface_height.saturating_sub(1 + band as u32);
                let color = if band % 2 == 0 {
                    Color::new(212, 180, 88, 200)
                } else {
                    Color::new(176, 132, 64, 180)
                }
                .modulate(lighting);
                let _ = self.surface.set_pixel(x, y, color);
            }
        }
    }

    /// Per-pixel landscape rendering from the sim plane: every pixel
    /// byte samples its texmap texture png tiled by WORLD coordinates —
    /// the same composition C4Landscape::MapToSurface bakes into
    /// Surface32. Returns false when no plane/textures exist (legacy
    /// column painter takes over).
    fn draw_ground_textured(&mut self, landscape: Option<&Landscape>) -> bool {
        let Some(grid) = landscape.and_then(|landscape| landscape.pixel_grid()) else {
            return false;
        };
        if self.material_textures.is_empty() {
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
            // Per texmap slot: the texture image or a flat fallback color.
            enum Slot<'a> {
                Empty,
                Texture(&'a ImageData, [u8; 3]),
                Flat([u8; 4]),
            }
            let slots: Vec<Slot> = (0..128usize)
                .map(|index| {
                    let base = materials
                        .get(index)
                        .and_then(|name| name.as_deref())
                        .and_then(|name| self.material_colors.get(&name.to_ascii_lowercase()))
                        .copied()
                        .unwrap_or([255, 255, 255]);
                    let texture = textures
                        .get(index)
                        .and_then(|name| name.as_deref())
                        .and_then(|name| self.material_textures.get(&name.to_ascii_lowercase()));
                    if let Some(texture) = texture {
                        return Slot::Texture(texture, base);
                    }
                    if materials.get(index).and_then(|name| name.as_ref()).is_some() {
                        let flat = if base != [255, 255, 255] {
                            [base[0], base[1], base[2], 255]
                        } else {
                            [120, 92, 56, 255]
                        };
                        return Slot::Flat(flat);
                    }
                    Slot::Empty
                })
                .collect();
            let mut pixels = vec![0u8; width as usize * height as usize * 4];
            for y in 0..height as usize {
                for x in 0..width as usize {
                    let byte = bytes[y * width as usize + x];
                    let index = (byte & 0x7f) as usize;
                    let out = (y * width as usize + x) * 4;
                    match &slots[index] {
                        Slot::Empty => {}
                        Slot::Flat(color) => pixels[out..out + 4].copy_from_slice(color),
                        Slot::Texture(texture, base) => {
                            let tw = texture.width().max(1) as usize;
                            let th = texture.height().max(1) as usize;
                            let src = ((y % th) * tw + (x % tw)) * 4;
                            let data = texture.pixels();
                            if src + 4 <= data.len() {
                                // ModulateClrA + LightenClr
                                // (CPattern::PatternClr): base x texture,
                                // doubled and clamped.
                                for channel in 0..3 {
                                    let modulated = (base[channel] as u32
                                        * data[src + channel] as u32)
                                        / 255;
                                    pixels[out + channel] =
                                        (modulated * 2).min(255) as u8;
                                }
                                pixels[out + 3] = 255;
                            }
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
                let color = Color::opaque(
                    cache_pixels[src],
                    cache_pixels[src + 1],
                    cache_pixels[src + 2],
                );
                let _ = self.surface.set_pixel(screen_x, screen_y, color);
            }
        }
        true
    }

    fn draw_ground(
        &mut self,
        ambient_temperature: i32,
        landscape: Option<&Landscape>,
        lighting: f32,
    ) {
        if self.draw_ground_textured(landscape) {
            return;
        }
        let ground_color = Self::apply_lighting(
            Self::ground_color_for_temperature(ambient_temperature),
            lighting,
        );
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
    }

    fn draw_liquids(
        &mut self,
        ambient_temperature: i32,
        landscape: Option<&Landscape>,
        lighting: f32,
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
                    let blended = match self.surface.get_pixel(x, y) {
                        Some(existing) => blend_color_over(base_color, existing),
                        None => base_color,
                    };
                    let _ = self.surface.set_pixel(x, y, blended);
                }
            }
        }
    }

    fn draw_objects(
        &mut self,
        objects: &[ObjectSnapshot],
        lighting: f32,
        owner_colors: &HashMap<i32, Color>,
    ) {
        let mut background = Vec::new();
        let mut midground = Vec::new();
        let mut foreground = Vec::new();
        let mut parallax = Vec::new();

        for object in objects {
            if object.status != ObjectStatus::Normal {
                continue;
            }
            // `if (Contained && !eDrawMode) return;` (src/C4Object.cpp:2363):
            // carried objects never draw into the landscape.
            if object.container.is_some() {
                continue;
            }
            if object.category & CATEGORY_BACKGROUND_FLAG != 0 {
                background.push(object);
            } else if object.category & CATEGORY_FOREGROUND_FLAG != 0 {
                if object.category & CATEGORY_PARALLAX_FLAG != 0 {
                    parallax.push(object);
                } else {
                    foreground.push(object);
                }
            } else {
                midground.push(object);
            }
        }

        fn sort_for_render(list: &mut Vec<&ObjectSnapshot>) {
            list.sort_by(|lhs, rhs| {
                (lhs.category & CATEGORY_SORT_LIMIT)
                    .cmp(&(rhs.category & CATEGORY_SORT_LIMIT))
                    .then_with(|| lhs.position.y.cmp(&rhs.position.y))
                    .then_with(|| lhs.position.x.cmp(&rhs.position.x))
                    .then_with(|| lhs.id.as_u64().cmp(&rhs.id.as_u64()))
            });
        }

        sort_for_render(&mut background);
        sort_for_render(&mut midground);
        sort_for_render(&mut foreground);
        sort_for_render(&mut parallax);

        for object in background
            .into_iter()
            .chain(midground.into_iter())
            .chain(foreground.into_iter())
        {
            self.paint_object(object, lighting, owner_colors);
        }

        for object in parallax {
            self.paint_object(object, lighting, owner_colors);
        }
    }

    fn paint_object(
        &mut self,
        object: &ObjectSnapshot,
        lighting: f32,
        owner_colors: &HashMap<i32, Color>,
    ) {
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let content_width = self.surface_width as f32;
        let content_height = self.surface_height as f32;
        let color = object_color(object).modulate(lighting);
        let owner_color = owner_colors.get(&object.owner).copied();
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
                &sprite,
                owner_color,
                zoom,
                rotation_degrees,
                base_transform,
            );
            self.draw_object_overlays(
                object,
                owner_color,
                screen_x,
                screen_y,
                zoom,
                rotation_degrees,
                base_transform,
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
            owner_color,
            screen_x,
            screen_y,
            zoom,
            rotation_degrees,
            base_transform,
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
        sprite: &DefinitionSprite,
        owner_color: Option<Color>,
        zoom: f32,
        rotation_degrees: f32,
        transform: Option<DrawTransform>,
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
            );
            return;
        };
        let direction_index = match object.direction {
            Direction::Left => 0,
            Direction::Right => 1,
        };
        let (draw_dir, flipped) = Self::resolve_draw_direction(graphics, direction_index);
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
                draw_dir as i32,
                flipped,
                owner_color,
                zoom,
                rotation_degrees,
                transform,
            );
        }
        let Some(facet) = &graphics.facet else {
            return;
        };
        if facet.width <= 0 || facet.height <= 0 {
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
            facet.y + facet.height.saturating_mul(draw_dir as i32),
            facet.width,
            facet.height,
        );
        // Full con: the facet at cox+FacetX/coy+FacetY; growing: the
        // con-scaled shape rect at cox/coy (src/C4Object.cpp:2450-2467).
        let cox = (object.position.x + inst_shape.x) as f32;
        let coy = (object.position.y + inst_shape.y) as f32;
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
        owner_color: Option<Color>,
        zoom: f32,
        rotation_degrees: f32,
        transform: Option<DrawTransform>,
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
        owner_color: Option<Color>,
        zoom: f32,
        rotation_degrees: f32,
        transform: Option<DrawTransform>,
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
            );
        }
    }

    fn draw_action_graphic(
        &mut self,
        sprite: &DefinitionSprite,
        action_name: &str,
        phase: i32,
        direction: Direction,
        owner_color: Option<Color>,
        screen_x: f32,
        screen_y: f32,
        zoom: f32,
        rotation_degrees: f32,
        transform: Option<DrawTransform>,
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

        let direction_index = match direction {
            Direction::Left => 0,
            Direction::Right => 1,
        };
        let (draw_dir, flipped) = Self::resolve_draw_direction(graphics, direction_index);

        let source_rect = SourceRect::new(
            facet.x + facet.width.saturating_mul(frame_index),
            facet.y + facet.height.saturating_mul(draw_dir as i32),
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
            );
        }
        true
    }

    fn draw_object_overlays(
        &mut self,
        object: &ObjectSnapshot,
        owner_color: Option<Color>,
        screen_x: f32,
        screen_y: f32,
        zoom: f32,
        rotation_degrees: f32,
        base_transform: Option<DrawTransform>,
    ) {
        if object.graphics_overlays.is_empty() {
            return;
        }
        for overlay in &object.graphics_overlays {
            let combined_transform = match (base_transform, overlay.transform) {
                (Some(base), Some(local)) => Some(base.combined(local)),
                (Some(base), None) => Some(base),
                (None, Some(local)) => Some(local),
                (None, None) => None,
            };
            match overlay.mode {
                GraphicsOverlayMode::Action => self.draw_overlay_action(
                    object,
                    overlay,
                    owner_color,
                    screen_x,
                    screen_y,
                    zoom,
                    rotation_degrees,
                    combined_transform,
                ),
                GraphicsOverlayMode::Base => self.draw_overlay_base(
                    object,
                    overlay,
                    owner_color,
                    screen_x,
                    screen_y,
                    zoom,
                    rotation_degrees,
                    combined_transform,
                ),
                _ => {}
            }
        }
    }

    fn draw_overlay_action(
        &mut self,
        object: &ObjectSnapshot,
        overlay: &ObjectGraphicsOverlay,
        owner_color: Option<Color>,
        screen_x: f32,
        screen_y: f32,
        zoom: f32,
        rotation_degrees: f32,
        transform: Option<DrawTransform>,
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
        );
    }

    fn draw_overlay_base(
        &mut self,
        object: &ObjectSnapshot,
        overlay: &ObjectGraphicsOverlay,
        owner_color: Option<Color>,
        screen_x: f32,
        screen_y: f32,
        zoom: f32,
        rotation_degrees: f32,
        transform: Option<DrawTransform>,
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
            );
        }
    }

    fn resolve_draw_direction(graphics: &DefinitionActionGraphics, direction: u32) -> (u32, bool) {
        let directions = graphics.directions.max(1);
        if let Some(flip_dir) = graphics.flip_dir {
            if flip_dir > 0 && direction >= flip_dir {
                let base = flip_dir - 1;
                let delta = direction - flip_dir;
                let draw_dir = base.saturating_sub(delta).min(directions.saturating_sub(1));
                return (draw_dir, true);
            }
        }
        (direction.min(directions.saturating_sub(1)), false)
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
        draw_image_region(&mut self.surface, &rect, &image, None, &source, false, None);

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
                font.draw(
                    &mut self.surface,
                    text_x,
                    text_y,
                    line,
                    Color::opaque(0xff, 0x00, 0x00),
                    lc_graphics::clonk_font::TextAlign::Center,
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
    fn draw_hud(&mut self) {
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
                hud::draw_cursor_info(
                    &mut self.surface,
                    &font,
                    &self.hud_graphics,
                    rect,
                    &crew.label,
                    crew.rank,
                    crew.portrait.as_ref(),
                    crew.rank_symbols.as_ref(),
                );
                hud::draw_energy_bar(
                    &mut self.surface,
                    &self.hud_graphics,
                    rect,
                    crew.energy_fraction,
                );
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
                hud::draw_commands(
                    &mut self.surface,
                    &tiny,
                    &self.hud_graphics,
                    rect,
                    &player.commands,
                    self.show_command_keys,
                );
            }

            hud::draw_player_fixed_items(
                &mut self.surface,
                &font,
                &self.hud_graphics,
                rect,
                player.wealth,
                player.score,
                player.select_count,
                player.crew.len() as i32,
                player.owner_color,
            );

            if player.show_startup {
                hud::draw_player_startup(
                    &mut self.surface,
                    &font,
                    &self.hud_graphics,
                    rect,
                    &player.name,
                    player.owner_color,
                );
            }
        }

        let font = hud::HudFont::from_set(self.clonk_fonts.as_deref(), self.font.as_ref());
        if self.hud_chrome_active() {
            hud::draw_message_board(
                &mut self.surface,
                &font,
                &self.hud_graphics,
                self.message_board_line.as_deref(),
            );
            hud::draw_upper_board(
                &mut self.surface,
                &font,
                &self.hud_graphics,
                &self.scenario_label_text,
                self.game_time_seconds,
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
            font.draw(
                &mut self.surface,
                hud::SYMBOL_BORDER,
                base_y,
                &frame_text,
                Color::opaque(255, 255, 255),
                lc_graphics::clonk_font::TextAlign::Left,
            );
            font.draw(
                &mut self.surface,
                hud::SYMBOL_BORDER,
                base_y + line_height,
                &status_text,
                Color::opaque(255, 255, 255),
                lc_graphics::clonk_font::TextAlign::Left,
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
    owner_color: Option<Color>,
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

            let mut color = Color::new(
                pixels[idx],
                pixels[idx + 1],
                pixels[idx + 2],
                pixels[idx + 3],
            );

            if let (Some(mask_map), Some(owner)) = (mask, owner_color) {
                if src_x >= 0 && src_y >= 0 {
                    let mask_value = mask_map.value_at(src_x as u32, src_y as u32);
                    if mask_value != 0 {
                        color = blend_color_by_owner(color, mask_value, owner);
                    }
                }
            }

            if color.a == 0 {
                continue;
            }

            let blended = if color.a == 255 {
                color
            } else {
                let background = surface
                    .get_pixel(target_x as u32, target_y as u32)
                    .unwrap_or_default();
                blend_colors(color, background)
            };

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
    owner_color: Option<Color>,
    rotation_degrees: f32,
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

            let mut color = Color::new(
                pixels[idx],
                pixels[idx + 1],
                pixels[idx + 2],
                pixels[idx + 3],
            );

            if let (Some(mask_map), Some(owner)) = (mask, owner_color) {
                if sample_x >= 0 && sample_y >= 0 {
                    let mask_value = mask_map.value_at(sample_x as u32, sample_y as u32);
                    if mask_value != 0 {
                        color = blend_color_by_owner(color, mask_value, owner);
                    }
                }
            }

            if color.a == 0 {
                continue;
            }

            if color.a < 255 {
                let background = surface.get_pixel(x as u32, y as u32).unwrap_or_default();
                color = blend_colors(color, background);
            }

            let _ = surface.set_pixel(x as u32, y as u32, color);
        }
    }
}

pub fn draw_image(surface: &mut Surface, rect: &GuiRect, image: &ImageData) {
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

    if dest_width == image.width() && dest_height == image.height() {
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

            let blended = if color.a == 255 {
                color
            } else {
                let background = surface
                    .get_pixel(target_x as u32, target_y as u32)
                    .unwrap_or_default();
                blend_colors(color, background)
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
                    encode_channel(gamma, f32::from(rgba[0])) as u8,
                    encode_channel(gamma, f32::from(rgba[1])) as u8,
                    encode_channel(gamma, f32::from(rgba[2])) as u8,
                    255,
                )
            } else if gamma.is_none() {
                let background = surface.get_pixel(tx as u32, ty as u32).unwrap_or_default();
                blend_colors(Color::new(rgba[0], rgba[1], rgba[2], rgba[3]), background)
            } else {
                let dst = surface.get_pixel(tx as u32, ty as u32).unwrap_or_default();
                let af = f32::from(rgba[3]) / 255.0;
                let blend = |src: u8, dst: u8| -> u8 {
                    (encode_channel(gamma, f32::from(src)) * af + f32::from(dst) * (1.0 - af))
                        .round()
                        .clamp(0.0, 255.0) as u8
                };
                Color::new(
                    blend(rgba[0], dst.r),
                    blend(rgba[1], dst.g),
                    blend(rgba[2], dst.b),
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
                            let blend = |src: f32, dst: u8| -> u8 {
                                (encode_channel(gamma, src) * af + f32::from(dst) * (1.0 - af))
                                    .round()
                                    .clamp(0.0, 255.0)
                                    as u8
                            };
                            Color::new(
                                blend(s[0], dst.r),
                                blend(s[1], dst.g),
                                blend(s[2], dst.b),
                                255,
                            )
                        }
                        BilinearBlend::Additive => {
                            let add = |src: f32, dst: u8| -> u8 {
                                (f32::from(dst) + encode_channel(gamma, src) * af)
                                    .round()
                                    .clamp(0.0, 255.0) as u8
                            };
                            Color::new(add(s[0], dst.r), add(s[1], dst.g), add(s[2], dst.b), dst.a)
                        }
                    };
                    let _ = surface.set_pixel(px as u32, py as u32, out);
                }
            }
        }
    }
}

/// Encodes a filtered colour channel the way the C++ blit shader does:
/// through the gamma 1D-texture lookup when a ramp is given
/// (StdGL.cpp:1082-1086), else round-to-nearest.
fn encode_channel(gamma: Option<&lc_graphics::GammaRamp>, x: f32) -> f32 {
    gamma
        .map(|ramp| f32::from(ramp.encode_float(x)))
        .unwrap_or_else(|| x.round().clamp(0.0, 255.0))
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
    use lc_engine::{
        CommandStackSnapshot, EnvironmentFrame, Landscape, LiquidSegment, ObjectId, ObjectVertex,
        PlayerState, RgbColor, Vector2,
    };
    use lc_graphics::{BitmapFont, PixelFormat};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn test_font() -> Arc<dyn TextFont> {
        Arc::new(BitmapFont::new())
    }

    fn gray(v: u8) -> Color {
        Color::new(v, v, v, 255)
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

    fn make_snapshot() -> SimulationSnapshot {
        SimulationSnapshot {
            frame: 0,
            game_over: false,
            physics: None,
            objects: vec![ObjectSnapshot {
                id: ObjectId::new(1),
                definition_id: "TestObject".to_string(),
                custom_name: None,
                position: Vector2::new(100, 100),
                velocity: Vector2::ZERO,
                rotation: 0,
                energy: 100,
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
                own_vertices: None,
                container: None,
                contents: Vec::new(),
                components: HashMap::new(),
                status: Default::default(),
                owner: 0,
                controller: 0,
                category: lc_engine::DEFAULT_CATEGORY,
                crew_member: true,
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
                fixed_position: None,
                fixed_velocity: None,
                rotation_velocity: None,
                fixed_rotation: None,
            }],
            environment: EnvironmentFrame::default(),
            sky: None,
            weather_events: Vec::new(),
            global_effects: Vec::new(),
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

    #[test]
    fn precipitation_renders_over_world() {
        let mut snapshot = make_snapshot();
        snapshot.environment.ambient_temperature = 10;
        let mut base_settings = snapshot.environment.settings;
        base_settings = base_settings.with_temperature(10);
        snapshot.environment.settings = base_settings;

        let focus = &snapshot.objects[0];
        let mut graphics = GraphicsSystem::new(
            320,
            180,
            150,
            "Weather Scenario",
            test_font(),
            empty_sprites(),
            empty_cursor_atlas(),
            empty_hud_graphics(),
        );

        let dry_viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &dry_viewports);
        let baseline = graphics.surface().pixels().to_vec();

        let mut rainy = snapshot.clone();
        rainy.environment.precipitation = 80;
        let mut rainy_settings = rainy.environment.settings;
        rainy_settings = rainy_settings.with_precipitation(80);
        rainy_settings = rainy_settings.with_precipitation_strength(80);
        rainy.environment.settings = rainy_settings;
        let rainy_viewports = vec![ViewportInput::from_focus(&rainy.objects[0])];

        graphics.render_frame(&rainy, &rainy_viewports);
        let rainy_pixels = graphics.surface().pixels();
        let differences = rainy_pixels
            .iter()
            .zip(baseline.iter())
            .filter(|(wet, dry)| wet != dry)
            .count();
        assert!(
            differences > 0,
            "expected precipitation to affect rendered frame"
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
        assert_eq!(bottom_view, 360 - 180);
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
            },
        );
        Arc::new(sprites)
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
            Some(green),
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
            Some(green),
            "expected the face inside the shape rect"
        );
        assert_ne!(
            graphics.surface().get_pixel((sx + 1) as u32, (sy + 1) as u32),
            Some(green),
            "face must not extend past the shape rect (centered draw would)"
        );
        assert_ne!(
            graphics.surface().get_pixel((sx - 9) as u32, (sy - 9) as u32),
            Some(green),
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
            Some(green),
            "expected the half-grown face inside the con-scaled shape"
        );
        assert_ne!(
            graphics.surface().get_pixel((sx - 2) as u32, (sy - 2) as u32),
            Some(green),
            "half-grown face must not spill left of the scaled shape"
        );
        assert_ne!(
            graphics.surface().get_pixel((sx + 5) as u32, (sy - 2) as u32),
            Some(green),
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
            Some(green),
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
            Some(green),
            "expected the facet at cox+FacetX/coy+FacetY sourcing Facet x/y"
        );
        assert_ne!(
            graphics.surface().get_pixel((sx - 3) as u32, (sy - 3) as u32),
            Some(green),
            "facet must not be centered on the position"
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
            crew: vec![CrewOverlay {
                object_id,
                label: "Joe".to_string(),
                energy_fraction: 1.0,
                is_focus: true,
                portrait: None,
                rank: 0,
                rank_symbols: None,
                info_name: info_name.map(str::to_string),
                rank_name: None,
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
        graphics
            .surface()
            .pixels()
            .chunks_exact(4)
            .filter(|chunk| *chunk == [255u8, 0, 0, 255])
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
            graphics
                .surface()
                .pixels()
                .chunks_exact(4)
                .enumerate()
                .filter(|(_, chunk)| *chunk == [255u8, 0, 0, 255])
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
}
