use std::f32::consts::PI;
use std::io::Cursor;
use std::path::PathBuf;
use std::str::FromStr;

use lc_audio::{AudioSystem, SoundHandle};
use lc_core::std_config::Config;
use lc_engine::{
    Definition, Engine, EngineError, Landscape, ObjectId, ObjectSnapshot, SimulationSnapshot,
    SpawnConfig, Vector2,
};
use lc_graphics::{Color, PixelFormat, SnapshotHasher, Surface};
use lc_gui::{
    DrawCommand, Gui, GuiError, Point as GuiPoint, Rect as GuiRect, Size as GuiSize, WidgetId,
};
use lc_network::{ClientId, ControlCoordinator, ControlError, ControlPacket, Lobby, LobbyError};
use lc_platform::{AppPaths, PathsError};
use lc_resources::{Group, GroupError};

const DEMO_CONFIG_BYTES: &[u8] = include_bytes!("demo_config.cfg");
const DEMO_SCRIPT: &str = include_str!("demo_script.aul");
const LOCAL_CLIENT_ID: ClientId = 1;
const SAMPLE_RATE: u32 = 44_100;
const SURFACE_WIDTH: u32 = 640;
const SURFACE_HEIGHT: u32 = 360;
const MIX_CHANNELS: usize = 8;

pub type GameResult<T> = Result<T, GameError>;

#[derive(Debug, thiserror::Error)]
pub enum GameError {
    #[error("configuration error: {0}")]
    Config(#[from] std::io::Error),
    #[error("platform error: {0}")]
    Platform(#[from] PathsError),
    #[error("engine error: {0}")]
    Engine(#[from] EngineError),
    #[error("audio error: {0}")]
    Audio(String),
    #[error("resource error: {0}")]
    Resources(#[from] GroupError),
    #[error("gui error: {0}")]
    Gui(#[from] GuiError),
    #[error("network error: {0}")]
    Network(#[from] ControlError),
    #[error("lobby error: {0}")]
    Lobby(#[from] LobbyError),
}

pub struct DemoGame {
    engine: Engine,
    paths: AppPaths,
    object_id: ObjectId,
    ground_height: i32,
    configured_ticks: u32,
    scenario_name: String,
    scenario_label: String,
    system_version: String,
    system_entry_count: usize,
    surface: Surface,
    gui: Gui,
    title_label: WidgetId,
    frame_label: WidgetId,
    status_label: WidgetId,
    audio: AudioSystem,
    bounce_sound: SoundHandle,
    control: ControlCoordinator,
    lobby: Lobby,
}

#[derive(Debug, Clone)]
pub struct GameSummary {
    pub ticks: u32,
    pub ground_hits: u32,
    pub ready_batches: u32,
    pub surface_hash: u64,
    pub final_snapshot: SimulationSnapshot,
    pub scenario_name: String,
    pub system_version: String,
    pub system_entry_count: usize,
    pub install_root: PathBuf,
    pub user_data_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub cache_dir: PathBuf,
}

impl DemoGame {
    pub fn new() -> GameResult<Self> {
        let config = load_demo_config()?;
        let configured_ticks = parse_config_value::<u32>(&config, "ticks").unwrap_or(180);
        let ground_height = parse_config_value::<i32>(&config, "ground_height").unwrap_or(220);
        let spawn_x = parse_config_value::<i32>(&config, "spawn_x").unwrap_or(64);
        let spawn_y = parse_config_value::<i32>(&config, "spawn_y").unwrap_or(48);
        let spawn_vx = parse_config_value::<i32>(&config, "spawn_velocity_x").unwrap_or(3);
        let spawn_vy = parse_config_value::<i32>(&config, "spawn_velocity_y").unwrap_or(0);
        let scenario_name = config
            .get_in(Some("Game"), "scenario_name")
            .map(|value| value.to_string())
            .unwrap_or_else(|| "Rust Demo Bounce".to_string());
        let scenario_label = format!("SCENARIO {}", sanitize_label(&scenario_name));

        let paths = AppPaths::discover()?;
        let mut engine = Engine::with_seed(12345);
        let definition = Definition::from_script("DemoBouncer", "Demo Bouncer", DEMO_SCRIPT)?;
        engine.register_definition(definition)?;
        engine.set_landscape(Landscape::flat(2048, ground_height));
        let object_id = engine.spawn_object(
            SpawnConfig::new("DemoBouncer")
                .with_position(Vector2::new(spawn_x, spawn_y))
                .with_velocity(Vector2::new(spawn_vx, spawn_vy))
                .with_energy(100),
        )?;

        let mut gui = Gui::new();
        let root = gui.root();
        let title_label = gui.add_label(root, &scenario_label);
        let frame_label = gui.add_label(root, "FRAME 000 X 000 Y 000 VX P00 VY P00");
        let status_label = gui.add_label(root, "READY 00OF00 GROUND 00 BATCH 000");
        gui.layout(GuiSize::new(SURFACE_WIDTH as f32, 96.0));

        let audio =
            AudioSystem::new(MIX_CHANNELS).map_err(|err| GameError::Audio(err.to_string()))?;
        let bounce_sound = audio
            .load_sound(&generate_bounce_wav(0.25, 660.0, SAMPLE_RATE))
            .map_err(|err| GameError::Audio(err.to_string()))?;

        let mut control = ControlCoordinator::with_start_tick(32, 0);
        control.register_client(LOCAL_CLIENT_ID)?;

        let mut lobby = Lobby::new(4);
        lobby.join_player(LOCAL_CLIENT_ID, "PlayerOne")?;
        lobby.join_observer(2, "Observer")?;
        lobby.set_ready(LOCAL_CLIENT_ID, true)?;
        lobby.set_ready(2, true)?;
        lobby.settings_mut().scenario = Some(scenario_name.clone());
        lobby.settings_mut().script_hash = Some("demo".to_string());

        let system_group = Group::open(paths.system_group_path())?;
        let system_entry_count = system_group.entries()?.len();
        let system_version = system_group
            .read_file("Version.txt")
            .ok()
            .and_then(|bytes| {
                let text = String::from_utf8_lossy(&bytes);
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                }
            })
            .unwrap_or_else(|| "Unknown".to_string());

        let mut surface = Surface::new(SURFACE_WIDTH, SURFACE_HEIGHT, PixelFormat::Rgba8888);
        surface.fill(Color::opaque(8, 12, 24));

        Ok(Self {
            engine,
            paths,
            object_id,
            ground_height,
            configured_ticks,
            scenario_name,
            scenario_label,
            system_version,
            system_entry_count,
            surface,
            gui,
            title_label,
            frame_label,
            status_label,
            audio,
            bounce_sound,
            control,
            lobby,
        })
    }

    pub fn configured_ticks(&self) -> u32 {
        self.configured_ticks
    }

    pub fn run(&mut self, ticks: u32) -> GameResult<GameSummary> {
        let mut hasher = SnapshotHasher::new();
        let mut ground_hits = 0u32;
        let mut ready_batches = 0u32;
        let mut last_snapshot = None;
        let mut was_grounded = false;

        for tick in 0..ticks {
            let payload = format!("FRAME{:03}", tick).into_bytes();
            let packet = ControlPacket::builder(LOCAL_CLIENT_ID, tick as u32)
                .timestamp_ms((tick as u64) * 16)
                .payload(payload);
            let outcome = self.control.ingest(packet)?;
            ready_batches += outcome.ready.len() as u32;

            let snapshot = self.engine.tick()?;
            let object = snapshot
                .objects
                .iter()
                .find(|object| object.id == self.object_id)
                .cloned()
                .unwrap_or_else(|| snapshot.objects[0].clone());
            let on_ground = object.position.y >= self.ground_height;

            if on_ground && !was_grounded {
                self.audio
                    .play_sound(&self.bounce_sound, false)
                    .map_err(|err| GameError::Audio(err.to_string()))?;
                ground_hits += 1;
            }
            was_grounded = on_ground;

            if tick % 60 == 0 {
                let ready_state = (tick / 60) % 2 == 0;
                self.lobby.set_ready(LOCAL_CLIENT_ID, ready_state)?;
            }

            self.update_gui(tick, &object, ready_batches, ground_hits)?;
            self.draw_frame(&snapshot);

            hasher.update_surface(&self.surface);
            last_snapshot = Some(snapshot);
        }

        let final_snapshot = last_snapshot.unwrap_or_else(|| self.engine.snapshot());
        Ok(GameSummary {
            ticks,
            ground_hits,
            ready_batches,
            surface_hash: hasher.finish(),
            final_snapshot,
            scenario_name: self.scenario_name.clone(),
            system_version: self.system_version.clone(),
            system_entry_count: self.system_entry_count,
            install_root: self.paths.install_root().to_path_buf(),
            user_data_dir: self.paths.user_data_dir().to_path_buf(),
            logs_dir: self.paths.logs_dir().to_path_buf(),
            cache_dir: self.paths.cache_dir().to_path_buf(),
        })
    }

    fn update_gui(
        &mut self,
        tick: u32,
        object: &ObjectSnapshot,
        ready_batches: u32,
        ground_hits: u32,
    ) -> GameResult<()> {
        let ready_players = self
            .lobby
            .participants()
            .filter(|(_, participant)| participant.ready)
            .count();
        let total_participants = self.lobby.participants().count();

        let frame_text = format!(
            "FRAME {:03} X {:03} Y {:03} VX {}{:02} VY {}{:02}",
            tick.min(999),
            clamp_positive(object.position.x, 999),
            clamp_positive(object.position.y, 999),
            sign_marker(object.velocity.x),
            abs_u32(object.velocity.x).min(99),
            sign_marker(object.velocity.y),
            abs_u32(object.velocity.y).min(99),
        );
        let status_text = format!(
            "READY {:02}OF{:02} GROUND {:02} BATCH {:03}",
            ready_players.min(99),
            total_participants.min(99),
            ground_hits.min(99),
            ready_batches.min(999),
        );

        self.gui
            .set_label_text(self.title_label, &self.scenario_label)?;
        self.gui.set_label_text(self.frame_label, frame_text)?;
        self.gui.set_label_text(self.status_label, status_text)?;

        Ok(())
    }

    fn draw_frame(&mut self, snapshot: &SimulationSnapshot) {
        self.surface.fill(Color::opaque(10, 16, 32));
        self.draw_ground();
        self.draw_objects(snapshot);
        self.draw_gui_overlay();
    }

    fn draw_ground(&mut self) {
        let ground = self
            .ground_height
            .clamp(0, (SURFACE_HEIGHT.saturating_sub(1)) as i32) as u32;
        for y in ground..SURFACE_HEIGHT {
            for x in 0..SURFACE_WIDTH {
                let _ = self.surface.set_pixel(x, y, Color::opaque(28, 84, 44));
            }
        }
    }

    fn draw_objects(&mut self, snapshot: &SimulationSnapshot) {
        for object in &snapshot.objects {
            self.paint_object(object);
        }
    }

    fn paint_object(&mut self, object: &ObjectSnapshot) {
        let x = clamp_positive(object.position.x, (SURFACE_WIDTH - 1) as i32) as i32;
        let y = clamp_positive(object.position.y, (SURFACE_HEIGHT - 1) as i32) as i32;
        let energy = object.energy.max(0).min(100) as u8;
        let color = if energy > 50 {
            Color::opaque(252, 196, 64)
        } else {
            Color::opaque(220, 72, 72)
        };
        let size = 6i32;
        let rect = GuiRect::from_origin_size(
            GuiPoint::new((x - size / 2).max(0) as f32, (y - size / 2).max(0) as f32),
            GuiSize::new(size as f32, size as f32),
        );
        fill_rect(&mut self.surface, &rect, color);
    }

    fn draw_gui_overlay(&mut self) {
        self.gui.layout(GuiSize::new(SURFACE_WIDTH as f32, 120.0));
        for command in self.gui.render() {
            match command {
                DrawCommand::Quad { rect, color } => fill_rect(&mut self.surface, &rect, color),
                DrawCommand::Text { rect, text, color } => {
                    draw_text(&mut self.surface, &rect, &text, color)
                }
            }
        }
    }
}

fn load_demo_config() -> std::io::Result<Config> {
    let mut cursor = Cursor::new(DEMO_CONFIG_BYTES);
    Config::from_reader(&mut cursor)
}

fn parse_config_value<T>(config: &Config, key: &str) -> Option<T>
where
    T: FromStr,
{
    config
        .get_in(Some("Game"), key)
        .and_then(|value| value.parse().ok())
}

fn sanitize_label(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    for ch in source.chars() {
        let upper = ch.to_ascii_uppercase();
        if upper.is_ascii_alphanumeric() || upper == ' ' {
            result.push(upper);
        }
    }
    if result.is_empty() {
        "UNKNOWN".to_string()
    } else {
        result
    }
}

fn sign_marker(value: i32) -> char {
    if value > 0 {
        'P'
    } else if value < 0 {
        'N'
    } else {
        'Z'
    }
}

fn clamp_positive(value: i32, upper: i32) -> u32 {
    value.clamp(0, upper) as u32
}

fn abs_u32(value: i32) -> u32 {
    if value == i32::MIN {
        i32::MAX as u32
    } else {
        value.abs() as u32
    }
}

fn fill_rect(surface: &mut Surface, rect: &GuiRect, color: Color) {
    let x0 = rect.origin.x.floor() as i32;
    let y0 = rect.origin.y.floor() as i32;
    let x1 = (rect.origin.x + rect.size.width).ceil() as i32;
    let y1 = (rect.origin.y + rect.size.height).ceil() as i32;

    let x0 = x0.clamp(0, SURFACE_WIDTH as i32);
    let y0 = y0.clamp(0, SURFACE_HEIGHT as i32);
    let x1 = x1.clamp(0, SURFACE_WIDTH as i32);
    let y1 = y1.clamp(0, SURFACE_HEIGHT as i32);

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
        if cursor_x > SURFACE_WIDTH as f32 {
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

fn generate_bounce_wav(duration_seconds: f32, frequency_hz: f32, sample_rate: u32) -> Vec<u8> {
    let sample_count = (duration_seconds * sample_rate as f32).max(1.0) as usize;
    let mut data = Vec::with_capacity(44 + sample_count * 2);

    let subchunk2_size = (sample_count * 2) as u32;
    let chunk_size = 36 + subchunk2_size;

    data.extend_from_slice(b"RIFF");
    data.extend_from_slice(&chunk_size.to_le_bytes());
    data.extend_from_slice(b"WAVE");
    data.extend_from_slice(b"fmt ");
    data.extend_from_slice(&16u32.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&1u16.to_le_bytes());
    data.extend_from_slice(&sample_rate.to_le_bytes());
    let byte_rate = sample_rate * 16 / 8;
    data.extend_from_slice(&byte_rate.to_le_bytes());
    let block_align = 16 / 8;
    data.extend_from_slice(&(block_align as u16).to_le_bytes());
    data.extend_from_slice(&16u16.to_le_bytes());
    data.extend_from_slice(b"data");
    data.extend_from_slice(&subchunk2_size.to_le_bytes());

    for index in 0..sample_count {
        let t = index as f32 / sample_rate as f32;
        let envelope = (1.0 - t / duration_seconds).clamp(0.0, 1.0);
        let sample = (f32::sin(2.0 * PI * frequency_hz * t) * envelope * 0.6 * i16::MAX as f32)
            .round() as i16;
        data.extend_from_slice(&sample.to_le_bytes());
    }

    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_wave_encoder_produces_pcm_data() {
        let wav = generate_bounce_wav(0.1, 440.0, SAMPLE_RATE);
        let system = AudioSystem::new(2).expect("audio system");
        let handle = system.load_sound(&wav).expect("sound loads");
        assert!(handle.duration_ms().unwrap_or(0) > 0);
    }

    #[test]
    fn demo_game_runs_short_session() {
        let mut game = DemoGame::new().expect("game constructed");
        let summary = game.run(8).expect("simulation runs");
        assert_eq!(summary.ticks, 8);
        assert!(summary.surface_hash != 0);
        assert!(!summary.final_snapshot.objects.is_empty());
    }
}
