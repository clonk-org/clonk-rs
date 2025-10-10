use std::f32::consts::PI;
use std::io::{stdout, Cursor};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::thread;
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
use lc_frontend::{GraphicsOverlay, GraphicsSystem};
use lc_graphics::{SnapshotHasher, Surface};
use lc_gui::GuiError;
use lc_network::{ClientId, ControlCoordinator, ControlError, ControlPacket, Lobby, LobbyError};
use lc_platform::{AppPaths, PathsError};
use lc_resources::{Group, GroupError};
use minifb::{Key, Window, WindowOptions};
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
    configured_ticks: u32,
    scenario_name: String,
    system_version: String,
    system_entry_count: usize,
    graphics: GraphicsSystem,
    audio: AudioSystem,
    bounce_sound: SoundHandle,
    control: ControlCoordinator,
    lobby: Lobby,
    control_mode: ControlMode,
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
    pub precipitation: i32,
    pub sky_color: Option<[u8; 3]>,
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

trait InteractiveFrontend {
    fn capture_events(&mut self, duration: Duration) -> GameResult<()>;
    fn current_payload(&self) -> ControlPayload;
    fn exit_requested(&self) -> bool;
    fn present_surface(&mut self, surface: &Surface) -> GameResult<()>;
    fn teardown(&mut self);
}

enum ControlMode {
    Scripted,
    Interactive(Box<dyn InteractiveFrontend>),
}

impl ControlMode {
    fn next_payload(&mut self, tick: u32) -> GameResult<Option<ControlPayload>> {
        match self {
            ControlMode::Scripted => Ok(Some(scripted_control_for_tick(tick))),
            ControlMode::Interactive(frontend) => {
                frontend.capture_events(Duration::from_millis(TICK_DURATION_MS))?;
                if frontend.exit_requested() {
                    Ok(None)
                } else {
                    Ok(Some(frontend.current_payload()))
                }
            }
        }
    }

    fn after_frame(&mut self, surface: &Surface) -> GameResult<()> {
        if let ControlMode::Interactive(frontend) = self {
            frontend.present_surface(surface)?;
        }
        Ok(())
    }

    fn teardown(&mut self) {
        if let ControlMode::Interactive(frontend) = self {
            frontend.teardown();
        }
    }
}

struct TerminalFrontend {
    left_held: bool,
    right_held: bool,
    jump_held: bool,
    exit_requested: bool,
    raw_mode: bool,
    cursor_hidden: bool,
}

impl TerminalFrontend {
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

    fn poll_events(&mut self, duration: Duration) -> GameResult<()> {
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

    fn payload(&self) -> ControlPayload {
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

    fn teardown_terminal(&mut self) {
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

impl InteractiveFrontend for TerminalFrontend {
    fn capture_events(&mut self, duration: Duration) -> GameResult<()> {
        self.poll_events(duration)
    }

    fn current_payload(&self) -> ControlPayload {
        self.payload()
    }

    fn exit_requested(&self) -> bool {
        self.exit_requested()
    }

    fn present_surface(&mut self, _surface: &Surface) -> GameResult<()> {
        Ok(())
    }

    fn teardown(&mut self) {
        self.teardown_terminal();
    }
}

impl Drop for TerminalFrontend {
    fn drop(&mut self) {
        self.teardown_terminal();
    }
}

struct WindowFrontend {
    window: Window,
    buffer: Vec<u32>,
    width: usize,
    height: usize,
    exit_requested: bool,
}

impl WindowFrontend {
    fn new(width: u32, height: u32) -> GameResult<Self> {
        let width = width as usize;
        let height = height as usize;
        let mut window = Window::new(
            "LegacyClonk (Rust demo)",
            width,
            height,
            WindowOptions::default(),
        )
        .map_err(|err| GameError::Input(err.to_string()))?;
        window.limit_update_rate(Some(Duration::from_millis(TICK_DURATION_MS)));
        Ok(Self {
            window,
            buffer: vec![0; width * height],
            width,
            height,
            exit_requested: false,
        })
    }

    fn write_surface(&mut self, surface: &Surface) -> GameResult<()> {
        let width = surface.width() as usize;
        let height = surface.height() as usize;
        if width != self.width || height != self.height {
            return Err(GameError::Input(format!(
                "surface size {}x{} does not match window {}x{}",
                width, height, self.width, self.height
            )));
        }

        for (chunk, pixel) in surface.pixels().chunks_exact(4).zip(self.buffer.iter_mut()) {
            let r = chunk[0] as u32;
            let g = chunk[1] as u32;
            let b = chunk[2] as u32;
            let a = chunk[3] as u32;
            *pixel = (a << 24) | (r << 16) | (g << 8) | b;
        }

        Ok(())
    }

    fn request_exit(&mut self) {
        self.exit_requested = true;
    }
}

impl InteractiveFrontend for WindowFrontend {
    fn capture_events(&mut self, duration: Duration) -> GameResult<()> {
        let start = Instant::now();

        if !self.window.is_open() {
            self.request_exit();
            return Ok(());
        }

        self.window.update();
        if !self.window.is_open() {
            self.request_exit();
        }

        if self.window.is_key_down(Key::Escape) || self.window.is_key_down(Key::Q) {
            self.request_exit();
        }

        if let Some(remaining) = duration.checked_sub(start.elapsed()) {
            if !remaining.is_zero() {
                thread::sleep(remaining);
            }
        }

        Ok(())
    }

    fn current_payload(&self) -> ControlPayload {
        let left = self.window.is_key_down(Key::Left) || self.window.is_key_down(Key::A);
        let right = self.window.is_key_down(Key::Right) || self.window.is_key_down(Key::D);
        let horizontal = match (left, right) {
            (true, true) | (false, false) => 0,
            (true, false) => -1,
            (false, true) => 1,
        };
        let jump = self.window.is_key_down(Key::Space)
            || self.window.is_key_down(Key::Up)
            || self.window.is_key_down(Key::W);
        ControlPayload { horizontal, jump }
    }

    fn exit_requested(&self) -> bool {
        self.exit_requested
    }

    fn present_surface(&mut self, surface: &Surface) -> GameResult<()> {
        self.write_surface(surface)?;
        self.window
            .update_with_buffer(&self.buffer, self.width, self.height)
            .map_err(|err| GameError::Input(err.to_string()))?;
        if !self.window.is_open() {
            self.request_exit();
        }
        Ok(())
    }

    fn teardown(&mut self) {
        self.request_exit();
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

        let mut graphics = GraphicsSystem::new(
            SURFACE_WIDTH,
            SURFACE_HEIGHT,
            ground_height,
            &scenario_label,
        );
        graphics.set_world_width(world_width);

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
            let frontend: Box<dyn InteractiveFrontend> = match WindowFrontend::new(
                SURFACE_WIDTH,
                SURFACE_HEIGHT,
            ) {
                Ok(window) => Box::new(window),
                Err(err) => {
                    eprintln!(
                            "warning: failed to start window frontend ({err}). Falling back to terminal input."
                        );
                    Box::new(TerminalFrontend::new()?)
                }
            };
            ControlMode::Interactive(frontend)
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

        Ok(Self {
            engine,
            paths,
            object_id,
            configured_ticks,
            scenario_name,
            system_version,
            system_entry_count,
            graphics,
            audio,
            bounce_sound,
            control,
            lobby,
            control_mode,
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
            let on_ground = self.is_on_ground(&object, snapshot.landscape.as_ref());

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
            self.graphics.render_frame(&snapshot, &object);
            self.control_mode.after_frame(self.graphics.surface())?;

            hasher.update_surface(self.graphics.surface());
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
            precipitation: environment_settings.precipitation(),
            sky_color: environment_settings
                .sky_color()
                .map(|color| [color.r, color.g, color.b]),
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
        let precip_display = environment.precipitation.clamp(-99, 99);
        let status_text = format!(
            "READY {:02}OF{:02} GROUND {:02} BATCH {:03} TEMP {:+03} WIND {:+03} PREC {:+03}",
            ready_players.min(99),
            total_participants.min(99),
            ground_hits.min(99),
            ready_batches.min(999),
            ambient_display,
            wind_display,
            precip_display,
        );

        let energy_fraction = (object.energy as f32).clamp(0.0, 100.0) / 100.0;
        let overlay = GraphicsOverlay {
            frame_text: &frame_text,
            status_text: &status_text,
            energy_fraction,
        };
        self.graphics.update_overlay(&overlay)?;

        Ok(())
    }

    fn apply_control(&mut self, control: ControlPayload) -> GameResult<()> {
        let snapshot = self
            .engine
            .object_snapshot(self.object_id)
            .ok_or_else(|| GameError::Engine(EngineError::UnknownObject(self.object_id)))?;

        let mut velocity = snapshot.velocity;
        velocity.x = (control.horizontal as i32) * HORIZONTAL_SPEED;
        if control.jump && self.is_on_ground(&snapshot, self.engine.landscape()) {
            velocity.y = JUMP_VELOCITY;
        }

        self.engine.apply_command(
            LOCAL_CLIENT_ID as i32,
            CrewCommandTarget::role("demo-bouncer"),
            ObjectUpdate::new().with_velocity(velocity),
        )?;

        Ok(())
    }

    fn is_on_ground(&self, object: &ObjectSnapshot, landscape: Option<&Landscape>) -> bool {
        let ground = self.graphics.ground_height_at(landscape, object.position.x);
        object.position.y >= ground
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
