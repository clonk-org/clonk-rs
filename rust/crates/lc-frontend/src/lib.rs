mod input;
mod startup_menu;

use lc_engine::{
    EnvironmentSettings, Landscape, ObjectSnapshot, SimulationSnapshot,
    SurfaceSnapshot as EngineSurfaceSnapshot,
};
use lc_graphics::{
    Color, Rect as SurfaceRect, Surface, SurfaceSnapshot as GraphicsSurfaceSnapshot,
};
use lc_gui::{DrawCommand, Gui, GuiResult, Rect as GuiRect, Size as GuiSize, WidgetId};
use std::collections::hash_map::DefaultHasher;
use std::convert::TryFrom;
use std::hash::{Hash, Hasher};

pub use input::InputDispatcher;
pub use lc_gui::{
    GuiError as StartupMenuError, GuiResult as StartupMenuResult, KeyCode, Point as GuiPoint,
    ScenarioEntry, ScenarioKind,
};
pub use startup_menu::{ScenarioSummary, StartupMenu, StartupMenuAction};

const OVERLAY_HEIGHT: f32 = 120.0;

pub struct GraphicsOverlay<'a> {
    pub frame_text: &'a str,
    pub status_text: &'a str,
    pub energy_fraction: f32,
}

pub struct GraphicsSystem {
    surface: Surface,
    gui: Gui,
    frame_label: WidgetId,
    status_label: WidgetId,
    energy_gauge: WidgetId,
    viewport_x: i32,
    viewport_y: i32,
    surface_width: u32,
    surface_height: u32,
    fallback_ground_height: i32,
    world_width: i32,
    world_height: i32,
}

impl GraphicsSystem {
    pub fn new(
        surface_width: u32,
        surface_height: u32,
        fallback_ground_height: i32,
        scenario_label: &str,
    ) -> Self {
        let mut gui = Gui::new();
        let root = gui.root();
        gui.add_label(root, scenario_label);
        let frame_label = gui.add_label(root, "FRAME 000 X 000 Y 000 VX P00 VY P00");
        let status_label = gui.add_label(root, "READY 00OF00 GROUND 00 BATCH 000");
        let energy_gauge = gui.add_gauge(root);
        gui.set_gauge_fraction(energy_gauge, 1.0)
            .expect("initial gauge");

        let mut surface = Surface::new(
            surface_width,
            surface_height,
            lc_graphics::PixelFormat::Rgba8888,
        );
        surface.fill(Color::opaque(8, 12, 24));

        Self {
            surface,
            gui,
            frame_label,
            status_label,
            energy_gauge,
            viewport_x: 0,
            viewport_y: 0,
            surface_width,
            surface_height,
            fallback_ground_height,
            world_width: surface_width as i32,
            world_height: fallback_ground_height.max(surface_height as i32).max(0),
        }
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

    pub fn surface(&self) -> &Surface {
        &self.surface
    }

    pub fn surface_mut(&mut self) -> &mut Surface {
        &mut self.surface
    }

    pub fn update_overlay(&mut self, overlay: &GraphicsOverlay<'_>) -> GuiResult<()> {
        self.gui
            .set_label_text(self.frame_label, overlay.frame_text)?;
        self.gui
            .set_label_text(self.status_label, overlay.status_text)?;
        self.gui
            .set_gauge_fraction(self.energy_gauge, overlay.energy_fraction.clamp(0.0, 1.0))?;
        Ok(())
    }

    pub fn render_frame(
        &mut self,
        snapshot: &SimulationSnapshot,
        focus: &ObjectSnapshot,
    ) -> Vec<EngineSurfaceSnapshot> {
        self.update_world_dimensions(snapshot.landscape.as_ref());
        self.update_viewport(focus);

        let environment = &snapshot.environment;
        let lighting = Self::lighting_factor(environment.settings.time_of_day);
        let sky = environment
            .sky_color
            .map(|color| Color::opaque(color.r, color.g, color.b))
            .unwrap_or_else(|| Self::sky_color_for_temperature(environment.ambient_temperature));
        let sky = Self::apply_lighting(sky, lighting);

        self.surface.fill(sky);
        self.draw_precipitation(environment.precipitation, snapshot.frame, lighting);
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
        self.draw_gui_overlay();

        self.collect_sprite_atlas(snapshot, focus)
    }

    pub fn ground_height_at(&self, landscape: Option<&Landscape>, x: i32) -> i32 {
        self.surface_height_at(landscape, x)
            .unwrap_or(self.fallback_ground_height)
    }

    fn update_viewport(&mut self, focus: &ObjectSnapshot) {
        let half_width = (self.surface_width / 2) as i32;
        let half_height = (self.surface_height / 2) as i32;

        let max_offset_x = (self.world_width - self.surface_width as i32).max(0);
        let max_offset_y = (self.world_height - self.surface_height as i32).max(0);

        let desired_x = (focus.position.x - half_width).clamp(0, max_offset_x);
        let desired_y = (focus.position.y - half_height).clamp(0, max_offset_y);

        self.viewport_x = desired_x;
        self.viewport_y = desired_y;
    }

    fn update_world_dimensions(&mut self, landscape: Option<&Landscape>) {
        if let Some(landscape) = landscape {
            let width = landscape.width() as i32;
            if width > 0 {
                self.world_width = width.max(self.surface_width as i32);
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
            self.world_height = max_surface_height.max(self.surface_height as i32);
        }
    }

    fn draw_precipitation(&mut self, precipitation: i32, frame: u64, lighting: f32) {
        if precipitation == 0 {
            return;
        }

        if precipitation > 0 {
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
        for screen_x in 0..self.surface_width {
            let world_x = self.viewport_x + screen_x as i32;
            let ground_world = self.ground_height_at(landscape, world_x);
            let mut ground_screen = ground_world - self.viewport_y;
            if ground_screen < 0 {
                ground_screen = 0;
            }
            if ground_screen >= self.surface_height as i32 {
                continue;
            }
            let ground_screen = ground_screen as u32;
            for y in ground_screen..self.surface_height {
                let _ = self.surface.set_pixel(screen_x, y, ground_color);
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

        let surface_width = self.surface_width as i32;
        let surface_height = self.surface_height as i32;

        for (world_x, column) in landscape.liquids().iter().enumerate() {
            if column.segments().is_empty() {
                continue;
            }

            let screen_x = world_x as i32 - self.viewport_x;
            if screen_x < 0 || screen_x >= surface_width {
                continue;
            }

            for segment in column.segments() {
                let mut start = segment.top - self.viewport_y;
                let mut end = segment.bottom - self.viewport_y;
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
        let color = object_color(object).modulate(lighting);
        if object.vertices.len() >= 3 {
            let mut points = Vec::with_capacity(object.vertices.len());
            let mut min_x = i32::MAX;
            let mut max_x = i32::MIN;
            let mut min_y = i32::MAX;
            let mut max_y = i32::MIN;
            for vertex in &object.vertices {
                let x = object.position.x + vertex.x - self.viewport_x;
                let y = object.position.y + vertex.y - self.viewport_y;
                points.push((x, y));
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }

            if max_x >= 0
                && min_x < self.surface_width as i32
                && max_y >= 0
                && min_y < self.surface_height as i32
                && fill_polygon(&mut self.surface, &points, color)
            {
                return;
            }
        }

        let screen_x = object.position.x - self.viewport_x;
        let screen_y = object.position.y - self.viewport_y;
        if screen_x < -10
            || screen_y < -10
            || screen_x > self.surface_width as i32 + 10
            || screen_y > self.surface_height as i32 + 10
        {
            return;
        }

        let size = 6i32;
        let rect = GuiRect::from_origin_size(
            GuiPoint::new(
                (screen_x - size / 2).max(0) as f32,
                (screen_y - size / 2).max(0) as f32,
            ),
            GuiSize::new(size as f32, size as f32),
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
                } => draw_text(&mut self.surface, &rect, &text, color, font_size, padding),
            }
        }
    }

    #[cfg(test)]
    pub fn viewport(&self) -> (i32, i32) {
        (self.viewport_x, self.viewport_y)
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

    fn collect_sprite_atlas(
        &self,
        snapshot: &SimulationSnapshot,
        focus: &ObjectSnapshot,
    ) -> Vec<EngineSurfaceSnapshot> {
        let mut atlas = Vec::with_capacity(
            2 + snapshot
                .objects
                .len()
                .saturating_add(snapshot.hud.players.len()),
        );

        let full_snapshot = self.surface.snapshot();
        atlas.push(Self::make_engine_surface(
            "back_buffer".to_string(),
            full_snapshot,
        ));

        let player_label = if focus.owner < 0 {
            "none".to_string()
        } else {
            focus.owner.to_string()
        };
        let viewport_label = format!("viewport#0:player={player_label}");
        atlas.push(Self::make_engine_surface(viewport_label, full_snapshot));

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

        for player in &snapshot.hud.players {
            if player.eliminated {
                continue;
            }
            if let Some(focus_object) = player.focus {
                if let Some(object) = snapshot
                    .objects
                    .iter()
                    .find(|object| object.id == focus_object)
                {
                    if let Some(rect) = self.object_screen_rect(object) {
                        if let Some(snap) = self.surface.snapshot_region(rect) {
                            let label =
                                format!("focus#{}:player={}", object.id.as_u64(), player.owner);
                            atlas.push(Self::make_engine_surface(label, snap));
                        }
                    }
                }
            }
        }

        for object in &snapshot.objects {
            if let Some(rect) = self.object_screen_rect(object) {
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

    fn object_screen_rect(&self, object: &ObjectSnapshot) -> Option<SurfaceRect> {
        if !object.status.is_active() || !object.alive {
            return None;
        }

        if object.vertices.is_empty() {
            let screen_x = object.position.x - self.viewport_x;
            let screen_y = object.position.y - self.viewport_y;
            let size = 6;
            let half = size / 2;
            return Some(SurfaceRect::new(
                screen_x - half,
                screen_y - half,
                size as u32,
                size as u32,
            ));
        }

        let mut min_x = i32::MAX;
        let mut max_x = i32::MIN;
        let mut min_y = i32::MAX;
        let mut max_y = i32::MIN;
        for vertex in &object.vertices {
            let x = object.position.x + vertex.x - self.viewport_x;
            let y = object.position.y + vertex.y - self.viewport_y;
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }

        if min_x > max_x || min_y > max_y {
            return None;
        }

        let padding = 2i64;
        let left = (min_x as i64 - padding).clamp(i32::MIN as i64, i32::MAX as i64);
        let top = (min_y as i64 - padding).clamp(i32::MIN as i64, i32::MAX as i64);
        let right = (max_x as i64 + padding).clamp(i32::MIN as i64, i32::MAX as i64);
        let bottom = (max_y as i64 + padding).clamp(i32::MIN as i64, i32::MAX as i64);

        if right < left || bottom < top {
            return None;
        }

        let width = (right - left + 1).max(1) as u32;
        let height = (bottom - top + 1).max(1) as u32;

        Some(SurfaceRect::new(left as i32, top as i32, width, height))
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

pub(crate) fn draw_text(
    surface: &mut Surface,
    rect: &GuiRect,
    text: &str,
    color: Color,
    font_size: f32,
    padding: f32,
) {
    let origin_x = rect.origin.x + padding;
    let origin_y = rect.origin.y + padding;
    let font = lc_graphics::BitmapFont::new();
    font.draw_text(surface, origin_x, origin_y, text, font_size.max(1.0), color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_engine::{
        EnvironmentFrame, Landscape, LiquidSegment, ObjectId, ObjectVertex, RgbColor, Vector2,
    };
    use lc_graphics::PixelFormat;
    use rand::SeedableRng;

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
            global_effects: Vec::new(),
            particles: Vec::new(),
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
        }
    }

    #[test]
    fn graphics_system_draws_ground() {
        let snapshot = make_snapshot();
        let focus = snapshot.objects[0].clone();
        let mut graphics = GraphicsSystem::new(320, 180, 150, "Test Scenario");
        graphics.set_world_width(256);

        let atlas = graphics.render_frame(&snapshot, &focus);
        assert!(!atlas.is_empty());

        let ground = graphics.surface().get_pixel(0, 179).unwrap();
        assert_ne!(ground, Color::opaque(8, 12, 24));
    }

    #[test]
    fn overlay_updates_clamp_energy() {
        let snapshot = make_snapshot();
        let focus = snapshot.objects[0].clone();
        let mut graphics = GraphicsSystem::new(320, 180, 150, "Test Scenario");
        graphics
            .update_overlay(&GraphicsOverlay {
                frame_text: "FRAME",
                status_text: "STATUS",
                energy_fraction: 2.5,
            })
            .expect("overlay updates");
        graphics.render_frame(&snapshot, &focus);
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
        let focus = snapshot.objects[0].clone();
        let mut graphics = GraphicsSystem::new(120, 80, 60, "Atlas Scenario");

        let atlas = graphics.render_frame(&snapshot, &focus);

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
        let mut graphics = GraphicsSystem::new(320, 180, 150, "Test Scenario");
        let focus = snapshot.objects[0].clone();
        graphics.render_frame(&snapshot, &focus);

        let (_, viewport_y) = graphics.viewport();
        assert!(viewport_y > 0);
    }

    #[test]
    fn viewport_clamps_to_world_height() {
        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(100, 30);
        snapshot.landscape = Some(Landscape::flat(256, 200));
        let mut graphics = GraphicsSystem::new(320, 180, 150, "Test Scenario");
        let focus = snapshot.objects[0].clone();
        graphics.render_frame(&snapshot, &focus);
        let (_, top_view) = graphics.viewport();
        assert_eq!(top_view, 0);

        let mut snapshot = make_snapshot();
        snapshot.objects[0].position = Vector2::new(100, 360);
        snapshot.landscape = Some(Landscape::flat(256, 360));
        let mut graphics = GraphicsSystem::new(320, 180, 150, "Test Scenario");
        let focus = snapshot.objects[0].clone();
        graphics.render_frame(&snapshot, &focus);
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

        let mut graphics = GraphicsSystem::new(80, 60, 60, "Polygon Scenario");
        let focus = snapshot.objects[0].clone();
        graphics.render_frame(&snapshot, &focus);

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

        let focus = daytime.objects[0].clone();
        let mut day_view = GraphicsSystem::new(120, 80, 60, "Day");
        day_view.render_frame(&daytime, &focus);
        let day_pixel = day_view.surface().get_pixel(0, 0).unwrap();

        let mut nighttime = daytime.clone();
        nighttime.environment.settings.time_of_day = 0;
        let mut night_view = GraphicsSystem::new(120, 80, 60, "Night");
        let night_focus = nighttime.objects[0].clone();
        night_view.render_frame(&nighttime, &night_focus);
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

        let mut day_view = GraphicsSystem::new(200, 150, 150, "Day Object");
        let day_focus = daytime.objects[0].clone();
        day_view.render_frame(&daytime, &day_focus);
        let (day_viewport_x, day_viewport_y) = day_view.viewport();
        let day_screen_x = (daytime.objects[0].position.x - day_viewport_x) as u32;
        let day_screen_y = (daytime.objects[0].position.y - day_viewport_y) as u32;
        let day_pixel = day_view
            .surface()
            .get_pixel(day_screen_x, day_screen_y)
            .unwrap();

        let mut nighttime = daytime.clone();
        nighttime.environment.settings.time_of_day = 0;
        let mut night_view = GraphicsSystem::new(200, 150, 150, "Night Object");
        let night_focus = nighttime.objects[0].clone();
        night_view.render_frame(&nighttime, &night_focus);
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
    fn liquids_overlay_ground_with_blending() {
        let mut snapshot = make_snapshot();
        snapshot.environment.settings.time_of_day = EnvironmentSettings::TIME_CYCLE / 2;
        snapshot.objects[0].position = Vector2::new(40, 50);
        if let Some(landscape) = snapshot.landscape.as_mut() {
            landscape.set_liquid_column(30, vec![LiquidSegment::new(40, 60)]);
        }
        let focus = snapshot.objects[0].clone();
        let mut graphics = GraphicsSystem::new(120, 80, 80, "Liquid Scenario");
        graphics.render_frame(&snapshot, &focus);

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
