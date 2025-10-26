mod input;
mod startup_main_menu;
mod startup_menu;
mod startup_options;

use lc_engine::{
    DefinitionActionGraphics, Direction, DrawTransform, EnvironmentFrame, EnvironmentSettings,
    FloatVector2, GraphicsOverlayMode, Landscape, ObjectGraphicsOverlay, ObjectId, ObjectSnapshot,
    ObjectStatus, RgbColor, SimulationSnapshot, SkyFrame, SkySettings,
    SurfaceSnapshot as EngineSurfaceSnapshot, Vector2, WeatherEvent, CATEGORY_SORT_LIMIT,
    OWNER_NONE,
};
use lc_graphics::{
    Color, PixelFormat, Point as SurfacePoint, Rect as SurfaceRect, Surface,
    SurfaceSnapshot as GraphicsSurfaceSnapshot, TextFont,
};
use lc_gui::{DrawCommand, Gui, GuiResult, Rect as GuiRect, Size as GuiSize, WidgetId};
use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::convert::TryFrom;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

pub use input::InputDispatcher;
pub use lc_gui::{
    GuiError as StartupMenuError, GuiResult as StartupMenuResult, ImageData, KeyCode,
    Point as GuiPoint, ScenarioEntry, ScenarioKind,
};
pub use startup_main_menu::{MainMenuAction, MainMenuItem, StartupMainMenu};
pub use startup_menu::{ScenarioSummary, StartupMenu, StartupMenuAction};
pub use startup_options::{ControlOptionItem, ControlOptionsAction, ControlOptionsView};

const OVERLAY_HEIGHT: f32 = 120.0;
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
    pub frame_text: &'a str,
    pub status_text: &'a str,
    pub energy_fraction: f32,
    pub players: Vec<PlayerOverlay>,
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
    pub crew: Vec<CrewOverlay>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CrewOverlay {
    pub object_id: ObjectId,
    pub label: String,
    pub energy_fraction: f32,
    pub is_focus: bool,
    pub portrait: Option<ImageData>,
}

#[derive(Debug)]
struct PlayerWidgets {
    owner: i32,
    header_label: WidgetId,
    status_label: WidgetId,
    wealth: PlayerStatWidgets,
    score: PlayerStatWidgets,
    crew: Vec<CrewWidgets>,
}

#[derive(Debug)]
struct CrewWidgets {
    portrait: WidgetId,
    label: WidgetId,
    gauge: WidgetId,
}

#[derive(Debug)]
struct PlayerStatWidgets {
    icon: WidgetId,
    label: WidgetId,
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
    gui: Gui,
    font: Arc<dyn TextFont>,
    scenario_label_text: String,
    scenario_label: WidgetId,
    frame_label: WidgetId,
    status_label: WidgetId,
    energy_gauge: WidgetId,
    players_container: WidgetId,
    player_widgets: Vec<PlayerWidgets>,
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
        let mut gui = Gui::new(font.clone());
        let root = gui.root();
        let scenario_label_widget = gui.add_label(root, scenario_label);
        let frame_label = gui.add_label(root, "FRAME 000 X 000 Y 000 VX P00 VY P00");
        let status_label = gui.add_label(root, "READY 00OF00 GROUND 00 BATCH 000");
        let energy_gauge = gui.add_gauge(root);
        gui.set_gauge_fraction(energy_gauge, 1.0)
            .expect("initial gauge");
        let players_container = gui.add_column(root, true);

        let mut surface = Surface::new(
            surface_width,
            surface_height,
            lc_graphics::PixelFormat::Rgba8888,
        );
        surface.fill(Color::opaque(8, 12, 24));

        Self {
            surface,
            gui,
            font,
            scenario_label_text: scenario_label.to_string(),
            scenario_label: scenario_label_widget,
            frame_label,
            status_label,
            energy_gauge,
            players_container,
            player_widgets: Vec::new(),
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

    pub fn update_overlay(&mut self, overlay: &GraphicsOverlay<'_>) -> GuiResult<()> {
        self.ensure_player_widgets(&overlay.players)?;
        self.gui
            .set_label_text(self.frame_label, overlay.frame_text)?;
        self.gui
            .set_label_text(self.status_label, overlay.status_text)?;
        self.gui
            .set_gauge_fraction(self.energy_gauge, overlay.energy_fraction.clamp(0.0, 1.0))?;

        let header_color = Color::opaque(208, 220, 252);
        let info_color = Color::opaque(208, 208, 208);
        let warning_color = Color::opaque(232, 174, 72);
        let eliminated_color = Color::opaque(224, 92, 92);
        let focus_color = Color::opaque(252, 242, 160);
        let crew_color = Color::opaque(212, 212, 212);
        let portrait_background = Color::opaque(18, 26, 40);
        let portrait_focus_background = Color::opaque(32, 40, 56);
        let portrait_frame_neutral = Color::opaque(44, 58, 84);
        let fallback_portrait = self.hud_graphics.crew.clone();
        let wealth_icon_image = self.hud_graphics.wealth.clone();
        let score_icon_image = self.hud_graphics.score.clone();

        for (player_overlay, widgets) in overlay.players.iter().zip(self.player_widgets.iter()) {
            let header_text = if player_overlay.name.is_empty() {
                format!("Player {}", player_overlay.owner)
            } else {
                player_overlay.name.clone()
            };
            self.gui.set_label_text(widgets.header_label, header_text)?;
            self.gui
                .set_label_color(widgets.header_label, header_color)?;

            self.gui
                .set_picture_image(widgets.wealth.icon, wealth_icon_image.clone())?;
            self.gui
                .set_picture_image(widgets.score.icon, score_icon_image.clone())?;

            if player_overlay.eliminated {
                self.gui.set_label_text(widgets.wealth.label, "--")?;
                self.gui
                    .set_label_color(widgets.wealth.label, eliminated_color)?;
                self.gui.set_label_text(widgets.score.label, "--")?;
                self.gui
                    .set_label_color(widgets.score.label, eliminated_color)?;
            } else {
                let wealth_text = format!("{}", player_overlay.wealth);
                self.gui.set_label_text(widgets.wealth.label, wealth_text)?;
                self.gui.set_label_color(widgets.wealth.label, info_color)?;

                let score_text = format!("{}", player_overlay.score);
                self.gui.set_label_text(widgets.score.label, score_text)?;
                self.gui.set_label_color(widgets.score.label, info_color)?;
            }

            if player_overlay.eliminated {
                self.gui
                    .set_label_text(widgets.status_label, "Eliminated")?;
                self.gui
                    .set_label_color(widgets.status_label, eliminated_color)?;
            } else if player_overlay.crew.is_empty() {
                self.gui
                    .set_label_text(widgets.status_label, "No crew available")?;
                self.gui
                    .set_label_color(widgets.status_label, warning_color)?;
            } else {
                let crew_count = player_overlay.crew.len();
                let status_text = if crew_count == 1 {
                    "Crew member ready".to_string()
                } else {
                    format!("Crew {crew_count}")
                };
                self.gui.set_label_text(widgets.status_label, status_text)?;
                self.gui.set_label_color(widgets.status_label, info_color)?;
            }

            for (crew_overlay, crew_widgets) in player_overlay.crew.iter().zip(widgets.crew.iter())
            {
                self.gui
                    .set_label_text(crew_widgets.label, &crew_overlay.label)?;
                if crew_overlay.is_focus {
                    self.gui.set_label_color(crew_widgets.label, focus_color)?;
                } else {
                    self.gui.set_label_color(crew_widgets.label, crew_color)?;
                }
                let image = crew_overlay
                    .portrait
                    .clone()
                    .or_else(|| fallback_portrait.clone());
                self.gui.set_picture_image(crew_widgets.portrait, image)?;
                let frame_color = if crew_overlay.is_focus {
                    focus_color
                } else if player_overlay.owner_color.a > 0 {
                    player_overlay.owner_color.modulate(0.85)
                } else {
                    portrait_frame_neutral
                };
                self.gui
                    .set_picture_frame_color(crew_widgets.portrait, frame_color)?;
                let background_color = if crew_overlay.is_focus {
                    portrait_focus_background
                } else {
                    portrait_background
                };
                self.gui
                    .set_picture_background_color(crew_widgets.portrait, background_color)?;
                self.gui.set_gauge_fraction(
                    crew_widgets.gauge,
                    crew_overlay.energy_fraction.clamp(0.0, 1.0),
                )?;
            }
        }
        Ok(())
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

        self.draw_gui_overlay();

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
        let highlight_ids = Self::collect_highlight_ids(snapshot, input.owner, input.focus.id);
        self.draw_object_energy_bars(
            snapshot,
            &highlight_ids,
            owner_colors,
            input.owner,
            origin_x,
            origin_y,
            zoom,
        );
        self.draw_selection_marks(snapshot, &highlight_ids, origin_x, origin_y, zoom);
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

    fn draw_object_energy_bars(
        &mut self,
        snapshot: &SimulationSnapshot,
        highlights: &HashSet<ObjectId>,
        owner_colors: &HashMap<i32, Color>,
        owner: i32,
        origin_x: f32,
        origin_y: f32,
        zoom: f32,
    ) {
        if self.hud_graphics.energy_bars.is_none() && self.hud_graphics.energy.is_none() {
            return;
        }
        let surface_width = self.surface_width as f32;
        let surface_height = self.surface_height as f32;
        for object in &snapshot.objects {
            if !object.crew_member || !object.status.is_active() || !object.alive {
                continue;
            }
            let highlighted = highlights.contains(&object.id);
            if object.owner != owner && !highlighted {
                continue;
            }

            let screen_x = (object.position.x as f32 - origin_x) * zoom;
            let screen_y = (object.position.y as f32 - origin_y) * zoom;
            let margin = 48.0;
            if screen_x < -margin
                || screen_x > surface_width + margin
                || screen_y < -margin
                || screen_y > surface_height + margin
            {
                continue;
            }

            let base_width = 32.0f32;
            let base_height = 4.0f32;
            let width = (base_width * zoom).clamp(18.0, 64.0);
            let height = (base_height * zoom).clamp(3.0, 10.0);
            let offset_y = (18.0 * zoom).clamp(12.0, 32.0);
            let base_origin = GuiPoint::new(screen_x - width / 2.0, screen_y - offset_y);

            let alpha = if highlighted { 255 } else { 220 };
            let owner_color = owner_colors
                .get(&object.owner)
                .copied()
                .unwrap_or_else(|| default_owner_color(object.owner));
            let mut energy_fill = if highlighted {
                owner_color.modulate(1.2)
            } else {
                owner_color.modulate(0.85)
            };
            energy_fill.a = alpha;
            let energy_fraction = (object.energy.max(0).min(100) as f32) / 100.0;

            let mut bars: Vec<(f32, Option<&ImageData>, Color)> = Vec::with_capacity(2);
            bars.push((
                energy_fraction.clamp(0.0, 1.0),
                self.hud_graphics.energy.as_ref(),
                energy_fill,
            ));

            if object.magic_capacity > 0 {
                let capacity = object.magic_capacity.max(1);
                let magic_fraction =
                    (object.magic_energy.max(0).min(capacity) as f32) / (capacity as f32);
                let mut magic_fill = Color::opaque(96, 148, 252);
                if highlighted {
                    magic_fill = magic_fill.modulate(1.15);
                }
                magic_fill.a = alpha;
                bars.push((
                    magic_fraction.clamp(0.0, 1.0),
                    self.hud_graphics.magic.as_ref(),
                    magic_fill,
                ));
            }

            let gap = (height * 0.6).clamp(2.0, 6.0);
            let background = Color::new(16, 24, 40, 210);

            for (index, &(fraction, icon, fill_color)) in bars.iter().enumerate() {
                let origin_y = base_origin.y + index as f32 * (height + gap);
                let origin = GuiPoint::new(base_origin.x, origin_y);
                let bar_rect = GuiRect::from_origin_size(origin, GuiSize::new(width, height));
                fill_rect(&mut self.surface, &bar_rect, background);

                if fraction > 0.0 {
                    let fill_width = (width * fraction).max(1.0);
                    let energy_rect =
                        GuiRect::from_origin_size(origin, GuiSize::new(fill_width, height));
                    fill_rect(&mut self.surface, &energy_rect, fill_color);
                }

                if let Some(icon) = icon {
                    let icon_scale = zoom.clamp(0.75, 1.25);
                    let icon_width = (icon.width() as f32 * icon_scale).clamp(14.0, 28.0);
                    let icon_height = (icon.height() as f32 * icon_scale).clamp(14.0, 28.0);
                    let icon_origin = GuiPoint::new(
                        origin.x - icon_width - 6.0,
                        origin.y - (icon_height - height) / 2.0,
                    );
                    let icon_rect = GuiRect::from_origin_size(
                        icon_origin,
                        GuiSize::new(icon_width, icon_height),
                    );
                    draw_image(&mut self.surface, &icon_rect, icon);
                }
            }
        }
    }

    fn draw_selection_marks(
        &mut self,
        snapshot: &SimulationSnapshot,
        highlights: &HashSet<ObjectId>,
        origin_x: f32,
        origin_y: f32,
        zoom: f32,
    ) {
        let Some(image) = self.hud_graphics.select_mark.as_ref() else {
            return;
        };
        let surface_width = self.surface_width as f32;
        let surface_height = self.surface_height as f32;
        let margin = (image.width().max(image.height()) as f32).max(16.0);
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

            let width = image.width() as f32;
            let height = image.height() as f32;
            let rect = GuiRect::from_origin_size(
                GuiPoint::new(screen_x - width / 2.0, screen_y - height / 2.0),
                GuiSize::new(width, height),
            );
            draw_image(&mut self.surface, &rect, image);
        }
    }

    fn layout_viewports(&self, count: usize) -> Vec<SurfaceRect> {
        if count == 0 {
            return Vec::new();
        }

        let mut overlay_height =
            (OVERLAY_HEIGHT.round() as i32).clamp(0, self.surface_height as i32);
        let mut available_height = (self.surface_height as i32).saturating_sub(overlay_height);
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
        let rows = ((count + columns - 1) / columns).max(1);

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
        let top = Self::mix_color_with_gamma(settings.fade_top, top_gamma);
        let bottom = Self::mix_color_with_gamma(settings.fade_bottom, bottom_gamma);
        self.fill_vertical_gradient(top, bottom, lighting);
    }

    fn fill_vertical_gradient(&mut self, top: Color, bottom: Color, lighting: f32) {
        if self.surface_width == 0 || self.surface_height == 0 {
            return;
        }
        let height = self.surface_height.saturating_sub(1).max(1);
        for y in 0..self.surface_height {
            let t = if height == 0 {
                0.0
            } else {
                y as f32 / height as f32
            };
            let blended = Self::lerp_color(bottom, top, t);
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
            chunk[0] = chunk[0].saturating_add(96).min(255);
            chunk[1] = chunk[1].saturating_add(96).min(255);
            chunk[2] = chunk[2].saturating_add(144).min(255);
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

    fn draw_ground(
        &mut self,
        ambient_temperature: i32,
        landscape: Option<&Landscape>,
        lighting: f32,
    ) {
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

        let screen_x = (object.position.x as f32 - self.viewport_x) * zoom;
        let screen_y = (object.position.y as f32 - self.viewport_y) * zoom;
        if screen_x < -10.0
            || screen_y < -10.0
            || screen_x > content_width + 10.0
            || screen_y > content_height + 10.0
        {
            return;
        }

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
        if sprite.is_none() {
            if base_graphics_name.is_some() {
                sprite = self
                    .object_sprites
                    .get(&sprite_map_key(&base_definition_id, None))
                    .cloned();
            }
        }
        if sprite.is_none() && base_definition_id != object.definition_id {
            sprite = self
                .object_sprites
                .get(&sprite_map_key(&object.definition_id, None))
                .cloned();
        }
        if let Some(sprite) = sprite {
            if self.draw_action_sprite(
                object,
                &sprite,
                owner_color,
                screen_x,
                screen_y,
                zoom,
                rotation_degrees,
                base_transform,
            ) {
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
            if let Some(transform) = base_transform {
                if (transform.scale_x).abs() > f32::EPSILON {
                    scale_x = transform.scale_x;
                }
                if (transform.scale_y).abs() > f32::EPSILON {
                    scale_y = transform.scale_y;
                }
                offset_x = transform.offset_x;
                offset_y = transform.offset_y;
            }
            let mut final_screen_x = screen_x + offset_x * zoom;
            let mut final_screen_y = screen_y + offset_y * zoom;
            if scale_x < 0.0 {
                flip_x = !flip_x;
                scale_x = -scale_x;
            }
            if scale_y < 0.0 {
                scale_y = -scale_y;
            }
            let sprite_width = (sprite.image.width() as f32 * zoom * scale_x).max(1.0);
            let sprite_height = (sprite.image.height() as f32 * zoom * scale_y).max(1.0);
            if rotation_degrees.abs() <= f32::EPSILON {
                let rect = GuiRect::from_origin_size(
                    GuiPoint::new(
                        final_screen_x - sprite_width / 2.0,
                        final_screen_y - sprite_height / 2.0,
                    ),
                    GuiSize::new(sprite_width, sprite_height),
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
                    sprite_width,
                    sprite_height,
                    &sprite.image,
                    sprite.color_mask.as_ref(),
                    &source_rect,
                    flip_x,
                    owner_color,
                    rotation_degrees,
                );
            }
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

    fn draw_action_sprite(
        &mut self,
        object: &ObjectSnapshot,
        sprite: &DefinitionSprite,
        owner_color: Option<Color>,
        screen_x: f32,
        screen_y: f32,
        zoom: f32,
        rotation_degrees: f32,
        transform: Option<DrawTransform>,
    ) -> bool {
        self.draw_action_graphic(
            sprite,
            object.action.name.as_str(),
            object.action.phase,
            object.direction,
            owner_color,
            screen_x,
            screen_y,
            zoom,
            rotation_degrees,
            transform,
        )
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

        let mut final_screen_x = screen_x + offset_x * zoom;
        let mut final_screen_y = screen_y + offset_y * zoom;

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
            .unwrap_or_else(|| object.action.name.as_str());
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

        let mut final_screen_x = screen_x + offset_x * zoom;
        let mut final_screen_y = screen_y + offset_y * zoom;
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

        let cursor_id = snapshot
            .players
            .iter()
            .find(|player| player.id == owner)
            .and_then(|player| player.cursor)
            .or_else(|| {
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

        let (cursor_definition_id, cursor_graphics_name) =
            if let Some(base) = object.base_graphics.as_ref() {
                (base.definition.clone(), base.graphics_name.clone())
            } else {
                (object.definition_id.clone(), None)
            };
        let sprite_height = {
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
                .map(|sprite| (sprite.image.height() as f32 * zoom).max(1.0))
                .unwrap_or(12.0 * zoom)
        };
        let cursor_width = image.width() as f32;
        let cursor_height = image.height() as f32;

        let rect = GuiRect::from_origin_size(
            GuiPoint::new(
                screen_x - cursor_width / 2.0,
                screen_y - sprite_height / 2.0 - cursor_height,
            ),
            GuiSize::new(cursor_width, cursor_height),
        );
        draw_image(&mut self.surface, &rect, &image);
    }

    fn draw_gui_overlay(&mut self) {
        self.gui
            .layout(GuiSize::new(self.surface_width as f32, OVERLAY_HEIGHT));
        for command in self.gui.render() {
            match command {
                DrawCommand::Quad { rect, color } => fill_rect(&mut self.surface, &rect, color),
                DrawCommand::Text {
                    rect,
                    text,
                    color,
                    font_size,
                    padding,
                } => draw_text(
                    &mut self.surface,
                    &rect,
                    &text,
                    color,
                    font_size,
                    padding,
                    self.font.as_ref(),
                ),
                DrawCommand::Image { rect, image } => draw_image(&mut self.surface, &rect, &image),
            }
        }
    }

    fn ensure_player_widgets(&mut self, players: &[PlayerOverlay]) -> GuiResult<()> {
        let mut structure_changed = players.len() != self.player_widgets.len();
        if !structure_changed {
            structure_changed =
                players
                    .iter()
                    .zip(self.player_widgets.iter())
                    .any(|(overlay, widgets)| {
                        overlay.owner != widgets.owner || overlay.crew.len() != widgets.crew.len()
                    });
        }
        if structure_changed {
            self.rebuild_overlay(players)?;
        }
        Ok(())
    }

    fn rebuild_overlay(&mut self, players: &[PlayerOverlay]) -> GuiResult<()> {
        let mut gui = Gui::new(self.font.clone());
        let root = gui.root();
        let scenario_label = gui.add_label(root, self.scenario_label_text.clone());
        let frame_label = gui.add_label(root, "");
        let status_label = gui.add_label(root, "");
        let energy_gauge = gui.add_gauge(root);
        gui.set_gauge_fraction(energy_gauge, 1.0)?;
        let players_container = gui.add_column(root, true);

        const CREW_PORTRAIT_SIZE: f32 = 72.0;
        const CREW_GAUGE_WIDTH: f32 = 72.0;
        const CREW_GAUGE_HEIGHT: f32 = 12.0;
        const STAT_ICON_SIZE: f32 = 20.0;

        let mut player_widgets = Vec::with_capacity(players.len());
        for player in players {
            let player_column = gui.add_column(players_container, true);
            let header_label = gui.add_label(player_column, "");
            let status_label = gui.add_label(player_column, "");
            let stats_row = gui.add_row(player_column, false);
            let wealth_icon = gui.add_picture(stats_row, STAT_ICON_SIZE, STAT_ICON_SIZE);
            let wealth_label = gui.add_label(stats_row, "");
            let score_icon = gui.add_picture(stats_row, STAT_ICON_SIZE, STAT_ICON_SIZE);
            let score_label = gui.add_label(stats_row, "");
            let crew_row = gui.add_row(player_column, false);
            let mut crew_widgets = Vec::with_capacity(player.crew.len());
            for _ in &player.crew {
                let crew_column = gui.add_column(crew_row, false);
                let portrait = gui.add_picture(crew_column, CREW_PORTRAIT_SIZE, CREW_PORTRAIT_SIZE);
                let label = gui.add_label(crew_column, "");
                let gauge = gui.add_gauge(crew_column);
                gui.set_gauge_fraction(gauge, 1.0)?;
                gui.set_gauge_size(gauge, CREW_GAUGE_WIDTH, CREW_GAUGE_HEIGHT)?;
                crew_widgets.push(CrewWidgets {
                    portrait,
                    label,
                    gauge,
                });
            }
            player_widgets.push(PlayerWidgets {
                owner: player.owner,
                header_label,
                status_label,
                wealth: PlayerStatWidgets {
                    icon: wealth_icon,
                    label: wealth_label,
                },
                score: PlayerStatWidgets {
                    icon: score_icon,
                    label: score_label,
                },
                crew: crew_widgets,
            });
        }

        self.gui = gui;
        self.scenario_label = scenario_label;
        self.frame_label = frame_label;
        self.status_label = status_label;
        self.energy_gauge = energy_gauge;
        self.players_container = players_container;
        self.player_widgets = player_widgets;
        Ok(())
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

        let overlay_height =
            (OVERLAY_HEIGHT.round() as i32).clamp(0, self.surface_height as i32) as u32;
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

            let idx = ((src_y as usize * image.width() as usize + src_x as usize) * 4) as usize;
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

            if normalized_x < 0.0 || normalized_x > 1.0 || normalized_y < 0.0 || normalized_y > 1.0
            {
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

            let idx =
                ((sample_y as usize * image.width() as usize + sample_x as usize) * 4) as usize;
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
        EnvironmentFrame, Landscape, LiquidSegment, ObjectId, ObjectVertex, PlayerState, RgbColor,
        Vector2,
    };
    use lc_graphics::{BitmapFont, PixelFormat};
    use rand::SeedableRng;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn test_font() -> Arc<dyn TextFont> {
        Arc::new(BitmapFont::new())
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
            physics: None,
            objects: vec![ObjectSnapshot {
                id: ObjectId::new(1),
                definition_id: "TestObject".to_string(),
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
                container: None,
                contents: Vec::new(),
                status: Default::default(),
                owner: 0,
                category: lc_engine::DEFAULT_CATEGORY,
                crew_member: true,
                alive: true,
                base_graphics: None,
                graphics_overlays: Vec::new(),
                draw_transform: None,
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
            rng: rand_chacha::ChaCha8Rng::seed_from_u64(0),
            surfaces: Vec::new(),
            hud: Default::default(),
            controls: Vec::new(),
            network_packets: Vec::new(),
            definition_categories: Default::default(),
            transfer_zones: Vec::new(),
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
    fn overlay_updates_clamp_energy() {
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
        graphics
            .update_overlay(&GraphicsOverlay {
                frame_text: "FRAME",
                status_text: "STATUS",
                energy_fraction: 2.5,
                players: Vec::new(),
            })
            .expect("overlay updates");
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);
        // if gauge update panicked the test would fail; no additional assertion needed here
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
    fn render_frame_draws_player_cursor() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].owner = 1;
        let object_id = snapshot.objects[0].id;
        snapshot.players.push(PlayerState {
            id: 1,
            cursor: Some(object_id),
            ..PlayerState::default()
        });

        let mut cursor_pixels = Vec::new();
        for _ in 0..4 {
            cursor_pixels.extend_from_slice(&[123, 45, 210, 255]);
        }
        let cursor_pixels = Arc::from(cursor_pixels.into_boxed_slice());
        let cursor_image = ImageData::from_arc(2, 2, cursor_pixels);
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

        let cursor_color = [123u8, 45, 210, 255];
        let mut found = false;
        for chunk in graphics.surface().pixels().chunks_exact(4) {
            if chunk == cursor_color {
                found = true;
                break;
            }
        }
        assert!(
            found,
            "expected to find player cursor color in surface pixels"
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
        nighttime.environment.settings.time_of_day = 0;
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
        nighttime.environment.settings.time_of_day = 0;
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
