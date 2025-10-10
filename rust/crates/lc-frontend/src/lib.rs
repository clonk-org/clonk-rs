use lc_engine::{Landscape, ObjectSnapshot, SimulationSnapshot};
use lc_graphics::{Color, Surface};
use lc_gui::{
    DrawCommand, Gui, GuiResult, Point as GuiPoint, Rect as GuiRect, Size as GuiSize, WidgetId,
};

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
        }
    }

    pub fn set_world_width(&mut self, world_width: i32) {
        self.world_width = world_width.max(self.surface_width as i32);
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

    pub fn render_frame(&mut self, snapshot: &SimulationSnapshot, focus: &ObjectSnapshot) {
        self.update_viewport(focus);

        let environment = &snapshot.environment;
        let sky = environment
            .sky_color
            .map(|color| Color::opaque(color.r, color.g, color.b))
            .unwrap_or_else(|| Self::sky_color_for_temperature(environment.ambient_temperature));

        self.surface.fill(sky);
        self.draw_precipitation(environment.precipitation, snapshot.frame);
        self.draw_ground(environment.ambient_temperature, snapshot.landscape.as_ref());
        self.draw_objects(&snapshot.objects);
        self.draw_gui_overlay();
    }

    pub fn ground_height_at(&self, landscape: Option<&Landscape>, x: i32) -> i32 {
        self.surface_height_at(landscape, x)
            .unwrap_or(self.fallback_ground_height)
    }

    fn update_viewport(&mut self, focus: &ObjectSnapshot) {
        let half_width = (self.surface_width / 2) as i32;
        let mut desired = focus.position.x - half_width;
        if desired < 0 {
            desired = 0;
        }
        let max_offset = (self.world_width - self.surface_width as i32).max(0);
        if desired > max_offset {
            desired = max_offset;
        }
        self.viewport_x = desired;
    }

    fn draw_precipitation(&mut self, precipitation: i32, frame: u64) {
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
                    let color = Color::new(148, 176, 220, 160);
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
                };
                let _ = self.surface.set_pixel(x, y, color);
            }
        }
    }

    fn draw_ground(&mut self, ambient_temperature: i32, landscape: Option<&Landscape>) {
        let ground_color = Self::ground_color_for_temperature(ambient_temperature);
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

    fn draw_objects(&mut self, objects: &[ObjectSnapshot]) {
        for object in objects {
            self.paint_object(object);
        }
    }

    fn paint_object(&mut self, object: &ObjectSnapshot) {
        let screen_x = object.position.x - self.viewport_x;
        let screen_y = object.position.y - self.viewport_y;
        if screen_x < -10
            || screen_y < -10
            || screen_x > self.surface_width as i32 + 10
            || screen_y > self.surface_height as i32 + 10
        {
            return;
        }
        let energy = object.energy.max(0).min(100) as u8;
        let color = if energy > 50 {
            Color::opaque(252, 196, 64)
        } else {
            Color::opaque(220, 72, 72)
        };
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
                DrawCommand::Text { rect, text, color } => {
                    draw_text(&mut self.surface, &rect, &text, color)
                }
            }
        }
    }

    fn surface_height_at(&self, landscape: Option<&Landscape>, x: i32) -> Option<i32> {
        landscape.and_then(|landscape| landscape.surface_height(x))
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

fn fill_rect(surface: &mut Surface, rect: &GuiRect, color: Color) {
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

fn draw_text(surface: &mut Surface, rect: &GuiRect, text: &str, color: Color) {
    let mut cursor_x = rect.origin.x;
    let baseline = rect.origin.y;
    let glyph_width = 6.0f32;
    let glyph_height = rect.size.height.clamp(6.0, 14.0);

    for ch in text.chars() {
        if cursor_x > surface.width() as f32 {
            break;
        }
        if ch == ' ' {
            cursor_x += glyph_width;
            continue;
        }
        let intensity = ((ch as u32).wrapping_mul(17) % 80) as u8;
        let glyph_color = Color::new(
            color.r.saturating_add(intensity / 2),
            color.g.saturating_add(intensity / 3),
            color.b.saturating_add(intensity / 4),
            255,
        );
        let glyph_rect = GuiRect::from_origin_size(
            GuiPoint::new(cursor_x, baseline),
            GuiSize::new(glyph_width - 1.0, glyph_height),
        );
        fill_rect(surface, &glyph_rect, glyph_color);
        cursor_x += glyph_width;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_engine::{EnvironmentFrame, Landscape, ObjectId, Vector2};
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
        }
    }

    #[test]
    fn graphics_system_draws_ground() {
        let snapshot = make_snapshot();
        let focus = &snapshot.objects[0];
        let mut graphics = GraphicsSystem::new(320, 180, 150, "Test Scenario");
        graphics.set_world_width(256);

        graphics.render_frame(&snapshot, focus);

        let ground = graphics.surface().get_pixel(0, 179).unwrap();
        assert_ne!(ground, Color::opaque(8, 12, 24));
    }

    #[test]
    fn overlay_updates_clamp_energy() {
        let snapshot = make_snapshot();
        let focus = &snapshot.objects[0];
        let mut graphics = GraphicsSystem::new(320, 180, 150, "Test Scenario");
        graphics
            .update_overlay(&GraphicsOverlay {
                frame_text: "FRAME",
                status_text: "STATUS",
                energy_fraction: 2.5,
            })
            .expect("overlay updates");
        graphics.render_frame(&snapshot, focus);
        // if gauge update panicked the test would fail; no additional assertion needed here
    }
}
