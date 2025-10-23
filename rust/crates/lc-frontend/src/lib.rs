mod input;
mod startup_menu;

use lc_engine::{
    EnvironmentFrame, EnvironmentSettings, Landscape, ObjectId, ObjectSnapshot, RgbColor,
    SimulationSnapshot, SkyFrame, SkySettings, SurfaceSnapshot as EngineSurfaceSnapshot, Vector2,
    WeatherEvent,
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
pub use startup_menu::{ScenarioSummary, StartupMenu, StartupMenuAction};

const OVERLAY_HEIGHT: f32 = 120.0;
const MIN_VIEWPORT_ZOOM: f32 = 0.125;
const MAX_VIEWPORT_ZOOM: f32 = 4.0;
const CAMERA_SMOOTHING_ALPHA: f32 = 0.2;
const CAMERA_SNAP_THRESHOLD: f32 = 1.0;
const CAMERA_JUMP_THRESHOLD: f32 = 256.0;

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
    pub cursor: Option<ObjectId>,
    pub eliminated: bool,
    pub crew: Vec<CrewOverlay>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CrewOverlay {
    pub label: String,
    pub energy_fraction: f32,
    pub is_focus: bool,
}

#[derive(Debug)]
struct PlayerWidgets {
    owner: i32,
    header_label: WidgetId,
    status_label: WidgetId,
    crew: Vec<CrewWidgets>,
}

#[derive(Debug)]
struct CrewWidgets {
    label: WidgetId,
    gauge: WidgetId,
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
    object_sprites: Arc<HashMap<String, ImageData>>,
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
        object_sprites: Arc<HashMap<String, ImageData>>,
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
            active_viewports: Vec::new(),
            camera_states: HashMap::new(),
            sky: None,
        }
    }

    pub fn set_object_sprites(&mut self, sprites: Arc<HashMap<String, ImageData>>) {
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

        for (player_overlay, widgets) in overlay.players.iter().zip(self.player_widgets.iter()) {
            let header_text = if player_overlay.name.is_empty() {
                format!("Player {}", player_overlay.owner)
            } else {
                player_overlay.name.clone()
            };
            self.gui.set_label_text(widgets.header_label, header_text)?;
            self.gui
                .set_label_color(widgets.header_label, header_color)?;

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
                let wealth_text = format!("Wealth {}", player_overlay.wealth);
                self.gui.set_label_text(widgets.status_label, wealth_text)?;
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

        let mut used_camera_keys = Vec::new();
        self.render_viewports(snapshot, viewports, &mut used_camera_keys);
        let used_keys: HashSet<_> = used_camera_keys.into_iter().collect();
        self.camera_states.retain(|key, _| used_keys.contains(key));

        self.draw_gui_overlay();

        self.collect_sprite_atlas(snapshot)
    }

    fn render_viewports(
        &mut self,
        snapshot: &SimulationSnapshot,
        viewports: &[ViewportInput<'_>],
        used_camera_keys: &mut Vec<CameraKey>,
    ) {
        if viewports.is_empty() {
            if let Some(object) = snapshot.objects.first() {
                let default = ViewportInput::from_focus(object);
                self.render_viewport(
                    snapshot,
                    &default,
                    SurfaceRect::new(0, 0, self.surface_width, self.surface_height),
                    used_camera_keys,
                );
            }
            return;
        }

        let layout = self.layout_viewports(viewports.len());
        for (input, rect) in viewports.iter().zip(layout.into_iter()) {
            self.render_viewport(snapshot, input, rect, used_camera_keys);
        }
    }

    fn render_viewport(
        &mut self,
        snapshot: &SimulationSnapshot,
        input: &ViewportInput<'_>,
        rect: SurfaceRect,
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
        self.draw_precipitation(
            environment.precipitation,
            environment.ambient_temperature,
            snapshot.frame,
            lighting,
        );
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
        self.draw_objects(&snapshot.objects, lighting);

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

    fn layout_viewports(&self, count: usize) -> Vec<SurfaceRect> {
        if count == 0 {
            return Vec::new();
        }

        let overlay_height = (OVERLAY_HEIGHT.round() as i32).clamp(0, self.surface_height as i32);
        let available_height = (self.surface_height as i32).saturating_sub(overlay_height);
        if available_height <= 0 {
            return vec![SurfaceRect::new(0, overlay_height, self.surface_width, 0)];
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
                    let margin = 2i32;
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

    fn draw_objects(&mut self, objects: &[ObjectSnapshot], lighting: f32) {
        for object in objects {
            self.paint_object(object, lighting);
        }
    }

    fn paint_object(&mut self, object: &ObjectSnapshot, lighting: f32) {
        let zoom = self.viewport_zoom.max(MIN_VIEWPORT_ZOOM);
        let content_width = self.surface_width as f32;
        let content_height = self.surface_height as f32;
        let color = object_color(object).modulate(lighting);
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

        if let Some(sprite) = self.object_sprites.get(&object.definition_id) {
            let sprite_width = (sprite.width() as f32 * zoom).max(1.0);
            let sprite_height = (sprite.height() as f32 * zoom).max(1.0);
            let rect = GuiRect::from_origin_size(
                GuiPoint::new(
                    screen_x - sprite_width / 2.0,
                    screen_y - sprite_height / 2.0,
                ),
                GuiSize::new(sprite_width, sprite_height),
            );
            draw_image(&mut self.surface, &rect, sprite);
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

        let mut player_widgets = Vec::with_capacity(players.len());
        for player in players {
            let player_column = gui.add_column(players_container, true);
            let header_label = gui.add_label(player_column, "");
            let status_label = gui.add_label(player_column, "");
            let crew_row = gui.add_row(player_column, false);
            let mut crew_widgets = Vec::with_capacity(player.crew.len());
            for _ in &player.crew {
                let crew_column = gui.add_column(crew_row, false);
                let label = gui.add_label(crew_column, "");
                let gauge = gui.add_gauge(crew_column);
                gui.set_gauge_fraction(gauge, 1.0)?;
                crew_widgets.push(CrewWidgets { label, gauge });
            }
            player_widgets.push(PlayerWidgets {
                owner: player.owner,
                header_label,
                status_label,
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
                (
                    viewport.viewport_x.round() as i32,
                    viewport.viewport_y.round() as i32,
                )
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

#[cfg(test)]
mod tests {
    use super::*;
    use lc_engine::{
        EnvironmentFrame, Landscape, LiquidSegment, ObjectId, ObjectVertex, RgbColor, Vector2,
    };
    use lc_graphics::{BitmapFont, PixelFormat};
    use rand::SeedableRng;
    use std::collections::HashMap;
    use std::sync::Arc;

    fn test_font() -> Arc<dyn TextFont> {
        Arc::new(BitmapFont::new())
    }

    fn empty_sprites() -> Arc<HashMap<String, ImageData>> {
        Arc::new(HashMap::new())
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
                energy: 100,
                damage: 0,
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
    fn graphics_system_draws_ground() {
        let snapshot = make_snapshot();
        let focus = &snapshot.objects[0];
        let mut graphics =
            GraphicsSystem::new(320, 180, 150, "Test Scenario", test_font(), empty_sprites());
        graphics.set_world_width(256);

        let viewports = vec![ViewportInput::from_focus(focus)];
        let atlas = graphics.render_frame(&snapshot, &viewports);
        assert!(!atlas.is_empty());

        let ground = graphics.surface().get_pixel(0, 179).unwrap();
        assert_ne!(ground, Color::opaque(8, 12, 24));
    }

    #[test]
    fn overlay_updates_clamp_energy() {
        let snapshot = make_snapshot();
        let focus = &snapshot.objects[0];
        let mut graphics =
            GraphicsSystem::new(320, 180, 150, "Test Scenario", test_font(), empty_sprites());
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
        let mut graphics =
            GraphicsSystem::new(120, 80, 60, "Atlas Scenario", test_font(), empty_sprites());

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
        let mut graphics =
            GraphicsSystem::new(320, 180, 150, "Test Scenario", test_font(), empty_sprites());
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
        let mut graphics =
            GraphicsSystem::new(320, 180, 150, "Test Scenario", test_font(), empty_sprites());
        let focus = &snapshot.objects[0];
        let viewports = vec![ViewportInput::from_focus(focus)];
        graphics.render_frame(&snapshot, &viewports);
        let (_, top_view) = graphics.viewport();
        assert_eq!(top_view, 0);

        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(100, 360);
        snapshot.landscape = Some(Landscape::flat(256, 360));
        let mut graphics =
            GraphicsSystem::new(320, 180, 150, "Test Scenario", test_font(), empty_sprites());
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

        let mut graphics =
            GraphicsSystem::new(80, 60, 60, "Polygon Scenario", test_font(), empty_sprites());
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
    fn lighting_darkens_sky_at_night() {
        let mut daytime = make_snapshot();
        daytime.environment.sky_color = Some(RgbColor::new(160, 160, 160));
        daytime.environment.settings.time_of_day = EnvironmentSettings::TIME_CYCLE / 2;

        let focus = &daytime.objects[0];
        let mut day_view = GraphicsSystem::new(120, 80, 60, "Day", test_font(), empty_sprites());
        let day_viewports = vec![ViewportInput::from_focus(focus)];
        day_view.render_frame(&daytime, &day_viewports);
        let day_pixel = day_view.surface().get_pixel(0, 0).unwrap();

        let mut nighttime = daytime.clone();
        nighttime.environment.settings.time_of_day = 0;
        let mut night_view =
            GraphicsSystem::new(120, 80, 60, "Night", test_font(), empty_sprites());
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

        let mut day_view =
            GraphicsSystem::new(200, 150, 150, "Day Object", test_font(), empty_sprites());
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
        let mut night_view =
            GraphicsSystem::new(200, 150, 150, "Night Object", test_font(), empty_sprites());
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
        let mut graphics =
            GraphicsSystem::new(120, 80, 40, "Letterbox", test_font(), empty_sprites());
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
        let mut graphics =
            GraphicsSystem::new(120, 80, 80, "Liquid Scenario", test_font(), empty_sprites());
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
