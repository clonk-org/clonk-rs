mod gamepad;
mod input;
mod settings;

use std::collections::{hash_map::DefaultHasher, HashMap, HashSet};
use std::f32::consts::PI;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use gamepad::{GamepadActionType, GamepadEvent, GamepadManager};
use input::KeyboardBindings;
use lc_audio::{AudioError, AudioSystem, ChannelId, MusicHandle, SoundHandle};
use lc_engine::{
    ActionSpec, ActionState, AudioCommand, ControlButton, ControlEvent, Definition, Engine,
    EngineError, EngineState, EnvironmentSettings, Landscape, MovementProfile, ObjectId,
    ObjectSnapshot, Scenario, SimulationSnapshot, SpawnConfig, Vector2,
};
use lc_frontend::{
    CrewOverlay, GraphicsOverlay, GraphicsSystem, GuiPoint, InputDispatcher, KeyCode,
    PlayerOverlay, ScenarioEntry, ScenarioKind, StartupMenu, StartupMenuAction,
};
use lc_graphics::Color;
use lc_platform::{AppPaths, PathsError};
use lc_resources::{scenario as resource_scenario, Group};
use pixels::{Pixels, SurfaceTexture};
use serde::{Deserialize, Serialize};
use settings::{AudioOptions, DisplayMode, DisplayOptions};
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{
    ElementState, Event, KeyboardInput, MouseButton, TouchPhase, VirtualKeyCode, WindowEvent,
};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::{Fullscreen, Window, WindowBuilder};

const PLAYER_OWNER: i32 = 1;
const FRAME_INTERVAL: Duration = Duration::from_micros(16_666); // ~60 FPS
const DEFAULT_SCENARIO_LABEL: &str = "Rust Sandbox";
const DEFAULT_GROUND_HEIGHT: i32 = 360;
const BACK_ENTRY_IDENTIFIER: &str = "__lc_menu_back";
const BACK_ENTRY_TITLE: &str = "← Back";
const SAVE_DIR_NAME: &str = "Savegames";
const QUICK_SAVE_FILE: &str = "quicksave.lcsave";
const SAVE_FILE_VERSION: u32 = 1;

fn main() -> Result<()> {
    let event_loop = EventLoop::new();
    let app_paths = cached_app_paths().ok();
    if let Some(paths) = app_paths {
        if let Err(err) = paths.ensure_user_dirs() {
            eprintln!(
                "warning: failed to ensure user data directories at {}: {err}",
                paths.user_data_dir().display()
            );
        }
    }
    let mut display_options = DisplayOptions::load(app_paths);
    let audio_options = AudioOptions::load(app_paths);
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

    let mut app = GameApp::new(size.width, size.height, audio_options, app_paths)
        .context("failed to initialise app state")?;

    let mut last_frame = Instant::now();

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Poll;
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
                    eprintln!("error: {err:?}");
                    control_flow.set_exit();
                }
            }
            Event::MainEventsCleared => {
                if let Err(err) = app.process_gamepad_events() {
                    eprintln!("gamepad input failed: {err:?}");
                    control_flow.set_exit();
                    return;
                }
                if last_frame.elapsed() >= FRAME_INTERVAL {
                    if let Err(err) = app.update() {
                        eprintln!("tick failed: {err:?}");
                        control_flow.set_exit();
                        return;
                    }
                    window.request_redraw();
                    last_frame = Instant::now();
                }
            }
            Event::RedrawRequested(id) if id == window.id() => {
                if let Err(err) = app.render(pixels.frame_mut()) {
                    eprintln!("render failed: {err:?}");
                    control_flow.set_exit();
                    return;
                }
                if let Err(err) = pixels.render() {
                    eprintln!("present failed: {err:?}");
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
            if let Some(paths) = app_paths {
                display_options.persist_if_dirty(paths);
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

    fn process_audio(&mut self, snapshot: &SimulationSnapshot, focus: Option<&ObjectSnapshot>) {
        let events = &snapshot.audio;
        if !events.is_empty() {
            self.handle_events(events, snapshot, focus);
        }
        self.update_channels(snapshot, focus);
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

    fn music_enabled(&self) -> bool {
        self.options.music_enabled
    }

    fn menu_music_enabled(&self) -> bool {
        self.options.menu_music_enabled
    }

    fn handle_events(
        &mut self,
        events: &[AudioCommand],
        snapshot: &SimulationSnapshot,
        focus: Option<&ObjectSnapshot>,
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
                    ) {
                        eprintln!("failed to play sound {name}: {err}");
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
                    self.update_sound_volume(name, *target, *volume, snapshot, focus);
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
    ) -> Result<(), AudioError> {
        if !self.options.sound_enabled {
            return Ok(());
        }
        let key = SoundInstanceKey::new(name, target);
        let handle = self.ensure_sound(name)?;
        let channel = self.system.play_sound(&handle, looped)?;
        let info = ChannelInfo {
            channel,
            looped,
            target,
            volume,
            custom_falloff,
        };
        let (mut mix_volume, pan) = compute_mix_values(&info, snapshot, focus);
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
    ) {
        let key = SoundInstanceKey::new(name, target);
        if let Some(info) = self.active_channels.get_mut(&key) {
            info.volume = volume;
            if !self.options.sound_enabled {
                return;
            }
            let channel = info.channel;
            let (mut mix_volume, pan) = compute_mix_values(info, snapshot, focus);
            mix_volume *= self.options.sound_volume;
            drop(info);
            self.system
                .channel_set_volume_and_pan(channel, mix_volume, pan);
        }
    }

    fn update_channels(&mut self, snapshot: &SimulationSnapshot, focus: Option<&ObjectSnapshot>) {
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
            let (mut mix_volume, pan) = compute_mix_values(info, snapshot, focus);
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

    fn ensure_sound(&mut self, name: &str) -> Result<SoundHandle, AudioError> {
        let request_key = name.to_ascii_lowercase();
        if let Some(resolved) = self.resolver.resolve_entry(name) {
            let cache_key = resolved.cache_key();
            if let Some(handle) = self.loaded_sounds.get(&cache_key) {
                return Ok(handle.clone());
            }
            match resolved.load_audio() {
                Ok(bytes) => {
                    let handle = self.system.load_sound(bytes.as_slice())?;
                    self.loaded_sounds.insert(cache_key.clone(), handle.clone());
                    return Ok(handle);
                }
                Err(err) => {
                    if self
                        .missing_sounds
                        .insert(format!("asset::{}", resolved.cache_marker()))
                    {
                        eprintln!(
                            "failed to load sound asset `{}` from {}: {err}",
                            name,
                            resolved.describe()
                        );
                    }
                }
            }
        }

        let fallback_cache_key = format!("__fallback::{}", request_key);
        if let Some(handle) = self.loaded_sounds.get(&fallback_cache_key) {
            return Ok(handle.clone());
        }

        let bytes = generate_tone_wav(name);
        let handle = self.system.load_sound(bytes.as_slice())?;
        self.loaded_sounds
            .insert(fallback_cache_key.clone(), handle.clone());
        if self
            .missing_sounds
            .insert(format!("request::{request_key}"))
        {
            eprintln!("missing sound asset `{}`; using synthetic fallback", name);
        }
        Ok(handle)
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
}

impl SoundResolver {
    fn new() -> Self {
        let global = discover_global_sound_libraries();
        Self {
            global,
            scenario: Vec::new(),
            scenario_root: None,
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
            if matches_question_pattern(pattern, &entry.file_name) {
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
        let normalized = prepared.replace('*', "?");
        let has_wildcards = normalized.contains('?');
        let normalized_lower = normalized.to_ascii_lowercase();

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
            eprintln!("sound asset discovery skipped: {err}");
        }
    }
    libraries
}

fn collect_sound_libraries_for_path(path: &Path) -> Vec<SoundLibrary> {
    let group = match Group::open(path) {
        Ok(group) => group,
        Err(err) => {
            eprintln!("failed to open sound group {}: {err}", path.display());
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
        eprintln!(
            "failed to inspect sound entries in {}: {err}",
            group.root().display()
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

fn matches_question_pattern(pattern: &str, candidate: &str) -> bool {
    if pattern.len() != candidate.len() {
        return false;
    }
    pattern
        .chars()
        .zip(candidate.chars())
        .all(|(p, c)| p == '?' || p == c)
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
    mode: AppMode,
    scenario_catalog: HashMap<String, FrontendScenario>,
    active_scenario: Option<FrontendScenario>,
    audio: Option<AudioContext>,
    last_save_path: Option<PathBuf>,
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
            eprintln!("failed to update startup menu entries: {err}");
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

#[derive(Clone, Debug)]
struct FrontendScenario {
    identifier: String,
    title: String,
    description: Option<String>,
    kind: ScenarioKind,
    is_editable: bool,
    is_playable: bool,
    path: Option<PathBuf>,
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
            children: Vec::new(),
        }
    }
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
            children: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SavedGameFile {
    version: u32,
    saved_at_seconds: u64,
    scenario: SavedScenarioInfo,
    focus_id: Option<ObjectId>,
    engine_state: EngineState,
}

fn cached_app_paths() -> std::result::Result<&'static AppPaths, PathsError> {
    static CACHE: OnceLock<std::result::Result<AppPaths, PathsError>> = OnceLock::new();
    match CACHE.get_or_init(|| AppPaths::discover()) {
        Ok(paths) => Ok(paths),
        Err(err) => Err(err.clone()),
    }
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
    ) -> Result<Self> {
        let engine = Engine::new();
        let snapshot = engine.snapshot();
        let scenario_label = DEFAULT_SCENARIO_LABEL.to_string();
        let mut graphics =
            GraphicsSystem::new(width, height, DEFAULT_GROUND_HEIGHT, &scenario_label);
        graphics.surface_mut().fill(Color::opaque(16, 28, 52));

        let scenarios = load_frontend_scenarios();
        let menu_entries = build_menu_entries(&scenarios, false);
        let mut menu = StartupMenu::new(menu_entries)
            .map_err(|err| anyhow!("failed to create startup menu: {err}"))?;
        menu.resize(width as f32, height as f32);

        let scenario_catalog = build_scenario_catalog(&scenarios);
        let menu_state = MenuState::new(menu, scenarios);
        let audio = match AudioContext::try_new(audio_options) {
            Ok(ctx) => Some(ctx),
            Err(err) => {
                eprintln!("audio initialisation failed: {err}");
                None
            }
        };

        let mut app = Self {
            engine,
            graphics,
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
            mode: AppMode::Menu,
            scenario_catalog,
            active_scenario: None,
            audio,
            last_save_path: None,
        };
        if let Some(existing) = existing_quick_save_path() {
            app.last_save_path = Some(existing);
        }
        Ok(app)
    }

    fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let mut graphics =
            GraphicsSystem::new(width, height, self.fallback_ground, &self.scenario_label);
        graphics.surface_mut().fill(Color::opaque(16, 28, 52));
        self.graphics = graphics;

        if self.mode == AppMode::Menu {
            self.menu_state.menu().resize(width as f32, height as f32);
            self.menu_state.set_pointer_position(None);
        }
        Ok(())
    }

    fn handle_key(&mut self, key: VirtualKeyCode, state: ElementState) -> Result<(), EngineError> {
        if state == ElementState::Pressed {
            match key {
                VirtualKeyCode::F5 => {
                    if let Err(err) = self.quick_save() {
                        eprintln!("quick save failed: {err:?}");
                    }
                    return Ok(());
                }
                VirtualKeyCode::F9 => {
                    if let Err(err) = self.quick_load() {
                        eprintln!("quick load failed: {err:?}");
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
                self.return_to_menu();
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
        let _ = self
            .input
            .handle_event(&mut self.engine, PLAYER_OWNER, event)?;
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
                        self.return_to_menu();
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
        let (start_identifier, updated_label) =
            GameApp::process_menu_actions(&mut self.menu_state, actions);

        if let Some(label) = updated_label {
            self.scenario_label = label;
        }

        if let Some(identifier) = start_identifier {
            if let Some(scenario) = self.scenario_catalog.get(&identifier).cloned() {
                self.start_scenario(scenario)?;
            } else {
                eprintln!("Selected scenario `{identifier}` is not available in Rust catalog");
            }
        }
        Ok(())
    }

    fn process_menu_actions(
        state: &mut MenuState,
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
                        state.leave_folder();
                        updated_label = Some(state.label_path());
                        continue;
                    }

                    let entry_kind = state
                        .current_entries()
                        .iter()
                        .find(|entry| entry.identifier == summary.identifier)
                        .map(|entry| entry.kind);

                    match entry_kind {
                        Some(ScenarioKind::Folder) => {
                            state.enter_folder(&summary.identifier);
                            updated_label = Some(state.label_path());
                        }
                        Some(ScenarioKind::Scenario) => {
                            start_identifier = Some(summary.identifier);
                        }
                        Some(ScenarioKind::Editor) => {
                            eprintln!(
                                "Editing entries is not yet implemented for Rust menu items: {}",
                                summary.identifier
                            );
                        }
                        None => {
                            state.enter_folder(&summary.identifier);
                            updated_label = Some(state.label_path());
                        }
                    }
                }
                StartupMenuAction::EditEntry(summary) => {
                    eprintln!(
                        "Editing entries is not yet implemented for Rust menu items: {}",
                        summary.identifier
                    );
                }
            }
        }

        (start_identifier, updated_label)
    }

    fn update(&mut self) -> Result<(), EngineError> {
        if matches!(self.mode, AppMode::Running) {
            self.snapshot = self.engine.tick()?;
            self.refresh_focus();
            self.update_audio();
        }
        Ok(())
    }

    fn update_audio(&mut self) {
        if let Some(audio) = self.audio.as_mut() {
            audio.process_audio(&self.snapshot, self.focus_snapshot.as_ref());
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
            render_menu_frame(&mut self.graphics, self.menu_state.menu(), frame);
            return Ok(());
        }
        self.render_running(frame)
    }

    fn render_running(&mut self, frame: &mut [u8]) -> Result<()> {
        if let Some(focus) = self.focus_snapshot.as_ref() {
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
            self.graphics.render_frame(&self.snapshot, focus);
        } else {
            self.graphics.surface_mut().fill(Color::opaque(12, 24, 40));
        }

        let surface = self.graphics.surface();
        let pixels = surface.pixels();
        if pixels.len() == frame.len() {
            frame.copy_from_slice(pixels);
        } else {
            copy_surface(pixels, surface.width(), surface.height(), frame);
        }
        Ok(())
    }

    fn return_to_menu(&mut self) {
        self.engine = Engine::new();
        self.input = InputDispatcher::new();
        self.snapshot = self.engine.snapshot();
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

        let width = self.graphics.surface().width();
        let height = self.graphics.surface().height();
        self.graphics =
            GraphicsSystem::new(width, height, self.fallback_ground, &self.scenario_label);
        self.graphics.surface_mut().fill(Color::opaque(16, 28, 52));

        self.menu_state.set_pointer_position(None);
        self.menu_state.refresh_menu_entries();
        self.menu_state.menu().resize(width as f32, height as f32);

        self.mode = AppMode::Menu;
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

        let scenario_data = match Scenario::load_from_path(path) {
            Ok(data) => data,
            Err(err) => {
                eprintln!(
                    "Failed to load scenario '{}' from {}: {err}",
                    scenario.title,
                    path.display()
                );
                return Ok(false);
            }
        };

        println!(
            "Starting scenario '{}' from {}",
            scenario.title,
            path.display()
        );

        self.engine = Engine::new();
        self.input = InputDispatcher::new();
        if let Some(audio) = self.audio.as_mut() {
            audio.configure_scenario(Some(path));
            audio.reset_sfx();
        }

        if let Err(err) = scenario_data.apply(&mut self.engine) {
            eprintln!(
                "Failed to apply scenario '{}' from {}: {err}",
                scenario.title,
                path.display()
            );
            return Ok(false);
        }

        self.snapshot = self.engine.snapshot();

        let label = scenario_data
            .name()
            .map(|name| name.to_string())
            .unwrap_or_else(|| scenario.title.clone());
        let ground = scenario_data
            .ground_height_hint()
            .unwrap_or(DEFAULT_GROUND_HEIGHT)
            .max(0);

        self.configure_running_state(label, ground);
        self.apply_focus_selection();
        self.snapshot = self.engine.snapshot();
        self.refresh_focus();
        self.active_scenario = Some(scenario.clone());
        self.play_scenario_audio(path);
        Ok(true)
    }

    fn start_sandbox_scenario(&mut self, scenario: FrontendScenario) -> Result<(), EngineError> {
        println!("Starting scenario '{}' (sandbox fallback)", scenario.title);

        self.engine = Engine::new();
        self.input = InputDispatcher::new();
        if let Some(audio) = self.audio.as_mut() {
            audio.configure_scenario(None);
            audio.reset_sfx();
        }

        configure_sandbox_engine(&mut self.engine)?;

        let spawn = SpawnConfig::new("Walker")
            .with_owner(PLAYER_OWNER)
            .with_position(Vector2::new(240, 180))
            .with_energy(100)
            .with_action(ActionState::new("Walk"))
            .with_crew_member(true);
        self.engine.spawn_object(spawn)?;

        self.snapshot = self.engine.snapshot();
        self.configure_running_state(scenario.title.clone(), DEFAULT_GROUND_HEIGHT);
        self.apply_focus_selection();
        self.snapshot = self.engine.snapshot();
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
        if save.version != SAVE_FILE_VERSION {
            anyhow::bail!(
                "unsupported quick save version {} (expected {})",
                save.version,
                SAVE_FILE_VERSION
            );
        }
        self.apply_loaded_game(save)?;
        self.last_save_path = Some(path.clone());
        Ok(())
    }

    fn apply_loaded_game(&mut self, save: SavedGameFile) -> Result<()> {
        let scenario_info = save.scenario.clone();
        let frontend = scenario_info.to_frontend();

        self.engine = Engine::new();
        self.input = InputDispatcher::new();

        if scenario_info.sandbox {
            configure_sandbox_engine(&mut self.engine)
                .context("failed to prepare sandbox engine for saved game")?;
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
            scenario_data.apply(&mut self.engine).with_context(|| {
                format!(
                    "failed to apply scenario `{}` from {}",
                    scenario_info.title,
                    path.display()
                )
            })?;
        }

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
                        eprintln!("failed to start music for {}: {err}", path.display());
                        audio.stop_music();
                    }
                }
                Ok(None) => audio.stop_music(),
                Err(err) => eprintln!("failed to load music from {}: {err}", path.display()),
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
                eprintln!("failed to start sandbox music: {err}");
                audio.stop_music();
            }
        }
    }

    fn configure_running_state(&mut self, label: String, fallback_ground: i32) {
        self.scenario_label = label;
        self.fallback_ground = fallback_ground;
        let width = self.graphics.surface().width();
        let height = self.graphics.surface().height();
        self.graphics =
            GraphicsSystem::new(width, height, self.fallback_ground, &self.scenario_label);
        self.graphics.surface_mut().fill(Color::opaque(12, 24, 40));
        self.frame_text.clear();
        self.status_text.clear();
        self.energy_fraction = 0.0;
        self.menu_state.set_pointer_position(None);
        self.mode = AppMode::Running;
    }

    fn apply_focus_selection(&mut self) {
        if let Some((object_id, owner, crew_member)) = select_focus_candidate(&self.snapshot) {
            self.focus_id = Some(object_id);
            if crew_member && owner >= 0 {
                if let Err(err) = self.engine.select_crew(owner, [object_id]) {
                    eprintln!(
                        "Failed to select crew member {} for owner {}: {err}",
                        object_id, owner
                    );
                } else if let Err(err) = self.engine.set_crew_cursor(owner, Some(object_id)) {
                    eprintln!(
                        "Failed to set crew cursor to {} for owner {}: {err}",
                        object_id, owner
                    );
                }
            }
        } else {
            self.focus_id = None;
        }
        self.focus_snapshot = None;
    }
}

fn render_menu_frame(graphics: &mut GraphicsSystem, menu: &mut StartupMenu, frame: &mut [u8]) {
    {
        let surface = graphics.surface_mut();
        surface.fill(Color::opaque(16, 28, 52));
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

fn collect_player_overlays(
    snapshot: &SimulationSnapshot,
    focus_id: Option<ObjectId>,
) -> Vec<PlayerOverlay> {
    let mut players = Vec::with_capacity(snapshot.hud.players.len());
    for player in &snapshot.hud.players {
        let mut crew = Vec::with_capacity(player.crew.len());
        for object_id in &player.crew {
            if let Some(object) = snapshot.object(*object_id) {
                let label = format!("{} #{}", object.definition_id, object.id.as_u64());
                let energy_fraction = (object.energy.max(0).min(100) as f32) / 100.0;
                let is_focus = focus_id == Some(object.id);
                crew.push(CrewOverlay {
                    label,
                    energy_fraction,
                    is_focus,
                });
            }
        }
        players.push(PlayerOverlay {
            owner: player.owner,
            eliminated: player.eliminated,
            crew,
        });
    }
    players
}

fn select_focus_candidate(snapshot: &SimulationSnapshot) -> Option<(ObjectId, i32, bool)> {
    for object in snapshot.objects.iter() {
        if is_focusable(object) && object.crew_member && object.owner == PLAYER_OWNER {
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

fn configure_sandbox_engine(engine: &mut Engine) -> Result<(), EngineError> {
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
    Ok(())
}

fn load_frontend_scenarios() -> Vec<FrontendScenario> {
    if let Ok(paths) = AppPaths::discover() {
        let roots = scenario_roots(&paths);
        let existing_roots: Vec<_> = roots.into_iter().filter(|path| path.exists()).collect();
        if !existing_roots.is_empty() {
            match resource_scenario::discover_many(existing_roots.iter()) {
                Ok(entries) => {
                    let mut seen = HashSet::new();
                    let mut scenarios = Vec::new();
                    for entry in entries {
                        if let Some(converted) = FrontendScenario::from_resource(entry, &mut seen) {
                            scenarios.push(converted);
                        }
                    }
                    if !scenarios.is_empty() {
                        scenarios.sort_by(|a, b| a.title.cmp(&b.title));
                        return scenarios;
                    }
                }
                Err(err) => {
                    eprintln!("failed to discover scenarios from install roots: {err}");
                }
            }
        }
    } else {
        eprintln!("App paths discovery failed; falling back to built-in sandbox scenario");
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

const DEFAULT_FALLOFF_DISTANCE: i32 = 400;

fn compute_mix_values(
    info: &ChannelInfo,
    snapshot: &SimulationSnapshot,
    focus: Option<&ObjectSnapshot>,
) -> (f32, f32) {
    let base_volume = (info.volume as f32 / 100.0).clamp(0.0, 1.0);
    let listener = focus.map(|obj| obj.position).unwrap_or(Vector2::new(0, 0));
    let source = info
        .target
        .and_then(|id| snapshot.object(id))
        .map(|obj| obj.position)
        .unwrap_or(listener);

    let dx = (source.x - listener.x) as f32;
    let dy = (source.y - listener.y) as f32;
    let distance = (dx * dx + dy * dy).sqrt();
    let falloff = info
        .custom_falloff
        .unwrap_or(DEFAULT_FALLOFF_DISTANCE)
        .max(1) as f32;
    let proximity = (1.0 - distance / falloff).clamp(0.0, 1.0);
    let volume = base_volume * proximity;
    let pan = (dx / falloff).clamp(-1.0, 1.0);
    (volume, pan)
}

fn generate_tone_wav(name: &str) -> Vec<u8> {
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    let hash = hasher.finish();
    let freq = 220.0 + (hash % 660) as f32;
    generate_sine_wave_wav(freq, 0.35)
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
        ObjectSnapshot, ObjectStatus, SimulationSnapshot, Vector2, DEFAULT_CATEGORY,
    };
    use rand::SeedableRng;
    use rand_chacha::ChaCha8Rng;
    use std::collections::HashMap;

    fn sample_scenarios() -> Vec<FrontendScenario> {
        let child = FrontendScenario {
            identifier: "scenario_alpha".to_string(),
            title: "Alpha".to_string(),
            description: None,
            kind: ScenarioKind::Scenario,
            is_editable: true,
            is_playable: true,
            path: None,
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
            children: vec![child],
        };

        vec![folder]
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

        let snapshot = SimulationSnapshot {
            frame: 0,
            physics: None,
            objects,
            environment: EnvironmentFrame::default(),
            global_effects: Vec::new(),
            particles: Vec::new(),
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
            },
            controls: Vec::new(),
            network_packets: Vec::new(),
            definition_categories: HashMap::new(),
            transfer_zones: Vec::new(),
            audio: Vec::new(),
        };

        let overlay = collect_player_overlays(&snapshot, Some(focus));
        assert_eq!(overlay.len(), 1);
        let player = &overlay[0];
        assert_eq!(player.owner, 1);
        assert!(!player.eliminated);
        assert_eq!(player.crew.len(), 2);

        let focus_entry = player
            .crew
            .iter()
            .find(|crew| crew.is_focus)
            .expect("focus highlight present");
        assert!(focus_entry.label.contains("Clonk"));
        assert!((focus_entry.energy_fraction - 0.8).abs() < f32::EPSILON);

        let other_entry = player
            .crew
            .iter()
            .find(|crew| !crew.is_focus)
            .expect("non-focus crew present");
        assert!(other_entry.label.contains("Balloon"));
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
        let menu = StartupMenu::new(entries).expect("startup menu");
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
}
