mod gamepad;
mod ingame_menu;
mod input;
mod network;
mod object_menu;
mod settings;

use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::convert::TryFrom;
use std::f32::consts::PI;
use std::fmt;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use parking_lot::ReentrantMutex;
use clap::Parser;
use gamepad::{GamepadActionType, GamepadEvent, GamepadManager};
use ingame_menu::{IngameMenuAction, IngameMenuState};
use input::KeyboardBindings;
use lc_audio::{AudioError, AudioSystem, ChannelId, MusicHandle, SoundHandle};
use lc_engine::scenario::LegacyDefinitionResolver;
use lc_engine::{
    ActionSpec, ActionState, AudioCommand, CommandKind, ControlButton, ControlCommand,
    ControlEvent, Definition, Engine, EngineError, EngineState, EnvironmentSettings, Landscape,
    MaterialSet, MenuCommandKind, MenuCommandSelection, MessageKind, MovementProfile, ObjectId,
    ObjectSnapshot, ObjectUpdate, PlayerStatus, Scenario, ScenarioError, SimulationSnapshot,
    SkyConfig, SpawnConfig, Vector2, FLAG_BOTTOM, FLAG_HCENTER, FLAG_LEFT, FLAG_RIGHT, FLAG_TOP,
    FLAG_VCENTER, FLAG_X_REL, FLAG_Y_REL, OWNER_NONE,
};
use lc_frontend::{
    draw_image, CrewOverlay, CursorAtlas, GraphicsOverlay, GraphicsSystem, GuiPoint, ImageData,
    InputDispatcher, KeyCode, PlayerOverlay, ScenarioEntry, ScenarioKind, SkyRenderState,
    StartupMenu, StartupMenuAction, ViewportInput,
};
use lc_graphics::{BitmapFont, Color, Rect, Surface, TextFont, TrueTypeFont};
use lc_gui::ButtonTextures;
use lc_platform::{AppPaths, PathsError};
use lc_resources::{
    load_endeavour_font, scenario as resource_scenario, DefCore as ResourceDefCore,
    DefinitionError as ResourceDefinitionError, GraphicsImage, GraphicsResource, Group, GroupError,
    ResourceDefinition as ResourceDefinitionData,
};
use network::{ClientSettings, HostSettings, NetworkEvent, NetworkManager, NetworkMode};
use object_menu::{ObjectMenuAction, ObjectMenuCommand, ObjectMenuSelection, ObjectMenuState};
use pixels::{Pixels, SurfaceTexture};
use serde::{
    de::{self, Unexpected, Visitor},
    ser::Serializer,
    Deserialize, Serialize,
};
use settings::{AudioOptions, DisplayMode, DisplayOptions};
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{
    ElementState, Event, KeyboardInput, MouseButton, TouchPhase, VirtualKeyCode, WindowEvent,
};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::{Fullscreen, Window, WindowBuilder};

const PLAYER_OWNER: i32 = 1;
const FRAME_INTERVAL: Duration = Duration::from_micros(16_666); // ~60 FPS
const MAX_ACCUMULATED_TIME: Duration = Duration::from_millis(250); // clamp backlog to avoid runaway catch-up
const DEFAULT_SCENARIO_LABEL: &str = "Rust Sandbox";
const DEFAULT_GROUND_HEIGHT: i32 = 360;
const BACK_ENTRY_IDENTIFIER: &str = "__lc_menu_back";
const BACK_ENTRY_TITLE: &str = "← Back";
const SAVE_DIR_NAME: &str = "Savegames";
const QUICK_SAVE_FILE: &str = "quicksave.lcsave";
const SAVE_FILE_VERSION: SaveFileVersion = SaveFileVersion::new(1, 0, 0);
static APP_PATH_CACHE: Mutex<Option<std::result::Result<Arc<AppPaths>, PathsError>>> =
    Mutex::new(None);

#[derive(Debug, Parser)]
#[command(name = "lc-app", about = "LegacyClonk Rust runtime", version)]
struct Cli {
    #[arg(long = "host", value_name = "ADDR", conflicts_with = "join")]
    host: Option<String>,

    #[arg(long = "join", value_name = "ADDR")]
    join: Option<String>,

    #[arg(long = "player-owner", value_name = "OWNER", default_value_t = PLAYER_OWNER)]
    player_owner: i32,

    #[arg(long = "player-name", value_name = "NAME", default_value = "Player")]
    player_name: String,
}

struct RuntimeConfig {
    player_owner: i32,
    network: Option<NetworkMode>,
}

struct FrontendAssets {
    font: Arc<dyn TextFont>,
    menu_background: Option<ImageData>,
    button_textures: Option<ButtonTextures>,
    base_sprites: HashMap<String, ImageData>,
    cursor_atlas: Arc<CursorAtlas>,
}

impl FrontendAssets {
    fn load(paths: Option<&AppPaths>) -> Self {
        let font = Self::load_font(paths);
        let mut menu_background = None;
        let mut button_textures = None;
        let mut sprites = HashMap::new();
        let mut cursor_atlas = CursorAtlas::empty();

        if let Some(paths) = paths {
            let graphics_path = paths.planet_dir().join("Graphics.c4g");
            match GraphicsResource::open(&graphics_path) {
                Ok(graphics) => {
                    menu_background = graphics
                        .load_image("StartupScenSelBG.png")
                        .ok()
                        .map(Self::image_to_data);
                    button_textures = Self::load_button_textures(&graphics);
                    if let Ok(sprite) = graphics.load_image("Crew.png") {
                        sprites.insert("Walker".to_string(), Self::image_to_data(sprite));
                    }
                    cursor_atlas = Self::load_cursor_atlas(&graphics);
                }
                Err(err) => {
                    tracing::warn!(
                        path = %graphics_path.display(),
                        error = %err,
                        "failed to load Graphics.c4g assets"
                    );
                }
            }
        }

        Self {
            font,
            menu_background,
            button_textures,
            base_sprites: sprites,
            cursor_atlas: Arc::new(cursor_atlas),
        }
    }

    fn load_font(paths: Option<&AppPaths>) -> Arc<dyn TextFont> {
        if let Some(paths) = paths {
            let system_path = paths.system_group_path();
            match Group::open(system_path) {
                Ok(group) => match load_endeavour_font(&group) {
                    Ok(resource) => match TrueTypeFont::from_bytes(resource.clone_bytes()) {
                        Ok(font) => return Arc::new(font),
                        Err(err) => {
                            tracing::warn!(error = ?err, "failed to parse Endeavour.ttf");
                        }
                    },
                    Err(err) => {
                        tracing::warn!(
                            error = %err,
                            "failed to load Endeavour.ttf from system resources"
                        );
                    }
                },
                Err(err) => {
                    tracing::warn!(
                        path = %system_path.display(),
                        error = %err,
                        "failed to open system group for fonts"
                    );
                }
            }
        }
        Arc::new(BitmapFont::new())
    }

    fn font_arc(&self) -> Arc<dyn TextFont> {
        self.font.clone()
    }

    fn menu_background(&self) -> Option<ImageData> {
        self.menu_background.clone()
    }

    fn button_textures(&self) -> Option<ButtonTextures> {
        self.button_textures.clone()
    }

    fn cursor_atlas(&self) -> Arc<CursorAtlas> {
        Arc::clone(&self.cursor_atlas)
    }

    fn base_sprite_map(&self) -> &HashMap<String, ImageData> {
        &self.base_sprites
    }

    fn load_button_textures(graphics: &GraphicsResource) -> Option<ButtonTextures> {
        let normal_image = graphics.load_image("StartupBigButton.png").ok()?;
        let pressed_image = graphics.load_image("StartupBigButtonDown.png").ok()?;

        let normal = Self::image_to_data(normal_image.clone());
        let pressed = Self::image_to_data(pressed_image.clone());
        let selected = Self::lighten_image(&normal_image, 0.25);
        let disabled = Self::darken_image(&normal_image, 0.4);

        Some(ButtonTextures {
            normal,
            pressed,
            selected,
            disabled: Some(disabled),
        })
    }

    fn load_cursor_atlas(graphics: &GraphicsResource) -> CursorAtlas {
        const CURSOR_FILES: [(&str, usize); 8] = [
            ("CursorXXXXXLarge.png", 0),
            ("CursorXXXXLarge.png", 1),
            ("CursorXXXLarge.png", 2),
            ("CursorXXLarge.png", 3),
            ("CursorXLarge.png", 4),
            ("CursorLarge.png", 5),
            ("CursorMedium.png", 6),
            ("CursorSmall.png", 7),
        ];

        let mut images = vec![None; CURSOR_FILES.len()];
        let mut loaded = false;
        for (name, index) in CURSOR_FILES {
            match graphics.load_image(name) {
                Ok(image) => {
                    images[index] = Some(Self::image_to_data(image));
                    loaded = true;
                }
                Err(err) => {
                    tracing::debug!(file = name, error = %err, "cursor image missing");
                }
            }
        }

        if loaded {
            CursorAtlas::new(images)
        } else {
            CursorAtlas::empty()
        }
    }

    fn image_to_data(image: GraphicsImage) -> ImageData {
        let (width, height, pixels) = image.into_parts();
        ImageData::from_arc(width, height, pixels)
    }

    fn lighten_image(image: &GraphicsImage, amount: f32) -> ImageData {
        let amount = amount.clamp(0.0, 1.0);
        let mut pixels = image.pixels().to_vec();
        for chunk in pixels.chunks_exact_mut(4) {
            chunk[0] = lighten_channel(chunk[0], amount);
            chunk[1] = lighten_channel(chunk[1], amount);
            chunk[2] = lighten_channel(chunk[2], amount);
        }
        ImageData::new(image.width(), image.height(), pixels)
    }

    fn darken_image(image: &GraphicsImage, amount: f32) -> ImageData {
        let amount = amount.clamp(0.0, 1.0);
        let mut pixels = image.pixels().to_vec();
        for chunk in pixels.chunks_exact_mut(4) {
            chunk[0] = darken_channel(chunk[0], amount);
            chunk[1] = darken_channel(chunk[1], amount);
            chunk[2] = darken_channel(chunk[2], amount);
        }
        ImageData::new(image.width(), image.height(), pixels)
    }
}

fn lighten_channel(value: u8, amount: f32) -> u8 {
    let value = value as f32;
    let adjusted = value + (255.0 - value) * amount;
    adjusted.round().clamp(0.0, 255.0) as u8
}

fn darken_channel(value: u8, amount: f32) -> u8 {
    let value = value as f32;
    let adjusted = value * (1.0 - amount);
    adjusted.round().clamp(0.0, 255.0) as u8
}

fn resolve_network_mode(cli: &Cli) -> Result<Option<NetworkMode>> {
    if let Some(ref host_addr) = cli.host {
        let bind_addr = parse_socket_addr(host_addr, "host")?;
        return Ok(Some(NetworkMode::Host(HostSettings { bind_addr })));
    }
    if let Some(ref join_addr) = cli.join {
        let server_addr = parse_socket_addr(join_addr, "join")?;
        return Ok(Some(NetworkMode::Client(ClientSettings {
            server_addr,
            player_name: cli.player_name.clone(),
        })));
    }
    Ok(None)
}

fn parse_socket_addr(input: &str, kind: &str) -> Result<SocketAddr> {
    input
        .parse::<SocketAddr>()
        .with_context(|| format!("invalid {kind} address `{input}`"))
}

fn main() -> Result<()> {
    lc_core::logging::init();

    let cli = Cli::parse();
    let runtime = RuntimeConfig {
        player_owner: cli.player_owner,
        network: resolve_network_mode(&cli)?,
    };

    let event_loop = EventLoop::new();
    let app_paths = cached_app_paths().ok();
    if let Some(paths) = app_paths.as_ref() {
        if let Err(err) = paths.ensure_user_dirs() {
            tracing::warn!(
                error = %err,
                path = %paths.user_data_dir().display(),
                "failed to ensure user data directories"
            );
        }
    }
    let mut display_options = DisplayOptions::load(app_paths.as_deref());
    let audio_options = AudioOptions::load(app_paths.as_deref());
    let (initial_width, initial_height) = display_options.actual_size();
    let mut window_builder = WindowBuilder::new().with_title("LegacyClonk (Rust preview)");
    if matches!(display_options.mode, DisplayMode::Window) && !display_options.maximized {
        if let Some((x, y)) = display_options.position {
            window_builder = window_builder.with_position(PhysicalPosition::new(x, y));
        }
    }
    window_builder = window_builder.with_inner_size(LogicalSize::new(
        f64::from(initial_width),
        f64::from(initial_height),
    ));
    let window = window_builder
        .build(&event_loop)
        .context("failed to create application window")?;
    if display_options.maximized && matches!(display_options.mode, DisplayMode::Window) {
        window.set_maximized(true);
    }
    if matches!(display_options.mode, DisplayMode::Fullscreen) {
        window.set_fullscreen(Some(Fullscreen::Borderless(None)));
    }

    let size = enforce_min_size(window.inner_size());
    let surface = SurfaceTexture::new(size.width, size.height, &window);
    let mut pixels = Pixels::new(size.width, size.height, surface)
        .context("failed to create pixel framebuffer")?;

    let mut app = GameApp::new(
        size.width,
        size.height,
        audio_options,
        app_paths.as_deref(),
        runtime,
    )
    .context("failed to initialise app state")?;

    let mut previous_instant = Instant::now();
    let mut accumulator = Duration::ZERO;

    event_loop.run(move |event, _, control_flow| {
        match event {
            Event::WindowEvent { window_id, event } if window_id == window.id() => {
                if let Err(err) = handle_window_event(
                    &window,
                    &mut app,
                    &mut pixels,
                    &mut display_options,
                    event,
                    control_flow,
                ) {
                    tracing::error!(error = ?err, "window event handling failed");
                    control_flow.set_exit();
                }
            }
            Event::MainEventsCleared => {
                if let Err(err) = app.process_gamepad_events() {
                    tracing::error!(error = ?err, "gamepad input failed");
                    control_flow.set_exit();
                    return;
                }
                let now = Instant::now();
                let frame_time = now.saturating_duration_since(previous_instant);
                previous_instant = now;
                let clamped = frame_time.min(MAX_ACCUMULATED_TIME);
                accumulator = (accumulator + clamped).min(MAX_ACCUMULATED_TIME);

                let mut did_update = false;
                while accumulator >= FRAME_INTERVAL {
                    if let Err(err) = app.update() {
                        tracing::error!(error = ?err, "tick failed");
                        control_flow.set_exit();
                        return;
                    }
                    accumulator -= FRAME_INTERVAL;
                    did_update = true;
                }

                if did_update {
                    window.request_redraw();
                }

                let wait_duration = FRAME_INTERVAL.saturating_sub(accumulator);
                *control_flow = ControlFlow::WaitUntil(now + wait_duration);
            }
            Event::RedrawRequested(id) if id == window.id() => {
                if let Err(err) = app.render(pixels.frame_mut()) {
                    tracing::error!(error = ?err, "render failed");
                    control_flow.set_exit();
                    return;
                }
                if let Err(err) = pixels.render() {
                    tracing::error!(error = ?err, "present failed");
                    control_flow.set_exit();
                }
            }
            Event::LoopDestroyed => {}
            _ => {}
        }
        if matches!(
            *control_flow,
            ControlFlow::Exit | ControlFlow::ExitWithCode(_)
        ) {
            if let Some(paths) = app_paths.as_ref() {
                display_options.persist_if_dirty(paths.as_ref());
            }
        }
    });
}

fn handle_window_event(
    window: &Window,
    app: &mut GameApp,
    pixels: &mut Pixels,
    display_options: &mut DisplayOptions,
    event: WindowEvent,
    control_flow: &mut ControlFlow,
) -> Result<()> {
    match event {
        WindowEvent::CloseRequested => {
            control_flow.set_exit();
        }
        WindowEvent::Resized(size) => {
            let clamped = enforce_min_size(size);
            pixels
                .resize_surface(clamped.width, clamped.height)
                .context("failed to resize pixel surface")?;
            pixels
                .resize_buffer(clamped.width, clamped.height)
                .context("failed to resize pixel buffer")?;
            app.resize(clamped.width, clamped.height)?;
            if display_options.mode == DisplayMode::Window {
                display_options.record_actual_size(clamped.width, clamped.height);
            }
            display_options.record_maximized(window.is_maximized());
        }
        WindowEvent::ScaleFactorChanged { new_inner_size, .. } => {
            let clamped = enforce_min_size(*new_inner_size);
            pixels
                .resize_surface(clamped.width, clamped.height)
                .context("failed to resize pixel surface")?;
            pixels
                .resize_buffer(clamped.width, clamped.height)
                .context("failed to resize pixel buffer")?;
            app.resize(clamped.width, clamped.height)?;
            if display_options.mode == DisplayMode::Window {
                display_options.record_actual_size(clamped.width, clamped.height);
            }
            display_options.record_maximized(window.is_maximized());
        }
        WindowEvent::CursorMoved { position, .. } => {
            app.handle_cursor_moved(position)
                .context("failed to process cursor movement")?;
        }
        WindowEvent::CursorLeft { .. } => {
            app.pointer_left();
        }
        WindowEvent::MouseInput { state, button, .. } => {
            if button == MouseButton::Left {
                app.handle_mouse_button(state)
                    .context("failed to process mouse button")?;
            }
        }
        WindowEvent::KeyboardInput {
            input:
                KeyboardInput {
                    state,
                    virtual_keycode: Some(keycode),
                    ..
                },
            ..
        } => {
            if state == ElementState::Pressed && keycode == VirtualKeyCode::F11 {
                toggle_fullscreen(window, display_options);
                return Ok(());
            }
            app.handle_key(keycode, state)
                .context("failed to process key input")?;
        }
        WindowEvent::Moved(position) => {
            if display_options.mode == DisplayMode::Window && !window.is_maximized() {
                display_options.record_position(position.x, position.y);
            }
            display_options.record_maximized(window.is_maximized());
        }
        WindowEvent::Touch(touch) => {
            let position = GuiPoint::new(touch.location.x as f32, touch.location.y as f32);
            app.handle_touch(touch.phase, position)
                .context("failed to process touch input")?;
        }
        WindowEvent::Focused(focused) => {
            if focused {
                window.request_redraw();
            } else {
                app.pointer_left();
            }
        }
        _ => {}
    }
    Ok(())
}

fn enforce_min_size(size: PhysicalSize<u32>) -> PhysicalSize<u32> {
    PhysicalSize::new(size.width.max(1), size.height.max(1))
}

fn toggle_fullscreen(window: &Window, display_options: &mut DisplayOptions) {
    if window.fullscreen().is_some() {
        window.set_fullscreen(None);
        display_options.record_mode(DisplayMode::Window);
        display_options.record_maximized(window.is_maximized());
    } else {
        window.set_fullscreen(Some(Fullscreen::Borderless(None)));
        display_options.record_mode(DisplayMode::Fullscreen);
        display_options.record_maximized(false);
    }
}

struct AudioContext {
    system: AudioSystem,
    options: AudioOptions,
    current_music: Option<MusicHandle>,
    loaded_sounds: HashMap<String, SoundHandle>,
    active_channels: HashMap<SoundInstanceKey, ChannelInfo>,
    resolver: SoundResolver,
    missing_sounds: HashSet<String>,
}

impl AudioContext {
    fn try_new(options: AudioOptions) -> Result<Self, AudioError> {
        Ok(Self {
            system: AudioSystem::new(options.max_channels)?,
            options,
            current_music: None,
            loaded_sounds: HashMap::new(),
            active_channels: HashMap::new(),
            resolver: SoundResolver::new(),
            missing_sounds: HashSet::new(),
        })
    }

    fn play_music(&mut self, data: &[u8], looped: bool) -> Result<(), AudioError> {
        self.stop_music();
        if !self.options.music_enabled {
            return Ok(());
        }
        let music = self.system.load_music(data)?;
        self.system.play_music(&music, looped)?;
        self.system.music_set_volume(self.options.music_volume);
        self.current_music = Some(music);
        Ok(())
    }

    fn stop_music(&mut self) {
        self.system.halt_music();
        self.current_music.take();
    }

    fn process_audio(
        &mut self,
        snapshot: &SimulationSnapshot,
        focus: Option<&ObjectSnapshot>,
        viewport_center: Vector2,
    ) {
        let events = &snapshot.audio;
        if !events.is_empty() {
            self.handle_events(events, snapshot, focus, viewport_center);
        }
        self.update_channels(snapshot, focus, viewport_center);
    }

    fn reset_sfx(&mut self) {
        for info in self.active_channels.values() {
            self.system.halt_channel(info.channel);
        }
        self.active_channels.clear();
    }

    fn configure_scenario(&mut self, path: Option<&Path>) {
        if self.resolver.configure_scenario(path) {
            self.loaded_sounds.clear();
            self.missing_sounds.clear();
        }
    }

    fn register_definition_sounds(&mut self, definition_id: &str, group: &Group) {
        self.resolver
            .register_definition_group(definition_id, group);
        self.loaded_sounds.clear();
        self.missing_sounds.clear();
    }

    fn music_enabled(&self) -> bool {
        self.options.music_enabled
    }

    fn menu_music_enabled(&self) -> bool {
        self.options.menu_music_enabled
    }

    fn music_is_playing(&self) -> bool {
        self.system.music_is_playing()
    }

    fn handle_events(
        &mut self,
        events: &[AudioCommand],
        snapshot: &SimulationSnapshot,
        focus: Option<&ObjectSnapshot>,
        viewport_center: Vector2,
    ) {
        for event in events {
            match event {
                AudioCommand::PlaySound {
                    name,
                    target,
                    volume,
                    looped,
                    custom_falloff,
                } => {
                    if !self.options.sound_enabled {
                        continue;
                    }
                    if let Err(err) = self.start_sound(
                        name,
                        *target,
                        *volume,
                        *looped,
                        *custom_falloff,
                        snapshot,
                        focus,
                        viewport_center,
                    ) {
                        tracing::error!(sound = %name, error = %err, "failed to play sound");
                    }
                }
                AudioCommand::StopSound { name, target } => {
                    self.stop_sound(name, *target);
                }
                AudioCommand::SetSoundVolume {
                    name,
                    target,
                    volume,
                } => {
                    self.update_sound_volume(
                        name,
                        *target,
                        *volume,
                        snapshot,
                        focus,
                        viewport_center,
                    );
                }
            }
        }
    }

    fn start_sound(
        &mut self,
        name: &str,
        target: Option<ObjectId>,
        volume: u8,
        looped: bool,
        custom_falloff: Option<i32>,
        snapshot: &SimulationSnapshot,
        focus: Option<&ObjectSnapshot>,
        viewport_center: Vector2,
    ) -> Result<(), AudioError> {
        if !self.options.sound_enabled {
            return Ok(());
        }
        let key = SoundInstanceKey::new(name, target);
        let Some(handle) = self.ensure_sound(name)? else {
            return Ok(());
        };
        let channel = self.system.play_sound(&handle, looped)?;
        let info = ChannelInfo {
            channel,
            looped,
            target,
            volume,
            custom_falloff,
        };
        let (mut mix_volume, pan) = compute_mix_values(&info, snapshot, focus, viewport_center);
        mix_volume *= self.options.sound_volume;
        self.system
            .channel_set_volume_and_pan(channel, mix_volume, pan);
        self.active_channels.insert(key, info);
        Ok(())
    }

    fn stop_sound(&mut self, name: &str, target: Option<ObjectId>) {
        let key = SoundInstanceKey::new(name, target);
        if let Some(info) = self.active_channels.remove(&key) {
            self.system.halt_channel(info.channel);
        }
    }

    fn update_sound_volume(
        &mut self,
        name: &str,
        target: Option<ObjectId>,
        volume: u8,
        snapshot: &SimulationSnapshot,
        focus: Option<&ObjectSnapshot>,
        viewport_center: Vector2,
    ) {
        let key = SoundInstanceKey::new(name, target);
        if let Some(info) = self.active_channels.get_mut(&key) {
            info.volume = volume;
            if !self.options.sound_enabled {
                return;
            }
            let channel = info.channel;
            let (mut mix_volume, pan) = compute_mix_values(info, snapshot, focus, viewport_center);
            mix_volume *= self.options.sound_volume;
            drop(info);
            self.system
                .channel_set_volume_and_pan(channel, mix_volume, pan);
        }
    }

    fn update_channels(
        &mut self,
        snapshot: &SimulationSnapshot,
        focus: Option<&ObjectSnapshot>,
        viewport_center: Vector2,
    ) {
        let mut finished = Vec::new();
        let mut updates: Vec<(ChannelId, f32, f32)> = Vec::new();
        if !self.options.sound_enabled {
            if !self.active_channels.is_empty() {
                self.reset_sfx();
            }
            return;
        }
        for (key, info) in self.active_channels.iter_mut() {
            if !info.looped && !self.system.channel_is_playing(info.channel) {
                finished.push(key.clone());
                continue;
            }
            let (mut mix_volume, pan) = compute_mix_values(info, snapshot, focus, viewport_center);
            mix_volume *= self.options.sound_volume;
            updates.push((info.channel, mix_volume, pan));
        }
        for (channel, volume, pan) in updates {
            self.system.channel_set_volume_and_pan(channel, volume, pan);
        }
        for key in finished {
            if let Some(info) = self.active_channels.remove(&key) {
                self.system.halt_channel(info.channel);
            }
        }
    }

    fn ensure_sound(&mut self, name: &str) -> Result<Option<SoundHandle>, AudioError> {
        let request_key = name.to_ascii_lowercase();
        if let Some(resolved) = self.resolver.resolve_entry(name) {
            let cache_key = resolved.cache_key();
            if let Some(handle) = self.loaded_sounds.get(&cache_key) {
                return Ok(Some(handle.clone()));
            }
            match resolved.load_audio() {
                Ok(bytes) => {
                    let handle = self.system.load_sound(bytes.as_slice())?;
                    self.loaded_sounds.insert(cache_key.clone(), handle.clone());
                    return Ok(Some(handle));
                }
                Err(err) => {
                    if self
                        .missing_sounds
                        .insert(format!("asset::{}", resolved.cache_marker()))
                    {
                        tracing::warn!(
                            sound = %name,
                            library = %resolved.describe(),
                            error = %err,
                            "failed to load sound asset"
                        );
                    }
                    return Ok(None);
                }
            }
        }

        if self
            .missing_sounds
            .insert(format!("request::{request_key}"))
        {
            tracing::warn!(sound = %name, "missing sound asset; skipping playback");
        }
        Ok(None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SoundInstanceKey {
    name: String,
    target: Option<ObjectId>,
}

impl SoundInstanceKey {
    fn new(name: &str, target: Option<ObjectId>) -> Self {
        Self {
            name: name.to_ascii_lowercase(),
            target,
        }
    }
}

#[derive(Clone)]
struct ChannelInfo {
    channel: ChannelId,
    looped: bool,
    target: Option<ObjectId>,
    volume: u8,
    custom_falloff: Option<i32>,
}

struct SoundResolver {
    global: Vec<SoundLibrary>,
    scenario: Vec<SoundLibrary>,
    scenario_root: Option<PathBuf>,
    registered_definitions: HashSet<String>,
}

impl SoundResolver {
    fn new() -> Self {
        let global = discover_global_sound_libraries();
        Self {
            global,
            scenario: Vec::new(),
            scenario_root: None,
            registered_definitions: HashSet::new(),
        }
    }

    fn configure_scenario(&mut self, path: Option<&Path>) -> bool {
        let new_root = path.map(|p| p.to_path_buf());
        if self
            .scenario_root
            .as_ref()
            .map(|existing| existing.as_path())
            == new_root.as_ref().map(|p| p.as_path())
        {
            return false;
        }

        self.scenario = match new_root.as_ref() {
            Some(root) => collect_sound_libraries_for_path(root),
            None => Vec::new(),
        };
        self.scenario_root = new_root;
        true
    }

    fn resolve_entry(&self, name: &str) -> Option<ResolvedSound<'_>> {
        let terms = SoundSearchTerms::new(name);
        for library in self.scenario.iter().chain(self.global.iter()) {
            if let Some(index) = library.find_entry(&terms) {
                return Some(ResolvedSound {
                    library,
                    entry_index: index,
                });
            }
        }
        None
    }

    fn register_definition_group(&mut self, definition_id: &str, group: &Group) {
        let key = format!(
            "{}::{}",
            definition_id.to_ascii_lowercase(),
            group.root().to_string_lossy().to_ascii_lowercase()
        );
        if !self.registered_definitions.insert(key) {
            return;
        }
        let label = format!("definition::{}", definition_id);
        let mut libs = collect_sound_libraries_from_group(group, label);
        if !libs.is_empty() {
            self.global.extend(libs);
        }
    }
}

struct SoundLibrary {
    label: String,
    cache_prefix: String,
    source: Arc<Group>,
    entries: Vec<SoundEntry>,
    by_file_name: HashMap<String, Vec<usize>>,
}

impl SoundLibrary {
    fn new(label: String, source: Arc<Group>) -> Self {
        let cache_prefix = source.root().to_string_lossy().to_ascii_lowercase();
        Self {
            label,
            cache_prefix,
            source,
            entries: Vec::new(),
            by_file_name: HashMap::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    fn add_entry(&mut self, relative_path: PathBuf) {
        let file_name = relative_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| relative_path.to_string_lossy().to_string());
        let file_key = file_name.to_ascii_lowercase();
        let entry = SoundEntry {
            relative_path,
            file_name: file_key.clone(),
            extension_rank: extension_rank(
                Path::new(&file_name)
                    .extension()
                    .and_then(|ext| ext.to_str()),
            ),
        };
        let index = self.entries.len();
        self.entries.push(entry);
        self.by_file_name.entry(file_key).or_default().push(index);
    }

    fn find_entry(&self, terms: &SoundSearchTerms) -> Option<usize> {
        if let Some(pattern) = &terms.wildcard_pattern {
            return self.find_wildcard(pattern);
        }
        for file_name in &terms.search_names {
            if let Some(indices) = self.by_file_name.get(file_name) {
                return Some(self.pick_best_index(indices));
            }
        }
        None
    }

    fn find_wildcard(&self, pattern: &str) -> Option<usize> {
        let mut matches = Vec::new();
        for (index, entry) in self.entries.iter().enumerate() {
            if matches_sound_pattern(pattern, &entry.file_name) {
                matches.push(index);
            }
        }
        match matches.len() {
            0 => None,
            1 => matches.first().copied(),
            _ => Some(self.pick_best_index(&matches)),
        }
    }

    fn pick_best_index(&self, indices: &[usize]) -> usize {
        let mut best = *indices.first().unwrap();
        let mut best_rank = self.entries[best].extension_rank;
        for &index in indices.iter().skip(1) {
            let rank = self.entries[index].extension_rank;
            if rank < best_rank || (rank == best_rank && index > best) {
                best = index;
                best_rank = rank;
            }
        }
        best
    }

    fn cache_key(&self, index: usize) -> String {
        format!(
            "{}::{}",
            self.cache_prefix,
            self.entries[index]
                .relative_path
                .to_string_lossy()
                .to_ascii_lowercase()
        )
    }

    fn cache_marker(&self, index: usize) -> String {
        self.cache_key(index)
    }

    fn describe_entry(&self, index: usize) -> String {
        format!(
            "{}::{}",
            self.label,
            self.entries[index].relative_path.display()
        )
    }

    fn read_bytes(&self, index: usize) -> Result<Vec<u8>, lc_resources::GroupError> {
        self.source.read_file(&self.entries[index].relative_path)
    }
}

struct SoundEntry {
    relative_path: PathBuf,
    file_name: String,
    extension_rank: usize,
}

struct ResolvedSound<'a> {
    library: &'a SoundLibrary,
    entry_index: usize,
}

impl<'a> ResolvedSound<'a> {
    fn cache_key(&self) -> String {
        self.library.cache_key(self.entry_index)
    }

    fn cache_marker(&self) -> String {
        self.library.cache_marker(self.entry_index)
    }

    fn describe(&self) -> String {
        self.library.describe_entry(self.entry_index)
    }

    fn load_audio(&self) -> Result<Vec<u8>, lc_resources::GroupError> {
        self.library.read_bytes(self.entry_index)
    }
}

struct SoundSearchTerms {
    wildcard_pattern: Option<String>,
    search_names: Vec<String>,
}

impl SoundSearchTerms {
    fn new(name: &str) -> Self {
        let trimmed = name.trim();
        let (stem_lower, has_extension) = split_stem_and_extension(trimmed);
        let mut prepared = trimmed.to_string();
        if !has_extension {
            prepared.push_str(".wav");
        }
        let has_wildcards = prepared.contains('*') || prepared.contains('?');
        let normalized_lower = prepared.to_ascii_lowercase();

        let wildcard_pattern = if has_wildcards {
            Some(normalized_lower.clone())
        } else {
            None
        };

        let mut search_names = Vec::new();
        if !has_wildcards {
            search_names.push(normalized_lower.clone());
            if !has_extension {
                for ext in ["ogg", "mp3"] {
                    let candidate = format!("{}.{}", stem_lower, ext);
                    if candidate != normalized_lower {
                        search_names.push(candidate);
                    }
                }
            }
        }

        Self {
            wildcard_pattern,
            search_names,
        }
    }
}

fn split_stem_and_extension(name: &str) -> (String, bool) {
    if let Some(pos) = name.rfind('.') {
        let stem = &name[..pos];
        let ext = &name[pos + 1..];
        if !stem.is_empty() && !ext.is_empty() && !ext.contains('*') && !ext.contains('?') {
            return (stem.to_ascii_lowercase(), true);
        }
    }
    (name.to_ascii_lowercase(), false)
}

fn discover_global_sound_libraries() -> Vec<SoundLibrary> {
    let mut libraries = Vec::new();
    match AppPaths::discover() {
        Ok(paths) => {
            let mut seen = HashSet::new();
            for root in [
                paths.install_root().to_path_buf(),
                paths.planet_dir().to_path_buf(),
                paths.user_data_dir().to_path_buf(),
            ] {
                for candidate in find_sound_group_candidates(&root) {
                    let key = candidate.to_string_lossy().to_ascii_lowercase();
                    if !seen.insert(key) {
                        continue;
                    }
                    let mut libs = collect_sound_libraries_for_path(&candidate);
                    libraries.append(&mut libs);
                }
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "sound asset discovery skipped");
        }
    }
    libraries
}

fn collect_sound_libraries_for_path(path: &Path) -> Vec<SoundLibrary> {
    let group = match Group::open(path) {
        Ok(group) => group,
        Err(err) => {
            tracing::warn!(path = %path.display(), error = %err, "failed to open sound group");
            return Vec::new();
        }
    };
    let label = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| path.to_string_lossy().to_string());
    collect_sound_libraries_from_group(&group, label)
}

fn collect_sound_libraries_from_group(group: &Group, label: String) -> Vec<SoundLibrary> {
    let mut libs = Vec::new();
    if let Err(err) = collect_sound_libraries_recursive(group, label.as_str(), &mut libs) {
        tracing::warn!(
            path = %group.root().display(),
            error = %err,
            "failed to inspect sound entries"
        );
    }
    libs
}

fn collect_sound_libraries_recursive(
    group: &Group,
    label: &str,
    libs: &mut Vec<SoundLibrary>,
) -> Result<(), lc_resources::GroupError> {
    let source = Arc::new(group.clone());
    let mut library = SoundLibrary::new(label.to_string(), source);
    for entry in group.entries()? {
        if entry.is_directory {
            let child = group.open_child(&entry.relative_path)?;
            let child_label = if label.is_empty() {
                entry.relative_path.to_string_lossy().into_owned()
            } else {
                format!("{}/{}", label, entry.relative_path.display())
            };
            collect_sound_libraries_recursive(&child, &child_label, libs)?;
        } else if is_audio_path(&entry.relative_path) {
            library.add_entry(entry.relative_path.clone());
        }
    }
    if !library.is_empty() {
        libs.push(library);
    }
    Ok(())
}

fn find_sound_group_candidates(root: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    for name in [
        "Sound.c4g",
        "sound.c4g",
        "Sound.ocg",
        "sound.ocg",
        "Sound.c4d",
        "sound.c4d",
    ] {
        let candidate = root.join(name);
        if candidate.exists() {
            result.push(candidate);
        }
    }

    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name_lower = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if is_probable_sound_container(&path, &name_lower) {
                result.push(path);
            }
        }
    }

    result
}

fn is_probable_sound_container(path: &Path, name_lower: &str) -> bool {
    if !name_lower.starts_with("sound") {
        return false;
    }
    if path.is_dir() {
        return true;
    }
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) => {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "c4g" | "ocg" | "c4d" | "c4s"
            )
        }
        None => false,
    }
}

fn matches_sound_pattern(pattern: &str, candidate: &str) -> bool {
    let pattern = pattern.as_bytes();
    let candidate = candidate.as_bytes();

    let mut p = 0;
    let mut c = 0;
    let mut star = None;
    let mut match_index = 0;

    while c < candidate.len() {
        if p < pattern.len() && (pattern[p] == candidate[c] || pattern[p] == b'?') {
            p += 1;
            c += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some(p);
            match_index = c;
            p += 1;
        } else if let Some(star_index) = star {
            p = star_index + 1;
            match_index += 1;
            c = match_index;
        } else {
            return false;
        }
    }

    while p < pattern.len() && pattern[p] == b'*' {
        p += 1;
    }

    p == pattern.len()
}

fn extension_rank(ext: Option<&str>) -> usize {
    match ext.map(|value| value.to_ascii_lowercase()) {
        Some(ref ext) if ext == "wav" => 0,
        Some(ref ext) if ext == "ogg" => 1,
        Some(ref ext) if ext == "mp3" => 2,
        _ => 3,
    }
}

struct GameApp {
    engine: Engine,
    graphics: GraphicsSystem,
    sky: Option<SkyRenderState>,
    input: InputDispatcher,
    bindings: KeyboardBindings,
    gamepads: GamepadManager,
    snapshot: SimulationSnapshot,
    focus_id: Option<ObjectId>,
    focus_snapshot: Option<lc_engine::ObjectSnapshot>,
    frame_text: String,
    status_text: String,
    energy_fraction: f32,
    scenario_label: String,
    fallback_ground: i32,
    menu_state: MenuState,
    object_menu: Option<ObjectMenuState>,
    ingame_menu: Option<IngameMenuState>,
    mode: AppMode,
    scenario_catalog: HashMap<String, FrontendScenario>,
    active_scenario: Option<FrontendScenario>,
    audio: Option<AudioContext>,
    assets: Arc<FrontendAssets>,
    material_library: Option<Arc<MaterialSet>>,
    network: Option<NetworkManager>,
    local_owner: i32,
    last_save_path: Option<PathBuf>,
    object_sprites: HashMap<String, ImageData>,
    sprite_cache: Arc<HashMap<String, ImageData>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppMode {
    Menu,
    Running,
}

struct MenuState {
    menu: StartupMenu,
    pointer_position: Option<GuiPoint>,
    stack: Vec<MenuLayer>,
}

#[derive(Clone, Debug)]
struct MenuLayer {
    title: String,
    entries: Vec<FrontendScenario>,
}

impl MenuLayer {
    fn new(title: impl Into<String>, entries: Vec<FrontendScenario>) -> Self {
        Self {
            title: title.into(),
            entries,
        }
    }
}

impl MenuState {
    fn new(menu: StartupMenu, entries: Vec<FrontendScenario>) -> Self {
        Self {
            menu,
            pointer_position: None,
            stack: vec![MenuLayer::new("Scenarios", entries)],
        }
    }

    fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer_position
    }

    fn set_pointer_position(&mut self, position: Option<GuiPoint>) {
        self.pointer_position = position;
    }

    fn current_entries(&self) -> &[FrontendScenario] {
        self.stack
            .last()
            .map(|layer| layer.entries.as_slice())
            .unwrap_or_default()
    }

    fn menu(&mut self) -> &mut StartupMenu {
        &mut self.menu
    }

    fn enter_folder(&mut self, identifier: &str) {
        let Some(folder) = self
            .current_entries()
            .iter()
            .find(|entry| {
                entry.identifier == identifier && matches!(entry.kind, ScenarioKind::Folder)
            })
            .cloned()
        else {
            return;
        };

        self.stack
            .push(MenuLayer::new(folder.title.clone(), folder.children));
        self.pointer_position = None;
        self.refresh_menu_entries();
    }

    fn leave_folder(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        self.stack.pop();
        self.pointer_position = None;
        self.refresh_menu_entries();
    }

    fn refresh_menu_entries(&mut self) {
        let include_back = self.stack.len() > 1;
        let entries = build_menu_entries(self.current_entries(), include_back);
        if let Err(err) = self.menu.set_entries(entries) {
            tracing::error!(error = %err, "failed to update startup menu entries");
        }
    }

    fn label_path(&self) -> String {
        if self.stack.len() <= 1 {
            return DEFAULT_SCENARIO_LABEL.to_string();
        }
        self.stack
            .iter()
            .skip(1)
            .map(|layer| layer.title.clone())
            .collect::<Vec<_>>()
            .join(" / ")
    }
}

const PLACEHOLDER_PREVIEW_WIDTH: u32 = 320;
const PLACEHOLDER_PREVIEW_HEIGHT: u32 = 200;

fn generate_preview_placeholder(kind: ScenarioKind, title: &str) -> ImageData {
    let (top, bottom, accent) = preview_palette(kind);
    let mut pixels =
        vec![0u8; (PLACEHOLDER_PREVIEW_WIDTH * PLACEHOLDER_PREVIEW_HEIGHT * 4) as usize];

    let mut hasher = DefaultHasher::new();
    title.hash(&mut hasher);
    let seed = hasher.finish();

    let stripe_spacing = 5 + (seed % 5) as u32;
    let stripe_offset = if stripe_spacing == 0 {
        0
    } else {
        (seed as u32) % stripe_spacing
    };
    let noise_seed = seed.rotate_left(17) ^ 0x9e37_79b9_7f4a_7c15;
    let highlight_start = PLACEHOLDER_PREVIEW_HEIGHT.saturating_sub(48);

    for y in 0..PLACEHOLDER_PREVIEW_HEIGHT {
        let t = if PLACEHOLDER_PREVIEW_HEIGHT > 1 {
            y as f32 / (PLACEHOLDER_PREVIEW_HEIGHT - 1) as f32
        } else {
            0.0
        };
        let mut base = lerp_color(top, bottom, t);
        if y >= highlight_start {
            let emphasis = (y - highlight_start) as f32 / 48.0;
            base = blend_toward(base, accent, (0.25 + emphasis * 0.45).clamp(0.0, 0.65));
        }

        for x in 0..PLACEHOLDER_PREVIEW_WIDTH {
            let mut color = base;
            if ((x + y + stripe_offset) % stripe_spacing) == 0 {
                color = blend_toward(color, accent, 0.35);
            }

            let base_noise = noise_seed
                .wrapping_add((x as u64 + 1).wrapping_mul(0x9e37_79b9_7f4a_7c15))
                .wrapping_add((y as u64 + 1).wrapping_mul(0xbf58_476d_1ce4_e5b9));
            let noise = (base_noise ^ (base_noise >> 32)) as u8;
            let jitter = (noise as i16 - 128) / 18;
            color = adjust_color_brightness(color, jitter);

            let idx = ((y * PLACEHOLDER_PREVIEW_WIDTH + x) * 4) as usize;
            pixels[idx] = color.r;
            pixels[idx + 1] = color.g;
            pixels[idx + 2] = color.b;
            pixels[idx + 3] = color.a;
        }
    }

    ImageData::new(
        PLACEHOLDER_PREVIEW_WIDTH,
        PLACEHOLDER_PREVIEW_HEIGHT,
        pixels,
    )
}

fn preview_palette(kind: ScenarioKind) -> (Color, Color, Color) {
    match kind {
        ScenarioKind::Scenario => (
            Color::opaque(36, 52, 104),
            Color::opaque(14, 20, 40),
            Color::opaque(220, 184, 104),
        ),
        ScenarioKind::Folder => (
            Color::opaque(30, 68, 72),
            Color::opaque(14, 26, 32),
            Color::opaque(160, 216, 200),
        ),
        ScenarioKind::Editor => (
            Color::opaque(96, 52, 32),
            Color::opaque(32, 20, 16),
            Color::opaque(228, 164, 100),
        ),
    }
}

fn lerp_color(start: Color, end: Color, t: f32) -> Color {
    let clamped = t.clamp(0.0, 1.0);
    let lerp_channel = |s: u8, e: u8| -> u8 {
        (s as f32 + (e as f32 - s as f32) * clamped)
            .round()
            .clamp(0.0, 255.0) as u8
    };
    Color::new(
        lerp_channel(start.r, end.r),
        lerp_channel(start.g, end.g),
        lerp_channel(start.b, end.b),
        255,
    )
}

fn blend_toward(base: Color, target: Color, factor: f32) -> Color {
    lerp_color(base, target, factor.clamp(0.0, 1.0))
}

fn adjust_color_brightness(color: Color, delta: i16) -> Color {
    let adjust = |channel: u8| -> u8 {
        let value = channel as i16 + delta;
        value.clamp(0, 255) as u8
    };
    Color::new(adjust(color.r), adjust(color.g), adjust(color.b), color.a)
}

#[derive(Clone, Debug)]
struct FrontendScenario {
    identifier: String,
    title: String,
    description: Option<String>,
    kind: ScenarioKind,
    is_editable: bool,
    is_playable: bool,
    path: Option<PathBuf>,
    preview: Option<ImageData>,
    children: Vec<FrontendScenario>,
}

impl FrontendScenario {
    fn to_ui_entry(&self) -> ScenarioEntry {
        ScenarioEntry {
            identifier: self.identifier.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            kind: self.kind,
            is_editable: self.is_editable,
            is_playable: self.is_playable,
            location: self.location_label(),
            preview: self.preview.clone(),
        }
    }

    fn from_resource(
        entry: resource_scenario::ScenarioEntry,
        seen: &mut HashSet<String>,
    ) -> Option<Self> {
        let identifier = entry.identifier.clone();
        let kind = match entry.kind {
            resource_scenario::ScenarioEntryKind::Scenario => ScenarioKind::Scenario,
            resource_scenario::ScenarioEntryKind::Folder => ScenarioKind::Folder,
            resource_scenario::ScenarioEntryKind::Editor => ScenarioKind::Editor,
        };

        let mut children = Vec::new();
        for child in entry.children {
            if let Some(converted) = FrontendScenario::from_resource(child, seen) {
                children.push(converted);
            }
        }

        let preview = entry
            .preview
            .as_ref()
            .map(|preview| {
                ImageData::from_arc(preview.width(), preview.height(), preview.clone_data())
            })
            .or_else(|| Some(generate_preview_placeholder(kind.clone(), &entry.title)));

        if matches!(kind, ScenarioKind::Scenario) && !seen.insert(identifier.clone()) {
            return None;
        }

        Some(Self {
            identifier,
            title: entry.title,
            description: entry.description,
            kind,
            is_editable: entry.is_editable,
            is_playable: entry.is_playable,
            path: Some(entry.path),
            preview,
            children,
        })
    }

    fn location_label(&self) -> Option<String> {
        if let Some(path) = self.path.as_ref() {
            return Some(path.display().to_string());
        }
        if self.path.is_none() && matches!(self.kind, ScenarioKind::Scenario) {
            return Some("Built-in Rust sandbox".to_string());
        }
        None
    }

    fn fallback() -> Self {
        Self {
            identifier: "rust_sandbox".to_string(),
            title: DEFAULT_SCENARIO_LABEL.to_string(),
            description: Some("Spawn a Rust-driven walker in a flat test landscape.".to_string()),
            kind: ScenarioKind::Scenario,
            is_editable: true,
            is_playable: true,
            path: None,
            preview: Some(generate_preview_placeholder(
                ScenarioKind::Scenario,
                DEFAULT_SCENARIO_LABEL,
            )),
            children: Vec::new(),
        }
    }
}

struct InstallDefinitionResolver {
    app_paths: Option<Arc<AppPaths>>,
}

impl InstallDefinitionResolver {
    fn new(app_paths: Option<Arc<AppPaths>>) -> Self {
        Self { app_paths }
    }

    fn sanitize_identifier(identifier: &str) -> Option<PathBuf> {
        let mut slice = identifier.trim();
        if slice.is_empty() {
            return None;
        }
        slice = slice.trim_matches(|c| c == '"' || c == '\'');
        while let Some(stripped) = slice.strip_prefix("./") {
            slice = stripped;
        }
        while let Some(stripped) = slice.strip_prefix(".\\") {
            slice = stripped;
        }
        slice = slice.trim_matches('/');
        if slice.is_empty() {
            return None;
        }
        let normalized = slice.replace('\\', "/");
        let normalized = normalized.trim_matches('/').to_string();
        if normalized.is_empty() {
            return None;
        }
        Some(PathBuf::from(normalized))
    }

    fn open_and_push(
        path: &Path,
        groups: &mut Vec<Group>,
        seen: &mut HashSet<PathBuf>,
    ) -> Result<(), ScenarioError> {
        match Group::open(path) {
            Ok(group) => Self::push_group(groups, seen, group),
            Err(err) if Self::should_ignore_error(&err) => {}
            Err(err) => return Err(ScenarioError::Resources(err)),
        }
        Ok(())
    }

    fn push_group(groups: &mut Vec<Group>, seen: &mut HashSet<PathBuf>, group: Group) {
        let root = group.root().to_path_buf();
        if seen.insert(root) {
            groups.push(group);
        }
    }

    fn should_ignore_error(err: &GroupError) -> bool {
        matches!(
            err,
            GroupError::Missing(_) | GroupError::NotDirectory(_) | GroupError::EntryNotFound(_)
        ) || matches!(err, GroupError::Io(io_err) if io_err.kind() == io::ErrorKind::NotFound)
    }
}

impl LegacyDefinitionResolver for InstallDefinitionResolver {
    fn resolve_definition_groups(
        &self,
        scenario: &Group,
        identifier: &str,
    ) -> Result<Vec<Group>, ScenarioError> {
        let Some(relative) = Self::sanitize_identifier(identifier) else {
            return Err(ScenarioError::LegacyDefinitionNotFound {
                path: identifier.to_string(),
            });
        };
        let mut groups = Vec::new();
        let mut seen = HashSet::new();

        if relative.is_absolute() {
            Self::open_and_push(&relative, &mut groups, &mut seen)?;
        } else {
            if let Some(child) =
                open_child_flexible(scenario, &relative).map_err(ScenarioError::Resources)?
            {
                Self::push_group(&mut groups, &mut seen, child);
            }

            for ancestor in scenario.root().ancestors() {
                if let Ok(group) = Group::open(ancestor) {
                    if let Some(child) =
                        open_child_flexible(&group, &relative).map_err(ScenarioError::Resources)?
                    {
                        Self::push_group(&mut groups, &mut seen, child);
                    }
                }

                let candidate = ancestor.join(&relative);
                match Group::open(&candidate) {
                    Ok(group) => Self::push_group(&mut groups, &mut seen, group),
                    Err(err) if Self::should_ignore_error(&err) => {}
                    Err(err) => return Err(ScenarioError::Resources(err)),
                }
            }

            if let Some(paths) = &self.app_paths {
                let mut base_candidates = vec![
                    paths.install_root().to_path_buf(),
                    paths.planet_dir().to_path_buf(),
                    paths.system_group_path().to_path_buf(),
                    paths.user_data_dir().to_path_buf(),
                    paths.scenario_dir(),
                ];
                if let Some(parent) = paths.system_group_path().parent() {
                    base_candidates.push(parent.to_path_buf());
                }
                let mut base_seen = HashSet::new();
                for base in base_candidates {
                    if !base_seen.insert(base.clone()) {
                        continue;
                    }

                    if let Ok(group) = Group::open(&base) {
                        if let Some(child) = open_child_flexible(&group, &relative)
                            .map_err(ScenarioError::Resources)?
                        {
                            Self::push_group(&mut groups, &mut seen, child);
                        }
                    }

                    let candidate = base.join(&relative);
                    match Group::open(&candidate) {
                        Ok(group) => Self::push_group(&mut groups, &mut seen, group),
                        Err(err) if Self::should_ignore_error(&err) => {}
                        Err(err) => return Err(ScenarioError::Resources(err)),
                    }
                }
            }
        }

        if groups.is_empty() {
            Err(ScenarioError::LegacyDefinitionNotFound {
                path: identifier.to_string(),
            })
        } else {
            Ok(groups)
        }
    }
}

fn open_child_flexible(group: &Group, relative: &Path) -> Result<Option<Group>, GroupError> {
    match group.open_child(relative) {
        Ok(child) => Ok(Some(child)),
        Err(err) => match err {
            GroupError::EntryNotFound(_) | GroupError::Missing(_) | GroupError::NotDirectory(_) => {
                match open_child_case_insensitive(group, relative) {
                    Ok(child) => Ok(Some(child)),
                    Err(GroupError::EntryNotFound(_)) => Ok(None),
                    Err(other) => Err(other),
                }
            }
            other => Err(other),
        },
    }
}

fn open_child_case_insensitive(group: &Group, relative: &Path) -> Result<Group, GroupError> {
    let mut current = group.clone();
    let mut consumed = PathBuf::new();

    for component in relative.components() {
        match component {
            Component::Normal(name) => {
                let target = name.to_string_lossy().to_ascii_lowercase();
                let entries = current.entries()?;
                let matched = entries.into_iter().find(|entry| {
                    if entry.relative_path.components().count() != 1 {
                        return false;
                    }
                    entry
                        .relative_path
                        .file_name()
                        .and_then(|candidate| candidate.to_str())
                        .map(|candidate| candidate.eq_ignore_ascii_case(&target))
                        .unwrap_or(false)
                });

                let entry = match matched {
                    Some(entry) => entry,
                    None => {
                        let mut missing = consumed.clone();
                        missing.push(name);
                        return Err(GroupError::EntryNotFound(missing));
                    }
                };

                consumed.push(&entry.relative_path);
                current = current.open_child(&entry.relative_path)?;
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                let mut invalid = consumed.clone();
                invalid.push(component.as_os_str());
                return Err(GroupError::EntryNotFound(invalid));
            }
        }
    }

    Ok(current)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedScenarioInfo {
    identifier: String,
    title: String,
    description: Option<String>,
    path: Option<PathBuf>,
    is_editable: bool,
    is_playable: bool,
    label: String,
    fallback_ground: i32,
    sandbox: bool,
}

impl SavedScenarioInfo {
    fn from_frontend(frontend: &FrontendScenario, label: &str, fallback_ground: i32) -> Self {
        Self {
            identifier: frontend.identifier.clone(),
            title: frontend.title.clone(),
            description: frontend.description.clone(),
            path: frontend.path.clone(),
            is_editable: frontend.is_editable,
            is_playable: frontend.is_playable,
            label: label.to_string(),
            fallback_ground,
            sandbox: frontend.path.is_none(),
        }
    }

    fn to_frontend(&self) -> FrontendScenario {
        FrontendScenario {
            identifier: self.identifier.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            kind: ScenarioKind::Scenario,
            is_editable: self.is_editable,
            is_playable: self.is_playable,
            path: self.path.clone(),
            preview: Some(generate_preview_placeholder(
                ScenarioKind::Scenario,
                &self.title,
            )),
            children: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SaveFileVersion {
    major: u16,
    minor: u16,
    patch: u16,
}

impl SaveFileVersion {
    const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    const fn major(self) -> u16 {
        self.major
    }

    fn parse_str(input: &str) -> Result<Self, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err("save file version cannot be empty".to_string());
        }

        let mut components = trimmed.split('.').collect::<Vec<_>>();
        if components.len() > 3 {
            return Err(format!(
                "save file version `{trimmed}` has too many components"
            ));
        }

        while components.len() < 3 {
            components.push("0");
        }

        let major = Self::parse_component(components[0], "major")?;
        let minor = Self::parse_component(components[1], "minor")?;
        let patch = Self::parse_component(components[2], "patch")?;
        Ok(Self::new(major, minor, patch))
    }

    fn from_numeric(value: u64) -> Result<Self, String> {
        if value > u16::MAX as u64 {
            return Err(format!(
                "legacy save file version `{value}` exceeds supported range"
            ));
        }
        Ok(Self::new(value as u16, 0, 0))
    }

    fn parse_component(component: &str, name: &str) -> Result<u16, String> {
        if component.is_empty() {
            return Err(format!("save file version has empty {name} component"));
        }
        component
            .parse::<u16>()
            .map_err(|_| format!("save file version `{component}` is not a valid {name} number"))
    }
}

impl fmt::Display for SaveFileVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Serialize for SaveFileVersion {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for SaveFileVersion {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: de::Deserializer<'de>,
    {
        struct SaveFileVersionVisitor;

        impl<'de> Visitor<'de> for SaveFileVersionVisitor {
            type Value = SaveFileVersion;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a semantic version string like \"1.0.0\" or legacy integer")
            }

            fn visit_u64<E>(self, value: u64) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                SaveFileVersion::from_numeric(value).map_err(E::custom)
            }

            fn visit_i64<E>(self, value: i64) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                if value < 0 {
                    return Err(E::invalid_value(
                        Unexpected::Signed(value),
                        &"non-negative version number",
                    ));
                }
                self.visit_u64(value as u64)
            }

            fn visit_str<E>(self, value: &str) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                SaveFileVersion::parse_str(value).map_err(E::custom)
            }

            fn visit_string<E>(self, value: String) -> std::result::Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_str(&value)
            }
        }

        deserializer.deserialize_any(SaveFileVersionVisitor)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedGameFile {
    version: SaveFileVersion,
    saved_at_seconds: u64,
    scenario: SavedScenarioInfo,
    focus_id: Option<ObjectId>,
    engine_state: EngineState,
}

struct SaveMigration {
    from: SaveFileVersion,
    to: SaveFileVersion,
    apply: fn(SavedGameFile) -> Result<SavedGameFile>,
}

const SAVE_MIGRATIONS: &[SaveMigration] = &[];

fn migrate_save_file(save: SavedGameFile) -> Result<SavedGameFile> {
    if save.version == SAVE_FILE_VERSION {
        return Ok(save);
    }

    if save.version.major() > SAVE_FILE_VERSION.major() {
        anyhow::bail!(
            "quick save requires lc-app {} or newer (current engine {})",
            save.version,
            SAVE_FILE_VERSION
        );
    }

    if save.version.major() < SAVE_FILE_VERSION.major() {
        anyhow::bail!(
            "quick save version {} cannot be loaded by this engine (current {})",
            save.version,
            SAVE_FILE_VERSION
        );
    }

    apply_save_migrations(save)
}

fn apply_save_migrations(mut save: SavedGameFile) -> Result<SavedGameFile> {
    let mut applied = 0usize;
    while save.version < SAVE_FILE_VERSION {
        if let Some(migration) = SAVE_MIGRATIONS
            .iter()
            .find(|candidate| candidate.from == save.version)
        {
            tracing::info!(
                from = %migration.from,
                to = %migration.to,
                "applying quick save migration"
            );
            save = (migration.apply)(save)?;
            applied = applied
                .checked_add(1)
                .ok_or_else(|| anyhow!("quick save migration overflow"))?;
            if applied > SAVE_MIGRATIONS.len() {
                anyhow::bail!("detected cycle in quick save migrations");
            }
            continue;
        }

        tracing::warn!(
            from = %save.version,
            to = %SAVE_FILE_VERSION,
            "no explicit migration for quick save version; assuming backward compatibility"
        );
        save.version = SAVE_FILE_VERSION;
    }

    Ok(save)
}

fn cached_app_paths() -> std::result::Result<Arc<AppPaths>, PathsError> {
    #[cfg(test)]
    let _env_guard = crate::tests::env_lock().lock();
    let mut cache = APP_PATH_CACHE.lock().unwrap();
    if let Some(result) = cache.as_ref() {
        return result.clone();
    }

    let discovered = AppPaths::discover().map(Arc::new);
    *cache = Some(discovered.clone());
    discovered
}

#[cfg(test)]
fn reset_cached_app_paths() {
    let mut cache = APP_PATH_CACHE.lock().unwrap();
    *cache = None;
}

fn resolve_save_directory() -> PathBuf {
    match cached_app_paths() {
        Ok(paths) => paths.user_data_dir().join(SAVE_DIR_NAME),
        Err(_) => PathBuf::from(SAVE_DIR_NAME),
    }
}

fn ensure_save_directory() -> Result<PathBuf> {
    let dir = resolve_save_directory();
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create save directory at {}", dir.display()))?;
    Ok(dir)
}

fn default_quick_save_path() -> PathBuf {
    resolve_save_directory().join(QUICK_SAVE_FILE)
}

fn existing_quick_save_path() -> Option<PathBuf> {
    let path = default_quick_save_path();
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

fn load_install_material_library(paths: Option<&AppPaths>) -> Option<Arc<MaterialSet>> {
    let paths = match paths {
        Some(paths) => paths,
        None => return None,
    };

    let mut seen = HashSet::new();
    for candidate in candidate_material_paths(paths) {
        if !candidate.exists() {
            continue;
        }
        let key = candidate.to_string_lossy().to_ascii_lowercase();
        if !seen.insert(key) {
            continue;
        }
        match try_materials_from_path(&candidate) {
            Ok(set) if !set.is_empty() => {
                let count = set.len();
                tracing::info!(path = %candidate.display(), count, "loaded material definitions");
                return Some(Arc::new(set));
            }
            Ok(_) => {
                tracing::debug!(path = %candidate.display(), "material candidate contained no definitions");
            }
            Err(lc_resources::MaterialError::NotFound) => {}
            Err(err) => {
                tracing::debug!(path = %candidate.display(), error = %err, "material discovery attempt failed");
            }
        }
    }
    tracing::info!("no install material definitions found; using sandbox defaults");
    None
}

fn candidate_material_paths(paths: &AppPaths) -> Vec<PathBuf> {
    const GROUP_NAMES: &[&str] = &[
        "Material.c4g",
        "Material.ocg",
        "Material.ocd",
        "MatDefs.ocg",
        "MatDefs.c4g",
    ];

    let mut candidates = Vec::new();

    let planet_dir = paths.planet_dir();
    if planet_dir.exists() {
        candidates.push(planet_dir.to_path_buf());
    }
    let install_root = paths.install_root();
    if install_root.exists() {
        candidates.push(install_root.to_path_buf());
    }
    let scenario_dir = paths.scenario_dir();
    if scenario_dir.exists() {
        candidates.push(scenario_dir);
    }
    let system_group = paths.system_group_path();
    if system_group.exists() {
        candidates.push(system_group.to_path_buf());
    }

    for base in [
        paths.planet_dir(),
        paths.install_root(),
        paths.system_group_path(),
    ]
    .into_iter()
    {
        for name in GROUP_NAMES {
            let path = base.join(name);
            if path.exists() {
                candidates.push(path);
            }
        }
    }

    candidates.sort();
    candidates.dedup();
    candidates
}

fn try_materials_from_path(path: &Path) -> Result<MaterialSet, lc_resources::MaterialError> {
    let group = Group::open(path)?;
    let library = lc_resources::MaterialLibrary::from_group(&group)?;
    Ok(MaterialSet::from_resource_library(&library))
}

fn sky_render_state_from_config(config: &SkyConfig) -> SkyRenderState {
    let image = config
        .surface
        .as_ref()
        .map(|image| ImageData::from_arc(image.width(), image.height(), image.clone_pixels()));
    SkyRenderState::new(config.settings.clone(), image)
}

fn current_unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

impl GameApp {
    fn new(
        width: u32,
        height: u32,
        audio_options: AudioOptions,
        paths: Option<&AppPaths>,
        runtime: RuntimeConfig,
    ) -> Result<Self> {
        let network = match runtime.network {
            Some(mode) => Some(NetworkManager::for_mode(mode, runtime.player_owner)?),
            None => None,
        };
        let assets = Arc::new(FrontendAssets::load(paths));
        let material_library = load_install_material_library(paths);
        let base_sprites = assets.base_sprite_map().clone();
        let sprite_cache = Arc::new(base_sprites.clone());

        let mut engine = Engine::new();
        if let Some(library) = material_library.as_ref() {
            engine.set_materials((**library).clone());
        }
        let snapshot = engine.snapshot();
        let scenario_label = DEFAULT_SCENARIO_LABEL.to_string();
        let mut graphics = GraphicsSystem::new(
            width,
            height,
            DEFAULT_GROUND_HEIGHT,
            &scenario_label,
            assets.font_arc(),
            Arc::clone(&sprite_cache),
            assets.cursor_atlas(),
        );
        graphics.surface_mut().fill(Color::opaque(16, 28, 52));

        let scenarios = load_frontend_scenarios();
        let menu_entries = build_menu_entries(&scenarios, false);
        let mut menu = StartupMenu::new(menu_entries, assets.font_arc(), assets.button_textures())
            .map_err(|err| anyhow!("failed to create startup menu: {err}"))?;
        menu.resize(width as f32, height as f32);

        let scenario_catalog = build_scenario_catalog(&scenarios);
        let menu_state = MenuState::new(menu, scenarios);
        let audio = match AudioContext::try_new(audio_options) {
            Ok(ctx) => Some(ctx),
            Err(err) => {
                tracing::warn!(error = %err, "audio initialisation failed");
                None
            }
        };

        let mut app = Self {
            engine,
            graphics,
            sky: None,
            input: InputDispatcher::new(),
            bindings: KeyboardBindings::load(paths),
            gamepads: GamepadManager::new(),
            snapshot,
            focus_id: None,
            focus_snapshot: None,
            frame_text: String::new(),
            status_text: String::new(),
            energy_fraction: 0.0,
            scenario_label,
            fallback_ground: DEFAULT_GROUND_HEIGHT,
            menu_state,
            object_menu: None,
            ingame_menu: None,
            mode: AppMode::Menu,
            scenario_catalog,
            active_scenario: None,
            audio,
            assets: assets.clone(),
            material_library: material_library.clone(),
            last_save_path: None,
            network,
            local_owner: runtime.player_owner,
            object_sprites: base_sprites,
            sprite_cache: Arc::clone(&sprite_cache),
        };
        if let Some(existing) = existing_quick_save_path() {
            app.last_save_path = Some(existing);
        }
        app.ensure_menu_music();
        Ok(app)
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let mut graphics = GraphicsSystem::new(
            width,
            height,
            self.fallback_ground,
            &self.scenario_label,
            self.assets.font_arc(),
            Arc::clone(&self.sprite_cache),
            self.assets.cursor_atlas(),
        );
        graphics.surface_mut().fill(Color::opaque(16, 28, 52));
        self.graphics = graphics;
        self.graphics.set_sky(self.sky.clone());

        if self.mode == AppMode::Menu {
            self.menu_state.menu().resize(width as f32, height as f32);
            self.menu_state.set_pointer_position(None);
        }
        Ok(())
    }

    fn apply_material_library(&mut self) {
        if let Some(materials) = self.material_library.as_ref() {
            self.engine.set_materials((**materials).clone());
        } else {
            self.engine.set_materials(MaterialSet::default());
        }
    }

    fn update_sprite_cache(&mut self) {
        self.sprite_cache = Arc::new(self.object_sprites.clone());
        self.graphics
            .set_object_sprites(Arc::clone(&self.sprite_cache));
    }

    fn derive_ground_height(engine: &Engine, fallback: i32) -> i32 {
        let fallback = fallback.max(0);
        engine
            .landscape()
            .and_then(|landscape| {
                landscape
                    .surface()
                    .iter()
                    .copied()
                    .filter(|height| *height >= 0)
                    .max()
            })
            .unwrap_or(fallback)
    }

    fn rebuild_definition_sprites(&mut self) {
        let mut sprites = self.assets.base_sprite_map().clone();
        for definition_id in self.engine.definition_ids() {
            if let Some(image) = self.engine.definition_sprite_image(definition_id) {
                let data = ImageData::from_arc(image.width(), image.height(), image.into_pixels());
                sprites.insert(definition_id.to_string(), data);
                continue;
            }
            if let Some(image) = self.engine.definition_picture_image(definition_id) {
                let data = ImageData::from_arc(image.width(), image.height(), image.into_pixels());
                sprites.insert(definition_id.to_string(), data);
            }
        }
        if sprites != self.object_sprites {
            self.object_sprites = sprites;
            self.update_sprite_cache();
        }
    }

    fn handle_key(&mut self, key: VirtualKeyCode, state: ElementState) -> Result<(), EngineError> {
        if state == ElementState::Pressed {
            match key {
                VirtualKeyCode::F5 => {
                    if let Err(err) = self.quick_save() {
                        tracing::error!(error = ?err, "quick save failed");
                    }
                    return Ok(());
                }
                VirtualKeyCode::F9 => {
                    if let Err(err) = self.quick_load() {
                        tracing::error!(error = ?err, "quick load failed");
                    }
                    return Ok(());
                }
                _ => {}
            }
        }

        if self.mode == AppMode::Menu {
            if let Some(gui_key) = map_key_code(key) {
                match state {
                    ElementState::Pressed => {
                        self.handle_menu_input(|menu| menu.menu().handle_key_down(gui_key))?
                    }
                    ElementState::Released => {
                        self.handle_menu_input(|menu| menu.menu().handle_key_up(gui_key))?
                    }
                }
            }
            return Ok(());
        }

        if self.mode == AppMode::Running {
            if key == VirtualKeyCode::Escape && state == ElementState::Pressed {
                if self.object_menu.is_some() {
                    self.close_object_menu();
                } else if self.ingame_menu.is_some() {
                    self.close_ingame_menu();
                } else {
                    self.open_ingame_menu();
                }
                return Ok(());
            }
            self.handle_engine_key(key, state)?;
        }
        Ok(())
    }

    fn handle_engine_key(
        &mut self,
        key: VirtualKeyCode,
        state: ElementState,
    ) -> Result<(), EngineError> {
        if let Some(event) = self.bindings.event_for_key(key, state) {
            self.dispatch_control_event(event)?;
        }
        Ok(())
    }

    fn dispatch_control_event(&mut self, event: ControlEvent) -> Result<(), EngineError> {
        if self.mode == AppMode::Running {
            let consumed = if let ControlEvent::Command { command, kind } = event {
                self.handle_menu_command(command, kind)?
            } else {
                false
            };
            if consumed {
                return Ok(());
            }
            if self.object_menu.is_some() || self.ingame_menu.is_some() {
                return Ok(());
            }
        }
        if let Some(network) = self.network.as_ref() {
            let frame = self.engine.frame();
            let tick = u32::try_from(frame).unwrap_or(u32::MAX);
            network.submit_local_control(self.local_owner, event, tick);
        }
        self.dispatch_control_event_for_owner(self.local_owner, event)
    }

    fn dispatch_control_event_for_owner(
        &mut self,
        owner: i32,
        event: ControlEvent,
    ) -> Result<(), EngineError> {
        if owner == self.local_owner {
            if let ControlEvent::Command { command, kind } = event {
                if self.handle_menu_command(command, kind)? {
                    return Ok(());
                }
            }
            if self.object_menu.is_some() || self.ingame_menu.is_some() {
                return Ok(());
            }
        }
        let _ = self.input.handle_event(&mut self.engine, owner, event)?;
        Ok(())
    }

    fn open_ingame_menu(&mut self) {
        if !matches!(self.mode, AppMode::Running) || self.ingame_menu.is_some() {
            return;
        }
        self.close_object_menu();
        let has_quick_save = self
            .last_save_path
            .as_ref()
            .map(|path| path.exists())
            .unwrap_or_else(|| existing_quick_save_path().is_some());
        self.ingame_menu = Some(IngameMenuState::new(has_quick_save));
        if self.status_text.is_empty() {
            self.status_text = "Paused".to_string();
        }
    }

    fn close_ingame_menu(&mut self) {
        self.ingame_menu = None;
        if self.status_text == "Paused" {
            self.status_text.clear();
        }
    }

    fn open_object_menu(&mut self) -> bool {
        if !matches!(self.mode, AppMode::Running) || self.object_menu.is_some() {
            return false;
        }
        match ObjectMenuState::for_player(self.local_owner, &mut self.engine, &self.snapshot) {
            Some(menu) => {
                self.object_menu = Some(menu);
                self.ingame_menu = None;
                if self.status_text.is_empty() {
                    self.status_text = "Inventory open".to_string();
                }
                true
            }
            None => {
                if self.status_text.is_empty() {
                    self.status_text = "No crew inventory available".to_string();
                }
                false
            }
        }
    }

    fn close_object_menu(&mut self) {
        if self.object_menu.is_some() {
            self.object_menu = None;
            if self.status_text == "Inventory open" {
                self.status_text.clear();
            }
        }
    }

    fn handle_menu_command(
        &mut self,
        command: ControlCommand,
        kind: CommandKind,
    ) -> Result<bool, EngineError> {
        if !matches!(self.mode, AppMode::Running) {
            return Ok(false);
        }

        let menu_command = matches!(
            command,
            ControlCommand::MenuEnter
                | ControlCommand::MenuEnterAll
                | ControlCommand::MenuClose
                | ControlCommand::MenuDown
                | ControlCommand::MenuLeft
                | ControlCommand::MenuRight
                | ControlCommand::MenuSelect
                | ControlCommand::MenuShowText
                | ControlCommand::MenuUp
        );

        if menu_command && self.object_menu.is_none() && self.ingame_menu.is_none() {
            return Ok(false);
        }

        if matches!(command, ControlCommand::PlayerMenu) {
            if matches!(
                kind,
                CommandKind::Press | CommandKind::Single | CommandKind::Double
            ) {
                if self.object_menu.is_some() {
                    self.close_object_menu();
                } else if !self.open_object_menu() {
                    self.open_ingame_menu();
                }
            }
            return Ok(true);
        }

        if let Some(menu) = self.object_menu.as_mut() {
            if let Some(action) = menu.handle_command(command, kind) {
                self.execute_object_menu_action(action)?;
            }
            return Ok(true);
        }

        let Some(menu) = self.ingame_menu.as_mut() else {
            return Ok(menu_command);
        };

        if let Some(action) = menu.handle_command(command, kind) {
            self.execute_ingame_menu_action(action)?;
        }
        Ok(true)
    }

    fn execute_object_menu_action(&mut self, action: ObjectMenuAction) -> Result<(), EngineError> {
        match action {
            ObjectMenuAction::Close => {
                self.close_object_menu();
            }
            ObjectMenuAction::Execute { command, selection } => match command {
                ObjectMenuCommand::Focus => {
                    let menu_selection = MenuCommandSelection {
                        primary_id: selection.primary_id,
                        instances: selection.instances.clone(),
                        definition_id: selection.definition_id.clone(),
                        label: selection.label.clone(),
                    };
                    let handled = self.engine.menu_command(
                        selection.crew_id,
                        MenuCommandKind::Focus,
                        menu_selection,
                    )?;
                    self.snapshot = self.engine.snapshot();
                    if handled {
                        self.refresh_object_menu();
                        self.refresh_focus();
                        self.object_menu = None;
                        self.status_text = format!("Executed {} via script", selection.label);
                        return Ok(());
                    }
                    self.object_menu = None;
                    self.focus_id = Some(selection.primary_id);
                    self.focus_snapshot = self.snapshot.object(selection.primary_id).cloned();
                    self.status_text =
                        format!("Selected {} (x{})", selection.label, selection.count());
                }
                ObjectMenuCommand::DropAll => {
                    let menu_selection = MenuCommandSelection {
                        primary_id: selection.primary_id,
                        instances: selection.instances.clone(),
                        definition_id: selection.definition_id.clone(),
                        label: selection.label.clone(),
                    };
                    let handled = self.engine.menu_command(
                        selection.crew_id,
                        MenuCommandKind::DropAll,
                        menu_selection,
                    )?;
                    self.snapshot = self.engine.snapshot();
                    if handled {
                        self.refresh_object_menu();
                        self.refresh_focus();
                        self.status_text = format!(
                            "Executed {} (x{}) via script",
                            selection.label,
                            selection.count()
                        );
                        return Ok(());
                    }
                    self.drop_inventory_selection(&selection)?;
                }
            },
            ObjectMenuAction::Context { selection } => {
                let handled = self
                    .engine
                    .execute_context_menu(selection.crew_id, &selection.function)?;
                self.snapshot = self.engine.snapshot();
                self.refresh_object_menu();
                self.refresh_focus();
                if handled {
                    self.object_menu = None;
                    self.status_text = format!("Executed {}", selection.label);
                } else if let Some(description) = selection.description.as_deref() {
                    self.status_text = description.to_string();
                } else if self.status_text.is_empty() {
                    self.status_text = format!("No scripted action for {}", selection.label);
                }
            }
            ObjectMenuAction::Build { selection, amount } => {
                if selection.owner == OWNER_NONE {
                    self.status_text = "Cannot construct without a player owner".to_string();
                    return Ok(());
                }
                let Some(crew_snapshot) = self.snapshot.object(selection.crew_id).cloned() else {
                    self.status_text = "Crew no longer available".to_string();
                    self.object_menu = None;
                    return Ok(());
                };
                let available = self
                    .engine
                    .player(selection.owner)
                    .and_then(|player| {
                        player
                            .home_base_material()
                            .get(&selection.definition_id)
                            .copied()
                    })
                    .unwrap_or(0);
                if available == 0 {
                    self.status_text = format!("No {} available", selection.label);
                    self.refresh_object_menu();
                    return Ok(());
                }
                let requested = amount.min(available);
                if requested == 0 {
                    self.status_text = format!("No {} available", selection.label);
                    self.refresh_object_menu();
                    return Ok(());
                }

                let definition_id = selection.definition_id.clone();
                let label = selection.label.clone();
                let owner = selection.owner;
                let crew_id = selection.crew_id;
                let mut delivered = 0u32;

                for _ in 0..requested {
                    self.engine.adjust_player_home_base_material(
                        owner,
                        definition_id.clone(),
                        -1,
                    )?;
                    match self.engine.spawn_object(
                        SpawnConfig::new(definition_id.clone())
                            .with_owner(owner)
                            .with_position(crew_snapshot.position)
                            .with_container(crew_id),
                    ) {
                        Ok(_) => delivered += 1,
                        Err(err) => {
                            self.engine.adjust_player_home_base_material(
                                owner,
                                definition_id.clone(),
                                1,
                            )?;
                            self.status_text = format!("Failed to deliver {}: {}", label, err);
                            break;
                        }
                    }
                }

                self.snapshot = self.engine.snapshot();
                self.refresh_object_menu();
                self.refresh_focus();

                if delivered > 0 {
                    let remaining = self
                        .engine
                        .player(owner)
                        .and_then(|player| player.home_base_material().get(&definition_id).copied())
                        .unwrap_or(0);
                    self.status_text = if remaining > 0 {
                        format!(
                            "Received {} (x{}), {} remaining",
                            label, delivered, remaining
                        )
                    } else {
                        format!("Received {} (x{})", label, delivered)
                    };
                } else if self.status_text.is_empty() {
                    self.status_text = format!("Unable to deliver {}", label);
                }
            }
        }
        Ok(())
    }

    fn drop_inventory_selection(
        &mut self,
        selection: &ObjectMenuSelection,
    ) -> Result<(), EngineError> {
        let Some(crew) = self.snapshot.object(selection.crew_id).cloned() else {
            self.status_text = "Crew no longer available".to_string();
            self.object_menu = None;
            return Ok(());
        };

        let mut dropped = 0usize;
        for object_id in &selection.instances {
            match self.engine.apply_object_update(
                *object_id,
                ObjectUpdate::new()
                    .clear_container()
                    .with_position(crew.position)
                    .with_velocity(Vector2::ZERO),
            ) {
                Ok(()) => dropped += 1,
                Err(EngineError::UnknownObject(_)) => {
                    tracing::warn!(
                        object = %object_id,
                        "inventory item missing while dropping"
                    );
                }
                Err(err) => return Err(err),
            }
        }

        if dropped == 0 {
            self.status_text = format!("No {} to drop", selection.label);
            return Ok(());
        }

        self.snapshot = self.engine.snapshot();
        self.refresh_object_menu();
        self.refresh_focus();
        self.status_text = format!("Dropped {} (x{})", selection.label, dropped);
        Ok(())
    }

    fn refresh_object_menu(&mut self) {
        if let Some(menu) = self.object_menu.as_mut() {
            if !menu.refresh(&mut self.engine, &self.snapshot) {
                self.object_menu = None;
            }
        }
    }

    fn execute_ingame_menu_action(&mut self, action: IngameMenuAction) -> Result<(), EngineError> {
        match action {
            IngameMenuAction::Resume => {
                self.close_ingame_menu();
            }
            IngameMenuAction::QuickSave => match self.quick_save() {
                Ok(_) => {
                    if let Some(menu) = self.ingame_menu.as_mut() {
                        menu.set_quick_save_available(true);
                    }
                }
                Err(err) => {
                    tracing::error!(error = ?err, "quick save failed");
                    self.status_text = format!("Quick save failed: {err:#}");
                }
            },
            IngameMenuAction::QuickLoad => match self.quick_load() {
                Ok(()) => {
                    self.close_ingame_menu();
                }
                Err(err) => {
                    tracing::error!(error = ?err, "quick load failed");
                    self.status_text = format!("Quick load failed: {err:#}");
                }
            },
            IngameMenuAction::AbortToMenu => {
                self.close_ingame_menu();
                self.return_to_menu();
            }
        }
        Ok(())
    }

    fn process_network_events(&mut self) -> Result<(), EngineError> {
        if let Some(network) = self.network.as_mut() {
            for event in network.poll_events() {
                match event {
                    NetworkEvent::Control { owner, event } => {
                        if self.mode == AppMode::Running {
                            self.dispatch_control_event_for_owner(owner, event)?;
                        }
                    }
                    NetworkEvent::PeerConnected { client_id } => {
                        tracing::info!(%client_id, "network client connected");
                    }
                    NetworkEvent::PeerDisconnected { client_id, reason } => match reason {
                        Some(reason) => {
                            tracing::info!(
                                %client_id,
                                reason = %reason,
                                "network client disconnected"
                            );
                        }
                        None => tracing::info!(%client_id, "network client disconnected"),
                    },
                    NetworkEvent::Error(message) => {
                        tracing::error!(message = %message, "network error");
                    }
                }
            }
        }
        Ok(())
    }

    fn process_gamepad_events(&mut self) -> Result<(), EngineError> {
        let events = self.gamepads.poll();
        for event in events {
            self.handle_gamepad_event(event)?;
        }
        Ok(())
    }

    fn handle_gamepad_event(&mut self, event: GamepadEvent) -> Result<(), EngineError> {
        match event {
            GamepadEvent::Direction { button, state } => {
                self.handle_gamepad_direction(button, state)?;
            }
            GamepadEvent::Action { action, state } => {
                self.handle_gamepad_action(action, state)?;
            }
        }
        Ok(())
    }

    fn handle_gamepad_direction(
        &mut self,
        button: ControlButton,
        state: ElementState,
    ) -> Result<(), EngineError> {
        match self.mode {
            AppMode::Menu => {
                if let Some(key) = menu_key_from_control_button(button) {
                    match state {
                        ElementState::Pressed => {
                            self.handle_menu_input(|menu| menu.menu().handle_key_down(key))?
                        }
                        ElementState::Released => {
                            self.handle_menu_input(|menu| menu.menu().handle_key_up(key))?
                        }
                    }
                }
            }
            AppMode::Running => {
                let event = match state {
                    ElementState::Pressed => ControlEvent::Press(button),
                    ElementState::Released => ControlEvent::Release(button),
                };
                self.dispatch_control_event(event)?;
            }
        }
        Ok(())
    }

    fn handle_gamepad_action(
        &mut self,
        action: GamepadActionType,
        state: ElementState,
    ) -> Result<(), EngineError> {
        match action {
            GamepadActionType::Select => match self.mode {
                AppMode::Menu => match state {
                    ElementState::Pressed => {
                        self.handle_menu_input(|menu| menu.menu().handle_key_down(KeyCode::Enter))?
                    }
                    ElementState::Released => {
                        self.handle_menu_input(|menu| menu.menu().handle_key_up(KeyCode::Enter))?
                    }
                },
                AppMode::Running => {
                    if state == ElementState::Pressed {
                        self.dispatch_control_event(ControlEvent::ClearPressed)?;
                    }
                }
            },
            GamepadActionType::Back => match self.mode {
                AppMode::Menu => match state {
                    ElementState::Pressed => {
                        self.handle_menu_input(|menu| menu.menu().handle_key_down(KeyCode::Escape))?
                    }
                    ElementState::Released => {
                        self.handle_menu_input(|menu| menu.menu().handle_key_up(KeyCode::Escape))?
                    }
                },
                AppMode::Running => {
                    if state == ElementState::Pressed {
                        if self.ingame_menu.is_some() {
                            self.close_ingame_menu();
                        } else {
                            self.open_ingame_menu();
                        }
                    }
                }
            },
        }
        Ok(())
    }

    fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) -> Result<(), EngineError> {
        let point = gui_point_from_position(position);
        self.handle_menu_input(|state| {
            state.set_pointer_position(Some(point));
            state.menu().handle_pointer_move(point)
        })
    }

    fn handle_mouse_button(&mut self, button_state: ElementState) -> Result<(), EngineError> {
        if self.mode != AppMode::Menu {
            return Ok(());
        }
        if let Some(point) = self.menu_state.pointer_position() {
            match button_state {
                ElementState::Pressed => {
                    self.handle_menu_input(|state| state.menu().handle_pointer_down(point))?
                }
                ElementState::Released => {
                    self.handle_menu_input(|state| state.menu().handle_pointer_up(point))?
                }
            }
        }
        Ok(())
    }

    fn handle_touch(&mut self, phase: TouchPhase, position: GuiPoint) -> Result<(), EngineError> {
        match phase {
            TouchPhase::Started => self.handle_menu_input(|state| {
                state.set_pointer_position(Some(position));
                state.menu().handle_pointer_down(position)
            }),
            TouchPhase::Moved => self.handle_menu_input(|state| {
                state.set_pointer_position(Some(position));
                state.menu().handle_pointer_move(position)
            }),
            TouchPhase::Ended => {
                let result = self.handle_menu_input(|state| {
                    state.set_pointer_position(Some(position));
                    state.menu().handle_pointer_up(position)
                });
                self.pointer_left();
                result
            }
            TouchPhase::Cancelled => {
                self.pointer_left();
                Ok(())
            }
        }
    }

    fn pointer_left(&mut self) {
        if self.mode == AppMode::Menu {
            self.menu_state.set_pointer_position(None);
        }
    }

    fn handle_menu_input<F>(&mut self, handler: F) -> Result<(), EngineError>
    where
        F: FnOnce(&mut MenuState) -> Vec<StartupMenuAction>,
    {
        if self.mode != AppMode::Menu {
            return Ok(());
        }

        let actions = handler(&mut self.menu_state);
        let (start_identifier, updated_label) = self.process_menu_actions(actions);

        if let Some(label) = updated_label {
            self.scenario_label = label;
        }

        if let Some(identifier) = start_identifier {
            if let Some(scenario) = self.scenario_catalog.get(&identifier).cloned() {
                self.start_scenario(scenario)?;
            } else {
                tracing::warn!(
                    scenario = %identifier,
                    "selected scenario is not available in Rust catalog"
                );
            }
        }
        Ok(())
    }

    fn process_menu_actions(
        &mut self,
        actions: Vec<StartupMenuAction>,
    ) -> (Option<String>, Option<String>) {
        let mut start_identifier: Option<String> = None;
        let mut updated_label: Option<String> = None;

        for action in actions {
            match action {
                StartupMenuAction::SelectionChanged(_) => {}
                StartupMenuAction::StartScenario(summary) => {
                    start_identifier = Some(summary.identifier);
                }
                StartupMenuAction::OpenEntry(summary) => {
                    if summary.identifier == BACK_ENTRY_IDENTIFIER {
                        self.menu_state.leave_folder();
                        updated_label = Some(self.menu_state.label_path());
                        continue;
                    }

                    let entry_kind = self
                        .menu_state
                        .current_entries()
                        .iter()
                        .find(|entry| entry.identifier == summary.identifier)
                        .map(|entry| entry.kind);

                    match entry_kind {
                        Some(ScenarioKind::Folder) => {
                            self.menu_state.enter_folder(&summary.identifier);
                            updated_label = Some(self.menu_state.label_path());
                        }
                        Some(ScenarioKind::Scenario) => {
                            start_identifier = Some(summary.identifier);
                        }
                        Some(ScenarioKind::Editor) => {
                            if let Err(err) = self.launch_editor_by_identifier(&summary.identifier)
                            {
                                tracing::error!(
                                    scenario = %summary.identifier,
                                    error = ?err,
                                    "failed to launch legacy editor"
                                );
                            }
                        }
                        None => {
                            self.menu_state.enter_folder(&summary.identifier);
                            updated_label = Some(self.menu_state.label_path());
                        }
                    }
                }
                StartupMenuAction::EditEntry(summary) => {
                    if let Err(err) = self.launch_editor_by_identifier(&summary.identifier) {
                        tracing::error!(
                            scenario = %summary.identifier,
                            error = ?err,
                            "failed to launch legacy editor"
                        );
                    }
                }
            }
        }

        (start_identifier, updated_label)
    }

    fn launch_editor_by_identifier(&mut self, identifier: &str) -> Result<()> {
        let scenario = self
            .scenario_catalog
            .get(identifier)
            .cloned()
            .ok_or_else(|| {
                anyhow!("selected scenario `{identifier}` is not available in the Rust catalog")
            })?;
        self.launch_editor_for_scenario(&scenario)
    }

    fn launch_editor_for_scenario(&mut self, scenario: &FrontendScenario) -> Result<()> {
        if !scenario.is_editable {
            anyhow::bail!(
                "scenario `{}` is not marked editable in the catalog",
                scenario.title
            );
        }
        let Some(path) = scenario.path.as_ref() else {
            anyhow::bail!(
                "scenario `{}` has no filesystem path and cannot be opened in the legacy editor",
                scenario.title
            );
        };
        let editor_binary =
            resolve_editor_binary().context("failed to resolve LegacyClonk editor binary")?;
        let mut command = Command::new(&editor_binary);
        command.arg(path);
        if let Some(parent) = editor_binary.parent() {
            command.current_dir(parent);
        }
        command
            .spawn()
            .with_context(|| format!("failed to launch editor at {}", editor_binary.display()))?;
        self.status_text = format!("Launching editor for {}", scenario.title);
        tracing::info!(
            editor = %editor_binary.display(),
            scenario = %path.display(),
            "launching LegacyClonk editor"
        );
        Ok(())
    }

    fn update(&mut self) -> Result<(), EngineError> {
        self.process_network_events()?;
        if matches!(self.mode, AppMode::Running) {
            self.snapshot = self.engine.tick()?;
            self.refresh_object_menu();
            self.refresh_focus();
            self.update_audio();
        }
        Ok(())
    }

    fn update_audio(&mut self) {
        let fallback_center = {
            let surface = self.graphics.surface();
            Vector2::new((surface.width() as i32) / 2, (surface.height() as i32) / 2)
        };
        let viewport_center = self
            .focus_snapshot
            .as_ref()
            .map(|object| object.position)
            .unwrap_or(fallback_center);
        if let Some(audio) = self.audio.as_mut() {
            audio.process_audio(
                &self.snapshot,
                self.focus_snapshot.as_ref(),
                viewport_center,
            );
        }
    }

    fn refresh_focus(&mut self) {
        if !matches!(self.mode, AppMode::Running) {
            self.focus_snapshot = None;
            return;
        }

        if self
            .focus_id
            .and_then(|id| self.snapshot.object(id))
            .is_none()
        {
            self.focus_id = self.snapshot.objects.first().map(|object| object.id);
        }

        self.focus_snapshot = self
            .focus_id
            .and_then(|id| self.snapshot.object(id).cloned());

        if let Some(object) = &self.focus_snapshot {
            self.frame_text = format!(
                "FRAME {:05} POS {:04}/{:04} VEL {:03}/{:03}",
                self.snapshot.frame,
                object.position.x,
                object.position.y,
                object.velocity.x,
                object.velocity.y
            );
            self.status_text = format!(
                "ENERGY {:03} DAMAGE {:03} OWNER {}",
                object.energy.max(0),
                object.damage.max(0),
                object.owner
            );
            self.energy_fraction = (object.energy.max(0).min(100) as f32) / 100.0;
        } else {
            self.frame_text = format!("FRAME {:05}", self.snapshot.frame);
            self.status_text.clear();
            self.energy_fraction = 0.0;
        }
    }

    fn render(&mut self, frame: &mut [u8]) -> Result<()> {
        if self.mode == AppMode::Menu {
            render_menu_frame(
                &mut self.graphics,
                self.menu_state.menu(),
                self.assets.as_ref(),
                frame,
            );
            return Ok(());
        }
        self.render_running(frame)
    }

    fn render_running(&mut self, frame: &mut [u8]) -> Result<()> {
        let viewports = collect_viewport_inputs(&self.snapshot, self.local_owner, self.focus_id);
        if let Some(_focus) = self.focus_snapshot.as_ref() {
            let players = collect_player_overlays(&self.snapshot, self.focus_id);
            let overlay = GraphicsOverlay {
                frame_text: &self.frame_text,
                status_text: &self.status_text,
                energy_fraction: self.energy_fraction,
                players,
            };
            self.graphics
                .update_overlay(&overlay)
                .context("failed to update overlay")?;
            self.graphics.render_frame(&self.snapshot, &viewports);
        } else if !viewports.is_empty() {
            self.graphics.render_frame(&self.snapshot, &viewports);
        } else {
            self.graphics.surface_mut().fill(Color::opaque(12, 24, 40));
        }

        if let Some(menu) = self.object_menu.as_ref() {
            let font = self.assets.font_arc();
            {
                let surface = self.graphics.surface_mut();
                menu.render(surface, font.as_ref());
            }
        } else if let Some(menu) = self.ingame_menu.as_ref() {
            let font = self.assets.font_arc();
            {
                let surface = self.graphics.surface_mut();
                menu.render(surface, font.as_ref());
            }
        }

        self.draw_messages();

        let surface = self.graphics.surface();
        let pixels = surface.pixels();
        if pixels.len() == frame.len() {
            frame.copy_from_slice(pixels);
        } else {
            copy_surface(pixels, surface.width(), surface.height(), frame);
        }
        Ok(())
    }

    fn draw_messages(&mut self) {
        if self.snapshot.hud.messages.is_empty() {
            return;
        }

        let surface_width = self.graphics.surface().width() as f32;
        let surface_height = self.graphics.surface().height() as f32;
        struct PreparedMessage {
            anchor: (f32, f32),
            lines: Vec<String>,
            color: Color,
            has_frame: bool,
            portrait: Option<Color>,
        }

        let mut prepared: Vec<PreparedMessage> = Vec::new();

        for message in &self.snapshot.hud.messages {
            if let Some(player) = message.player {
                if player != self.local_owner {
                    continue;
                }
            }

            let color = Color::new(
                ((message.color >> 16) & 0xff) as u8,
                ((message.color >> 8) & 0xff) as u8,
                (message.color & 0xff) as u8,
                ((message.color >> 24) & 0xff) as u8,
            );

            match message.kind {
                MessageKind::Global | MessageKind::GlobalPlayer => {
                    let mut x = if (message.flags & FLAG_X_REL) != 0 {
                        surface_width * (message.offset.x as f32 / 100.0)
                    } else if message.offset.x >= 0 {
                        message.offset.x as f32
                    } else {
                        surface_width * 0.5
                    };
                    let mut y = if (message.flags & FLAG_Y_REL) != 0 {
                        surface_height * (message.offset.y as f32 / 100.0)
                    } else if message.offset.y >= 0 {
                        message.offset.y as f32
                    } else {
                        surface_height * 0.66
                    };

                    if (message.flags & FLAG_HCENTER) != 0 {
                        x = surface_width * 0.5;
                    } else if (message.flags & FLAG_LEFT) != 0 {
                        x = 32.0;
                    } else if (message.flags & FLAG_RIGHT) != 0 {
                        x = surface_width - 196.0;
                    }

                    if (message.flags & FLAG_VCENTER) != 0 {
                        y = surface_height * 0.5;
                    } else if (message.flags & FLAG_TOP) != 0 {
                        y = 48.0;
                    } else if (message.flags & FLAG_BOTTOM) != 0 {
                        y = surface_height - 160.0;
                    }

                    let has_decoration = message
                        .decoration
                        .as_ref()
                        .map(|decor| !decor.trim().is_empty())
                        .unwrap_or(false);
                    let has_portrait = message.portrait.is_some();
                    let portrait_color = message
                        .portrait
                        .as_ref()
                        .and_then(|spec| Self::parse_portrait_color(spec))
                        .or_else(|| {
                            if has_portrait {
                                Some(Color::new(color.r, color.g, color.b, 255))
                            } else {
                                None
                            }
                        });
                    let has_frame = has_portrait || has_decoration;

                    prepared.push(PreparedMessage {
                        anchor: (x, y),
                        lines: message.lines.clone(),
                        color,
                        has_frame,
                        portrait: portrait_color,
                    });
                }
                MessageKind::Target | MessageKind::TargetPlayer => {
                    let target_id = match message.target {
                        Some(id) => id,
                        None => continue,
                    };
                    let Some(target) = self.snapshot.object(target_id) else {
                        continue;
                    };
                    let base_position = Vector2::new(
                        target.position.x + message.offset.x,
                        target.position.y + message.offset.y,
                    );
                    let owner = message.player.unwrap_or(self.local_owner);
                    let Some((screen_x, screen_y)) =
                        self.graphics.world_to_screen(owner, base_position)
                    else {
                        continue;
                    };
                    let has_decoration = message
                        .decoration
                        .as_ref()
                        .map(|decor| !decor.trim().is_empty())
                        .unwrap_or(false);
                    let has_portrait = message.portrait.is_some();
                    let portrait_color = message
                        .portrait
                        .as_ref()
                        .and_then(|spec| Self::parse_portrait_color(spec))
                        .or_else(|| {
                            if has_portrait {
                                Some(Color::new(color.r, color.g, color.b, 255))
                            } else {
                                None
                            }
                        });
                    let has_frame = has_portrait || has_decoration;

                    prepared.push(PreparedMessage {
                        anchor: (screen_x, screen_y),
                        lines: message.lines.clone(),
                        color,
                        has_frame,
                        portrait: portrait_color,
                    });
                }
            }
        }

        if prepared.is_empty() {
            return;
        }

        let font = self.assets.font_arc();
        let line_height = 20.0;

        const FONT_SIZE: f32 = 18.0;
        const FRAME_PADDING: f32 = 8.0;
        const PORTRAIT_SIZE: f32 = 42.0;
        const PORTRAIT_GAP: f32 = 8.0;

        {
            let surface = self.graphics.surface_mut();
            for message in prepared {
                if message.has_frame {
                    let portrait_space = if message.portrait.is_some() {
                        PORTRAIT_SIZE + PORTRAIT_GAP
                    } else {
                        0.0
                    };

                    let mut text_width = 0.0f32;
                    for line in &message.lines {
                        let width = font.measure_text(line, FONT_SIZE).width;
                        text_width = text_width.max(width);
                    }
                    let text_height = message.lines.len() as f32 * line_height;

                    let frame_width = (text_width + portrait_space + FRAME_PADDING * 2.0)
                        .max(1.0)
                        .ceil();
                    let frame_height = (text_height + FRAME_PADDING * 2.0).max(1.0).ceil();

                    let rect = Rect::new(
                        (message.anchor.0 - portrait_space - FRAME_PADDING).floor() as i32,
                        (message.anchor.1 - FRAME_PADDING).floor() as i32,
                        frame_width as u32,
                        frame_height as u32,
                    );

                    let background = Color::new(12, 20, 36, 192);
                    Self::fill_rect(surface, rect, background);
                    let border = Color::new(
                        message.color.r.saturating_add(24),
                        message.color.g.saturating_add(24),
                        message.color.b.saturating_add(24),
                        255,
                    );
                    Self::draw_border(surface, rect, border);

                    if let Some(portrait_color) = message.portrait {
                        let portrait_rect = Rect::new(
                            rect.x + FRAME_PADDING as i32,
                            rect.y + FRAME_PADDING as i32,
                            PORTRAIT_SIZE as u32,
                            PORTRAIT_SIZE as u32,
                        );
                        Self::fill_rect(surface, portrait_rect, portrait_color);
                        Self::draw_border(surface, portrait_rect, border);
                    }

                    let text_x = rect.x as f32
                        + FRAME_PADDING
                        + if message.portrait.is_some() {
                            PORTRAIT_SIZE + PORTRAIT_GAP
                        } else {
                            0.0
                        };
                    let mut text_y = rect.y as f32 + FRAME_PADDING;

                    for line in &message.lines {
                        font.draw_text(surface, text_x, text_y, line, FONT_SIZE, message.color);
                        text_y += line_height;
                    }
                } else {
                    let mut y = message.anchor.1;
                    for line in &message.lines {
                        font.draw_text(
                            surface,
                            message.anchor.0,
                            y,
                            line,
                            FONT_SIZE,
                            message.color,
                        );
                        y += line_height;
                    }
                }
            }
        }
    }

    fn parse_portrait_color(spec: &str) -> Option<Color> {
        let trimmed = spec.trim();
        let rest = trimmed.strip_prefix("Portrait:")?;
        let mut parts = rest.split("::");
        let _id = parts.next()?;
        let color_token = parts.next()?.trim();
        let hex = color_token.chars().take(6).collect::<String>();
        if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return None;
        }
        let value = u32::from_str_radix(&hex, 16).ok()?;
        Some(Color::new(
            ((value >> 16) & 0xff) as u8,
            ((value >> 8) & 0xff) as u8,
            (value & 0xff) as u8,
            255,
        ))
    }

    fn fill_rect(surface: &mut Surface, rect: Rect, color: Color) {
        if let Some(clipped) = rect.intersection(surface.bounds()) {
            for y in clipped.y..(clipped.y + clipped.height as i32) {
                for x in clipped.x..(clipped.x + clipped.width as i32) {
                    let result = if color.a == 255 {
                        surface.set_pixel(x as u32, y as u32, color)
                    } else {
                        surface.blend_pixel(x as u32, y as u32, color)
                    };
                    if result.is_err() {
                        break;
                    }
                }
            }
        }
    }

    fn draw_border(surface: &mut Surface, rect: Rect, color: Color) {
        if rect.width == 0 || rect.height == 0 {
            return;
        }
        let top = Rect::new(rect.x, rect.y, rect.width, 1);
        let bottom = Rect::new(rect.x, rect.y + rect.height as i32 - 1, rect.width, 1);
        let left = Rect::new(rect.x, rect.y, 1, rect.height);
        let right = Rect::new(rect.x + rect.width as i32 - 1, rect.y, 1, rect.height);
        Self::fill_rect(surface, top, color);
        Self::fill_rect(surface, bottom, color);
        Self::fill_rect(surface, left, color);
        Self::fill_rect(surface, right, color);
    }

    fn return_to_menu(&mut self) {
        self.close_ingame_menu();
        self.object_menu = None;
        self.engine = Engine::new();
        self.apply_material_library();
        self.input = InputDispatcher::new();
        self.sky = None;
        self.snapshot = self.engine.snapshot();
        self.refresh_object_menu();
        self.focus_id = None;
        self.focus_snapshot = None;
        self.frame_text.clear();
        self.status_text.clear();
        self.energy_fraction = 0.0;
        self.active_scenario = None;
        if let Some(audio) = self.audio.as_mut() {
            audio.stop_music();
            audio.reset_sfx();
            audio.configure_scenario(None);
        }

        self.fallback_ground = DEFAULT_GROUND_HEIGHT;
        self.scenario_label = self.menu_state.label_path();
        self.object_sprites = self.assets.base_sprite_map().clone();
        self.sprite_cache = Arc::new(self.object_sprites.clone());

        let width = self.graphics.surface().width();
        let height = self.graphics.surface().height();
        self.graphics = GraphicsSystem::new(
            width,
            height,
            self.fallback_ground,
            &self.scenario_label,
            self.assets.font_arc(),
            Arc::clone(&self.sprite_cache),
            self.assets.cursor_atlas(),
        );
        self.graphics.surface_mut().fill(Color::opaque(16, 28, 52));
        self.graphics.set_sky(self.sky.clone());

        self.menu_state.set_pointer_position(None);
        self.menu_state.refresh_menu_entries();
        self.menu_state.menu().resize(width as f32, height as f32);

        self.mode = AppMode::Menu;
        self.ensure_menu_music();
    }

    fn ensure_menu_music(&mut self) {
        if !matches!(self.mode, AppMode::Menu) {
            return;
        }
        if let Some(audio) = self.audio.as_mut() {
            audio.configure_scenario(None);
            if !audio.menu_music_enabled() {
                audio.stop_music();
                return;
            }
            if audio.music_is_playing() {
                return;
            }
            if let Err(err) = audio.play_music(sandbox_music_bytes(), true) {
                tracing::warn!(error = %err, "failed to start menu music");
                audio.stop_music();
            }
        }
    }

    fn start_scenario(&mut self, scenario: FrontendScenario) -> Result<(), EngineError> {
        if self.try_start_real_scenario(&scenario)? {
            return Ok(());
        }
        self.start_sandbox_scenario(scenario)
    }

    fn try_start_real_scenario(
        &mut self,
        scenario: &FrontendScenario,
    ) -> Result<bool, EngineError> {
        let Some(path) = scenario.path.as_ref() else {
            return Ok(false);
        };

        let resolver = InstallDefinitionResolver::new(cached_app_paths().ok());
        let scenario_data = match Scenario::load_from_path_with(path, &resolver) {
            Ok(data) => data,
            Err(err) => {
                tracing::error!(
                    scenario = %scenario.title,
                    path = %path.display(),
                    error = %err,
                    "failed to load scenario"
                );
                return Ok(false);
            }
        };

        tracing::info!(
            scenario = %scenario.title,
            path = %path.display(),
            "starting scenario from disk"
        );

        self.engine = Engine::new();
        self.apply_material_library();
        self.input = InputDispatcher::new();
        if let Some(audio) = self.audio.as_mut() {
            audio.configure_scenario(Some(path));
            audio.reset_sfx();
            scenario_data.visit_definition_groups(|id, group| {
                audio.register_definition_sounds(id, group);
            });
        }

        if let Err(err) = scenario_data.apply(&mut self.engine) {
            tracing::error!(
                scenario = %scenario.title,
                path = %path.display(),
                error = %err,
                "failed to apply scenario"
            );
            return Ok(false);
        }

        self.sky = scenario_data.sky().map(sky_render_state_from_config);
        self.snapshot = self.engine.snapshot();
        self.rebuild_definition_sprites();

        let label = scenario_data
            .name()
            .map(|name| name.to_string())
            .unwrap_or_else(|| scenario.title.clone());
        let ground = match scenario_data.ground_height_hint() {
            Some(hint) => hint.max(0),
            None => Self::derive_ground_height(&self.engine, DEFAULT_GROUND_HEIGHT),
        };

        self.configure_running_state(label, ground);
        self.apply_focus_selection();
        self.snapshot = self.engine.snapshot();
        self.refresh_object_menu();
        self.refresh_focus();
        self.active_scenario = Some(scenario.clone());
        self.play_scenario_audio(path);
        Ok(true)
    }

    fn start_sandbox_scenario(&mut self, scenario: FrontendScenario) -> Result<(), EngineError> {
        tracing::info!(
            scenario = %scenario.title,
            "starting sandbox fallback scenario"
        );

        self.engine = Engine::new();
        self.apply_material_library();
        self.input = InputDispatcher::new();
        self.sky = None;
        if let Some(audio) = self.audio.as_mut() {
            audio.configure_scenario(None);
            audio.reset_sfx();
        }

        let spawn_definition = match self.audio.as_mut() {
            Some(audio) => configure_sandbox_engine(&mut self.engine, Some(audio))?,
            None => configure_sandbox_engine(&mut self.engine, None)?,
        };

        let spawn = SpawnConfig::new(spawn_definition)
            .with_owner(self.local_owner)
            .with_position(Vector2::new(240, 180))
            .with_energy(100)
            .with_action(ActionState::new("Walk"))
            .with_crew_member(true);
        self.engine.spawn_object(spawn)?;

        self.snapshot = self.engine.snapshot();
        self.rebuild_definition_sprites();
        let fallback_ground = Self::derive_ground_height(&self.engine, DEFAULT_GROUND_HEIGHT);
        self.configure_running_state(scenario.title.clone(), fallback_ground);
        self.apply_focus_selection();
        self.snapshot = self.engine.snapshot();
        self.refresh_object_menu();
        self.refresh_focus();
        self.active_scenario = Some(scenario);
        self.play_sandbox_audio();
        Ok(())
    }

    fn quick_save(&mut self) -> Result<()> {
        if self.mode != AppMode::Running {
            anyhow::bail!("cannot quick save while not running a scenario");
        }

        let scenario = self
            .active_scenario
            .clone()
            .unwrap_or_else(FrontendScenario::fallback);
        let engine_state = self.engine.capture_state();
        let saved = SavedGameFile {
            version: SAVE_FILE_VERSION,
            saved_at_seconds: current_unix_timestamp(),
            scenario: SavedScenarioInfo::from_frontend(
                &scenario,
                &self.scenario_label,
                self.fallback_ground,
            ),
            focus_id: self.focus_id,
            engine_state,
        };

        let dir = ensure_save_directory()?;
        let path = dir.join(QUICK_SAVE_FILE);
        let mut file = File::create(&path)
            .with_context(|| format!("failed to create quick save at {}", path.display()))?;
        serde_json::to_writer_pretty(&mut file, &saved)
            .context("failed to serialize quick save data")?;
        file.flush().context("failed to flush quick save data")?;
        self.last_save_path = Some(path.clone());
        self.status_text = format!("Saved {}", saved.scenario.title);
        Ok(())
    }

    fn quick_load(&mut self) -> Result<()> {
        let candidate = self
            .last_save_path
            .clone()
            .unwrap_or_else(default_quick_save_path);
        let path = if candidate.exists() {
            candidate
        } else {
            let fallback = default_quick_save_path();
            if fallback.exists() {
                fallback
            } else if fallback == candidate {
                anyhow::bail!("no quick save found at {}", candidate.display());
            } else {
                anyhow::bail!(
                    "no quick save found (checked {} and {})",
                    candidate.display(),
                    fallback.display()
                );
            }
        };

        let contents = fs::read_to_string(&path)
            .with_context(|| format!("failed to read quick save from {}", path.display()))?;
        let save: SavedGameFile =
            serde_json::from_str(&contents).context("failed to parse quick save data")?;
        let save =
            migrate_save_file(save).context("failed to migrate quick save to current schema")?;
        self.apply_loaded_game(save)?;
        self.last_save_path = Some(path.clone());
        Ok(())
    }

    fn apply_loaded_game(&mut self, save: SavedGameFile) -> Result<()> {
        let scenario_info = save.scenario.clone();
        let frontend = scenario_info.to_frontend();

        self.engine = Engine::new();
        self.apply_material_library();
        self.input = InputDispatcher::new();

        if scenario_info.sandbox {
            match self.audio.as_mut() {
                Some(audio) => configure_sandbox_engine(&mut self.engine, Some(audio))
                    .context("failed to prepare sandbox engine for saved game")?,
                None => configure_sandbox_engine(&mut self.engine, None)
                    .context("failed to prepare sandbox engine for saved game")?,
            };
        } else {
            let path = frontend.path.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "saved scenario `{}` does not include a playable path",
                    scenario_info.title
                )
            })?;
            let scenario_data = Scenario::load_from_path(path).with_context(|| {
                format!(
                    "failed to reload scenario `{}` from {}",
                    scenario_info.title,
                    path.display()
                )
            })?;
            if let Some(audio) = self.audio.as_mut() {
                audio.configure_scenario(Some(path));
                audio.reset_sfx();
                scenario_data.visit_definition_groups(|id, group| {
                    audio.register_definition_sounds(id, group);
                });
            }
            scenario_data.apply(&mut self.engine).with_context(|| {
                format!(
                    "failed to apply scenario `{}` from {}",
                    scenario_info.title,
                    path.display()
                )
            })?;
        }

        self.rebuild_definition_sprites();

        self.configure_running_state(scenario_info.label.clone(), scenario_info.fallback_ground);
        self.active_scenario = Some(frontend.clone());

        if scenario_info.sandbox {
            self.play_sandbox_audio();
        } else if let Some(path) = frontend.path.as_ref() {
            self.play_scenario_audio(path);
        }

        self.engine
            .restore_state(&save.engine_state)
            .context("failed to restore saved engine state")?;

        self.snapshot = self.engine.snapshot();
        self.focus_id = save.focus_id;
        if self
            .focus_id
            .and_then(|id| self.snapshot.object(id))
            .is_none()
        {
            self.focus_id = None;
        }
        self.refresh_focus();

        self.scenario_catalog
            .insert(frontend.identifier.clone(), frontend.clone());

        self.status_text = format!("Loaded {}", scenario_info.title);
        Ok(())
    }

    fn play_scenario_audio(&mut self, path: &Path) {
        if let Some(audio) = self.audio.as_mut() {
            audio.configure_scenario(Some(path));
            if !audio.music_enabled() {
                audio.stop_music();
                return;
            }
            match load_scenario_music_bytes(path) {
                Ok(Some(bytes)) => {
                    if let Err(err) = audio.play_music(bytes.as_slice(), true) {
                        tracing::warn!(
                            path = %path.display(),
                            error = %err,
                            "failed to start music"
                        );
                        audio.stop_music();
                    }
                }
                Ok(None) => audio.stop_music(),
                Err(err) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %err,
                        "failed to load music"
                    );
                }
            }
        }
    }

    fn play_sandbox_audio(&mut self) {
        if let Some(audio) = self.audio.as_mut() {
            audio.configure_scenario(None);
            if !audio.menu_music_enabled() {
                audio.stop_music();
                return;
            }
            if let Err(err) = audio.play_music(sandbox_music_bytes(), true) {
                tracing::warn!(error = %err, "failed to start sandbox music");
                audio.stop_music();
            }
        }
    }

    fn configure_running_state(&mut self, label: String, fallback_ground: i32) {
        self.scenario_label = label;
        self.fallback_ground = fallback_ground;
        let width = self.graphics.surface().width();
        let height = self.graphics.surface().height();
        self.graphics = GraphicsSystem::new(
            width,
            height,
            self.fallback_ground,
            &self.scenario_label,
            self.assets.font_arc(),
            Arc::clone(&self.sprite_cache),
            self.assets.cursor_atlas(),
        );
        self.graphics.surface_mut().fill(Color::opaque(12, 24, 40));
        self.graphics.set_sky(self.sky.clone());
        self.frame_text.clear();
        self.status_text.clear();
        self.energy_fraction = 0.0;
        self.menu_state.set_pointer_position(None);
        self.object_menu = None;
        self.ingame_menu = None;
        self.mode = AppMode::Running;
    }

    fn apply_focus_selection(&mut self) {
        if let Some((object_id, owner, crew_member)) =
            select_focus_candidate(&self.snapshot, self.local_owner)
        {
            self.focus_id = Some(object_id);
            if crew_member && owner >= 0 {
                if let Err(err) = self.engine.select_crew(owner, [object_id]) {
                    tracing::warn!(
                        object_id = %object_id,
                        owner,
                        error = %err,
                        "failed to select crew member"
                    );
                } else if let Err(err) = self.engine.set_crew_cursor(owner, Some(object_id)) {
                    tracing::warn!(
                        object_id = %object_id,
                        owner,
                        error = %err,
                        "failed to set crew cursor"
                    );
                }
            }
        } else {
            self.focus_id = None;
        }
        self.focus_snapshot = None;
    }
}

fn render_menu_frame(
    graphics: &mut GraphicsSystem,
    menu: &mut StartupMenu,
    assets: &FrontendAssets,
    frame: &mut [u8],
) {
    {
        let surface = graphics.surface_mut();
        if let Some(background) = assets.menu_background() {
            let rect = lc_gui::Rect::from_origin_size(
                GuiPoint::new(0.0, 0.0),
                lc_gui::Size::new(surface.width() as f32, surface.height() as f32),
            );
            draw_image(surface, &rect, &background);
        } else {
            surface.fill(Color::opaque(16, 28, 52));
        }
        menu.render(surface);
    }
    let surface = graphics.surface();
    let pixels = surface.pixels();
    if pixels.len() == frame.len() {
        frame.copy_from_slice(pixels);
    } else {
        copy_surface(pixels, surface.width(), surface.height(), frame);
    }
}

fn copy_surface(src: &[u8], width: u32, height: u32, dest: &mut [u8]) {
    const BYTES_PER_PIXEL: usize = 4;
    if width == 0 || height == 0 {
        return;
    }
    let stride = width as usize * BYTES_PER_PIXEL;
    for row in 0..height as usize {
        let src_offset = row * stride;
        let dest_offset = row * stride;
        let end = src_offset + stride;
        if end <= src.len() && dest_offset + stride <= dest.len() {
            dest[dest_offset..dest_offset + stride].copy_from_slice(&src[src_offset..end]);
        }
    }
}

fn collect_viewport_inputs<'a>(
    snapshot: &'a SimulationSnapshot,
    local_owner: i32,
    fallback_focus: Option<ObjectId>,
) -> Vec<ViewportInput<'a>> {
    let mut seen = HashSet::new();
    let mut inputs = Vec::new();

    for state in &snapshot.players {
        if state.status == PlayerStatus::Eliminated {
            continue;
        }
        for viewport in &state.viewports {
            let focus_id = viewport
                .focus
                .or(state.cursor)
                .or_else(|| state.crew.first().copied());
            let Some(focus_id) = focus_id else {
                continue;
            };
            if !seen.insert(focus_id) {
                continue;
            }
            if let Some(object) = snapshot.object(focus_id) {
                let center = Vector2::new(viewport.center.x, viewport.center.y);
                inputs.push(ViewportInput::new(state.id, center, viewport.zoom, object));
            }
        }
    }

    if inputs.is_empty() {
        if let Some(focus_id) = fallback_focus {
            if let Some(object) = snapshot.object(focus_id) {
                let center = Vector2::new(object.position.x, object.position.y);
                if seen.insert(object.id) {
                    inputs.push(ViewportInput::new(object.owner, center, 1.0, object));
                }
            }
        }
    }

    if inputs.is_empty() {
        if let Some(object) = snapshot.objects.first() {
            let center = Vector2::new(object.position.x, object.position.y);
            inputs.push(ViewportInput::new(object.owner, center, 1.0, object));
        }
    }

    inputs.sort_by(|a, b| {
        let a_key = (a.owner != local_owner, a.owner);
        let b_key = (b.owner != local_owner, b.owner);
        a_key.cmp(&b_key)
    });

    inputs
}

fn collect_player_overlays(
    snapshot: &SimulationSnapshot,
    focus_id: Option<ObjectId>,
) -> Vec<PlayerOverlay> {
    let detail_map: HashMap<_, _> = snapshot
        .players
        .iter()
        .map(|state| (state.id, state))
        .collect();
    let mut players = Vec::with_capacity(snapshot.hud.players.len());
    for player in &snapshot.hud.players {
        let mut crew = Vec::with_capacity(player.crew.len());
        let cursor = detail_map.get(&player.owner).and_then(|state| state.cursor);
        for object_id in &player.crew {
            if let Some(object) = snapshot.object(*object_id) {
                let label = format!("{} #{}", object.definition_id, object.id.as_u64());
                let energy_fraction = (object.energy.max(0).min(100) as f32) / 100.0;
                let is_focus = focus_id == Some(object.id) || cursor == Some(object.id);
                crew.push(CrewOverlay {
                    label,
                    energy_fraction,
                    is_focus,
                });
            }
        }
        let name = detail_map
            .get(&player.owner)
            .and_then(|state| {
                if state.name.trim().is_empty() {
                    None
                } else {
                    Some(state.name.clone())
                }
            })
            .unwrap_or_else(|| format!("Player {}", player.owner));
        let wealth = detail_map
            .get(&player.owner)
            .map(|state| state.wealth)
            .unwrap_or(0);
        players.push(PlayerOverlay {
            owner: player.owner,
            name,
            wealth,
            cursor,
            eliminated: player.eliminated,
            crew,
        });
    }
    players
}

fn select_focus_candidate(
    snapshot: &SimulationSnapshot,
    preferred_owner: i32,
) -> Option<(ObjectId, i32, bool)> {
    for object in snapshot.objects.iter() {
        if is_focusable(object) && object.crew_member && object.owner == preferred_owner {
            return Some((object.id, object.owner, true));
        }
    }
    for object in snapshot.objects.iter() {
        if is_focusable(object) && object.crew_member && object.owner >= 0 {
            return Some((object.id, object.owner, true));
        }
    }
    for object in snapshot.objects.iter() {
        if is_focusable(object) && object.crew_member {
            return Some((object.id, object.owner, true));
        }
    }
    for object in snapshot.objects.iter() {
        if is_focusable(object) && object.owner >= 0 {
            return Some((object.id, object.owner, object.crew_member));
        }
    }
    for object in snapshot.objects.iter() {
        if is_focusable(object) {
            return Some((object.id, object.owner, object.crew_member));
        }
    }
    snapshot
        .objects
        .first()
        .map(|object| (object.id, object.owner, object.crew_member))
}

fn is_focusable(object: &ObjectSnapshot) -> bool {
    object.alive && object.status.is_active()
}

fn map_key_code(code: VirtualKeyCode) -> Option<KeyCode> {
    match code {
        VirtualKeyCode::Return => Some(KeyCode::Enter),
        VirtualKeyCode::Escape => Some(KeyCode::Escape),
        VirtualKeyCode::Space => Some(KeyCode::Space),
        VirtualKeyCode::Tab => Some(KeyCode::Tab),
        VirtualKeyCode::Up => Some(KeyCode::Up),
        VirtualKeyCode::Down => Some(KeyCode::Down),
        VirtualKeyCode::Left => Some(KeyCode::Left),
        VirtualKeyCode::Right => Some(KeyCode::Right),
        _ => None,
    }
}

fn menu_key_from_control_button(button: ControlButton) -> Option<KeyCode> {
    match button {
        ControlButton::Left => Some(KeyCode::Left),
        ControlButton::Right => Some(KeyCode::Right),
        ControlButton::Up => Some(KeyCode::Up),
        ControlButton::Down => Some(KeyCode::Down),
    }
}

fn gui_point_from_position(position: PhysicalPosition<f64>) -> GuiPoint {
    GuiPoint::new(position.x as f32, position.y as f32)
}

fn build_menu_entries(entries: &[FrontendScenario], include_back: bool) -> Vec<ScenarioEntry> {
    let mut result = Vec::new();
    if include_back {
        result.push(ScenarioEntry {
            identifier: BACK_ENTRY_IDENTIFIER.to_string(),
            title: BACK_ENTRY_TITLE.to_string(),
            description: Some("Return to the previous folder.".to_string()),
            kind: ScenarioKind::Folder,
            is_editable: false,
            is_playable: false,
            location: None,
            preview: None,
        });
    }
    result.extend(entries.iter().map(FrontendScenario::to_ui_entry));
    result
}

fn build_scenario_catalog(entries: &[FrontendScenario]) -> HashMap<String, FrontendScenario> {
    let mut catalog = HashMap::new();
    for entry in entries {
        insert_scenario_recursive(entry, &mut catalog);
    }
    if catalog.is_empty() {
        catalog.insert("rust_sandbox".to_string(), FrontendScenario::fallback());
    }
    catalog
}

fn insert_scenario_recursive(
    entry: &FrontendScenario,
    catalog: &mut HashMap<String, FrontendScenario>,
) {
    catalog
        .entry(entry.identifier.clone())
        .or_insert_with(|| entry.clone());
    for child in &entry.children {
        insert_scenario_recursive(child, catalog);
    }
}

fn resolve_editor_binary() -> Result<PathBuf> {
    if let Some(override_path) = std::env::var_os("LC_EDITOR_BINARY") {
        let path = PathBuf::from(override_path);
        if path.exists() {
            return Ok(path);
        }
        anyhow::bail!(
            "LC_EDITOR_BINARY points to `{}` but no such file exists",
            path.display()
        );
    }

    let paths = cached_app_paths()
        .map_err(|err| anyhow!("unable to locate editor binary via app paths: {err}"))?;
    let install_root = paths.install_root();
    for candidate in editor_binary_candidates(install_root) {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(anyhow!(
        "LegacyClonk editor binary not found under {}; set LC_EDITOR_BINARY to override",
        install_root.display()
    ))
}

fn editor_binary_candidates(base: &Path) -> Vec<PathBuf> {
    fn push_candidate(candidates: &mut Vec<PathBuf>, candidate: PathBuf) {
        if !candidates.iter().any(|existing| existing == &candidate) {
            candidates.push(candidate);
        }
    }

    let mut candidates = Vec::new();
    let bin_dir = base.join("bin");

    push_candidate(&mut candidates, base.join("Editor.exe"));
    push_candidate(&mut candidates, base.join("editor.exe"));
    push_candidate(&mut candidates, base.join("Editor"));
    push_candidate(&mut candidates, base.join("editor"));
    push_candidate(&mut candidates, base.join("lc-editor"));
    push_candidate(&mut candidates, bin_dir.join("Editor.exe"));
    push_candidate(&mut candidates, bin_dir.join("editor.exe"));
    push_candidate(&mut candidates, bin_dir.join("Editor"));
    push_candidate(&mut candidates, bin_dir.join("editor"));
    push_candidate(&mut candidates, bin_dir.join("lc-editor"));

    #[cfg(target_os = "macos")]
    {
        push_candidate(
            &mut candidates,
            base.join("Editor.app")
                .join("Contents")
                .join("MacOS")
                .join("Editor"),
        );
        push_candidate(
            &mut candidates,
            base.join("Editor.app")
                .join("Contents")
                .join("MacOS")
                .join("LegacyClonk Editor"),
        );
        push_candidate(
            &mut candidates,
            base.join("LegacyClonk Editor.app")
                .join("Contents")
                .join("MacOS")
                .join("LegacyClonk Editor"),
        );
        push_candidate(
            &mut candidates,
            base.join("LegacyClonk Editor.app")
                .join("Contents")
                .join("MacOS")
                .join("Editor"),
        );
    }

    candidates
}

fn load_install_definitions(
    engine: &mut Engine,
    paths: &AppPaths,
    audio: Option<&mut AudioContext>,
) -> Result<Option<String>, EngineError> {
    let group = match open_install_objects_group(paths) {
        Some(group) => group,
        None => {
            tracing::debug!(
                install_root = %paths.install_root().display(),
                planet = %paths.planet_dir().display(),
                "no install object definitions found; continuing with sandbox fallback"
            );
            return Ok(None);
        }
    };

    let mut seen = HashSet::new();
    let mut spawn_candidate = None;
    let audio_ptr = audio.map(NonNull::from);
    let _ =
        load_definitions_from_group(engine, &group, audio_ptr, &mut seen, &mut spawn_candidate)?;
    Ok(spawn_candidate)
}

fn open_install_objects_group(paths: &AppPaths) -> Option<Group> {
    const OBJECT_GROUP_NAMES: &[&str] = &["Objects.ocd", "Objects.c4d", "Objects.ocg"];

    let mut bases = Vec::new();
    bases.push(paths.planet_dir().to_path_buf());
    bases.push(paths.install_root().to_path_buf());
    bases.sort();
    bases.dedup();

    for base in bases {
        if let Ok(group) = Group::open(&base) {
            for name in OBJECT_GROUP_NAMES {
                match open_child_flexible(&group, Path::new(name)) {
                    Ok(Some(child)) => return Some(child),
                    Ok(None) => {}
                    Err(err) => {
                        tracing::debug!(
                            base = %base.display(),
                            candidate = *name,
                            error = %err,
                            "error while probing install object group"
                        );
                    }
                }
            }
        }

        for name in OBJECT_GROUP_NAMES {
            let candidate = base.join(name);
            if let Ok(group) = Group::open(&candidate) {
                return Some(group);
            }
        }
    }

    None
}

fn load_definitions_from_group(
    engine: &mut Engine,
    group: &Group,
    mut audio: Option<NonNull<AudioContext>>,
    seen: &mut HashSet<String>,
    spawn_candidate: &mut Option<String>,
) -> Result<Option<NonNull<AudioContext>>, EngineError> {
    if group.exists("DefCore.txt") {
        match ResourceDefinitionData::load(group) {
            Ok(resource) => {
                let id_normalized = resource.core.id.to_ascii_lowercase();
                if seen.insert(id_normalized) {
                    match Definition::from_resource(&resource) {
                        Ok(definition) => match engine.register_definition(definition) {
                            Ok(()) => {
                                if resource.core.crew_member {
                                    if spawn_candidate
                                        .as_ref()
                                        .map(|existing| existing.eq_ignore_ascii_case("Clonk"))
                                        .unwrap_or(false)
                                    {
                                        // Clonk already selected; keep it.
                                    } else if resource.core.id.eq_ignore_ascii_case("Clonk")
                                        || spawn_candidate.is_none()
                                    {
                                        *spawn_candidate = Some(resource.core.id.clone());
                                    }
                                }
                            }
                            Err(EngineError::DefinitionAlreadyExists(_)) => {}
                            Err(error) => {
                                tracing::warn!(
                                    definition = %resource.core.id,
                                    error = ?error,
                                    "failed to register install definition"
                                );
                            }
                        },
                        Err(error) => {
                            tracing::warn!(
                                definition = %resource.core.id,
                                error = ?error,
                                "failed to compile install definition script"
                            );
                        }
                    }
                    if let Some(mut ptr) = audio {
                        unsafe {
                            ptr.as_mut()
                                .register_definition_sounds(&resource.core.id, group);
                        }
                    }
                }
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    group = %group.root().display(),
                    "failed to load definition resources"
                );
            }
        }
    }

    let entries = match group.entries() {
        Ok(entries) => entries,
        Err(err) => {
            tracing::warn!(
                error = %err,
                group = %group.root().display(),
                "unable to list definition contents"
            );
            return Ok(audio);
        }
    };

    for entry in entries {
        if !entry.is_directory {
            continue;
        }
        match group.open_child(&entry.relative_path) {
            Ok(child) => {
                audio = load_definitions_from_group(engine, &child, audio, seen, spawn_candidate)?;
            }
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    path = %entry.relative_path.display(),
                    "failed to inspect nested definition group"
                );
            }
        }
    }

    Ok(audio)
}

fn configure_sandbox_engine(
    engine: &mut Engine,
    audio: Option<&mut AudioContext>,
) -> Result<String, EngineError> {
    if let Ok(paths) = cached_app_paths() {
        match load_install_definitions(engine, &paths, audio) {
            Ok(Some(spawn_definition)) => {
                engine.set_environment(EnvironmentSettings::default());
                engine.set_landscape(Landscape::flat(2048, DEFAULT_GROUND_HEIGHT));
                return Ok(spawn_definition);
            }
            Ok(None) => {
                // No install definitions found; fall back to targeted loader.
            }
            Err(error) => {
                tracing::warn!(
                    error = ?error,
                    "encountered error while loading install definitions; falling back to sandbox walker"
                );
            }
        }
    }

    let install_definition_id = "Clonk";
    if let Some(resource_def) = try_load_install_definition(install_definition_id) {
        match Definition::from_resource(&resource_def) {
            Ok(definition) => {
                engine.register_definition(definition)?;
                engine.set_environment(EnvironmentSettings::default());
                engine.set_landscape(Landscape::flat(2048, DEFAULT_GROUND_HEIGHT));
                return Ok(resource_def.core.id);
            }
            Err(err) => {
                tracing::warn!(
                    definition = install_definition_id,
                    error = %err,
                    "failed to compile install definition; falling back to sandbox walker"
                );
            }
        }
    }

    let mut definition = Definition::from_script("Walker", "Rust Walker", walker_script())?;
    let mut actions = HashMap::new();
    actions.insert(
        "Walk".to_string(),
        ActionSpec::default().with_procedure("Walk"),
    );
    definition.configure_actions(Some("Walk".to_string()), actions);
    definition.set_crew_member(true);
    let profile = MovementProfile::default()
        .with_walk_speed(8)
        .with_walk_acceleration(2);
    definition.set_movement_profile(profile);
    engine.register_definition(definition)?;
    engine.set_environment(EnvironmentSettings::default());
    engine.set_landscape(Landscape::flat(2048, DEFAULT_GROUND_HEIGHT));
    Ok("Walker".to_string())
}

fn try_load_install_definition(definition_id: &str) -> Option<ResourceDefinitionData> {
    let paths = match cached_app_paths() {
        Ok(paths) => paths,
        Err(err) => {
            tracing::debug!(
                definition = definition_id,
                error = %err,
                "install root unavailable; cannot load real definition"
            );
            return None;
        }
    };

    let objects_path = paths.planet_dir().join("Objects.ocd");
    let objects_group = match Group::open(objects_path) {
        Ok(group) => group,
        Err(err) => {
            tracing::debug!(
                definition = definition_id,
                error = %err,
                "failed to open Objects.ocd; cannot load real definition"
            );
            return None;
        }
    };

    match find_definition_in_group(&objects_group, definition_id) {
        Ok(result) => result,
        Err(err) => {
            tracing::warn!(
                definition = definition_id,
                error = %err,
                "error while searching for definition in install data"
            );
            None
        }
    }
}

fn find_definition_in_group(
    group: &Group,
    definition_id: &str,
) -> Result<Option<ResourceDefinitionData>, ResourceDefinitionError> {
    for entry in group.entries()? {
        if !entry.is_directory {
            continue;
        }
        let child = group.open_child(&entry.relative_path)?;
        match ResourceDefCore::load(&child) {
            Ok(core) => {
                if core.id.eq_ignore_ascii_case(definition_id) {
                    let definition = ResourceDefinitionData::load(&child)?;
                    return Ok(Some(definition));
                }
            }
            Err(ResourceDefinitionError::DefCoreMissing) => {}
            Err(ResourceDefinitionError::Resources(err)) => match err {
                GroupError::EntryNotFound(_) => {}
                other => return Err(ResourceDefinitionError::Resources(other)),
            },
            Err(other) => return Err(other),
        }
        if let Some(found) = find_definition_in_group(&child, definition_id)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

fn load_frontend_scenarios() -> Vec<FrontendScenario> {
    match AppPaths::discover() {
        Ok(paths) => {
            let roots = scenario_roots(&paths);
            let existing_roots: Vec<_> = roots.into_iter().filter(|path| path.exists()).collect();
            if !existing_roots.is_empty() {
                match resource_scenario::discover_many(existing_roots.iter()) {
                    Ok(entries) => {
                        let mut seen = HashSet::new();
                        let mut scenarios = Vec::new();
                        for entry in entries {
                            if let Some(converted) =
                                FrontendScenario::from_resource(entry, &mut seen)
                            {
                                scenarios.push(converted);
                            }
                        }
                        if !scenarios.is_empty() {
                            scenarios.sort_by(|a, b| a.title.cmp(&b.title));
                            return scenarios;
                        }
                    }
                    Err(err) => {
                        tracing::warn!(
                            error = %err,
                            "failed to discover scenarios from install roots"
                        );
                    }
                }
            }
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                "app paths discovery failed; falling back to built-in sandbox scenario"
            );
        }
    }

    vec![FrontendScenario::fallback()]
}

fn scenario_roots(paths: &AppPaths) -> Vec<PathBuf> {
    let mut roots = vec![
        paths.scenario_dir(),
        paths.install_root().join("Scenarios"),
        paths.install_root().join("scenarios"),
        paths.planet_dir().to_path_buf(),
        paths.system_group_path().to_path_buf(),
    ];
    roots.sort();
    roots.dedup();
    roots
}

fn load_scenario_music_bytes(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    let group = Group::open(path)
        .with_context(|| format!("failed to open scenario group at {}", path.display()))?;
    find_music_asset(&group)
        .with_context(|| format!("failed to inspect {} for music", path.display()))
}

fn find_music_asset(group: &Group) -> Result<Option<Vec<u8>>, lc_resources::GroupError> {
    let entries = group.entries()?;
    let mut best: Option<(PathBuf, (u8, u8, String))> = None;

    for entry in &entries {
        if entry.is_directory || !is_audio_path(&entry.relative_path) {
            continue;
        }
        let key = music_sort_key(&entry.relative_path);
        if best
            .as_ref()
            .map(|(_, current)| key < *current)
            .unwrap_or(true)
        {
            best = Some((entry.relative_path.clone(), key));
        }
    }

    if let Some((path, _)) = best {
        let data = group.read_file(&path)?;
        return Ok(Some(data));
    }

    for entry in entries.into_iter().filter(|entry| entry.is_directory) {
        let child = group.open_child(&entry.relative_path)?;
        if let Some(data) = find_music_asset(&child)? {
            return Ok(Some(data));
        }
    }

    Ok(None)
}

fn music_sort_key(path: &Path) -> (u8, u8, String) {
    let in_music_dir = path
        .components()
        .any(|component| matches!(component.as_os_str().to_str(), Some(name) if name.eq_ignore_ascii_case("music")));
    let extension_rank = match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("ogg") => 0,
        Some("mp3") => 1,
        Some("wav") => 2,
        _ => 3,
    };
    let name = path.to_string_lossy().to_string();
    (if in_music_dir { 0 } else { 1 }, extension_rank, name)
}

fn is_audio_path(path: &Path) -> bool {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("ogg") | Some("mp3") | Some("wav") => true,
        _ => false,
    }
}

fn sandbox_music_bytes() -> &'static [u8] {
    static DATA: OnceLock<Vec<u8>> = OnceLock::new();
    DATA.get_or_init(|| generate_sine_wave_wav(220.0, 1.5))
        .as_slice()
}

fn compute_mix_values(
    info: &ChannelInfo,
    snapshot: &SimulationSnapshot,
    focus: Option<&ObjectSnapshot>,
    viewport_center: Vector2,
) -> (f32, f32) {
    const AUDIBILITY_RADIUS: f32 = 700.0;
    const PAN_DIVISOR: f32 = 5.0;
    const PAN_LIMIT: f32 = 100.0;

    let base_volume = (info.volume as f32 / 100.0).clamp(0.0, 1.0);
    let Some(target_id) = info.target else {
        return (base_volume, 0.0);
    };
    let Some(target) = snapshot.object(target_id) else {
        return (base_volume, 0.0);
    };
    let source = target.position;

    let mut listeners: Vec<Vector2> = Vec::new();
    if let Some(focus_object) = focus {
        listeners.push(focus_object.position);
    }
    for player in &snapshot.hud.players {
        if let Some(focus_id) = player.focus {
            if let Some(object) = snapshot.object(focus_id) {
                listeners.push(object.position);
            }
        }
    }
    if listeners.is_empty() {
        listeners.push(viewport_center);
    }

    let mut best_audibility: f32 = 0.0;
    for listener in &listeners {
        let dx = (source.x - listener.x) as f32;
        let dy = (source.y - listener.y) as f32;
        let distance = (dx * dx + dy * dy).sqrt();
        let mut audibility = (1.0 - distance / AUDIBILITY_RADIUS).clamp(0.0, 1.0);
        if let Some(falloff_distance) = info.custom_falloff {
            if falloff_distance != 0 {
                let scale = AUDIBILITY_RADIUS / falloff_distance as f32;
                audibility = (1.0 + (audibility - 1.0) * scale).clamp(0.0, 1.0);
            }
        }
        best_audibility = best_audibility.max(audibility);
    }

    let mut pan_accumulator = 0.0;
    let mut pan_contributors = 0;
    if snapshot.hud.players.is_empty() {
        pan_accumulator += (source.x - viewport_center.x) as f32 / PAN_DIVISOR;
        pan_contributors = 1;
    } else {
        for player in &snapshot.hud.players {
            let center = player
                .focus
                .and_then(|focus_id| snapshot.object(focus_id))
                .map(|obj| obj.position)
                .unwrap_or(viewport_center);
            pan_accumulator += (source.x - center.x) as f32 / PAN_DIVISOR;
            pan_contributors += 1;
        }
    }
    if pan_contributors == 0 {
        pan_accumulator += (source.x - viewport_center.x) as f32 / PAN_DIVISOR;
    }
    let pan = (pan_accumulator.clamp(-PAN_LIMIT, PAN_LIMIT)) / PAN_LIMIT;

    (base_volume * best_audibility, pan.clamp(-1.0, 1.0))
}

fn generate_sine_wave_wav(frequency_hz: f32, duration_seconds: f32) -> Vec<u8> {
    let safe_duration = duration_seconds.max(0.1);
    let sample_rate = 44_100u32;
    let channels = 2u16;
    let bits_per_sample = 16u16;
    let frame_count = (sample_rate as f32 * safe_duration).round().max(1.0) as usize;
    let block_align = (channels * (bits_per_sample / 8)) as u16;
    let byte_rate = sample_rate * block_align as u32;
    let data_len = frame_count * block_align as usize;
    let chunk_size = 36 + data_len;

    let mut buffer = Vec::with_capacity(44 + data_len);
    buffer.extend_from_slice(b"RIFF");
    buffer.extend_from_slice(&(chunk_size as u32).to_le_bytes());
    buffer.extend_from_slice(b"WAVE");
    buffer.extend_from_slice(b"fmt ");
    buffer.extend_from_slice(&16u32.to_le_bytes());
    buffer.extend_from_slice(&1u16.to_le_bytes());
    buffer.extend_from_slice(&channels.to_le_bytes());
    buffer.extend_from_slice(&sample_rate.to_le_bytes());
    buffer.extend_from_slice(&byte_rate.to_le_bytes());
    buffer.extend_from_slice(&block_align.to_le_bytes());
    buffer.extend_from_slice(&bits_per_sample.to_le_bytes());
    buffer.extend_from_slice(b"data");
    buffer.extend_from_slice(&(data_len as u32).to_le_bytes());

    let amplitude = i16::MAX as f32 * 0.2;
    for frame in 0..frame_count {
        let t = frame as f32 / sample_rate as f32;
        let sample = (2.0 * PI * frequency_hz * t).sin();
        let value = (sample * amplitude).round() as i16;
        buffer.extend_from_slice(&value.to_le_bytes());
        buffer.extend_from_slice(&value.to_le_bytes());
    }

    buffer
}

fn walker_script() -> &'static str {
    r#"
global func Initialize(state, random) { return nil; }
global func Step(state, frame, random) { return nil; }
"#
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_audio::decode_audio;
    use lc_engine::{
        ActionState, CommandDirection, Direction, EnvironmentFrame, HudPlayerSnapshot, HudSnapshot,
        ObjectId, ObjectSnapshot, ObjectStatus, PlayerState, PlayerStatus, SimulationSnapshot,
        Vector2, DEFAULT_CATEGORY,
    };
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    use std::collections::HashMap;
    use std::env;
    use std::ffi::OsString;
    use std::fs;
    use std::path::Path;
    use std::sync::{Mutex, MutexGuard, OnceLock};
    use tempfile::tempdir;

    fn make_object(id: u64, definition: &str, position: Vector2) -> ObjectSnapshot {
        ObjectSnapshot {
            id: ObjectId::new(id),
            definition_id: definition.to_string(),
            position,
            velocity: Vector2::new(0, 0),
            energy: 100,
            damage: 0,
            action: ActionState::default(),
            direction: Direction::default(),
            command_direction: CommandDirection::default(),
            action_procedure: None,
            effects: Vec::new(),
            vertices: Vec::new(),
            container: None,
            contents: Vec::new(),
            status: ObjectStatus::Normal,
            owner: 1,
            category: DEFAULT_CATEGORY,
            crew_member: true,
            alive: true,
        }
    }

    fn make_snapshot(
        objects: Vec<ObjectSnapshot>,
        hud_players: Vec<HudPlayerSnapshot>,
    ) -> SimulationSnapshot {
        let mut known_crew_owners: Vec<i32> =
            hud_players.iter().map(|player| player.owner).collect();
        known_crew_owners.sort_unstable();
        known_crew_owners.dedup();

        SimulationSnapshot {
            frame: 0,
            physics: None,
            objects,
            environment: EnvironmentFrame::default(),
            sky: None,
            weather_events: Vec::new(),
            global_effects: Vec::new(),
            particles: Vec::new(),
            players: Vec::new(),
            crew_selection: HashMap::new(),
            crew_roles: HashMap::new(),
            known_crew_owners,
            eliminated_crew_owners: Vec::new(),
            landscape: None,
            rng: ChaCha8Rng::seed_from_u64(1),
            surfaces: Vec::new(),
            hud: HudSnapshot {
                players: hud_players,
                messages: Vec::new(),
            },
            controls: Vec::new(),
            network_packets: Vec::new(),
            definition_categories: HashMap::new(),
            transfer_zones: Vec::new(),
            audio: Vec::new(),
        }
    }

    struct EnvGuard {
        _lock: parking_lot::ReentrantMutexGuard<'static, ()>,
        saved: Vec<(String, Option<OsString>)>,
    }

    impl EnvGuard {
        fn set(vars: &[(&str, Option<&Path>)]) -> Self {
            let lock = env_lock().lock();
            super::reset_cached_app_paths();
            let mut saved = Vec::with_capacity(vars.len());
            for (key, value) in vars {
                let original = env::var_os(key);
                saved.push((key.to_string(), original));
                match value {
                    Some(path) => env::set_var(key, path.as_os_str()),
                    None => env::remove_var(key),
                }
            }
            Self { _lock: lock, saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, value) in self.saved.drain(..) {
                match value {
                    Some(val) => env::set_var(&key, val),
                    None => env::remove_var(&key),
                }
            }
        }
    }

    pub(super) fn env_lock() -> &'static ReentrantMutex<()> {
        static LOCK: OnceLock<ReentrantMutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| ReentrantMutex::new(()))
    }

    #[test]
    fn matches_sound_pattern_handles_glob_wildcards() {
        assert!(matches_sound_pattern("clonk*", "clonk.wav"));
        assert!(matches_sound_pattern("clonk*", "clonk001.wav"));
        assert!(matches_sound_pattern("*.wav", "sound.wav"));
        assert!(matches_sound_pattern("sound?.wav", "sound1.wav"));
        assert!(!matches_sound_pattern("sound?.wav", "sound12.wav"));
        assert!(matches_sound_pattern("mix???.ogg", "mix001.ogg"));
        assert!(!matches_sound_pattern("mix???.ogg", "mix01.ogg"));
    }

    #[test]
    fn sound_search_terms_preserves_wildcards() {
        let terms = SoundSearchTerms::new("Sound*");
        assert_eq!(terms.wildcard_pattern.as_deref(), Some("sound*.wav"));
        assert!(terms.search_names.is_empty());
    }

    #[test]
    fn compute_mix_values_matches_cxx_audibility() {
        let listener = make_object(1, "Listener", Vector2::new(1000, 1000));
        let source = make_object(2, "Source", Vector2::new(1350, 1000));
        let snapshot = make_snapshot(
            vec![listener.clone(), source.clone()],
            vec![HudPlayerSnapshot {
                owner: 1,
                crew: vec![listener.id],
                focus: Some(listener.id),
                eliminated: false,
            }],
        );
        let info = ChannelInfo {
            channel: ChannelId(0),
            looped: false,
            target: Some(source.id),
            volume: 100,
            custom_falloff: None,
        };
        let (volume, pan) =
            compute_mix_values(&info, &snapshot, Some(&listener), Vector2::new(1000, 1000));
        assert!((volume - 0.5).abs() < 1e-6, "volume={volume}");
        assert!((pan - 0.7).abs() < 1e-6, "pan={pan}");
    }

    #[test]
    fn compute_mix_values_respects_custom_falloff() {
        let listener = make_object(1, "Listener", Vector2::new(1000, 1000));
        let source = make_object(2, "Source", Vector2::new(1700, 1000));
        let snapshot = make_snapshot(
            vec![listener.clone(), source.clone()],
            vec![HudPlayerSnapshot {
                owner: 1,
                crew: vec![listener.id],
                focus: Some(listener.id),
                eliminated: false,
            }],
        );
        let info = ChannelInfo {
            channel: ChannelId(0),
            looped: false,
            target: Some(source.id),
            volume: 100,
            custom_falloff: Some(1400),
        };
        let (volume, pan) =
            compute_mix_values(&info, &snapshot, Some(&listener), Vector2::new(1000, 1000));
        assert!((volume - 0.5).abs() < 1e-6, "volume={volume}");
        assert!((pan - 1.0).abs() < 1e-6, "pan={pan}");
    }

    #[test]
    fn compute_mix_values_for_global_sound_preserves_base_mix() {
        let listener = make_object(1, "Listener", Vector2::new(0, 0));
        let snapshot = make_snapshot(
            vec![listener.clone()],
            vec![HudPlayerSnapshot {
                owner: 1,
                crew: vec![listener.id],
                focus: Some(listener.id),
                eliminated: false,
            }],
        );
        let info = ChannelInfo {
            channel: ChannelId(0),
            looped: false,
            target: None,
            volume: 80,
            custom_falloff: None,
        };
        let (volume, pan) =
            compute_mix_values(&info, &snapshot, Some(&listener), Vector2::new(0, 0));
        assert!((volume - 0.8).abs() < 1e-6);
        assert_eq!(pan, 0.0);
    }

    #[test]
    fn save_file_version_deserializes_legacy_integer() {
        let version: SaveFileVersion = serde_json::from_str("1").expect("parse legacy version");
        assert_eq!(version, SaveFileVersion::new(1, 0, 0));
    }

    #[test]
    fn save_file_version_deserializes_string() {
        let version: SaveFileVersion =
            serde_json::from_str("\"2.3.4\"").expect("parse semantic string version");
        assert_eq!(version, SaveFileVersion::new(2, 3, 4));
    }

    #[test]
    fn migration_allows_previous_minor_version() {
        let engine = Engine::new();
        let engine_state = engine.capture_state();
        let save = SavedGameFile {
            version: SaveFileVersion::new(1, 0, 0),
            saved_at_seconds: 0,
            scenario: SavedScenarioInfo {
                identifier: "test".to_string(),
                title: "Test Scenario".to_string(),
                description: None,
                path: None,
                is_editable: false,
                is_playable: true,
                label: "Test".to_string(),
                fallback_ground: 0,
                sandbox: true,
            },
            focus_id: None,
            engine_state,
        };

        let migrated =
            migrate_save_file(save).expect("legacy save should migrate to current schema");
        assert_eq!(migrated.version, SAVE_FILE_VERSION);
    }

    fn test_font() -> Arc<dyn TextFont> {
        Arc::new(BitmapFont::new())
    }

    fn sample_scenarios() -> Vec<FrontendScenario> {
        let child = FrontendScenario {
            identifier: "scenario_alpha".to_string(),
            title: "Alpha".to_string(),
            description: None,
            kind: ScenarioKind::Scenario,
            is_editable: true,
            is_playable: true,
            path: None,
            preview: None,
            children: Vec::new(),
        };

        let folder = FrontendScenario {
            identifier: "folder_missions".to_string(),
            title: "Missions".to_string(),
            description: Some("Mission pack".to_string()),
            kind: ScenarioKind::Folder,
            is_editable: false,
            is_playable: false,
            path: None,
            preview: None,
            children: vec![child],
        };

        vec![folder]
    }

    #[test]
    fn placeholder_preview_has_expected_dimensions() {
        let preview = generate_preview_placeholder(ScenarioKind::Scenario, "Alpha");
        assert_eq!(preview.width(), PLACEHOLDER_PREVIEW_WIDTH);
        assert_eq!(preview.height(), PLACEHOLDER_PREVIEW_HEIGHT);
        let pixels = preview.pixels();
        let mut chunks = pixels.chunks_exact(4);
        let mut varied = false;
        if let Some(first) = chunks.next() {
            for chunk in chunks {
                if chunk != first {
                    varied = true;
                    break;
                }
            }
        }
        assert!(varied, "placeholder preview should contain color variation");
    }

    #[test]
    fn collect_player_overlay_marks_focus_and_energy() {
        let focus = ObjectId::new(1);
        let teammate = ObjectId::new(2);

        let objects = vec![
            ObjectSnapshot {
                id: focus,
                definition_id: "Clonk".into(),
                position: Vector2::new(0, 0),
                velocity: Vector2::ZERO,
                energy: 80,
                damage: 0,
                action: ActionState::default(),
                direction: Direction::default(),
                command_direction: CommandDirection::default(),
                action_procedure: None,
                effects: Vec::new(),
                vertices: Vec::new(),
                container: None,
                contents: Vec::new(),
                status: ObjectStatus::Normal,
                owner: 1,
                category: DEFAULT_CATEGORY,
                crew_member: true,
                alive: true,
            },
            ObjectSnapshot {
                id: teammate,
                definition_id: "Balloon".into(),
                position: Vector2::new(10, 0),
                velocity: Vector2::ZERO,
                energy: 40,
                damage: 0,
                action: ActionState::default(),
                direction: Direction::default(),
                command_direction: CommandDirection::default(),
                action_procedure: None,
                effects: Vec::new(),
                vertices: Vec::new(),
                container: None,
                contents: Vec::new(),
                status: ObjectStatus::Normal,
                owner: 1,
                category: DEFAULT_CATEGORY,
                crew_member: true,
                alive: true,
            },
        ];

        let mut snapshot = SimulationSnapshot {
            frame: 0,
            physics: None,
            objects,
            environment: EnvironmentFrame::default(),
            sky: None,
            weather_events: Vec::new(),
            global_effects: Vec::new(),
            particles: Vec::new(),
            players: Vec::new(),
            crew_selection: HashMap::new(),
            crew_roles: HashMap::new(),
            known_crew_owners: vec![1],
            eliminated_crew_owners: Vec::new(),
            landscape: None,
            rng: ChaCha8Rng::seed_from_u64(1),
            surfaces: Vec::new(),
            hud: HudSnapshot {
                players: vec![HudPlayerSnapshot {
                    owner: 1,
                    crew: vec![focus, teammate],
                    focus: Some(focus),
                    eliminated: false,
                }],
                messages: Vec::new(),
            },
            controls: Vec::new(),
            network_packets: Vec::new(),
            definition_categories: HashMap::new(),
            transfer_zones: Vec::new(),
            audio: Vec::new(),
        };

        snapshot.players.push(PlayerState {
            id: 1,
            name: "Alice".into(),
            status: PlayerStatus::Active,
            wealth: 120,
            cursor: Some(focus),
            crew: vec![focus, teammate],
            ..PlayerState::default()
        });

        let overlay = collect_player_overlays(&snapshot, Some(focus));
        assert_eq!(overlay.len(), 1);
        let player = &overlay[0];
        assert_eq!(player.owner, 1);
        assert_eq!(player.name, "Alice");
        assert_eq!(player.wealth, 120);
        assert_eq!(player.cursor, Some(focus));
        assert!(!player.eliminated);
        assert_eq!(player.crew.len(), 2);

        let mut focused = player
            .crew
            .iter()
            .filter(|crew| crew.is_focus)
            .collect::<Vec<_>>();
        assert_eq!(focused.len(), 1, "only cursor object highlighted");
        let focus_entry = focused.pop().expect("focus highlight present");
        assert!(focus_entry.label.contains("Clonk"));
        assert!((focus_entry.energy_fraction - 0.8).abs() < f32::EPSILON);

        let other_entry = player
            .crew
            .iter()
            .find(|crew| crew.label.contains("Balloon"))
            .expect("non-focus crew present");
        assert!(!other_entry.is_focus);
        assert!((other_entry.energy_fraction - 0.4).abs() < f32::EPSILON);
    }

    #[test]
    fn saved_scenario_round_trips_basic_metadata() {
        let original = FrontendScenario {
            identifier: "test".into(),
            title: "Test Scenario".into(),
            description: Some("desc".into()),
            kind: ScenarioKind::Scenario,
            is_editable: true,
            is_playable: true,
            path: Some(PathBuf::from("/tmp/test.c4s")),
            preview: None,
            children: Vec::new(),
        };
        let info = SavedScenarioInfo::from_frontend(&original, "Label", 123);
        assert_eq!(info.identifier, original.identifier);
        assert_eq!(info.title, original.title);
        assert_eq!(info.path, original.path);
        assert_eq!(info.label, "Label");
        assert_eq!(info.fallback_ground, 123);
        let restored = info.to_frontend();
        assert_eq!(restored.identifier, original.identifier);
        assert_eq!(restored.title, original.title);
        assert_eq!(restored.path, original.path);
        assert!(restored.children.is_empty());
        assert_eq!(restored.kind, ScenarioKind::Scenario);
    }

    #[test]
    fn menu_state_navigates_folders() {
        let scenarios = sample_scenarios();
        let entries = build_menu_entries(&scenarios, false);
        let menu = StartupMenu::new(entries, test_font(), None).expect("startup menu");
        let mut state = MenuState::new(menu, scenarios);

        assert_eq!(state.current_entries().len(), 1);
        let root_entries = build_menu_entries(state.current_entries(), false);
        assert_eq!(root_entries.len(), 1);
        assert_eq!(root_entries[0].identifier, "folder_missions");
        assert_eq!(state.label_path(), DEFAULT_SCENARIO_LABEL.to_string());

        state.enter_folder("folder_missions");
        assert_eq!(state.current_entries().len(), 1);
        assert_eq!(state.stack.len(), 2);
        let folder_entries = build_menu_entries(state.current_entries(), state.stack.len() > 1);
        assert_eq!(folder_entries.len(), 2);
        assert_eq!(folder_entries[0].identifier, BACK_ENTRY_IDENTIFIER);
        assert_eq!(folder_entries[1].identifier, "scenario_alpha");
        assert_eq!(state.label_path(), "Missions".to_string());

        state.leave_folder();
        assert_eq!(state.current_entries().len(), 1);
        assert_eq!(state.stack.len(), 1);
        let root_again = build_menu_entries(state.current_entries(), false);
        assert_eq!(root_again.len(), 1);
        assert_eq!(root_again[0].identifier, "folder_missions");
        assert_eq!(state.label_path(), DEFAULT_SCENARIO_LABEL.to_string());
    }

    #[test]
    fn sandbox_music_is_decodable() {
        let audio = sandbox_music_bytes();
        let decoded = decode_audio(audio).expect("sandbox music decodes");
        assert_eq!(decoded.sample_rate, 44_100);
        assert!(decoded.frames.len() > 2_000);
    }

    #[test]
    fn menu_music_runs_in_menu_cycle() {
        lc_core::logging::init();

        let mut app = GameApp::new(
            320,
            200,
            AudioOptions::default(),
            None,
            RuntimeConfig {
                player_owner: 1,
                network: None,
            },
        )
        .expect("initialise app with audio");

        assert!(
            app.audio
                .as_ref()
                .map(|audio| audio.music_is_playing())
                .unwrap_or(false),
            "menu music should start on launch"
        );

        app.start_sandbox_scenario(FrontendScenario::fallback())
            .expect("start sandbox scenario");
        assert!(
            app.audio
                .as_ref()
                .map(|audio| audio.music_is_playing())
                .unwrap_or(false),
            "sandbox scenario should have looping music"
        );

        app.return_to_menu();
        assert!(
            app.audio
                .as_ref()
                .map(|audio| audio.music_is_playing())
                .unwrap_or(false),
            "menu music should resume after returning to the menu"
        );
    }

    #[test]
    fn quick_save_round_trips_state() {
        lc_core::logging::init();

        reset_cached_app_paths();
        {
            let _guard = EnvGuard::set(&[]);
            reset_cached_app_paths();

            let mut app = GameApp::new(
                320,
                200,
                AudioOptions::default(),
                None,
                RuntimeConfig {
                    player_owner: 1,
                    network: None,
                },
            )
            .expect("initialise app");
            app.start_sandbox_scenario(FrontendScenario::fallback())
                .expect("start sandbox scenario");

            for _ in 0..5 {
                app.update().expect("tick before save");
            }
            let saved_frame = app.snapshot.frame;

            app.quick_save().expect("quick save succeeds");
            assert!(
                app.last_save_path
                    .as_ref()
                    .map(|path| path.ends_with(QUICK_SAVE_FILE))
                    .unwrap_or(false),
                "quick save should note the save path"
            );

            for _ in 0..3 {
                app.update().expect("advance after save");
            }
            assert!(
                app.snapshot.frame > saved_frame,
                "frame should advance after save"
            );

            app.quick_load().expect("quick load succeeds");
            assert_eq!(
                app.snapshot.frame, saved_frame,
                "quick load should restore saved frame"
            );
            assert!(
                matches!(app.mode, AppMode::Running),
                "quick load should keep the game running"
            );

            cleanup_quicksave_file();
        }
        reset_cached_app_paths();
    }

    #[test]
    fn quick_save_persists_across_sessions() {
        lc_core::logging::init();

        reset_cached_app_paths();

        let install_dir = tempdir().unwrap();

        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let scenario_dir = install_dir.path().join("Scenarios").join("Alpha.c4s");
        let scripts_dir = scenario_dir.join("scripts");
        fs::create_dir_all(&scripts_dir).unwrap();
        fs::write(
            scenario_dir.join("Scenario.json"),
            r#"
            {
                "name": "Alpha Mission",
                "ground_height": 72,
                "landscape": { "kind": "flat", "width": 160, "height": 80 },
                "definitions": [
                    { "id": "Mover", "name": "Mover", "script": "scripts/mover.aul" }
                ],
                "initial_objects": [
                    {
                        "definition": "Mover",
                        "position": [40, 48],
                        "owner": 1,
                        "crew_member": true
                    }
                ]
            }
            "#,
        )
        .unwrap();
        fs::write(scripts_dir.join("mover.aul"), walker_script()).unwrap();

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();

        let quicksave_path = user_dir.join(SAVE_DIR_NAME).join(QUICK_SAVE_FILE);

        {
            let _guard = EnvGuard::set(&[
                ("LC_INSTALL_ROOT", Some(install_dir.path())),
                ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
            ]);

            reset_cached_app_paths();

            let saved_frame = {
                let mut app = GameApp::new(
                    320,
                    200,
                    AudioOptions::default(),
                    None,
                    RuntimeConfig {
                        player_owner: 1,
                        network: None,
                    },
                )
                .expect("initialise app");

                let scenario = app
                    .scenario_catalog
                    .get("Alpha.c4s")
                    .cloned()
                    .expect("scenario discovered");
                app.start_scenario(scenario).expect("start disk scenario");

                for _ in 0..5 {
                    app.update().expect("advance simulation before save");
                }
                let frame_before_save = app.snapshot.frame;

                app.quick_save().expect("quick save succeeds");
                assert!(
                    quicksave_path.exists(),
                    "expected quick save file to be written"
                );

                frame_before_save
            };

            {
                let mut app = GameApp::new(
                    320,
                    200,
                    AudioOptions::default(),
                    None,
                    RuntimeConfig {
                        player_owner: 1,
                        network: None,
                    },
                )
                .expect("initialise app after restart");

                assert!(
                    app.last_save_path
                        .as_ref()
                        .map(|path| path.ends_with(QUICK_SAVE_FILE))
                        .unwrap_or(false),
                    "expected quick save path to be remembered"
                );
                assert!(
                    matches!(app.mode, AppMode::Menu),
                    "new session should start in menu"
                );

                app.quick_load().expect("quick load succeeds");

                assert!(
                    matches!(app.mode, AppMode::Running),
                    "quick load should enter running mode"
                );
                assert_eq!(
                    app.snapshot.frame, saved_frame,
                    "quick load should restore the saved frame"
                );
                assert!(
                    app.active_scenario
                        .as_ref()
                        .and_then(|scenario| scenario.path.as_ref())
                        .map(|path| path.ends_with("Alpha.c4s"))
                        .unwrap_or(false),
                    "loaded scenario should reference disk path"
                );
            }

            reset_cached_app_paths();
        }
        reset_cached_app_paths();
    }

    #[test]
    fn load_frontend_scenarios_discovers_install_entries() {
        let install_dir = tempdir().unwrap();

        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let scenario_dir = install_dir.path().join("Scenarios");
        let alpha_dir = scenario_dir.join("Alpha.c4s");
        fs::create_dir_all(&alpha_dir).unwrap();
        fs::write(
            alpha_dir.join("Scenario.json"),
            br#"{"name":"Alpha Mission"}"#,
        )
        .unwrap();

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);

        let scenarios = load_frontend_scenarios();
        assert_eq!(
            scenarios.len(),
            1,
            "expected discovered scenario without fallback"
        );
        let scenario = &scenarios[0];
        assert_eq!(scenario.identifier, "Alpha.c4s");
        assert_eq!(scenario.title, "Alpha Mission");
        assert!(scenario.is_playable);
        assert_eq!(
            scenario
                .path
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str()),
            Some("Alpha.c4s")
        );
    }

    #[test]
    fn start_real_scenario_loads_from_disk() {
        lc_core::logging::init();

        let install_dir = tempdir().unwrap();

        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let scenario_dir = install_dir.path().join("Scenarios").join("Alpha.c4s");
        let scripts_dir = scenario_dir.join("scripts");
        fs::create_dir_all(&scripts_dir).unwrap();
        fs::write(
            scenario_dir.join("Scenario.json"),
            r#"
            {
                "name": "Alpha Mission",
                "ground_height": 72,
                "landscape": { "kind": "flat", "width": 160, "height": 80 },
                "definitions": [
                    { "id": "Mover", "name": "Mover", "script": "scripts/mover.aul" }
                ],
                "initial_objects": [
                    {
                        "definition": "Mover",
                        "position": [40, 48],
                        "owner": 1,
                        "crew_member": true
                    }
                ]
            }
            "#,
        )
        .unwrap();
        fs::write(scripts_dir.join("mover.aul"), walker_script()).unwrap();

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);

        let mut app = GameApp::new(
            320,
            200,
            AudioOptions::default(),
            None,
            RuntimeConfig {
                player_owner: 1,
                network: None,
            },
        )
        .expect("initialise app");

        let scenario = app
            .scenario_catalog
            .get("Alpha.c4s")
            .cloned()
            .expect("scenario discovered");
        assert_eq!(scenario.title, "Alpha Mission");

        app.start_scenario(scenario).expect("start disk scenario");

        assert!(
            matches!(app.mode, AppMode::Running),
            "mode should be Running"
        );
        assert_eq!(app.scenario_label, "Alpha Mission");
        assert_eq!(app.fallback_ground, 72);
        assert!(
            app.snapshot
                .objects
                .iter()
                .any(|object| object.definition_id == "Mover"),
            "expected spawned Mover object"
        );
        assert!(
            app.focus_id.is_some(),
            "expected focus to be assigned for crew member"
        );
        assert_eq!(
            app.active_scenario
                .as_ref()
                .and_then(|active| active.path.as_ref())
                .map(|path| path.as_path()),
            Some(scenario_dir.as_path()),
            "active scenario should track disk path"
        );
    }

    #[test]
    fn install_definition_resolver_handles_case_insensitive_paths() {
        lc_core::logging::init();
        reset_cached_app_paths();

        let install_dir = tempdir().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let objects_dir = planet_dir.join("objects.ocd").join("clonk.c4d");
        fs::create_dir_all(&objects_dir).unwrap();
        fs::write(
            objects_dir.join("DefCore.txt"),
            "[DefCore]\nid=Clonk\nName=Clonk\nCategory=1\nCrewMember=1\nValue=100\nMass=40\n",
        )
        .unwrap();
        fs::write(objects_dir.join("Script.c"), walker_script()).unwrap();

        let scenario_dir = install_dir.path().join("Scenarios").join("Alpha.c4s");
        fs::create_dir_all(&scenario_dir).unwrap();
        let scenario_group = Group::open(&scenario_dir).unwrap();

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);

        let paths = cached_app_paths().expect("discover app paths");
        let resolver = InstallDefinitionResolver::new(Some(paths.clone()));
        let groups = resolver
            .resolve_definition_groups(&scenario_group, "Objects.ocd\\Clonk.c4d")
            .expect("resolve definition groups");
        let found_definition = groups.iter().any(|group| {
            group
                .root()
                .to_string_lossy()
                .to_ascii_lowercase()
                .ends_with("clonk.c4d")
        });
        assert!(found_definition, "expected to locate definition group");

        reset_cached_app_paths();
    }

    #[test]
    fn load_install_definitions_discovers_mixed_case_objects_group() {
        lc_core::logging::init();
        reset_cached_app_paths();

        let install_dir = tempdir().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let objects_dir = planet_dir.join("objects.c4d").join("clonk.c4d");
        fs::create_dir_all(&objects_dir).unwrap();
        fs::write(
            objects_dir.join("DefCore.txt"),
            "[DefCore]\nid=Clonk\nName=Clonk\nCategory=1\nCrewMember=1\nValue=100\nMass=40\n",
        )
        .unwrap();
        fs::write(objects_dir.join("Script.c"), walker_script()).unwrap();

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);

        let paths = cached_app_paths().expect("discover app paths");
        let mut engine = Engine::new();
        let spawn =
            load_install_definitions(&mut engine, &paths, None).expect("load install definitions");
        assert_eq!(spawn.as_deref(), Some("Clonk"));
        assert!(
            engine
                .definition_ids()
                .any(|id| id.eq_ignore_ascii_case("Clonk")),
            "expected Clonk definition to be registered"
        );

        reset_cached_app_paths();
    }

    fn cleanup_quicksave_file() {
        let dir = resolve_save_directory();
        let path = dir.join(QUICK_SAVE_FILE);
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
}
