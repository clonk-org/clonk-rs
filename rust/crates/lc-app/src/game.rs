use std::f32::consts::PI;
use std::io::{stdout, Cursor};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute, terminal,
};
use lc_audio::{AudioSystem, SoundHandle};
use lc_core::std_config::Config;
use lc_engine::{
    CrewCommandTarget, CrewRole, Definition, Engine, EngineError, EngineState, EngineStateIoError,
    EnvironmentFrame, Landscape, ObjectId, ObjectSnapshot, ObjectUpdate, PhysicsSettings, Scenario,
    ScenarioError, SimulationSnapshot, SpawnConfig, Vector2,
};
use lc_graphics::{Color, PixelFormat, SnapshotHasher, Surface};
use lc_gui::{
    DrawCommand, Gui, GuiError, Point as GuiPoint, Rect as GuiRect, Size as GuiSize, WidgetId,
};
use lc_network::{ClientId, ControlCoordinator, ControlError, ControlPacket, Lobby, LobbyError};
use lc_platform::{AppPaths, PathsError};
use lc_resources::{Group, GroupError};
use serde::{Deserialize, Serialize};

const DEMO_CONFIG_BYTES: &[u8] = include_bytes!("demo_config.cfg");
const DEMO_SCRIPT: &str = include_str!("demo_script.aul");
const LOCAL_CLIENT_ID: ClientId = 1;
const SAMPLE_RATE: u32 = 44_100;
const SURFACE_WIDTH: u32 = 640;
const SURFACE_HEIGHT: u32 = 360;
const MIX_CHANNELS: usize = 8;
const HORIZONTAL_SPEED: i32 = 6;
const JUMP_VELOCITY: i32 = -18;
const TICK_DURATION_MS: u64 = 16;

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
    #[error("scenario error: {0}")]
    Scenario(#[from] ScenarioError),
    #[error("scenario did not spawn any objects")]
    ScenarioNoObjects,
    #[error("control payload error: {0}")]
    ControlPayload(String),
    #[error("summary output error: {0}")]
    SummaryOutput(std::io::Error),
    #[error("summary serialization error: {0}")]
    SummarySerialize(#[from] serde_json::Error),
    #[error("input error: {0}")]
    Input(String),
    #[error("engine state error: {0}")]
    EngineStateIo(#[from] EngineStateIoError),
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
    control_mode: ControlMode,
    world_width: i32,
    viewport_x: i32,
    viewport_y: i32,
}

#[derive(Debug, Clone)]
pub struct DemoGameOptions {
    pub config_path: Option<PathBuf>,
    pub scenario_path: Option<PathBuf>,
    pub interactive: bool,
    pub load_state_path: Option<PathBuf>,
}

impl Default for DemoGameOptions {
    fn default() -> Self {
        Self {
            config_path: None,
            scenario_path: None,
            interactive: false,
            load_state_path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct EnvironmentSummary {
    pub base_wind: i32,
    pub wind_variation: i32,
    pub wind_period: u32,
    pub temperature: i32,
    pub climate: i32,
    pub temperature_variation: i32,
    pub temperature_period: u32,
    pub temperature_phase: u32,
    pub ambient_temperature: i32,
    pub current_wind: i32,
    pub time_of_day: u16,
    pub time_speed: i16,
}

#[derive(Debug, Clone, Serialize)]
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
    pub gravity: i32,
    pub max_fall_speed: i32,
    pub max_rise_speed: i32,
    pub max_horizontal_speed: i32,
    pub environment: EnvironmentSummary,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct ControlPayload {
    horizontal: i8,
    jump: bool,
}

impl ControlPayload {
    const fn neutral() -> Self {
        Self {
            horizontal: 0,
            jump: false,
        }
    }
}

enum ControlMode {
    Scripted,
    Interactive(InteractiveInput),
}

impl ControlMode {
    fn next_payload(&mut self, tick: u32) -> GameResult<Option<ControlPayload>> {
        match self {
            ControlMode::Scripted => Ok(Some(scripted_control_for_tick(tick))),
            ControlMode::Interactive(input) => {
                input.capture_events(Duration::from_millis(TICK_DURATION_MS))?;
                if input.exit_requested() {
                    Ok(None)
                } else {
                    Ok(Some(input.current_payload()))
                }
            }
        }
    }

    fn teardown(&mut self) {
        if let ControlMode::Interactive(input) = self {
            input.teardown();
        }
    }
}

struct InteractiveInput {
    left_held: bool,
    right_held: bool,
    jump_held: bool,
    exit_requested: bool,
    raw_mode: bool,
    cursor_hidden: bool,
}

impl InteractiveInput {
    fn new() -> GameResult<Self> {
        terminal::enable_raw_mode().map_err(|err| GameError::Input(err.to_string()))?;
        let mut stdout = stdout();
        execute!(stdout, cursor::Hide).map_err(|err| GameError::Input(err.to_string()))?;
        Ok(Self {
            left_held: false,
            right_held: false,
            jump_held: false,
            exit_requested: false,
            raw_mode: true,
            cursor_hidden: true,
        })
    }

    fn capture_events(&mut self, duration: Duration) -> GameResult<()> {
        let start = Instant::now();
        let mut remaining = duration;

        while event::poll(remaining).map_err(|err| GameError::Input(err.to_string()))? {
            let event = event::read().map_err(|err| GameError::Input(err.to_string()))?;
            self.handle_event(event);

            let elapsed = start.elapsed();
            if elapsed >= duration {
                break;
            }
            remaining = duration - elapsed;
        }

        Ok(())
    }

    fn handle_event(&mut self, event: Event) {
        if let Event::Key(key) = event {
            self.handle_key_event(key);
        }
    }

    fn handle_key_event(&mut self, key: KeyEvent) {
        let is_press = matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat);
        let is_release = matches!(key.kind, KeyEventKind::Release);

        match key.code {
            KeyCode::Left | KeyCode::Char('a') | KeyCode::Char('A') => {
                if is_press {
                    self.left_held = true;
                } else if is_release {
                    self.left_held = false;
                }
            }
            KeyCode::Right | KeyCode::Char('d') | KeyCode::Char('D') => {
                if is_press {
                    self.right_held = true;
                } else if is_release {
                    self.right_held = false;
                }
            }
            KeyCode::Up | KeyCode::Char('w') | KeyCode::Char('W') | KeyCode::Char(' ') => {
                if is_press {
                    self.jump_held = true;
                } else if is_release {
                    self.jump_held = false;
                }
            }
            KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
                if is_press {
                    self.exit_requested = true;
                }
            }
            KeyCode::Char('c') | KeyCode::Char('C') => {
                if is_press && key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.exit_requested = true;
                }
            }
            _ => {}
        }
    }

    fn current_payload(&self) -> ControlPayload {
        let horizontal = match (self.left_held, self.right_held) {
            (true, true) | (false, false) => 0,
            (true, false) => -1,
            (false, true) => 1,
        };
        ControlPayload {
            horizontal,
            jump: self.jump_held,
        }
    }

    fn exit_requested(&self) -> bool {
        self.exit_requested
    }

    fn teardown(&mut self) {
        if self.raw_mode {
            let _ = terminal::disable_raw_mode();
            self.raw_mode = false;
        }
        if self.cursor_hidden {
            let mut stdout = stdout();
            let _ = execute!(stdout, cursor::Show);
            self.cursor_hidden = false;
        }
    }
}

impl Drop for InteractiveInput {
    fn drop(&mut self) {
        self.teardown();
    }
}

impl DemoGame {
    pub fn new(options: DemoGameOptions) -> GameResult<Self> {
        let config = load_demo_config(options.config_path.as_deref())?;
        let mut configured_ticks = parse_config_value::<u32>(&config, "ticks").unwrap_or(180);
        let mut ground_height = parse_config_value::<i32>(&config, "ground_height").unwrap_or(220);
        let spawn_x = parse_config_value::<i32>(&config, "spawn_x").unwrap_or(64);
        let spawn_y = parse_config_value::<i32>(&config, "spawn_y").unwrap_or(48);
        let spawn_vx = parse_config_value::<i32>(&config, "spawn_velocity_x").unwrap_or(3);
        let spawn_vy = parse_config_value::<i32>(&config, "spawn_velocity_y").unwrap_or(0);
        let mut scenario_name = config
            .get_in(Some("Game"), "scenario_name")
            .map(|value| value.to_string())
            .unwrap_or_else(|| "Rust Demo Bounce".to_string());
        let default_physics = PhysicsSettings::default();
        let gravity =
            parse_config_value::<i32>(&config, "gravity").unwrap_or(default_physics.gravity);
        let max_fall_speed = parse_config_value::<i32>(&config, "max_fall_speed")
            .unwrap_or(default_physics.max_fall_speed);
        let max_rise_speed = parse_config_value::<i32>(&config, "max_rise_speed")
            .unwrap_or(default_physics.max_rise_speed);
        let max_horizontal_speed = parse_config_value::<i32>(&config, "max_horizontal_speed")
            .unwrap_or(default_physics.max_horizontal_speed);
        let base_physics = PhysicsSettings::checked(gravity, max_fall_speed, max_rise_speed)
            .map_err(|detail| GameError::Input(detail.to_string()))?
            .with_max_horizontal_speed(max_horizontal_speed)
            .map_err(|detail| GameError::Input(detail.to_string()))?;

        let scenario = if let Some(path) = options.scenario_path.as_deref() {
            Some(Scenario::load_from_path(path)?)
        } else {
            None
        };

        let paths = AppPaths::discover()?;
        let mut engine = Engine::with_seed(12345);
        engine.set_physics(base_physics);
        let mut object_id = if let Some(scenario) = scenario.as_ref() {
            if let Some(ticks) = scenario.configured_ticks() {
                configured_ticks = ticks;
            }
            if let Some(height) = scenario.ground_height_hint() {
                ground_height = height;
            }
            let created = scenario.apply(&mut engine)?;
            if engine.landscape().is_none() {
                engine.set_landscape(Landscape::flat(2048, ground_height));
            }
            let fallback_name = options
                .scenario_path
                .as_ref()
                .and_then(|path| path.file_stem())
                .and_then(|stem| stem.to_str())
                .map(|value| value.to_string());
            scenario_name = scenario
                .name()
                .map(|value| value.to_string())
                .or(fallback_name)
                .unwrap_or(scenario_name);

            created
                .first()
                .copied()
                .ok_or(GameError::ScenarioNoObjects)?
        } else {
            let definition = Definition::from_script("DemoBouncer", "Demo Bouncer", DEMO_SCRIPT)?;
            engine.register_definition(definition)?;
            engine.set_landscape(Landscape::flat(2048, ground_height));
            engine.spawn_object(
                SpawnConfig::new("DemoBouncer")
                    .with_position(Vector2::new(spawn_x, spawn_y))
                    .with_velocity(Vector2::new(spawn_vx, spawn_vy))
                    .with_energy(100),
            )?
        };
        let scenario_label = format!("SCENARIO {}", sanitize_label(&scenario_name));

        let owner_id = LOCAL_CLIENT_ID as i32;
        if let Some(state_path) = options.load_state_path.as_deref() {
            let state = EngineState::load_from_path(state_path)?;
            engine.restore_state(&state)?;
            if engine.object_snapshot(object_id).is_none() {
                let restored = engine.snapshot();
                let candidate = restored
                    .objects
                    .first()
                    .ok_or(GameError::ScenarioNoObjects)?
                    .id;
                object_id = candidate;
            }
        }

        let snapshot = engine
            .object_snapshot(object_id)
            .ok_or(GameError::Engine(EngineError::UnknownObject(object_id)))?;
        let mut update = ObjectUpdate::new();
        let mut needs_update = false;
        if snapshot.owner != owner_id {
            update = update.with_owner(owner_id);
            needs_update = true;
        }
        if !snapshot.crew_member {
            update = update.with_crew_member(true);
            needs_update = true;
        }
        if needs_update {
            engine.apply_object_update(object_id, update)?;
        }

        engine.set_crew_role(owner_id, object_id, CrewRole::from("demo-bouncer"))?;
        let current_selection = engine.selected_crew(owner_id);
        if !current_selection.iter().any(|id| *id == object_id) {
            engine.select_crew(owner_id, [object_id])?;
        }
        if engine.crew_cursor(owner_id) != Some(object_id) {
            engine.set_crew_cursor(owner_id, Some(object_id))?;
        }

        let world_width = engine
            .landscape()
            .map(|landscape| landscape.width() as i32)
            .unwrap_or(SURFACE_WIDTH as i32);

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

        let control_mode = if options.interactive {
            ControlMode::Interactive(InteractiveInput::new()?)
        } else {
            ControlMode::Scripted
        };

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
            control_mode,
            world_width,
            viewport_x: 0,
            viewport_y: 0,
        })
    }

    pub fn configured_ticks(&self) -> u32 {
        self.configured_ticks
    }

    pub fn save_state_to_path(&self, path: &Path) -> GameResult<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|err| GameError::EngineStateIo(err.into()))?;
            }
        }
        let state = self.engine.capture_state();
        state.save_to_path(path)?;
        Ok(())
    }

    pub fn run(&mut self, ticks: u32) -> GameResult<GameSummary> {
        let mut hasher = SnapshotHasher::new();
        let mut ground_hits = 0u32;
        let mut ready_batches = 0u32;
        let mut last_snapshot = None;
        let mut was_grounded = false;
        let mut current_control = ControlPayload::neutral();

        let mut executed_ticks = 0u32;
        while executed_ticks < ticks {
            let tick = executed_ticks;
            let Some(control_payload) = self.control_mode.next_payload(tick)? else {
                break;
            };
            let payload = encode_control_payload(&control_payload)?;
            let packet = ControlPacket::builder(LOCAL_CLIENT_ID, tick as u32)
                .timestamp_ms((tick as u64) * TICK_DURATION_MS)
                .payload(payload);
            let outcome = self.control.ingest(packet)?;
            ready_batches += outcome.ready.len() as u32;

            for batch in &outcome.ready {
                for packet in batch.packets() {
                    if packet.client_id() == LOCAL_CLIENT_ID {
                        current_control = decode_control_payload(packet.payload())?;
                    }
                }
            }

            self.apply_control(current_control)?;

            let snapshot = self.engine.tick()?;
            let object = snapshot
                .objects
                .iter()
                .find(|object| object.id == self.object_id)
                .cloned()
                .unwrap_or_else(|| snapshot.objects[0].clone());
            self.update_viewport(&object);
            let on_ground = self.is_on_ground(&object);

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

            self.update_gui(
                tick,
                &object,
                ready_batches,
                ground_hits,
                &snapshot.environment,
            )?;
            self.draw_frame(&snapshot);

            hasher.update_surface(&self.surface);
            last_snapshot = Some(snapshot);
            executed_ticks += 1;
        }

        self.control_mode.teardown();

        let final_snapshot = last_snapshot.unwrap_or_else(|| self.engine.snapshot());
        let physics = self.engine.physics();
        let environment_settings = self.engine.environment();
        let environment = EnvironmentSummary {
            base_wind: environment_settings.wind,
            wind_variation: environment_settings.wind_variation,
            wind_period: environment_settings.wind_period,
            temperature: environment_settings.temperature,
            climate: environment_settings.climate,
            temperature_variation: environment_settings.temperature_variation,
            temperature_period: environment_settings.temperature_period,
            temperature_phase: environment_settings.temperature_phase,
            ambient_temperature: environment_settings.ambient_temperature(self.engine.frame()),
            current_wind: environment_settings.wind_force(self.engine.frame()),
            time_of_day: environment_settings.time_of_day(),
            time_speed: environment_settings.time_speed(),
        };
        Ok(GameSummary {
            ticks: executed_ticks,
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
            gravity: physics.gravity,
            max_fall_speed: physics.max_fall_speed,
            max_rise_speed: physics.max_rise_speed,
            max_horizontal_speed: physics.max_horizontal_speed,
            environment,
        })
    }

    fn update_gui(
        &mut self,
        tick: u32,
        object: &ObjectSnapshot,
        ready_batches: u32,
        ground_hits: u32,
        environment: &EnvironmentFrame,
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
        let ambient_display = environment.ambient_temperature.clamp(-99, 99);
        let wind_display = environment.wind_force.clamp(-99, 99);
        let status_text = format!(
            "READY {:02}OF{:02} GROUND {:02} BATCH {:03} TEMP {:+03} WIND {:+03}",
            ready_players.min(99),
            total_participants.min(99),
            ground_hits.min(99),
            ready_batches.min(999),
            ambient_display,
            wind_display,
        );

        self.gui
            .set_label_text(self.title_label, &self.scenario_label)?;
        self.gui.set_label_text(self.frame_label, frame_text)?;
        self.gui.set_label_text(self.status_label, status_text)?;

        Ok(())
    }

    fn apply_control(&mut self, control: ControlPayload) -> GameResult<()> {
        let snapshot = self
            .engine
            .object_snapshot(self.object_id)
            .ok_or_else(|| GameError::Engine(EngineError::UnknownObject(self.object_id)))?;

        let mut velocity = snapshot.velocity;
        velocity.x = (control.horizontal as i32) * HORIZONTAL_SPEED;
        if control.jump && self.is_on_ground(&snapshot) {
            velocity.y = JUMP_VELOCITY;
        }

        self.engine.apply_command(
            LOCAL_CLIENT_ID as i32,
            CrewCommandTarget::role("demo-bouncer"),
            ObjectUpdate::new().with_velocity(velocity),
        )?;

        Ok(())
    }

    fn draw_frame(&mut self, snapshot: &SimulationSnapshot) {
        let sky = Self::sky_color_for_temperature(snapshot.environment.ambient_temperature);
        self.surface.fill(sky);
        self.draw_ground(snapshot.environment.ambient_temperature);
        self.draw_objects(snapshot);
        self.draw_gui_overlay();
    }

    fn draw_ground(&mut self, ambient_temperature: i32) {
        let ground_color = Self::ground_color_for_temperature(ambient_temperature);
        for screen_x in 0..SURFACE_WIDTH {
            let world_x = self.viewport_x + screen_x as i32;
            let ground_world = self.ground_height_at(world_x);
            let mut ground_screen = ground_world - self.viewport_y;
            if ground_screen < 0 {
                ground_screen = 0;
            }
            if ground_screen >= SURFACE_HEIGHT as i32 {
                continue;
            }
            let ground_screen = ground_screen as u32;
            for y in ground_screen..SURFACE_HEIGHT {
                let _ = self.surface.set_pixel(screen_x, y, ground_color);
            }
        }
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

    fn draw_objects(&mut self, snapshot: &SimulationSnapshot) {
        for object in &snapshot.objects {
            self.paint_object(object);
        }
    }

    fn paint_object(&mut self, object: &ObjectSnapshot) {
        let screen_x = object.position.x - self.viewport_x;
        let screen_y = object.position.y - self.viewport_y;
        if screen_x < -10
            || screen_y < -10
            || screen_x > SURFACE_WIDTH as i32 + 10
            || screen_y > SURFACE_HEIGHT as i32 + 10
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

    fn update_viewport(&mut self, focus: &ObjectSnapshot) {
        let half_width = (SURFACE_WIDTH / 2) as i32;
        let mut desired = focus.position.x - half_width;
        if desired < 0 {
            desired = 0;
        }
        let max_offset = (self.world_width - SURFACE_WIDTH as i32).max(0);
        if desired > max_offset {
            desired = max_offset;
        }
        self.viewport_x = desired;
    }

    fn is_on_ground(&self, object: &ObjectSnapshot) -> bool {
        let ground = self.ground_height_at(object.position.x);
        object.position.y >= ground
    }

    fn ground_height_at(&self, x: i32) -> i32 {
        self.surface_height_at(x).unwrap_or(self.ground_height)
    }

    fn surface_height_at(&self, x: i32) -> Option<i32> {
        self.engine
            .landscape()
            .and_then(|landscape| landscape.surface_height(x))
    }
}

fn load_demo_config(path: Option<&Path>) -> std::io::Result<Config> {
    match path {
        Some(path) => Config::load(path),
        None => {
            let mut cursor = Cursor::new(DEMO_CONFIG_BYTES);
            Config::from_reader(&mut cursor)
        }
    }
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

fn encode_control_payload(payload: &ControlPayload) -> GameResult<Vec<u8>> {
    serde_json::to_vec(payload).map_err(|err| GameError::ControlPayload(err.to_string()))
}

fn decode_control_payload(bytes: &[u8]) -> GameResult<ControlPayload> {
    serde_json::from_slice(bytes).map_err(|err| GameError::ControlPayload(err.to_string()))
}

fn scripted_control_for_tick(tick: u32) -> ControlPayload {
    let phase = (tick / 90) % 4;
    let horizontal = match phase {
        0 => 1,
        1 => 0,
        2 => -1,
        _ => 0,
    };
    let jump = tick % 120 == 0;
    ControlPayload { horizontal, jump }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn sine_wave_encoder_produces_pcm_data() {
        let wav = generate_bounce_wav(0.1, 440.0, SAMPLE_RATE);
        let system = AudioSystem::new(2).expect("audio system");
        let handle = system.load_sound(&wav).expect("sound loads");
        assert!(handle.duration_ms().unwrap_or(0) > 0);
    }

    #[test]
    fn demo_game_runs_short_session() {
        let mut game = DemoGame::new(DemoGameOptions::default()).expect("game constructed");
        let summary = game.run(8).expect("simulation runs");
        assert_eq!(summary.ticks, 8);
        assert!(summary.surface_hash != 0);
        assert!(!summary.final_snapshot.objects.is_empty());
        assert_eq!(
            summary.max_horizontal_speed,
            PhysicsSettings::default().max_horizontal_speed
        );
    }

    #[test]
    fn demo_game_saves_and_restores_engine_state() {
        let mut game = DemoGame::new(DemoGameOptions::default()).expect("game constructed");
        let _ = game.run(4).expect("simulation runs");

        let temp = NamedTempFile::new().expect("temp file");
        game.save_state_to_path(temp.path()).expect("state saves");

        let options = DemoGameOptions {
            load_state_path: Some(temp.path().to_path_buf()),
            ..DemoGameOptions::default()
        };
        let mut restored = DemoGame::new(options).expect("restored game constructed");
        let summary = restored.run(2).expect("restored simulation runs");
        assert!(summary.ticks > 0);
    }
}
