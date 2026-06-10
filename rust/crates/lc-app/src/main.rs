#![allow(dead_code)]
#![allow(
    clippy::explicit_counter_loop,
    clippy::large_enum_variant,
    clippy::manual_clamp,
    clippy::manual_strip,
    clippy::match_like_matches_macro,
    clippy::too_many_arguments
)]

mod clonk_fonts;
mod control_options;
mod game_over;
mod gamepad;
mod ingame_menu;
mod input;
mod menu_controls;
mod network;
mod object_menu;
mod save_browser;
mod settings;

use std::cmp::Ordering;
use std::collections::{
    hash_map::DefaultHasher, hash_map::Entry, BTreeMap, HashMap, HashSet, VecDeque,
};
use std::convert::TryFrom;
use std::fmt;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::ptr::NonNull;
use std::sync::{
    mpsc::{self, Receiver, TryRecvError},
    Arc, Mutex, OnceLock,
};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use control_options::{
    binding_display_name, format_key_label, ControlOptionsCommand, ControlOptionsState,
};
use game_over::{GameOverEntry, GameOverOutcome, GameOverState};
use gamepad::{GamepadActionType, GamepadEvent, GamepadManager};
use ingame_menu::{IngameMenuAction, IngameMenuState};
use input::{ControlBindingId, KeyboardBindings};
use lc_audio::{AudioError, AudioSystem, ChannelId, MusicHandle, SoundHandle};
use lc_core::std_config::Config;
use lc_engine::scenario::LegacyDefinitionResolver;
use lc_engine::{
    ActionSpec, ActionState, AudioCommand, CommandKind, ControlButton, ControlCommand,
    ControlEvent, Definition, Engine, EngineError, EngineState, EnvironmentSettings, FloatVector2,
    Landscape, MaterialSet, MenuCommandKind, MenuCommandSelection, MenuRequestKind, MessageKind,
    MovementProfile, ObjectId, ObjectSnapshot, ObjectUpdate, PlayerConfig, PlayerStatus, Recorder,
    Recording, RgbColor, Scenario, ScenarioError, SimulationSnapshot, SkyConfig, SpawnConfig,
    SyncCheckPacket, Vector2, FLAG_ALIGN_CENTER, FLAG_ALIGN_LEFT, FLAG_ALIGN_RIGHT, FLAG_BOTTOM,
    FLAG_HCENTER, FLAG_LEFT, FLAG_NO_BREAK, FLAG_RIGHT, FLAG_TOP, FLAG_VCENTER, FLAG_WIDTH_REL,
    FLAG_X_REL, FLAG_Y_REL, OWNER_NONE,
};
use lc_frontend::{
    default_owner_color, draw_image, AboutAction, ColorByOwnerMask, CrewOverlay, CursorAtlas,
    DefinitionSprite, GraphicsOverlay, GraphicsSystem, GuiPoint, HudGraphics, ImageData,
    InputDispatcher, KeyCode, MainMenuAction, MainMenuItem, PlayerOverlay, ScenarioEntry,
    ScenarioKind, SkyRenderState, StartupAboutDialog, StartupMainMenu, StartupMenu,
    StartupMenuAction, ViewportInput, ViewportPointer,
};
use lc_graphics::{BitmapFont, Color, Rect, Surface, TextFont, TrueTypeFont};
use lc_gui::{ButtonTextures, Rect as GuiRect};
use lc_network::{ClientId, ParticipantKind};
use lc_platform::{AppPaths, PathsError};
use lc_resources::{
    load_endeavour_font, scenario as resource_scenario, DefCore as ResourceDefCore,
    DefinitionError as ResourceDefinitionError, GraphicsImage, GraphicsResource, Group, GroupError,
    ResourceDefinition as ResourceDefinitionData,
};
use menu_controls::map_menu_control_event;
use network::{ClientSettings, HostSettings, NetworkEvent, NetworkManager, NetworkMode};
use object_menu::{ObjectMenuAction, ObjectMenuCommand, ObjectMenuSelection, ObjectMenuState};
use pixels::{Pixels, SurfaceTexture};
use png::{BitDepth, ColorType, Decoder, Encoder};
use save_browser::{SaveBrowserAction, SaveBrowserMode, SaveBrowserState, SaveEntry};
use serde::{
    de::{self, Unexpected, Visitor},
    ser::Serializer,
    Deserialize, Serialize,
};
use settings::{AudioOptions, DisplayMode, DisplayOptions};
use time::{macros::format_description, OffsetDateTime};
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{
    ElementState, Event, KeyboardInput, MouseButton, TouchPhase, VirtualKeyCode, WindowEvent,
};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::{Fullscreen, Window, WindowBuilder};

const PLAYER_OWNER: i32 = 1;
const FRAME_INTERVAL: Duration = Duration::from_micros(16_666); // ~60 FPS
const MAX_ACCUMULATED_TIME: Duration = Duration::from_millis(250); // clamp backlog to avoid runaway catch-up
const FALLBACK_SCENARIO_TITLE: &str = "Rust Sandbox";
const DEFAULT_GROUND_HEIGHT: i32 = 360;
const BACK_ENTRY_IDENTIFIER: &str = "__lc_menu_back";
const BACK_ENTRY_TITLE: &str = "← Back";
const SAVE_DIR_NAME: &str = "Savegames";
const QUICK_SAVE_FILE: &str = "quicksave.lcsave";
const SAVE_FILE_VERSION: SaveFileVersion = SaveFileVersion::new(1, 0, 0);
const RECORD_FILE_VERSION: u32 = 1;
const MOUSE_DRAG_THRESHOLD: f32 = 6.0;
const MIN_THROW_DRAG_DISTANCE: f32 = 12.0;
static APP_PATH_CACHE: Mutex<Option<std::result::Result<Arc<AppPaths>, PathsError>>> =
    Mutex::new(None);

fn sprite_map_key(definition_id: &str, graphics_name: Option<&str>) -> String {
    match graphics_name {
        Some(name) if !name.is_empty() => {
            format!("{}::{}", definition_id, name.to_ascii_lowercase())
        }
        _ => definition_id.to_string(),
    }
}

#[derive(Debug, Parser)]
#[command(name = "lc-app", about = "LegacyClonk Rust runtime", version)]
struct Cli {
    #[arg(
        long = "test-load",
        value_name = "PATH",
        help = "Test scenario loading without starting the UI"
    )]
    test_load: Option<std::path::PathBuf>,

    #[arg(
        long = "integration-test",
        value_name = "PATH",
        help = "Run full scenario integration test (load, apply, start, run frames)"
    )]
    integration_test: Option<std::path::PathBuf>,

    #[arg(
        long = "test-frames",
        value_name = "N",
        default_value_t = 60,
        help = "Number of frames to run during integration test"
    )]
    test_frames: u32,

    #[arg(long = "host", value_name = "ADDR", conflicts_with = "join")]
    host: Option<String>,

    #[arg(long = "join", value_name = "ADDR")]
    join: Option<String>,

    #[arg(long = "player-owner", value_name = "OWNER", default_value_t = PLAYER_OWNER)]
    player_owner: i32,

    #[arg(long = "player-name", value_name = "NAME", default_value = "Player")]
    player_name: String,

    #[arg(
        long = "sandbox",
        help = "Boot straight into the built-in sandbox scenario (skips the menu); useful for capturing the in-game scene"
    )]
    sandbox: bool,

    #[arg(
        long = "dump-frame",
        value_name = "PATH",
        help = "Headless: boot the sandbox, advance --test-frames frames, render one in-game frame to a PNG at PATH, and exit (no window). For visual rendering-parity checks."
    )]
    dump_frame: Option<std::path::PathBuf>,

    #[arg(
        long = "dump-menu-frame",
        value_name = "PATH",
        help = "Headless: boot to the startup main menu, render one frame to a PNG at PATH, and exit (no window). For menu rendering-parity checks."
    )]
    dump_menu_frame: Option<std::path::PathBuf>,

    #[arg(
        long = "menu-view",
        value_name = "VIEW",
        default_value = "main",
        help = "Startup view for --dump-menu-frame: main, scenarios, options, or about."
    )]
    menu_view: String,
}

/// Graphics.c4g files the startup-dialog parity renderers draw with
/// (C4StartupGraphics::Init, C4Startup.cpp:38-90 + GUI resource assets,
/// C4Gui.cpp:1087-1097).
const STARTUP_DIALOG_IMAGES: &[&str] = &[
    "StartupScenSelBG.png",
    "StartupPlrSelBG.png",
    "StartupNetworkBG.png",
    "StartupDlgPaper.png",
    "StartupTabClip.png",
    "StartupOptionIcons.png",
    "StartupScenSelIcons.png",
    "StartupScenSelTitleOv.png",
    "StartupBookScroll.png",
    "StartupNetGetRef.png",
    "LoaderWatercave1.png",
    "GUIButton.png",
    "GUIButtonDown.png",
    "GUIButtonHighlight.png",
    "GUICaption.png",
    "GUICheckbox.png",
    "GUIIcons.png",
    "GUIIcons2.png",
    "GUIScroll.png",
];

struct RuntimeConfig {
    player_owner: i32,
    player_name: String,
    network: Option<NetworkMode>,
    record_enabled: bool,
}

const SYNC_CHECK_RATE: u32 = if cfg!(debug_assertions) { 1 } else { 100 };
const SYNC_CHECK_HISTORY: i32 = 50;

const DEFAULT_LOADING_MESSAGE: &str = "Preparing scenario";

enum ScenarioLoadingEvent {
    Progress { fraction: f32, message: String },
    Finished(Result<Scenario, String>),
}

struct ScenarioLoadingState {
    scenario: FrontendScenario,
    label: String,
    progress: f32,
    message: String,
    receiver: Receiver<ScenarioLoadingEvent>,
}

impl ScenarioLoadingState {
    fn new(scenario: FrontendScenario, receiver: Receiver<ScenarioLoadingEvent>) -> Self {
        let label = scenario.title.clone();
        Self {
            label,
            progress: 0.0,
            message: DEFAULT_LOADING_MESSAGE.to_string(),
            scenario,
            receiver,
        }
    }

    fn update(&mut self, fraction: f32, message: String) {
        self.progress = fraction.clamp(0.0, 1.0);
        if !message.trim().is_empty() {
            self.message = message;
        }
    }
}

enum BootLoadingEvent {
    Finished(Option<Arc<MaterialSet>>),
}

struct BootLoadingState {
    receiver: Receiver<BootLoadingEvent>,
}

impl BootLoadingState {
    fn new(receiver: Receiver<BootLoadingEvent>) -> Self {
        Self { receiver }
    }
}

struct SyncCheckState {
    local: HashMap<i32, SyncCheckPacket>,
    remote: HashMap<i32, SyncCheckPacket>,
}

impl SyncCheckState {
    fn new() -> Self {
        Self {
            local: HashMap::new(),
            remote: HashMap::new(),
        }
    }

    fn clear(&mut self) {
        self.local.clear();
        self.remote.clear();
    }

    fn record_local(
        &mut self,
        check: SyncCheckPacket,
    ) -> Option<(SyncCheckPacket, SyncCheckPacket)> {
        let frame = check.frame;
        let remote = self.remote.remove(&frame);
        self.local.insert(frame, check.clone());
        remote.map(|remote_check| (check, remote_check))
    }

    fn record_remote(
        &mut self,
        check: SyncCheckPacket,
    ) -> Option<(SyncCheckPacket, SyncCheckPacket)> {
        let frame = check.frame;
        if let Some(local) = self.local.get(&frame).cloned() {
            Some((local, check))
        } else {
            self.remote.insert(frame, check);
            None
        }
    }

    fn prune_before(&mut self, threshold: i32) {
        self.local.retain(|&frame, _| frame >= threshold);
        self.remote.retain(|&frame, _| frame >= threshold);
    }
}

struct FrontendAssets {
    font: Arc<dyn TextFont>,
    menu_background: Option<ImageData>,
    scenario_browser_background: Option<ImageData>,
    options_background: Option<ImageData>,
    about_background: Option<ImageData>,
    /// CStdFont-faithful GUI fonts for pixel-parity startup text.
    clonk_fonts: Option<Arc<lc_frontend::ClonkFontSet>>,
    logo: Option<ImageData>,
    button_textures: Option<ButtonTextures>,
    /// GUIButtonHighlight.png — additive focus/hover overlay for GUI buttons
    /// (C4GraphicsResource.cpp:1089-1093, C4GuiButton.cpp:94-98).
    button_highlight: Option<ImageData>,
    /// Graphics.c4g images used by the startup dialog parity renderers,
    /// keyed by file name (see `STARTUP_DIALOG_IMAGES`).
    startup_dialog_images: HashMap<String, ImageData>,
    base_sprites: HashMap<String, DefinitionSprite>,
    cursor_atlas: Arc<CursorAtlas>,
    hud_graphics: Arc<HudGraphics>,
}

impl FrontendAssets {
    fn load(paths: Option<&AppPaths>) -> Self {
        let font = Self::load_font(paths);
        let clonk_fonts = Self::load_clonk_fonts(paths);
        let mut startup_dialog_images = HashMap::new();
        let mut menu_background = None;
        let mut scenario_browser_background = None;
        let mut options_background = None;
        let mut about_background = None;
        let mut logo = None;
        let mut button_textures = None;
        let mut button_highlight = None;
        let mut sprites = HashMap::new();
        let mut cursor_atlas = CursorAtlas::empty();
        let mut hud_graphics = HudGraphics::default();

        if let Some(paths) = paths {
            let graphics_path = paths.planet_dir().join("Graphics.c4g");
            match GraphicsResource::open(&graphics_path) {
                Ok(graphics) => {
                    menu_background = graphics
                        .load_image("LoaderGoldmine1.png")
                        .ok()
                        .map(Self::image_to_data);
                    scenario_browser_background = graphics
                        .load_image("StartupScenSelBG.png")
                        .ok()
                        .map(Self::image_to_data);
                    options_background = graphics
                        .load_image("StartupDlgPaper.png")
                        .ok()
                        .map(Self::image_to_data);
                    about_background = graphics
                        .load_image("LoaderWatercave1.png")
                        .ok()
                        .map(Self::image_to_data);
                    logo = graphics
                        .load_image("Logo.png")
                        .ok()
                        .map(Self::image_to_data);
                    button_textures = Self::load_button_textures(&graphics);
                    button_highlight = graphics
                        .load_image("GUIButtonHighlight.png")
                        .ok()
                        .map(Self::image_to_data);
                    if let Ok(sprite) = graphics.load_image("Crew.png") {
                        let image = Self::image_to_data(sprite);
                        sprites.insert(
                            "Walker".to_string(),
                            DefinitionSprite {
                                image,
                                actions: HashMap::new(),
                                color_mask: None,
                            },
                        );
                    }
                    for name in STARTUP_DIALOG_IMAGES {
                        match graphics.load_image(name) {
                            Ok(image) => {
                                startup_dialog_images
                                    .insert((*name).to_string(), Self::image_to_data(image));
                            }
                            Err(err) => {
                                tracing::warn!(name, error = %err, "startup dialog image missing");
                            }
                        }
                    }
                    cursor_atlas = Self::load_cursor_atlas(&graphics);
                    hud_graphics = Self::load_hud_graphics(&graphics);
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
            clonk_fonts,
            menu_background,
            scenario_browser_background,
            options_background,
            about_background,
            logo,
            button_textures,
            button_highlight,
            startup_dialog_images,
            base_sprites: sprites,
            cursor_atlas: Arc::new(cursor_atlas),
            hud_graphics: Arc::new(hud_graphics),
        }
    }

    /// Builds the CStdFont-faithful GUI fonts (FreeType + baked shadows) used
    /// for pixel-parity startup text; falls back to None on any failure.
    fn load_clonk_fonts(paths: Option<&AppPaths>) -> Option<Arc<lc_frontend::ClonkFontSet>> {
        let paths = paths?;
        let group = Group::open(paths.system_group_path()).ok()?;
        let resource = load_endeavour_font(&group).ok()?;
        match clonk_fonts::build_font_set(resource.bytes()) {
            Ok(set) => Some(Arc::new(set)),
            Err(err) => {
                tracing::warn!(error = %err, "failed to build CStdFont-faithful fonts");
                None
            }
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

    fn scenario_browser_background(&self) -> Option<ImageData> {
        self.scenario_browser_background.clone()
    }

    fn options_background(&self) -> Option<ImageData> {
        self.options_background.clone()
    }

    fn about_background(&self) -> Option<ImageData> {
        self.about_background.clone()
    }

    fn logo(&self) -> Option<ImageData> {
        self.logo.clone()
    }

    fn button_textures(&self) -> Option<ButtonTextures> {
        self.button_textures.clone()
    }

    fn cursor_atlas(&self) -> Arc<CursorAtlas> {
        Arc::clone(&self.cursor_atlas)
    }

    fn hud_graphics(&self) -> Arc<HudGraphics> {
        Arc::clone(&self.hud_graphics)
    }

    fn base_sprite_map(&self) -> &HashMap<String, DefinitionSprite> {
        &self.base_sprites
    }

    fn load_hud_graphics(graphics: &GraphicsResource) -> HudGraphics {
        let mut missing = Vec::new();
        let mut load = |name: &str| Self::load_hud_image(graphics, name, &mut missing);

        let hud = HudGraphics {
            player: load("Player.png"),
            flag: load("Flag.png"),
            crew: load("Crew.png"),
            score: load("Score.png"),
            wealth: load("Wealth.png"),
            rank: load("Rank.png"),
            captain: load("Captain.png"),
            fire: load("Fire.png"),
            menu: load("Menu.png"),
            upper_board: load("UpperBoard.png"),
            logo: load("Logo.png"),
            construction: load("Construction.png"),
            energy: load("Energy.png"),
            magic: load("Magic.png"),
            arrow: load("Arrow.png"),
            exit: load("Exit.png"),
            hand: load("Hand.png"),
            build: load("Build.png"),
            energy_bars: load("EnergyBars.png"),
            select_mark: load("SelectMark.png"),
        };

        if !missing.is_empty() {
            tracing::warn!(
                files = ?missing,
                "failed to load one or more HUD graphics from Graphics.c4g"
            );
        }

        hud
    }

    fn load_hud_image(
        graphics: &GraphicsResource,
        name: &str,
        missing: &mut Vec<String>,
    ) -> Option<ImageData> {
        match graphics.load_image(name) {
            Ok(image) => Some(Self::image_to_data(image)),
            Err(err) => {
                missing.push(format!("{name}: {err}"));
                None
            }
        }
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
        return Ok(Some(NetworkMode::Host(HostSettings {
            bind_addr,
            player_name: cli.player_name.clone(),
        })));
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

fn test_scenario_load(path: &std::path::Path, app_paths: Option<&Arc<AppPaths>>) -> Result<()> {
    use std::time::Instant;

    println!("Testing scenario load from: {}", path.display());
    println!(
        "Using InstallDefinitionResolver with app paths: {}",
        if app_paths.is_some() { "yes" } else { "no" }
    );

    let resolver = InstallDefinitionResolver::new(app_paths.cloned());
    let start = Instant::now();

    match Scenario::load_from_path_with(path, &resolver) {
        Ok(scenario) => {
            let elapsed = start.elapsed();
            println!(
                "\n✓ Successfully loaded scenario in {:.2}s",
                elapsed.as_secs_f32()
            );
            println!("  Name: {}", scenario.name().unwrap_or("<unnamed>"));
            println!(
                "  Description: {}",
                scenario.description().unwrap_or("<no description>")
            );

            let mut def_count = 0;
            scenario.visit_definition_groups(|_id, _group| {
                def_count += 1;
            });
            println!("  Definitions: {} loaded", def_count);
            println!("  Has initial objects: {}", scenario.has_initial_objects());

            Ok(())
        }
        Err(err) => {
            let elapsed = start.elapsed();
            eprintln!(
                "\n✗ Failed to load scenario after {:.2}s",
                elapsed.as_secs_f32()
            );
            eprintln!("  Error: {}", err);
            Err(anyhow::anyhow!("Scenario load failed: {}", err))
        }
    }
}

/// Headless: boot the sandbox scenario, advance `test_frames` simulation frames,
/// render one in-game frame to the renderer's CPU surface, and write it as a PNG.
/// No window/event loop, so the in-game scene can be captured for rendering-parity
/// checks without depending on window focus/compositing.
fn run_sandbox_dump(
    dump_path: &std::path::Path,
    test_frames: u32,
    app_paths: Option<&Arc<AppPaths>>,
    runtime: RuntimeConfig,
) -> Result<()> {
    use std::thread;
    use std::time::Duration;

    let mut app = GameApp::new(
        1280,
        720,
        AudioOptions::default(),
        app_paths.map(|v| &**v),
        runtime,
    )
    .context("failed to initialise app for frame dump")?;
    app.auto_start_sandbox = true;

    // Pump update() until async boot finishes and the sandbox auto-starts (Running).
    let mut booted = false;
    for _ in 0..600 {
        if matches!(app.mode, AppMode::Running) {
            booted = true;
            break;
        }
        app.update().context("update while booting sandbox")?;
        thread::sleep(Duration::from_millis(2));
    }
    if !booted {
        anyhow::bail!("sandbox did not reach running mode for frame dump");
    }

    // Advance the simulation so the scene has settled.
    for _ in 0..test_frames {
        app.update().context("update while advancing sandbox")?;
    }

    // Render one frame to the CPU surface, then encode it.
    let (w, h) = {
        let s = app.graphics.surface();
        (s.width(), s.height())
    };
    let mut frame = vec![0u8; (w as usize) * (h as usize) * 4];
    app.render(&mut frame)
        .context("failed to render dump frame")?;
    let png = encode_surface_to_png(app.graphics.surface())
        .context("failed to encode dump frame to PNG")?;
    std::fs::write(dump_path, &png)
        .with_context(|| format!("failed to write {}", dump_path.display()))?;
    println!(
        "wrote {} ({}x{}, after {} frames)",
        dump_path.display(),
        w,
        h,
        test_frames
    );
    Ok(())
}

/// Headless: boot to the startup main menu (`AppMode::Menu`), render one frame to
/// the renderer's CPU surface, and write it as a PNG. Counterpart of
/// `run_sandbox_dump` for startup-menu rendering-parity checks against the C++
/// engine's F9 screenshots.
fn run_menu_dump(
    dump_path: &std::path::Path,
    menu_view: &str,
    app_paths: Option<&Arc<AppPaths>>,
    runtime: RuntimeConfig,
) -> Result<()> {
    use std::thread;
    use std::time::Duration;

    let mut app = GameApp::new(
        1280,
        720,
        AudioOptions::default(),
        app_paths.map(|v| &**v),
        runtime,
    )
    .context("failed to initialise app for menu dump")?;

    // Pump update() until async boot finishes and the startup menu is shown.
    let mut booted = false;
    for _ in 0..600 {
        if matches!(app.mode, AppMode::Menu) {
            booted = true;
            break;
        }
        app.update().context("update while booting to menu")?;
        thread::sleep(Duration::from_millis(2));
    }
    if !booted {
        anyhow::bail!("app did not reach the startup menu for menu dump");
    }

    // Switch to the requested startup view through the same activation path
    // the UI uses, so per-view state objects exist.
    let item = match menu_view {
        "main" => None,
        "scenarios" => Some(MainMenuItem::LocalGame),
        "options" => Some(MainMenuItem::Options),
        "about" => Some(MainMenuItem::About),
        "plrsel" => Some(MainMenuItem::PlayerSelection),
        "net" => Some(MainMenuItem::NetworkGame),
        other => {
            anyhow::bail!("unknown --menu-view `{other}` (main|scenarios|options|about|plrsel|net)")
        }
    };
    if let Some(item) = item {
        app.handle_main_menu_activation(item)
            .map_err(|err| anyhow::anyhow!("activating menu view `{menu_view}`: {err}"))?;
    }

    // Render one frame to the CPU surface, then encode it.
    let (w, h) = {
        let s = app.graphics.surface();
        (s.width(), s.height())
    };
    let mut frame = vec![0u8; (w as usize) * (h as usize) * 4];
    app.render(&mut frame)
        .context("failed to render menu frame")?;
    let png = encode_surface_to_png(app.graphics.surface())
        .context("failed to encode menu frame to PNG")?;
    std::fs::write(dump_path, &png)
        .with_context(|| format!("failed to write {}", dump_path.display()))?;
    println!("wrote {} ({}x{}, startup menu)", dump_path.display(), w, h);
    Ok(())
}

fn run_integration_test(
    scenario_path: &std::path::Path,
    test_frames: u32,
    app_paths: Option<&Arc<AppPaths>>,
    runtime: RuntimeConfig,
) -> Result<()> {
    use std::thread;
    use std::time::{Duration, Instant};

    println!("Running integration test: {}", scenario_path.display());
    println!("Test frames: {}", test_frames);

    let start = Instant::now();

    // Create app (reuses test infrastructure)
    let mut app = GameApp::new(
        640, // width
        480, // height
        AudioOptions::default(),
        app_paths.map(|v| &**v),
        runtime,
    )
    .context("failed to initialize app for integration test")?;

    // Create FrontendScenario from path
    let title = scenario_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Test Scenario")
        .to_string();

    let scenario = FrontendScenario {
        identifier: title.clone(),
        title,
        description: None,
        kind: ScenarioKind::Scenario,
        is_editable: false,
        is_playable: true,
        path: Some(scenario_path.to_path_buf()),
        root_label: None,
        preview: None,
        children: Vec::new(),
        folder_index: None,
        icon_index: None,
        difficulty: None,
    };

    println!("Starting scenario: {}", scenario.title);

    // Start scenario (begins async loading)
    app.start_scenario(scenario)
        .context("failed to start scenario")?;

    // Wait for running state (reuses test helper pattern)
    let mut waited_frames = 0;
    for _ in 0..480 {
        if matches!(app.mode, AppMode::Running) {
            println!(
                "Scenario reached Running state after {} update cycles",
                waited_frames
            );
            break;
        }
        app.update()
            .context("failed to update app while waiting for Running state")?;
        waited_frames += 1;
        thread::sleep(Duration::from_millis(2));
    }

    if !matches!(app.mode, AppMode::Running) {
        anyhow::bail!("Scenario did not enter Running mode after 480 update cycles");
    }

    // Run test frames
    println!("Running {} test frames...", test_frames);
    for frame in 0..test_frames {
        app.update()
            .with_context(|| format!("failed to update app at frame {}", frame))?;
    }

    let elapsed = start.elapsed();
    println!(
        "\n✓ Integration test PASSED in {:.2}s",
        elapsed.as_secs_f32()
    );
    println!("  Scenario started successfully");
    println!("  Ran {} frames without errors", test_frames);

    Ok(())
}

fn main() -> Result<()> {
    lc_core::logging::init();

    let cli = Cli::parse();
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
    // Handle test-load mode: load scenario and exit without starting UI
    if let Some(test_path) = &cli.test_load {
        return test_scenario_load(test_path, app_paths.as_ref());
    }

    let runtime = RuntimeConfig {
        player_owner: cli.player_owner,
        player_name: cli.player_name.clone(),
        network: resolve_network_mode(&cli)?,
        record_enabled: load_recording_flag(app_paths.as_deref()),
    };

    // Handle integration-test mode: full scenario lifecycle test
    if let Some(test_path) = &cli.integration_test {
        return run_integration_test(test_path, cli.test_frames, app_paths.as_ref(), runtime);
    }

    // Handle headless frame dump: render one in-game sandbox frame to PNG and exit.
    if let Some(dump_path) = &cli.dump_frame {
        return run_sandbox_dump(dump_path, cli.test_frames, app_paths.as_ref(), runtime);
    }

    // Handle headless menu dump: render one startup-menu frame to PNG and exit.
    if let Some(dump_path) = &cli.dump_menu_frame {
        return run_menu_dump(dump_path, &cli.menu_view, app_paths.as_ref(), runtime);
    }

    let event_loop = EventLoop::new();
    let mut display_options = DisplayOptions::load(app_paths.as_deref());
    let audio_options = AudioOptions::load(app_paths.as_deref());
    let (initial_width, initial_height) = display_options.actual_size();
    let mut window_builder = WindowBuilder::new().with_title("Clonk Rust");
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
    app.auto_start_sandbox = cli.sandbox;

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
    if app.take_exit_request() {
        control_flow.set_exit();
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

    fn play_ui_sound(&mut self, name: &str) {
        if !self.options.sound_enabled || !self.options.menu_sound_enabled {
            return;
        }
        let handle = match self.ensure_sound(name) {
            Ok(Some(handle)) => handle,
            Ok(None) => return,
            Err(err) => {
                tracing::error!(sound = %name, error = %err, "failed to load ui sound");
                return;
            }
        };
        match self.system.play_sound(&handle, false) {
            Ok(channel) => {
                self.system
                    .channel_set_volume_and_pan(channel, self.options.sound_volume, 0.0);
            }
            Err(err) => {
                tracing::error!(sound = %name, error = %err, "failed to play ui sound");
            }
        }
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
        if self.scenario_root.as_deref() == new_root.as_deref() {
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
        let libs = collect_sound_libraries_from_group(group, label);
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
            let mut roots = vec![
                paths.install_root().to_path_buf(),
                paths.planet_dir().to_path_buf(),
                paths.user_data_dir().to_path_buf(),
            ];
            if let Some(content) = paths.content_dir() {
                roots.push(content.to_path_buf());
            }
            roots.sort();
            roots.dedup();
            for root in roots {
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

#[derive(Clone)]
struct MessageTextSpan {
    text: String,
    color: Color,
}

#[derive(Clone)]
struct MessageWordSegment {
    text: String,
    color: Color,
    width: f32,
}

#[derive(Clone)]
struct MessageLineLayout {
    segments: Vec<MessageWordSegment>,
    width: f32,
}

#[derive(Clone)]
enum MessageWordUnit {
    Segment(MessageWordSegment),
    ForcedBreak,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HorizontalAlignment {
    Left,
    Center,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VerticalAlignment {
    Top,
    Center,
    Bottom,
    Baseline,
}

fn parse_message_spans(line: &str, base_color: Color) -> Vec<MessageTextSpan> {
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut color_stack = vec![base_color];
    let mut pos = 0usize;
    let line_len = line.len();

    while pos < line_len {
        let rest = &line[pos..];
        if rest.starts_with('<') {
            let mut handled = false;
            if let Some(close) = rest.find('>') {
                let raw_tag = &rest[1..close];
                if !raw_tag.is_empty() {
                    if raw_tag.starts_with('/') {
                        let name = raw_tag[1..].trim().to_ascii_lowercase();
                        if !current.is_empty() {
                            let text = std::mem::take(&mut current);
                            spans.push(MessageTextSpan {
                                text,
                                color: *color_stack.last().unwrap_or(&base_color),
                            });
                        }
                        match name.as_str() {
                            "c" => {
                                if color_stack.len() > 1 {
                                    color_stack.pop();
                                }
                                handled = true;
                            }
                            "i" => {
                                handled = true;
                            }
                            _ => {
                                // treat as literal
                            }
                        }
                    } else {
                        let mut parts = raw_tag.splitn(2, ' ');
                        let name = parts.next().unwrap_or("").trim();
                        let params = parts.next().map(str::trim);
                        let name_lower = name.to_ascii_lowercase();
                        match name_lower.as_str() {
                            "c" => {
                                if let Some(param) = params {
                                    if let Some(color) = parse_markup_color(param) {
                                        if !current.is_empty() {
                                            let text = std::mem::take(&mut current);
                                            spans.push(MessageTextSpan {
                                                text,
                                                color: *color_stack.last().unwrap_or(&base_color),
                                            });
                                        }
                                        color_stack.push(color);
                                        handled = true;
                                    }
                                }
                            }
                            "i" => {
                                if !current.is_empty() {
                                    let text = std::mem::take(&mut current);
                                    spans.push(MessageTextSpan {
                                        text,
                                        color: *color_stack.last().unwrap_or(&base_color),
                                    });
                                }
                                handled = true;
                            }
                            _ => {
                                // unknown tag: treat as literal
                            }
                        }
                    }
                }
                if handled {
                    pos += close + 1;
                    continue;
                }
            }
        }

        if rest.starts_with("{{") && rest.len() > 2 && !rest[2..].starts_with('{') {
            if let Some(end) = rest[2..].find("}}") {
                current.push(' ');
                pos += 2 + end + 2;
                continue;
            }
        }

        if let Some(ch) = rest.chars().next() {
            current.push(ch);
            pos += ch.len_utf8();
        } else {
            break;
        }
    }

    if !current.is_empty() {
        spans.push(MessageTextSpan {
            text: current,
            color: *color_stack.last().unwrap_or(&base_color),
        });
    }

    spans
}

fn parse_markup_color(param: &str) -> Option<Color> {
    let token = param.trim();
    if token.is_empty() || token.len() > 8 || !token.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    let mut value = u32::from_str_radix(token, 16).ok()?;
    if token.len() <= 6 {
        value |= 0xff00_0000;
    }
    let inverted = (value & 0x00ff_ffff) | ((255 - ((value >> 24) & 0xff)) << 24);
    Some(Color::new(
        ((inverted >> 16) & 0xff) as u8,
        ((inverted >> 8) & 0xff) as u8,
        (inverted & 0xff) as u8,
        ((inverted >> 24) & 0xff) as u8,
    ))
}

fn split_span_into_segments(
    span: MessageTextSpan,
    font: &dyn TextFont,
    font_size: f32,
) -> Vec<MessageWordSegment> {
    if span.text.is_empty() {
        return Vec::new();
    }
    let mut segments = Vec::new();
    let mut current = String::new();
    for ch in span.text.chars() {
        current.push(ch);
        if ch.is_whitespace() {
            let width = font.measure_text(&current, font_size).width;
            segments.push(MessageWordSegment {
                text: std::mem::take(&mut current),
                color: span.color,
                width,
            });
        }
    }
    if !current.is_empty() {
        let width = font.measure_text(&current, font_size).width;
        segments.push(MessageWordSegment {
            text: current,
            color: span.color,
            width,
        });
    }
    segments
}

fn split_segment_to_fit(
    segment: MessageWordSegment,
    max_width: f32,
    font: &dyn TextFont,
    font_size: f32,
) -> Vec<MessageWordSegment> {
    if max_width <= 0.0 || segment.width <= max_width {
        return vec![segment];
    }

    let mut pieces = Vec::new();
    let mut current = String::new();

    for ch in segment.text.chars() {
        current.push(ch);
        let width = font.measure_text(&current, font_size).width;
        if width > max_width && current.len() > ch.len_utf8() {
            current.pop();
            let chunk = std::mem::take(&mut current);
            if !chunk.is_empty() {
                let chunk_width = font.measure_text(&chunk, font_size).width;
                pieces.push(MessageWordSegment {
                    text: chunk,
                    color: segment.color,
                    width: chunk_width,
                });
            }
            current.push(ch);
        }
    }

    if !current.is_empty() {
        let width = font.measure_text(&current, font_size).width;
        pieces.push(MessageWordSegment {
            text: current,
            color: segment.color,
            width,
        });
    }

    if pieces.is_empty() {
        pieces.push(segment);
    }

    pieces
}

fn wrap_word_units(
    units: Vec<MessageWordUnit>,
    max_width: Option<f32>,
    font: &dyn TextFont,
    font_size: f32,
) -> Vec<MessageLineLayout> {
    let mut lines = Vec::new();
    let mut current_segments: Vec<MessageWordSegment> = Vec::new();
    let mut current_width = 0.0f32;

    let push_line = |lines: &mut Vec<MessageLineLayout>,
                     segments: &mut Vec<MessageWordSegment>,
                     width: &mut f32| {
        lines.push(MessageLineLayout {
            width: *width,
            segments: std::mem::take(segments),
        });
        *width = 0.0;
    };

    let last_is_break = matches!(units.last(), Some(MessageWordUnit::ForcedBreak));

    for unit in units.into_iter() {
        match unit {
            MessageWordUnit::ForcedBreak => {
                push_line(&mut lines, &mut current_segments, &mut current_width);
            }
            MessageWordUnit::Segment(segment) => {
                if let Some(limit) = max_width {
                    let limit = if limit < 0.0 { 0.0 } else { limit };
                    let parts = split_segment_to_fit(segment, limit, font, font_size);
                    for piece in parts {
                        let piece_width = piece.width;
                        if limit > 0.0
                            && current_width + piece_width > limit
                            && !current_segments.is_empty()
                        {
                            push_line(&mut lines, &mut current_segments, &mut current_width);
                        }
                        if piece.text.trim().is_empty() && current_segments.is_empty() {
                            continue;
                        }
                        current_width += piece_width;
                        current_segments.push(piece);
                    }
                } else {
                    if segment.text.trim().is_empty() && current_segments.is_empty() {
                        continue;
                    }
                    current_width += segment.width;
                    current_segments.push(segment);
                }
            }
        }
    }

    if !current_segments.is_empty() || last_is_break {
        push_line(&mut lines, &mut current_segments, &mut current_width);
    }

    lines
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
    main_menu_state: MainMenuState,
    control_options: Option<ControlOptionsState>,
    about_dialog: Option<AboutDialogState>,
    startup_view: StartupView,
    object_menu: Option<ObjectMenuState>,
    ingame_menu: Option<IngameMenuState>,
    save_browser: Option<SaveBrowserState>,
    save_browser_return_to_menu: bool,
    mode: AppMode,
    scenario_catalog: HashMap<String, FrontendScenario>,
    active_scenario: Option<FrontendScenario>,
    audio: Option<AudioContext>,
    assets: Arc<FrontendAssets>,
    material_library: Option<Arc<MaterialSet>>,
    network: Option<NetworkManager>,
    network_mode: Option<NetworkMode>,
    network_lobby: Option<NetworkLobbyState>,
    sync_checks: SyncCheckState,
    recording_enabled: bool,
    recordings_dir: Option<PathBuf>,
    recording: Option<RecordingSession>,
    local_owner: i32,
    player_name: String,
    last_save_path: Option<PathBuf>,
    object_sprites: HashMap<String, DefinitionSprite>,
    sprite_cache: Arc<HashMap<String, DefinitionSprite>>,
    loading_state: Option<ScenarioLoadingState>,
    boot_loading: Option<BootLoadingState>,
    /// When set, boot straight into the sandbox scenario once boot loading
    /// finishes (the `--sandbox` flag), instead of showing the menu. Cleared
    /// after the first auto-start so returning to the menu behaves normally.
    auto_start_sandbox: bool,
    ingame_pointer: Option<ViewportPointer>,
    mouse_state: Option<IngameMouseState>,
    exit_requested: bool,
    game_over_dialog: Option<GameOverState>,
    game_over_handled: bool,
}

struct RecordingSession {
    recorder: Recorder,
    scenario_title: String,
    scenario_identifier: String,
    scenario_path: Option<PathBuf>,
    started_at: SystemTime,
}

impl RecordingSession {
    fn new(
        recorder: Recorder,
        scenario_title: String,
        scenario_identifier: String,
        scenario_path: Option<PathBuf>,
    ) -> Self {
        Self {
            recorder,
            scenario_title,
            scenario_identifier,
            scenario_path,
            started_at: SystemTime::now(),
        }
    }

    fn sanitized_base_name(&self) -> String {
        let raw = self
            .scenario_path
            .as_ref()
            .and_then(|path| path.file_stem().and_then(|stem| stem.to_str()))
            .unwrap_or(self.scenario_identifier.as_str());
        sanitize_record_name(raw)
    }
}

#[derive(Serialize)]
struct ScenarioRecordingFile {
    version: u32,
    scenario_title: String,
    scenario_identifier: String,
    scenario_path: Option<String>,
    started_at_unix_millis: u128,
    frame_count: u64,
    frames: Vec<SimulationSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AppMode {
    Menu,
    Loading,
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

struct MainMenuState {
    menu: StartupMainMenu,
    participants_label: String,
}

struct AboutDialogState {
    dialog: lc_frontend::StartupAboutDialog,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StartupView {
    MainMenu,
    ScenarioBrowser,
    NetworkLobby,
    /// C4StartupNetDlg — the network game browser ("Start Network Game").
    NetworkGame,
    Options,
    About,
    /// C4StartupPlrSelDlg — the player selection dialog.
    PlayerSelection,
}

#[derive(Clone, Copy, Debug)]
struct IngameMouseState {
    start: ViewportPointer,
    last: ViewportPointer,
    moved: bool,
}

#[derive(Clone, Debug)]
struct LobbyParticipantState {
    name: String,
    ready: bool,
    #[allow(dead_code)]
    kind: ParticipantKind,
}

impl LobbyParticipantState {
    fn new(name: impl Into<String>, kind: ParticipantKind) -> Self {
        let ready = matches!(kind, ParticipantKind::Observer);
        Self {
            name: name.into(),
            ready,
            kind,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LobbyPointerRegion {
    Menu,
    Panel,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LobbyButton {
    Ready,
    Start,
}

#[derive(Clone, Debug)]
struct NetworkLobbyLayout {
    panel: GuiRect,
    ready_button: GuiRect,
    start_button: Option<GuiRect>,
    menu_region_max_x: f32,
    scenario_rect: GuiRect,
    participants_rect: GuiRect,
}

#[derive(Clone, Debug)]
struct NetworkLobbyState {
    participants: BTreeMap<ClientId, LobbyParticipantState>,
    local_client_id: ClientId,
    is_host: bool,
    selected_identifier: Option<String>,
    selected_title: Option<String>,
    hover_button: Option<LobbyButton>,
    pressed_button: Option<LobbyButton>,
    layout: Option<NetworkLobbyLayout>,
    pointer: Option<GuiPoint>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LobbyAction {
    ToggleReady,
    StartGame,
}

impl NetworkLobbyState {
    fn new(local_client_id: ClientId, local_name: String, is_host: bool) -> Self {
        let mut participants = BTreeMap::new();
        participants.insert(
            local_client_id,
            LobbyParticipantState::new(local_name, ParticipantKind::Player),
        );
        if !is_host && local_client_id != 0 {
            participants
                .entry(0)
                .or_insert_with(|| LobbyParticipantState::new("Host", ParticipantKind::Player));
        }
        Self {
            participants,
            local_client_id,
            is_host,
            selected_identifier: None,
            selected_title: None,
            hover_button: None,
            pressed_button: None,
            layout: None,
            pointer: None,
        }
    }

    fn scenario_label(&self) -> String {
        self.selected_title
            .clone()
            .unwrap_or_else(|| "Select a scenario from the list".to_string())
    }

    fn selected_identifier(&self) -> Option<&str> {
        self.selected_identifier.as_deref()
    }

    fn select_scenario(&mut self, identifier: &str, title: &str) {
        self.selected_identifier = Some(identifier.to_string());
        self.selected_title = Some(title.to_string());
    }

    fn update_layout(&mut self, width: f32, height: f32) -> &NetworkLobbyLayout {
        let panel_margin = 24.0;
        let min_menu_width = 240.0;
        let mut panel_width = (width * 0.4).clamp(240.0, 420.0);
        if width - panel_width - panel_margin < min_menu_width {
            panel_width = (width - min_menu_width - panel_margin).max(220.0);
        }
        panel_width = panel_width.clamp(220.0, width - panel_margin * 2.0);
        let mut panel_left = width - panel_width - panel_margin;
        if panel_left < panel_margin {
            panel_left = panel_margin;
        }
        let panel_height = (height - panel_margin * 2.0).max(220.0);
        let panel_rect = GuiRect::new(panel_left, panel_margin, panel_width, panel_height);
        let menu_region_max_x = (panel_left - 12.0).max(0.0);

        let scenario_rect = GuiRect::new(
            panel_left + 18.0,
            panel_rect.origin.y + 60.0,
            panel_width - 36.0,
            46.0,
        );

        let button_height = 46.0;
        let button_y = panel_rect.origin.y + panel_rect.size.height - button_height - 24.0;
        let ready_button;
        let start_button;
        if self.is_host {
            let total_width = panel_width - 48.0;
            let button_width = (total_width - 12.0) * 0.5;
            ready_button = GuiRect::new(panel_left + 24.0, button_y, button_width, button_height);
            start_button = Some(GuiRect::new(
                ready_button.origin.x + button_width + 12.0,
                button_y,
                button_width,
                button_height,
            ));
        } else {
            ready_button = GuiRect::new(
                panel_left + 24.0,
                button_y,
                panel_width - 48.0,
                button_height,
            );
            start_button = None;
        }

        let participants_top = scenario_rect.origin.y + scenario_rect.size.height + 18.0;
        let participants_height = (button_y - participants_top - 16.0).max(80.0);
        let participants_rect = GuiRect::new(
            panel_left + 18.0,
            participants_top,
            panel_width - 36.0,
            participants_height,
        );

        self.layout = Some(NetworkLobbyLayout {
            panel: panel_rect,
            ready_button,
            start_button,
            menu_region_max_x,
            scenario_rect,
            participants_rect,
        });
        self.layout.as_ref().expect("layout just initialised")
    }

    fn layout(&mut self, width: f32, height: f32) -> &NetworkLobbyLayout {
        if self.layout.is_none() {
            self.update_layout(width, height);
        }
        self.layout
            .as_ref()
            .expect("network lobby layout should exist")
    }

    fn pointer_region(&self, point: GuiPoint) -> LobbyPointerRegion {
        if let Some(layout) = self.layout.as_ref() {
            if point.x <= layout.menu_region_max_x {
                LobbyPointerRegion::Menu
            } else {
                LobbyPointerRegion::Panel
            }
        } else {
            LobbyPointerRegion::Menu
        }
    }

    fn handle_panel_pointer_move(&mut self, point: GuiPoint) {
        self.pointer = Some(point);
        self.hover_button = self.hit_test_button(point);
    }

    fn handle_panel_pointer_down(&mut self, point: GuiPoint) {
        self.pressed_button = self.hit_test_button(point);
    }

    fn handle_panel_pointer_up(&mut self, point: GuiPoint) -> Option<LobbyAction> {
        let pressed = self.pressed_button.take();
        let hit = self.hit_test_button(point);
        if pressed.is_some() && hit == pressed {
            match hit {
                Some(LobbyButton::Ready) => Some(LobbyAction::ToggleReady),
                Some(LobbyButton::Start) => Some(LobbyAction::StartGame),
                None => None,
            }
        } else {
            None
        }
    }

    fn pointer_left(&mut self) {
        self.hover_button = None;
        self.pressed_button = None;
        self.pointer = None;
    }

    fn register_peer(&mut self, client_id: ClientId, name: String, kind: ParticipantKind) {
        let mut ready = self
            .participants
            .get(&client_id)
            .map(|participant| participant.ready)
            .unwrap_or(matches!(kind, ParticipantKind::Observer));
        if matches!(kind, ParticipantKind::Observer) {
            ready = true;
        }
        self.participants
            .insert(client_id, LobbyParticipantState { name, ready, kind });
    }

    fn unregister_peer(&mut self, client_id: ClientId) {
        if client_id == self.local_client_id {
            return;
        }
        self.participants.remove(&client_id);
        if !self.is_host && client_id == 0 {
            self.participants
                .entry(0)
                .or_insert_with(|| LobbyParticipantState::new("Host", ParticipantKind::Player));
        }
    }

    fn toggle_local_ready(&mut self) -> bool {
        if let Some(participant) = self.participants.get_mut(&self.local_client_id) {
            participant.ready = !participant.ready;
            participant.ready
        } else {
            false
        }
    }

    fn local_ready(&self) -> bool {
        self.participants
            .get(&self.local_client_id)
            .map(|participant| participant.ready)
            .unwrap_or(false)
    }

    fn render_overlay(&mut self, surface: &mut Surface, assets: &FrontendAssets) {
        let width = surface.width() as f32;
        let height = surface.height() as f32;
        let layout = self.layout(width, height).clone();

        fill_gui_rect(surface, &layout.panel, Color::new(16, 28, 52, 232));
        draw_panel_outline(surface, &layout.panel, Color::new(28, 44, 72, 255));

        let font = assets.font_arc();
        let font_ref = font.as_ref();

        font_ref.draw_text(
            surface,
            layout.panel.origin.x + 20.0,
            layout.panel.origin.y + 32.0,
            "Network Lobby",
            28.0,
            Color::opaque(224, 232, 248),
        );

        let scenario_text = self
            .selected_title
            .as_deref()
            .unwrap_or("Select a scenario from the list");
        font_ref.draw_text(
            surface,
            layout.scenario_rect.origin.x,
            layout.scenario_rect.origin.y,
            scenario_text,
            20.0,
            Color::opaque(196, 208, 228),
        );

        let participants_title = "Participants";
        font_ref.draw_text(
            surface,
            layout.participants_rect.origin.x,
            layout.participants_rect.origin.y - 6.0,
            participants_title,
            22.0,
            Color::opaque(204, 214, 230),
        );

        let mut row_y = layout.participants_rect.origin.y + 20.0;
        let row_spacing = 8.0;
        let row_height = 26.0;
        let name_color = Color::opaque(220, 230, 248);
        let local_name_color = Color::opaque(236, 224, 180);
        let ready_color = Color::opaque(136, 220, 156);
        let waiting_color = Color::opaque(236, 148, 132);

        for (client_id, participant) in &self.participants {
            if row_y + row_height
                > layout.participants_rect.origin.y + layout.participants_rect.size.height
            {
                break;
            }

            let background = GuiRect::new(
                layout.participants_rect.origin.x,
                row_y - 18.0,
                layout.participants_rect.size.width,
                row_height + 8.0,
            );
            fill_gui_rect(surface, &background, Color::new(22, 36, 60, 180));

            let label_color = if *client_id == self.local_client_id {
                local_name_color
            } else {
                name_color
            };
            font_ref.draw_text(
                surface,
                layout.participants_rect.origin.x + 8.0,
                row_y,
                &participant.name,
                20.0,
                label_color,
            );

            let status_text = if participant.ready {
                "Ready"
            } else {
                "Waiting"
            };
            let status_color = if participant.ready {
                ready_color
            } else {
                waiting_color
            };

            let status_width = font_ref.measure_text(status_text, 18.0).width;
            let status_x = layout.participants_rect.origin.x + layout.participants_rect.size.width
                - status_width
                - 8.0;

            font_ref.draw_text(surface, status_x, row_y, status_text, 18.0, status_color);

            row_y += row_height + row_spacing;
        }

        self.draw_ready_button(surface, font_ref, &layout);
        if let Some(start) = layout.start_button {
            self.draw_start_button(surface, font_ref, start);
        }
    }

    fn handle_key(&mut self, key: KeyCode, state: ElementState) -> Option<LobbyAction> {
        if state != ElementState::Pressed {
            return None;
        }
        match key {
            KeyCode::Enter if self.is_host => Some(LobbyAction::StartGame),
            KeyCode::Space | KeyCode::Enter => Some(LobbyAction::ToggleReady),
            _ => None,
        }
    }

    fn pointer_position(&self) -> Option<GuiPoint> {
        self.pointer
    }

    fn draw_ready_button(
        &self,
        surface: &mut Surface,
        font: &dyn TextFont,
        layout: &NetworkLobbyLayout,
    ) {
        let base = if self.local_ready() {
            Color::new(28, 76, 58, 236)
        } else {
            Color::new(52, 72, 108, 226)
        };
        let mut color = base;
        if self.hover_button == Some(LobbyButton::Ready) {
            color = offset_color(color, 18);
        }
        if self.pressed_button == Some(LobbyButton::Ready) {
            color = offset_color(color, 32);
        }
        fill_gui_rect(surface, &layout.ready_button, color);
        draw_panel_outline(surface, &layout.ready_button, Color::new(16, 24, 40, 255));

        let label = if self.local_ready() {
            "Unready"
        } else {
            "Ready"
        };
        let size = 20.0;
        let metrics = font.measure_text(label, size);
        let text_x =
            layout.ready_button.origin.x + (layout.ready_button.size.width - metrics.width) * 0.5;
        let text_y = layout.ready_button.origin.y + 12.0;
        let text_color = Color::opaque(236, 240, 248);
        font.draw_text(surface, text_x, text_y, label, size, text_color);
    }

    fn draw_start_button(&self, surface: &mut Surface, font: &dyn TextFont, rect: GuiRect) {
        let enabled = self.selected_identifier.is_some();
        let mut color = if enabled {
            Color::new(32, 96, 72, 236)
        } else {
            Color::new(40, 48, 68, 200)
        };
        if enabled && self.hover_button == Some(LobbyButton::Start) {
            color = offset_color(color, 22);
        }
        if enabled && self.pressed_button == Some(LobbyButton::Start) {
            color = offset_color(color, 36);
        }
        fill_gui_rect(surface, &rect, color);
        draw_panel_outline(surface, &rect, Color::new(16, 24, 40, 255));

        let label = if enabled { "Start" } else { "Select Scenario" };
        let size = 20.0;
        let metrics = font.measure_text(label, size);
        let text_x = rect.origin.x + (rect.size.width - metrics.width) * 0.5;
        let text_y = rect.origin.y + 12.0;
        let text_color = if enabled {
            Color::opaque(230, 244, 236)
        } else {
            Color::opaque(200, 204, 214)
        };
        font.draw_text(surface, text_x, text_y, label, size, text_color);
    }

    fn hit_test_button(&self, point: GuiPoint) -> Option<LobbyButton> {
        let layout = self.layout.as_ref()?;
        if point_in_rect(point, &layout.ready_button) {
            return Some(LobbyButton::Ready);
        }
        if let Some(rect) = layout.start_button.as_ref() {
            if point_in_rect(point, rect) {
                return Some(LobbyButton::Start);
            }
        }
        None
    }
}

fn point_in_rect(point: GuiPoint, rect: &GuiRect) -> bool {
    point.x >= rect.origin.x
        && point.x <= rect.origin.x + rect.size.width
        && point.y >= rect.origin.y
        && point.y <= rect.origin.y + rect.size.height
}

fn fill_gui_rect(surface: &mut Surface, rect: &GuiRect, color: Color) {
    let x0 = rect.origin.x.floor().clamp(0.0, surface.width() as f32) as i32;
    let y0 = rect.origin.y.floor().clamp(0.0, surface.height() as f32) as i32;
    let x1 = (rect.origin.x + rect.size.width)
        .ceil()
        .clamp(0.0, surface.width() as f32) as i32;
    let y1 = (rect.origin.y + rect.size.height)
        .ceil()
        .clamp(0.0, surface.height() as f32) as i32;

    for y in y0..y1 {
        for x in x0..x1 {
            let _ = surface.set_pixel(x as u32, y as u32, color);
        }
    }
}

fn draw_panel_outline(surface: &mut Surface, rect: &GuiRect, color: Color) {
    let top = GuiRect::new(rect.origin.x, rect.origin.y, rect.size.width, 2.0);
    let bottom = GuiRect::new(
        rect.origin.x,
        rect.origin.y + rect.size.height - 2.0,
        rect.size.width,
        2.0,
    );
    let left = GuiRect::new(rect.origin.x, rect.origin.y, 2.0, rect.size.height);
    let right = GuiRect::new(
        rect.origin.x + rect.size.width - 2.0,
        rect.origin.y,
        2.0,
        rect.size.height,
    );
    fill_gui_rect(surface, &top, color);
    fill_gui_rect(surface, &bottom, color);
    fill_gui_rect(surface, &left, color);
    fill_gui_rect(surface, &right, color);
}

fn offset_color(color: Color, delta: i16) -> Color {
    let adjust = |channel: u8| -> u8 { (channel as i16 + delta).clamp(0, 255) as u8 };
    Color::new(adjust(color.r), adjust(color.g), adjust(color.b), color.a)
}

impl IngameMouseState {
    fn new(start: ViewportPointer) -> Self {
        Self {
            start,
            last: start,
            moved: false,
        }
    }

    fn update(&mut self, pointer: ViewportPointer) {
        self.last = pointer;
        if !self.moved {
            let dx = (self.last.world.x - self.start.world.x).abs();
            let dy = (self.last.world.y - self.start.world.y).abs();
            if dx >= MOUSE_DRAG_THRESHOLD || dy >= MOUSE_DRAG_THRESHOLD {
                self.moved = true;
            }
        }
    }

    fn delta(&self) -> FloatVector2 {
        FloatVector2::new(
            self.last.world.x - self.start.world.x,
            self.last.world.y - self.start.world.y,
        )
    }
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
        let include_back = true;
        let entries = build_menu_entries(self.current_entries(), include_back);
        if let Err(err) = self.menu.set_entries(entries) {
            tracing::error!(error = %err, "failed to update startup menu entries");
        }
    }

    fn label_path(&self) -> String {
        if self.stack.is_empty() {
            return String::new();
        }
        self.stack
            .iter()
            .map(|layer| layer.title.clone())
            .collect::<Vec<_>>()
            .join(" / ")
    }

    fn select_default_entry(&mut self) -> Vec<StartupMenuAction> {
        if self.current_entries().is_empty() {
            return Vec::new();
        }
        let target_index = 1;
        match self.menu.select_entry_by_index(target_index) {
            Ok(actions) => actions,
            Err(err) => {
                tracing::error!(error = %err, "failed to select default scenario entry");
                Vec::new()
            }
        }
    }
}

impl MainMenuState {
    fn new(menu: StartupMainMenu, participants_label: String) -> Self {
        Self {
            menu,
            participants_label,
        }
    }

    fn pointer_position(&self) -> Option<GuiPoint> {
        self.menu.pointer_position()
    }

    fn set_pointer_position(&mut self, position: Option<GuiPoint>) {
        self.menu.set_pointer_position(position);
    }

    fn handle_pointer_move(&mut self, point: GuiPoint) -> Vec<MainMenuAction> {
        self.menu.handle_pointer_move(point)
    }

    fn handle_pointer_down(&mut self, point: GuiPoint) -> Vec<MainMenuAction> {
        self.menu.handle_pointer_down(point)
    }

    fn handle_pointer_up(&mut self, point: GuiPoint) -> Vec<MainMenuAction> {
        self.menu.handle_pointer_up(point)
    }

    fn handle_key_down(&mut self, key: KeyCode) -> Vec<MainMenuAction> {
        self.menu.handle_key_down(key)
    }

    fn handle_key_up(&mut self, key: KeyCode) -> Vec<MainMenuAction> {
        self.menu.handle_key_up(key)
    }

    fn pointer_left(&mut self) {
        self.menu.pointer_left();
    }

    fn resize(&mut self, width: f32, height: f32) {
        self.menu.resize(width, height);
    }

    fn render(&mut self, surface: &mut Surface) {
        self.menu.render(surface, &self.participants_label);
    }

    fn update_participants_label(&mut self, label: String) {
        self.participants_label = label;
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
    root_label: Option<String>,
    preview: Option<ImageData>,
    children: Vec<FrontendScenario>,
    folder_index: Option<i32>,
    icon_index: Option<i32>,
    difficulty: Option<i32>,
}

impl FrontendScenario {
    fn to_ui_entry(&self) -> ScenarioEntry {
        let preview = self
            .preview
            .clone()
            .or_else(|| Some(generate_preview_placeholder(self.kind, &self.title)));
        ScenarioEntry {
            identifier: self.identifier.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            kind: self.kind,
            is_editable: self.is_editable,
            is_playable: self.is_playable,
            location: self.location_label(),
            preview,
        }
    }

    fn from_resource(entry: resource_scenario::ScenarioEntry, root_label: &str) -> Self {
        let resource_scenario::ScenarioEntry {
            identifier,
            path,
            title,
            description,
            kind,
            is_editable,
            is_playable,
            preview,
            children,
            folder_index,
            icon_index,
            difficulty,
        } = entry;

        let kind = match kind {
            resource_scenario::ScenarioEntryKind::Scenario => ScenarioKind::Scenario,
            resource_scenario::ScenarioEntryKind::Folder => ScenarioKind::Folder,
            resource_scenario::ScenarioEntryKind::Editor => ScenarioKind::Editor,
        };

        let children = children
            .into_iter()
            .map(|child| FrontendScenario::from_resource(child, root_label))
            .collect();

        let preview = preview.map(|preview| {
            let (width, height, pixels) = preview.into_arc();
            ImageData::from_arc(width, height, pixels)
        });

        Self {
            identifier,
            title,
            description,
            kind,
            is_editable,
            is_playable,
            path: Some(path),
            root_label: Some(root_label.to_string()),
            preview,
            children,
            folder_index,
            icon_index,
            difficulty,
        }
    }

    fn location_label(&self) -> Option<String> {
        if let Some(path) = self.path.as_ref() {
            if let Some(root) = self.root_label.as_ref() {
                let components: Vec<&str> = self
                    .identifier
                    .split('/')
                    .filter(|component| !component.is_empty())
                    .collect();
                let relative = if components.is_empty() {
                    String::new()
                } else {
                    components.join(" / ")
                };
                if relative.is_empty() {
                    return Some(root.clone());
                }
                return Some(format!("{root} / {relative}"));
            }
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
            title: FALLBACK_SCENARIO_TITLE.to_string(),
            description: Some("Spawn a Rust-driven walker in a flat test landscape.".to_string()),
            kind: ScenarioKind::Scenario,
            is_editable: true,
            is_playable: true,
            path: None,
            root_label: None,
            preview: Some(generate_preview_placeholder(
                ScenarioKind::Scenario,
                FALLBACK_SCENARIO_TITLE,
            )),
            children: Vec::new(),
            folder_index: None,
            icon_index: None,
            difficulty: None,
        }
    }
}

fn merge_frontend_scenarios(entries: Vec<FrontendScenario>) -> Vec<FrontendScenario> {
    let mut result: Vec<FrontendScenario> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();

    for entry in entries {
        if let Some(&existing_idx) = index.get(&entry.identifier) {
            let existing = &mut result[existing_idx];
            if existing.kind == entry.kind {
                if is_container_kind(&existing.kind) && is_container_kind(&entry.kind) {
                    merge_container(existing, entry);
                } else {
                    merge_leaf(existing, entry);
                }
            } else {
                tracing::warn!(
                    identifier = %existing.identifier,
                    existing_kind = ?existing.kind,
                    incoming_kind = ?entry.kind,
                    "scenario catalog contained identifier with mismatched kinds; keeping existing entry"
                );
            }
            continue;
        }
        index.insert(entry.identifier.clone(), result.len());
        result.push(entry);
    }

    sort_frontend_entries(&mut result);
    result
}

fn merge_leaf(existing: &mut FrontendScenario, mut incoming: FrontendScenario) {
    merge_metadata(existing, &mut incoming);
}

fn merge_container(existing: &mut FrontendScenario, mut incoming: FrontendScenario) {
    merge_metadata(existing, &mut incoming);
    merge_children(&mut existing.children, incoming.children);
    sort_frontend_entries(&mut existing.children);
}

fn merge_metadata(existing: &mut FrontendScenario, incoming: &mut FrontendScenario) {
    if existing.description.is_none() {
        existing.description = incoming.description.take();
    }
    if existing.preview.is_none() {
        existing.preview = incoming.preview.take();
    }
    if existing.path.is_none() {
        existing.path = incoming.path.take();
    }
    if existing.root_label.is_none() {
        existing.root_label = incoming.root_label.take();
    }
    existing.is_editable |= incoming.is_editable;
    existing.is_playable |= incoming.is_playable;
    if existing.folder_index.is_none() {
        existing.folder_index = incoming.folder_index;
    }
    if existing.icon_index.is_none() {
        existing.icon_index = incoming.icon_index;
    }
    if existing.difficulty.is_none() {
        existing.difficulty = incoming.difficulty;
    }
}

fn merge_children(
    existing_children: &mut Vec<FrontendScenario>,
    incoming_children: Vec<FrontendScenario>,
) {
    if incoming_children.is_empty() {
        return;
    }

    let mut index: HashMap<String, usize> = existing_children
        .iter()
        .enumerate()
        .map(|(idx, child)| (child.identifier.clone(), idx))
        .collect();

    for child in incoming_children {
        if let Some(&existing_idx) = index.get(&child.identifier) {
            if is_container_kind(&existing_children[existing_idx].kind)
                && is_container_kind(&child.kind)
            {
                let existing_child = &mut existing_children[existing_idx];
                merge_container(existing_child, child);
            }
            continue;
        }
        index.insert(child.identifier.clone(), existing_children.len());
        existing_children.push(child);
    }
}

fn is_container_kind(kind: &ScenarioKind) -> bool {
    matches!(kind, ScenarioKind::Folder | ScenarioKind::Editor)
}

fn sort_frontend_entries(entries: &mut [FrontendScenario]) {
    entries.sort_by(compare_frontend_entries);
    for entry in entries.iter_mut() {
        sort_frontend_entries(&mut entry.children);
    }
}

fn compare_frontend_entries(a: &FrontendScenario, b: &FrontendScenario) -> Ordering {
    let a_is_folder = matches!(a.kind, ScenarioKind::Folder);
    let b_is_folder = matches!(b.kind, ScenarioKind::Folder);
    if a_is_folder != b_is_folder {
        return if a_is_folder {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }

    let a_folder_index = a.folder_index.unwrap_or(0);
    let b_folder_index = b.folder_index.unwrap_or(0);
    if a_folder_index != 0 || b_folder_index != 0 {
        if a_folder_index == 0 {
            return Ordering::Greater;
        }
        if b_folder_index == 0 {
            return Ordering::Less;
        }
        match a_folder_index.cmp(&b_folder_index) {
            Ordering::Equal => {}
            other => return other,
        }
    }

    if let Some(icon) = a.icon_index {
        if (2..=11).contains(&icon) {
            let other_icon = b.icon_index.unwrap_or(-1);
            let diff = icon - other_icon;
            if diff != 0 {
                return diff.cmp(&0);
            }
        }
    }

    let a_difficulty = a.difficulty.unwrap_or(0);
    let b_difficulty = b.difficulty.unwrap_or(0);
    if a_difficulty != 0 || b_difficulty != 0 {
        if a_difficulty == 0 {
            return Ordering::Greater;
        }
        if b_difficulty == 0 {
            return Ordering::Less;
        }
        match a_difficulty.cmp(&b_difficulty) {
            Ordering::Equal => {}
            other => return other,
        }
    }

    let title_order = compare_case_insensitive(&a.title, &b.title);
    if title_order != Ordering::Equal {
        return title_order;
    }

    compare_case_insensitive(&a.identifier, &b.identifier)
}

fn compare_case_insensitive(a: &str, b: &str) -> Ordering {
    a.to_ascii_lowercase().cmp(&b.to_ascii_lowercase())
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
    #[serde(default)]
    root_label: Option<String>,
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
            root_label: frontend.root_label.clone(),
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
            root_label: self.root_label.clone(),
            preview: Some(generate_preview_placeholder(
                ScenarioKind::Scenario,
                &self.title,
            )),
            children: Vec::new(),
            folder_index: None,
            icon_index: None,
            difficulty: None,
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
    #[serde(default)]
    user_label: Option<String>,
    engine_state: EngineState,
}

#[derive(Debug, Clone, Deserialize)]
struct SavedGameHeader {
    #[allow(dead_code)]
    version: SaveFileVersion,
    saved_at_seconds: u64,
    scenario: SavedScenarioInfo,
    #[serde(default)]
    user_label: Option<String>,
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

fn quick_save_exists() -> bool {
    existing_quick_save_path().is_some()
}

fn is_save_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("lcsave"))
        .unwrap_or(false)
}

fn any_saved_games_exist() -> bool {
    let dir = resolve_save_directory();
    match fs::read_dir(&dir) {
        Ok(entries) => entries.flatten().any(|entry| {
            let path = entry.path();
            is_save_file(&path)
        }),
        Err(_) => quick_save_exists(),
    }
}

fn load_install_material_library(paths: Option<&AppPaths>) -> Option<Arc<MaterialSet>> {
    let paths = paths?;

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
    if let Some(content) = paths.content_dir() {
        if content.exists() {
            candidates.push(content.to_path_buf());
        }
    }
    let scenario_dir = paths.scenario_dir();
    if scenario_dir.exists() {
        candidates.push(scenario_dir);
    }
    let system_group = paths.system_group_path();
    if system_group.exists() {
        candidates.push(system_group.to_path_buf());
    }

    let mut group_bases = vec![
        paths.planet_dir().to_path_buf(),
        paths.install_root().to_path_buf(),
        paths.system_group_path().to_path_buf(),
    ];
    if let Some(content) = paths.content_dir() {
        group_bases.push(content.to_path_buf());
    }

    for base in group_bases {
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

fn format_saved_timestamp(seconds: u64) -> String {
    if let Ok(datetime) = OffsetDateTime::from_unix_timestamp(seconds as i64) {
        let format = format_description!("[year]-[month]-[day] [hour]:[minute]:[second] UTC");
        match datetime.format(&format) {
            Ok(value) => value,
            Err(_) => format!("{}", seconds),
        }
    } else {
        format!("{}", seconds)
    }
}

fn sanitize_save_label(label: &str) -> String {
    let mut result = String::new();
    let mut last_was_separator = false;
    for ch in label.chars() {
        if result.len() >= 64 {
            break;
        }
        if ch.is_ascii_alphanumeric() {
            result.push(ch);
            last_was_separator = false;
        } else if (ch.is_ascii_whitespace() || matches!(ch, '-' | '_'))
            && !last_was_separator
            && !result.is_empty()
        {
            result.push('_');
            last_was_separator = true;
        }
    }
    let trimmed = result.trim_matches('_');
    if trimmed.is_empty() {
        "save".to_string()
    } else {
        trimmed.to_string()
    }
}

fn unique_save_path(dir: &Path, base: &str) -> PathBuf {
    let mut index = 0u32;
    loop {
        let candidate = if index == 0 {
            dir.join(format!("{}.lcsave", base))
        } else {
            dir.join(format!("{}_{:02}.lcsave", base, index))
        };
        if !candidate.exists() {
            return candidate;
        }
        index = index.saturating_add(1);
    }
}

fn next_recording_index(dir: &Path) -> io::Result<u32> {
    let mut max_index = 0u32;
    if dir.exists() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            if let Some(name) = entry.file_name().to_str() {
                if let Some((prefix, _)) = name.split_once('-') {
                    if prefix.chars().all(|c| c.is_ascii_digit()) {
                        if let Ok(index) = prefix.parse::<u32>() {
                            max_index = max_index.max(index);
                        }
                    }
                }
            }
        }
    }
    Ok(max_index + 1)
}

fn sanitize_record_name(raw: &str) -> String {
    let trimmed = raw.trim();
    let without_digits = trimmed.trim_end_matches(|c: char| c.is_ascii_digit());
    let candidate = if without_digits.is_empty() {
        trimmed
    } else {
        without_digits
    };
    let mut result = String::new();
    let mut last_was_separator = false;
    for ch in candidate.chars() {
        if ch.is_ascii_alphanumeric() {
            result.push(ch);
            last_was_separator = false;
        } else if ch.is_ascii_whitespace() || matches!(ch, '-' | '_') {
            if !last_was_separator && !result.is_empty() {
                result.push('_');
                last_was_separator = true;
            }
        } else if !last_was_separator && !result.is_empty() {
            result.push('_');
            last_was_separator = true;
        }
    }
    let sanitized = result.trim_matches('_');
    if sanitized.is_empty() {
        "scenario".to_string()
    } else {
        sanitized.to_string()
    }
}

fn encode_surface_to_png(surface: &Surface) -> Result<Vec<u8>> {
    let width = surface.width();
    let height = surface.height();
    let mut buffer = Vec::new();
    {
        let mut encoder = Encoder::new(&mut buffer, width, height);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .context("failed to initialise PNG encoder")?;
        writer
            .write_image_data(surface.pixels())
            .context("failed to encode PNG surface")?;
        writer.finish().context("failed to finish PNG encoding")?;
    }
    Ok(buffer)
}

fn load_save_entry(path: &Path) -> Result<SaveEntry> {
    let file =
        File::open(path).with_context(|| format!("failed to open save file {}", path.display()))?;
    let header: SavedGameHeader = serde_json::from_reader(file)
        .with_context(|| format!("failed to parse save metadata from {}", path.display()))?;
    let is_quick_save = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.eq_ignore_ascii_case(QUICK_SAVE_FILE))
        .unwrap_or(false);
    let display_name = header
        .user_label
        .clone()
        .filter(|label| !label.trim().is_empty())
        .or_else(|| {
            if is_quick_save {
                Some("Quick Save".to_string())
            } else {
                None
            }
        })
        .unwrap_or_else(|| header.scenario.title.clone());
    let saved_label = format_saved_timestamp(header.saved_at_seconds);
    let thumbnail_path = path.with_extension("png");
    let thumbnail = load_save_thumbnail(&thumbnail_path);
    Ok(SaveEntry {
        display_name,
        scenario_title: header.scenario.title.clone(),
        saved_at_seconds: header.saved_at_seconds,
        saved_label,
        path: path.to_path_buf(),
        thumbnail,
    })
}

fn load_save_thumbnail(path: &Path) -> Option<ImageData> {
    let file = File::open(path).ok()?;
    let decoder = Decoder::new(file);
    let mut reader = decoder.read_info().ok()?;
    let palette = reader.info().palette.clone();
    let transparency = reader.info().trns.clone();
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let data = &buf[..info.buffer_size()];
    let pixels = match info.color_type {
        ColorType::Rgba => data.to_vec(),
        ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(data.len() / 3 * 4);
            for chunk in data.chunks_exact(3) {
                rgba.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
            }
            rgba
        }
        ColorType::Grayscale => {
            let mut rgba = Vec::with_capacity(data.len() * 4);
            for &value in data {
                rgba.extend_from_slice(&[value, value, value, 255]);
            }
            rgba
        }
        ColorType::GrayscaleAlpha => {
            let mut rgba = Vec::with_capacity(data.len() / 2 * 4);
            for chunk in data.chunks_exact(2) {
                let value = chunk[0];
                let alpha = chunk[1];
                rgba.extend_from_slice(&[value, value, value, alpha]);
            }
            rgba
        }
        ColorType::Indexed => {
            let palette = palette?;
            let mut rgba = Vec::with_capacity(data.len() * 4);
            for &index in data {
                let position = (index as usize) * 3;
                if position + 2 >= palette.len() {
                    continue;
                }
                let r = palette[position];
                let g = palette[position + 1];
                let b = palette[position + 2];
                let a = transparency
                    .as_ref()
                    .and_then(|values| values.get(index as usize))
                    .copied()
                    .unwrap_or(255);
                rgba.extend_from_slice(&[r, g, b, a]);
            }
            rgba
        }
    };
    Some(ImageData::new(info.width, info.height, pixels))
}

fn parse_config_bool(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn load_recording_flag(paths: Option<&AppPaths>) -> bool {
    let Some(paths) = paths else {
        return false;
    };
    let config_path = paths.config_file();
    match Config::load(&config_path) {
        Ok(config) => config
            .get_in(Some("General"), "Record")
            .map(parse_config_bool)
            .unwrap_or(false),
        Err(err) => {
            if err.kind() != io::ErrorKind::NotFound {
                tracing::warn!(
                    error = %err,
                    path = %config_path.display(),
                    "failed to read record setting from config"
                );
            }
            false
        }
    }
}

fn load_participants_label(paths: Option<&AppPaths>) -> String {
    // C++ C4StartupMainDlg::UpdateParticipants (C4StartupMainDlg.cpp:174-200):
    // IDS_DESC_PLRS ("Players: ") + comma-separated player file basenames
    // without extension, or IDS_DLG_NOPLAYERSSELECTED ("none selected").
    let mut label = String::from("Players: ");
    let Some(paths) = paths else {
        label.push_str("none selected");
        return label;
    };

    let config_path = paths.config_file();
    match Config::load(&config_path) {
        Ok(config) => {
            let entries = config
                .get_in(Some("General"), "Participants")
                .map(|raw| raw.split(';').collect::<Vec<_>>())
                .unwrap_or_default();
            let mut names = Vec::new();
            for entry in entries {
                let trimmed = entry.trim().trim_matches('"');
                if trimmed.is_empty() {
                    continue;
                }
                let name = Path::new(trimmed)
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| trimmed.to_string());
                if !name.is_empty() {
                    names.push(name);
                }
            }
            if names.is_empty() {
                label.push_str("none selected");
            } else {
                label.push_str(&names.join(", "));
            }
        }
        Err(err) => {
            if err.kind() != io::ErrorKind::NotFound {
                tracing::warn!(
                    error = %err,
                    path = %config_path.display(),
                    "failed to read participants from config"
                );
            }
            label.push_str("none selected");
        }
    }

    label
}

fn overlay_text_needs_update(current: &str, default_prefix: &str) -> bool {
    current.is_empty() || current.starts_with(default_prefix)
}

impl GameApp {
    fn new(
        width: u32,
        height: u32,
        audio_options: AudioOptions,
        paths: Option<&AppPaths>,
        runtime: RuntimeConfig,
    ) -> Result<Self> {
        let network_mode = runtime.network.clone();
        let network = match network_mode.clone() {
            Some(mode) => Some(NetworkManager::for_mode(mode, runtime.player_owner)?),
            None => None,
        };
        let player_name = runtime.player_name.clone();
        let network_lobby = match (&network_mode, &network) {
            (Some(mode), Some(manager)) => Some(NetworkLobbyState::new(
                manager.local_client_id(),
                player_name.clone(),
                matches!(mode, NetworkMode::Host(_)),
            )),
            _ => None,
        };
        let assets = Arc::new(FrontendAssets::load(paths));

        // Start async material loading
        let (tx, rx) = std::sync::mpsc::channel();
        let paths_clone = paths.cloned();
        std::thread::spawn(move || {
            let material_library = load_install_material_library(paths_clone.as_ref());
            let _ = tx.send(BootLoadingEvent::Finished(material_library));
        });
        let boot_loading = Some(BootLoadingState::new(rx));

        let base_sprites = assets.base_sprite_map().clone();
        let sprite_cache = Arc::new(base_sprites.clone());

        // Engine starts with default materials; will be updated when boot loading finishes
        let engine = Engine::new();
        let snapshot = engine.snapshot();

        let scenarios = load_frontend_scenarios();
        let button_textures = assets.button_textures();
        let menu_entries = build_menu_entries(&scenarios, false);
        let mut menu = StartupMenu::new(menu_entries, assets.font_arc(), button_textures.clone())
            .map_err(|err| anyhow!("failed to create startup menu: {err}"))?;
        menu.resize(width as f32, height as f32);
        let mut main_menu = StartupMainMenu::new(assets.font_arc(), button_textures.clone());
        main_menu.set_highlight_texture(assets.button_highlight.clone());
        main_menu.set_clonk_fonts(assets.clonk_fonts.clone());
        main_menu.set_gamma_ramp(Some(Arc::new(lc_graphics::GammaRamp::standard())));
        main_menu.resize(width as f32, height as f32);
        let participants_label = load_participants_label(paths);
        let main_menu_state = MainMenuState::new(main_menu, participants_label);

        let scenario_catalog = build_scenario_catalog(&scenarios);
        let menu_state = MenuState::new(menu, scenarios);
        let scenario_label = menu_state.label_path();
        let mut graphics = GraphicsSystem::new(
            width,
            height,
            DEFAULT_GROUND_HEIGHT,
            &scenario_label,
            assets.font_arc(),
            Arc::clone(&sprite_cache),
            assets.cursor_atlas(),
            assets.hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(16, 28, 52));
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
            main_menu_state,
            control_options: None,
            about_dialog: None,
            startup_view: StartupView::MainMenu,
            object_menu: None,
            ingame_menu: None,
            save_browser: None,
            save_browser_return_to_menu: false,
            mode: AppMode::Loading,
            scenario_catalog,
            active_scenario: None,
            audio,
            assets: assets.clone(),
            material_library: None,
            network,
            network_mode,
            network_lobby,
            sync_checks: SyncCheckState::new(),
            recording_enabled: runtime.record_enabled && paths.is_some(),
            recordings_dir: paths.map(|p| p.recordings_dir()),
            recording: None,
            local_owner: runtime.player_owner,
            player_name: player_name.clone(),
            last_save_path: None,
            object_sprites: base_sprites,
            sprite_cache: Arc::clone(&sprite_cache),
            loading_state: None,
            boot_loading,
            auto_start_sandbox: false,
            ingame_pointer: None,
            mouse_state: None,
            exit_requested: false,
            game_over_dialog: None,
            game_over_handled: false,
        };
        if let Some(existing) = existing_quick_save_path() {
            app.last_save_path = Some(existing);
        }
        // Don't show menu yet; we're in Loading mode for boot loading
        // show_main_menu() and ensure_menu_music() will be called when boot loading finishes
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
            self.assets.hud_graphics(),
        );
        graphics.surface_mut().fill(Color::opaque(16, 28, 52));
        self.graphics = graphics;
        self.graphics.set_sky(self.sky.clone());

        if self.mode == AppMode::Menu {
            let width_f = width as f32;
            let height_f = height as f32;
            self.menu_state.menu().resize(width_f, height_f);
            self.menu_state.set_pointer_position(None);
            self.main_menu_state.resize(width_f, height_f);
            self.main_menu_state.set_pointer_position(None);
            if let Some(options) = self.control_options.as_mut() {
                options.resize(width_f, height_f);
                options.set_pointer_position(None);
            }
            if let Some(about) = self.about_dialog.as_mut() {
                about.dialog.resize(width_f, height_f);
                about.dialog.set_pointer_position(None);
            }
            if let Some(lobby) = self.network_lobby.as_mut() {
                lobby.update_layout(width_f, height_f);
                lobby.pointer_left();
            }
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

    fn apply_material_library_to(&self, engine: &mut Engine) {
        if let Some(materials) = self.material_library.as_ref() {
            engine.set_materials((**materials).clone());
        } else {
            engine.set_materials(MaterialSet::default());
        }
    }

    fn update_sprite_cache(&mut self) {
        self.sprite_cache = Arc::new(self.object_sprites.clone());
        self.graphics
            .set_object_sprites(Arc::clone(&self.sprite_cache));
    }

    fn ensure_local_player_registered(&mut self) -> Result<(), EngineError> {
        if self.engine.player(self.local_owner).is_some() {
            return Ok(());
        }
        let config = PlayerConfig::new(self.local_owner, self.player_name.clone());
        self.engine.register_player(config)?;
        Ok(())
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
            let actions = self
                .engine
                .definition_action_graphics(definition_id)
                .unwrap_or_default();

            let default_key = sprite_map_key(definition_id, None);
            if let Some(image) = self.engine.definition_sprite_image(definition_id, None) {
                let width = image.width();
                let height = image.height();
                let mask = image
                    .color_mask()
                    .map(|mask| ColorByOwnerMask::new(width, height, mask));
                let pixels = image.into_pixels();
                sprites.insert(
                    default_key.clone(),
                    DefinitionSprite {
                        image: ImageData::from_arc(width, height, pixels),
                        actions: actions.clone(),
                        color_mask: mask,
                    },
                );
            } else if let Some(image) = self.engine.definition_picture_image(definition_id) {
                let width = image.width();
                let height = image.height();
                sprites.insert(
                    default_key.clone(),
                    DefinitionSprite {
                        image: ImageData::from_arc(width, height, image.into_pixels()),
                        actions: actions.clone(),
                        color_mask: None,
                    },
                );
            } else if let Some(existing) = sprites.get_mut(&default_key) {
                existing.actions = actions.clone();
            }

            for variant in self.engine.definition_sprite_variant_names(definition_id) {
                if let Some(image) = self
                    .engine
                    .definition_sprite_image(definition_id, Some(&variant))
                {
                    let width = image.width();
                    let height = image.height();
                    let mask = image
                        .color_mask()
                        .map(|mask| ColorByOwnerMask::new(width, height, mask));
                    let pixels = image.into_pixels();
                    let key = sprite_map_key(definition_id, Some(&variant));
                    sprites.insert(
                        key,
                        DefinitionSprite {
                            image: ImageData::from_arc(width, height, pixels),
                            actions: actions.clone(),
                            color_mask: mask,
                        },
                    );
                }
            }
        }
        if sprites != self.object_sprites {
            self.object_sprites = sprites;
            self.update_sprite_cache();
        }
    }

    fn populate_crew_portraits(&self, players: &mut [PlayerOverlay]) {
        let hud_graphics = self.graphics.hud_graphics();
        let fallback_portrait = hud_graphics.crew.clone();
        let mut cache: HashMap<String, ImageData> = HashMap::new();

        for player in players.iter_mut() {
            for crew in player.crew.iter_mut() {
                let Some(object) = self.snapshot.object(crew.object_id) else {
                    crew.portrait = fallback_portrait.clone();
                    continue;
                };

                let definition_id = object.definition_id.clone();
                if let Some(picture) = self.engine.definition_picture_image(&definition_id) {
                    let image = match cache.entry(definition_id) {
                        Entry::Occupied(entry) => entry.get().clone(),
                        Entry::Vacant(entry) => {
                            let image = ImageData::from_arc(
                                picture.width(),
                                picture.height(),
                                picture.pixels(),
                            );
                            entry.insert(image.clone());
                            image
                        }
                    };
                    crew.portrait = Some(image);
                } else {
                    crew.portrait = fallback_portrait.clone();
                }
            }
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

        match self.mode {
            AppMode::Menu => {
                if self.game_over_dialog.is_some() {
                    if state == ElementState::Pressed
                        && matches!(
                            key,
                            VirtualKeyCode::Return
                                | VirtualKeyCode::NumpadEnter
                                | VirtualKeyCode::Space
                                | VirtualKeyCode::Escape
                        )
                    {
                        self.dismiss_game_over_dialog();
                    }
                    return Ok(());
                }
                if self.startup_view == StartupView::Options {
                    if let Some(options) = self.control_options.as_mut() {
                        let mut commands = Vec::new();
                        if state == ElementState::Pressed {
                            if let Some(command) =
                                options.handle_virtual_key(key, &mut self.bindings)
                            {
                                commands.push(command);
                            }
                        }
                        if let Some(gui_key) = map_key_code(key) {
                            let action_commands = match state {
                                ElementState::Pressed => options.handle_key_down(gui_key),
                                ElementState::Released => options.handle_key_up(gui_key),
                            };
                            commands.extend(action_commands);
                        }
                        let _ = options;
                        self.process_control_options_commands(commands)?;
                    }
                    return Ok(());
                }
                if let Some(gui_key) = map_key_code(key) {
                    match self.startup_view {
                        StartupView::ScenarioBrowser => match state {
                            ElementState::Pressed => {
                                if gui_key == KeyCode::Escape && self.menu_state.stack.len() <= 1 {
                                    self.show_main_menu();
                                } else {
                                    self.handle_menu_input(|menu| {
                                        menu.menu().handle_key_down(gui_key)
                                    })?
                                }
                            }
                            ElementState::Released => {
                                self.handle_menu_input(|menu| menu.menu().handle_key_up(gui_key))?
                            }
                        },
                        StartupView::NetworkGame | StartupView::PlayerSelection => {
                            if state == ElementState::Pressed && gui_key == KeyCode::Escape {
                                self.show_main_menu();
                            }
                        }
                        StartupView::MainMenu => {
                            let actions = match state {
                                ElementState::Pressed => {
                                    self.main_menu_state.handle_key_down(gui_key)
                                }
                                ElementState::Released => {
                                    self.main_menu_state.handle_key_up(gui_key)
                                }
                            };
                            self.process_main_menu_actions(actions)?;
                        }
                        StartupView::NetworkLobby => {
                            if let Some(action) = self
                                .network_lobby
                                .as_mut()
                                .and_then(|lobby| lobby.handle_key(gui_key, state))
                            {
                                self.process_lobby_action(action)?;
                                return Ok(());
                            }
                            match state {
                                ElementState::Pressed => self.handle_menu_input(|menu| {
                                    menu.menu().handle_key_down(gui_key)
                                })?,
                                ElementState::Released => self
                                    .handle_menu_input(|menu| menu.menu().handle_key_up(gui_key))?,
                            }
                        }
                        StartupView::Options => {}
                        StartupView::About => {
                            if let Some(about) = self.about_dialog.as_mut() {
                                let actions = match state {
                                    ElementState::Pressed => about.dialog.handle_key_down(gui_key),
                                    ElementState::Released => about.dialog.handle_key_up(gui_key),
                                };
                                self.process_about_actions(actions)?;
                            }
                        }
                    }
                }
                Ok(())
            }
            AppMode::Running => {
                if key == VirtualKeyCode::Escape && state == ElementState::Pressed {
                    if self.object_menu.is_some() {
                        self.close_object_menu();
                    } else if self.ingame_menu.is_some() {
                        self.close_ingame_menu();
                    } else {
                        self.open_ingame_menu()?;
                    }
                    return Ok(());
                }
                self.handle_engine_key(key, state)?;
                Ok(())
            }
            AppMode::Loading => Ok(()),
        }
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
        let mut event = event;
        if self.menu_controls_active() {
            if let Some(mapped) = map_menu_control_event(event) {
                event = mapped;
            }
        }
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
        if let Err(err) = self.input.handle_event(&mut self.engine, owner, event) {
            let status = control_script_error_to_status(err)?;
            tracing::error!(status, "control script error (non-fatal like C++)");
            self.status_text = status;
        }
        Ok(())
    }

    fn menu_controls_active(&self) -> bool {
        matches!(self.mode, AppMode::Running)
            && (self.object_menu.is_some() || self.ingame_menu.is_some())
    }

    fn clear_local_controls(&mut self) -> Result<(), EngineError> {
        if let Some(network) = self.network.as_ref() {
            let frame = self.engine.frame();
            let tick = u32::try_from(frame).unwrap_or(u32::MAX);
            network.submit_local_control(self.local_owner, ControlEvent::ClearPressed, tick);
        }
        let _ = self.input.handle_event(
            &mut self.engine,
            self.local_owner,
            ControlEvent::ClearPressed,
        )?;
        Ok(())
    }

    fn open_ingame_menu(&mut self) -> Result<(), EngineError> {
        if !matches!(self.mode, AppMode::Running) || self.ingame_menu.is_some() {
            return Ok(());
        }
        self.close_object_menu();
        self.clear_local_controls()?;
        let has_quick_save = quick_save_exists();
        let has_saved_games = any_saved_games_exist();
        self.ingame_menu = Some(IngameMenuState::new(has_quick_save, has_saved_games));
        if self.status_text.is_empty() {
            self.status_text = "Paused".to_string();
        }
        Ok(())
    }

    fn close_ingame_menu(&mut self) {
        self.ingame_menu = None;
        if self.status_text == "Paused" {
            self.status_text.clear();
        }
    }

    fn open_object_menu(&mut self) -> Result<bool, EngineError> {
        if !matches!(self.mode, AppMode::Running) || self.object_menu.is_some() {
            return Ok(false);
        }
        match ObjectMenuState::for_player(self.local_owner, &mut self.engine, &self.snapshot) {
            Some(menu) => {
                self.clear_local_controls()?;
                self.object_menu = Some(menu);
                self.ingame_menu = None;
                if self.status_text.is_empty() {
                    self.status_text = "Inventory open".to_string();
                }
                Ok(true)
            }
            None => {
                if self.status_text.is_empty() {
                    self.status_text = "No crew inventory available".to_string();
                }
                Ok(false)
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

        if menu_command
            && self.object_menu.is_none()
            && self.ingame_menu.is_none()
            && self.save_browser.is_none()
        {
            return Ok(false);
        }

        if matches!(command, ControlCommand::PlayerMenu) {
            if matches!(
                kind,
                CommandKind::Press | CommandKind::Single | CommandKind::Double
            ) {
                if self.save_browser.take().is_some() {
                    let reopen = self.save_browser_return_to_menu;
                    self.save_browser_return_to_menu = false;
                    if reopen {
                        self.open_ingame_menu()?;
                    }
                } else if self.object_menu.is_some() {
                    self.close_object_menu();
                } else if !self.open_object_menu()? {
                    self.open_ingame_menu()?;
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

        if let Some(browser) = self.save_browser.as_mut() {
            if let Some(action) = browser.handle_command(command, kind) {
                self.execute_save_browser_action(action)?;
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
                ObjectMenuCommand::Take => {
                    self.take_from_container(&selection, 1)?;
                }
                ObjectMenuCommand::TakeAll => {
                    let amount = selection.instances.len().max(1);
                    self.take_from_container(&selection, amount)?;
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

    fn take_from_container(
        &mut self,
        selection: &ObjectMenuSelection,
        amount: usize,
    ) -> Result<(), EngineError> {
        if amount == 0 {
            return Ok(());
        }
        let Some(container_id) = selection.source_container else {
            self.status_text = "Container no longer available".to_string();
            self.refresh_object_menu();
            return Ok(());
        };

        let Some(_) = self.snapshot.object(selection.crew_id) else {
            self.status_text = "Crew no longer available".to_string();
            self.object_menu = None;
            return Ok(());
        };

        if self.snapshot.object(container_id).is_none() {
            self.status_text = "Container no longer available".to_string();
            self.object_menu = None;
            return Ok(());
        }

        let mut taken = 0usize;
        for object_id in selection.instances.iter().take(amount) {
            match self.engine.apply_object_update(
                *object_id,
                ObjectUpdate::new().with_container(selection.crew_id),
            ) {
                Ok(()) => taken += 1,
                Err(EngineError::UnknownObject(_)) => {
                    tracing::warn!(
                        object = %object_id,
                        "container item missing while taking"
                    );
                }
                Err(err) => return Err(err),
            }
        }

        if taken == 0 {
            self.status_text = format!("No {} to take", selection.label);
            return Ok(());
        }

        self.snapshot = self.engine.snapshot();
        self.refresh_object_menu();
        self.refresh_focus();
        self.status_text = format!("Took {} (x{})", selection.label, taken);
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
                        let quick_available = quick_save_exists();
                        let saved_available = any_saved_games_exist();
                        menu.update_save_options(quick_available, saved_available);
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
            IngameMenuAction::SaveGame => {
                if let Err(err) = self.open_save_browser() {
                    tracing::error!(error = ?err, "failed to open save menu");
                    self.status_text = format!("Save menu failed: {err:#}");
                    self.open_ingame_menu()?;
                }
            }
            IngameMenuAction::LoadGame => {
                if let Err(err) = self.open_load_browser() {
                    tracing::error!(error = ?err, "failed to open load menu");
                    self.status_text = format!("Load menu failed: {err:#}");
                    self.open_ingame_menu()?;
                }
            }
            IngameMenuAction::AbortToMenu => {
                self.close_ingame_menu();
                self.return_to_menu();
            }
        }
        Ok(())
    }

    fn open_save_browser(&mut self) -> Result<()> {
        let entries = self.collect_save_entries()?;
        let suggested_label = self.generate_default_save_label();
        let state = SaveBrowserState::new(SaveBrowserMode::Save { suggested_label }, entries);
        self.save_browser = Some(state);
        self.save_browser_return_to_menu = true;
        self.ingame_menu = None;
        self.object_menu = None;
        Ok(())
    }

    fn open_load_browser(&mut self) -> Result<()> {
        let entries = self.collect_save_entries()?;
        if entries.is_empty() {
            self.status_text = "No saved games found".to_string();
        }
        let state = SaveBrowserState::new(SaveBrowserMode::Load, entries);
        self.save_browser = Some(state);
        self.save_browser_return_to_menu = true;
        self.ingame_menu = None;
        self.object_menu = None;
        Ok(())
    }

    fn dismiss_save_browser(&mut self, reopen_ingame_menu: bool) -> Result<(), EngineError> {
        self.save_browser = None;
        let reopen = reopen_ingame_menu && self.save_browser_return_to_menu;
        self.save_browser_return_to_menu = false;
        if reopen {
            self.open_ingame_menu()?;
        }
        Ok(())
    }

    fn execute_save_browser_action(
        &mut self,
        action: SaveBrowserAction,
    ) -> Result<(), EngineError> {
        match action {
            SaveBrowserAction::Close => {
                self.dismiss_save_browser(true)?;
            }
            SaveBrowserAction::SaveNew { label } => match self.perform_named_save(&label, None) {
                Ok(_) => {
                    self.dismiss_save_browser(true)?;
                }
                Err(err) => {
                    tracing::error!(error = ?err, "failed to save game");
                    self.status_text = format!("Save failed: {err:#}");
                }
            },
            SaveBrowserAction::SaveExisting { entry } => {
                match self.perform_named_save(&entry.display_name, Some(entry.path.clone())) {
                    Ok(_) => {
                        self.dismiss_save_browser(true)?;
                    }
                    Err(err) => {
                        tracing::error!(error = ?err, "failed to overwrite save");
                        self.status_text = format!("Save failed: {err:#}");
                    }
                }
            }
            SaveBrowserAction::Load { entry } => {
                match self.load_saved_game_from_path(&entry.path) {
                    Ok(_) => {
                        self.save_browser = None;
                        self.save_browser_return_to_menu = false;
                        self.close_ingame_menu();
                    }
                    Err(err) => {
                        tracing::error!(
                            error = ?err,
                            path = %entry.path.display(),
                            "failed to load saved game"
                        );
                        self.status_text = format!("Load failed: {err:#}");
                    }
                }
            }
        }
        Ok(())
    }

    fn collect_save_entries(&self) -> Result<Vec<SaveEntry>> {
        let dir = resolve_save_directory();
        let mut entries = Vec::new();
        let read_dir = match fs::read_dir(&dir) {
            Ok(iter) => iter,
            Err(_) => return Ok(entries),
        };
        for entry in read_dir {
            let entry = match entry {
                Ok(value) => value,
                Err(err) => {
                    tracing::warn!(error = %err, "failed to iterate save directory entry");
                    continue;
                }
            };
            let path = entry.path();
            if !is_save_file(&path) {
                continue;
            }
            match load_save_entry(&path) {
                Ok(save_entry) => entries.push(save_entry),
                Err(err) => {
                    tracing::warn!(path = %path.display(), error = %err, "failed to read save metadata");
                }
            }
        }
        Ok(entries)
    }

    fn generate_default_save_label(&self) -> String {
        let base = self
            .active_scenario
            .as_ref()
            .map(|scenario| scenario.title.clone())
            .unwrap_or_else(|| self.scenario_label.clone());
        format!("{} {}", base, current_unix_timestamp())
    }

    fn perform_named_save(&mut self, label: &str, target: Option<PathBuf>) -> Result<PathBuf> {
        if self.mode != AppMode::Running {
            anyhow::bail!("cannot save while not running a scenario");
        }

        let scenario = self
            .active_scenario
            .clone()
            .unwrap_or_else(FrontendScenario::fallback);
        let engine_state = self.engine.capture_state();
        let sanitized_label = if label.trim().is_empty() {
            self.generate_default_save_label()
        } else {
            label.trim().to_string()
        };

        let saved = SavedGameFile {
            version: SAVE_FILE_VERSION,
            saved_at_seconds: current_unix_timestamp(),
            scenario: SavedScenarioInfo::from_frontend(
                &scenario,
                &self.scenario_label,
                self.fallback_ground,
            ),
            focus_id: self.focus_id,
            user_label: Some(sanitized_label.clone()),
            engine_state,
        };

        let dir = ensure_save_directory()?;
        let path = match target {
            Some(path) => path,
            None => {
                let base = sanitize_save_label(&sanitized_label);
                unique_save_path(&dir, &base)
            }
        };

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create save directory at {}", parent.display())
            })?;
        }

        let mut file = File::create(&path)
            .with_context(|| format!("failed to create save file at {}", path.display()))?;
        serde_json::to_writer_pretty(&mut file, &saved).context("failed to serialise save data")?;
        file.flush().context("failed to flush save data")?;

        self.write_save_thumbnail(&path)?;
        self.last_save_path = Some(path.clone());
        self.status_text = format!("Saved {}", saved.scenario.title);
        Ok(path)
    }

    fn write_save_thumbnail(&mut self, path: &Path) -> Result<()> {
        let surface = self.graphics.surface();
        let encoded =
            encode_surface_to_png(surface).context("failed to encode save thumbnail image")?;
        let target = path.with_extension("png");
        let mut file = File::create(&target)
            .with_context(|| format!("failed to create thumbnail at {}", target.display()))?;
        file.write_all(&encoded)
            .context("failed to write save thumbnail")?;
        file.flush()
            .context("failed to flush save thumbnail to disk")?;
        Ok(())
    }

    fn load_saved_game_from_path(&mut self, path: &Path) -> Result<()> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read save from {}", path.display()))?;
        let save: SavedGameFile = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse save data from {}", path.display()))?;
        let save = migrate_save_file(save)?;
        self.apply_loaded_game(save)?;
        self.last_save_path = Some(path.to_path_buf());
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
                    NetworkEvent::SyncCheck { packet } => {
                        self.handle_sync_check(packet);
                    }
                    NetworkEvent::PeerConnected {
                        client_id,
                        name,
                        kind,
                    } => {
                        tracing::info!(%client_id, %name, ?kind, "network client connected");
                        if let Some(lobby) = self.network_lobby.as_mut() {
                            lobby.register_peer(client_id, name.clone(), kind);
                        }
                        self.status_text = format!("{name} joined the lobby");
                    }
                    NetworkEvent::PeerDisconnected { client_id, reason } => {
                        if let Some(lobby) = self.network_lobby.as_mut() {
                            lobby.unregister_peer(client_id);
                        }
                        match reason {
                            Some(reason) => {
                                tracing::info!(
                                    %client_id,
                                    reason = %reason,
                                    "network client disconnected"
                                );
                                self.status_text = format!("Client {client_id} left: {reason}");
                            }
                            None => {
                                tracing::info!(%client_id, "network client disconnected");
                                self.status_text = format!("Client {client_id} left the lobby");
                            }
                        }
                    }
                    NetworkEvent::Error(message) => {
                        tracing::error!(message = %message, "network error");
                    }
                }
            }
        }
        Ok(())
    }

    fn handle_sync_check(&mut self, packet: SyncCheckPacket) {
        if matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            return;
        }
        if let Some((local, remote)) = self.sync_checks.record_remote(packet) {
            self.evaluate_sync_checks(local, remote);
        }
    }

    fn evaluate_sync_checks(&mut self, local: SyncCheckPacket, remote: SyncCheckPacket) {
        if local.matches(&remote) {
            return;
        }
        self.handle_desync(local, remote);
    }

    fn handle_desync(&mut self, local: SyncCheckPacket, remote: SyncCheckPacket) {
        tracing::error!(
            frame = local.frame,
            local = ?local,
            host = ?remote,
            "network desync detected"
        );
        self.sync_checks.clear();
        if matches!(self.network_mode, Some(NetworkMode::Host(_))) {
            self.status_text = "Network desync detected".to_string();
            return;
        }
        self.network = None;
        self.return_to_menu();
        self.status_text = "Network desync detected; disconnected from host".to_string();
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
            GamepadEvent::Command { command, state } => {
                self.handle_gamepad_command(command, state)?;
            }
            GamepadEvent::Clear => {
                if matches!(self.mode, AppMode::Running) {
                    self.dispatch_control_event(ControlEvent::ClearPressed)?;
                }
            }
            GamepadEvent::Action { action, state } => {
                self.handle_gamepad_action(action, state)?;
            }
        }
        Ok(())
    }

    fn handle_gamepad_command(
        &mut self,
        command: ControlCommand,
        state: ElementState,
    ) -> Result<(), EngineError> {
        if !matches!(self.mode, AppMode::Running) {
            return Ok(());
        }
        let kind = match state {
            ElementState::Pressed => CommandKind::Press,
            ElementState::Released => CommandKind::Release,
        };
        self.dispatch_control_event(ControlEvent::Command { command, kind })
    }

    fn handle_gamepad_direction(
        &mut self,
        button: ControlButton,
        state: ElementState,
    ) -> Result<(), EngineError> {
        match self.mode {
            AppMode::Menu => {
                if let Some(key) = menu_key_from_control_button(button) {
                    match self.startup_view {
                        StartupView::ScenarioBrowser => match state {
                            ElementState::Pressed => {
                                self.handle_menu_input(|menu| menu.menu().handle_key_down(key))?
                            }
                            ElementState::Released => {
                                self.handle_menu_input(|menu| menu.menu().handle_key_up(key))?
                            }
                        },
                        StartupView::NetworkGame | StartupView::PlayerSelection => {}
                        StartupView::MainMenu => {
                            let actions = match state {
                                ElementState::Pressed => self.main_menu_state.handle_key_down(key),
                                ElementState::Released => self.main_menu_state.handle_key_up(key),
                            };
                            self.process_main_menu_actions(actions)?;
                        }
                        StartupView::NetworkLobby => match state {
                            ElementState::Pressed => {
                                self.handle_menu_input(|menu| menu.menu().handle_key_down(key))?
                            }
                            ElementState::Released => {
                                self.handle_menu_input(|menu| menu.menu().handle_key_up(key))?
                            }
                        },
                        StartupView::Options => {
                            if let Some(commands) =
                                self.control_options.as_mut().map(|options| match state {
                                    ElementState::Pressed => options.handle_key_down(key),
                                    ElementState::Released => options.handle_key_up(key),
                                })
                            {
                                self.process_control_options_commands(commands)?;
                            }
                        }
                        StartupView::About => {
                            if let Some(about) = self.about_dialog.as_mut() {
                                let actions = match state {
                                    ElementState::Pressed => about.dialog.handle_key_down(key),
                                    ElementState::Released => about.dialog.handle_key_up(key),
                                };
                                self.process_about_actions(actions)?;
                            }
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
            AppMode::Loading => {}
        }
        Ok(())
    }

    fn handle_menu_cancel_action(&mut self, state: ElementState) -> Result<(), EngineError> {
        if self.game_over_dialog.is_some() {
            if state == ElementState::Pressed {
                self.dismiss_game_over_dialog();
            }
            return Ok(());
        }
        match self.startup_view {
            StartupView::ScenarioBrowser => match state {
                ElementState::Pressed => {
                    self.handle_menu_input(|menu| menu.menu().handle_key_down(KeyCode::Escape))?
                }
                ElementState::Released => {
                    self.handle_menu_input(|menu| menu.menu().handle_key_up(KeyCode::Escape))?
                }
            },
            StartupView::NetworkGame | StartupView::PlayerSelection => {
                if state == ElementState::Pressed {
                    self.show_main_menu();
                }
            }
            StartupView::MainMenu => {
                let actions = match state {
                    ElementState::Pressed => self.main_menu_state.handle_key_down(KeyCode::Escape),
                    ElementState::Released => self.main_menu_state.handle_key_up(KeyCode::Escape),
                };
                self.process_main_menu_actions(actions)?;
            }
            StartupView::NetworkLobby => {
                if state == ElementState::Pressed {
                    self.show_main_menu();
                }
            }
            StartupView::Options => {
                if state == ElementState::Pressed {
                    self.process_control_options_command(ControlOptionsCommand::Close)?;
                }
            }
            StartupView::About => {
                if let Some(about) = self.about_dialog.as_mut() {
                    let actions = match state {
                        ElementState::Pressed => about.dialog.handle_key_down(KeyCode::Escape),
                        ElementState::Released => about.dialog.handle_key_up(KeyCode::Escape),
                    };
                    self.process_about_actions(actions)?;
                }
            }
        }
        Ok(())
    }

    fn handle_gamepad_action(
        &mut self,
        action: GamepadActionType,
        state: ElementState,
    ) -> Result<(), EngineError> {
        if matches!(self.mode, AppMode::Menu) && self.game_over_dialog.is_some() {
            if state == ElementState::Pressed
                && matches!(
                    action,
                    GamepadActionType::Select
                        | GamepadActionType::Cancel
                        | GamepadActionType::MenuToggle
                )
            {
                self.dismiss_game_over_dialog();
            }
            return Ok(());
        }
        match action {
            GamepadActionType::Select => match self.mode {
                AppMode::Menu => match self.startup_view {
                    StartupView::ScenarioBrowser => match state {
                        ElementState::Pressed => self.handle_menu_input(|menu| {
                            menu.menu().handle_key_down(KeyCode::Enter)
                        })?,
                        ElementState::Released => self
                            .handle_menu_input(|menu| menu.menu().handle_key_up(KeyCode::Enter))?,
                    },
                    StartupView::NetworkGame | StartupView::PlayerSelection => {}
                    StartupView::MainMenu => {
                        let actions = match state {
                            ElementState::Pressed => {
                                self.main_menu_state.handle_key_down(KeyCode::Enter)
                            }
                            ElementState::Released => {
                                self.main_menu_state.handle_key_up(KeyCode::Enter)
                            }
                        };
                        self.process_main_menu_actions(actions)?;
                    }
                    StartupView::NetworkLobby => match state {
                        ElementState::Pressed => self.handle_menu_input(|menu| {
                            menu.menu().handle_key_down(KeyCode::Enter)
                        })?,
                        ElementState::Released => self
                            .handle_menu_input(|menu| menu.menu().handle_key_up(KeyCode::Enter))?,
                    },
                    StartupView::Options => {
                        if let Some(commands) =
                            self.control_options.as_mut().map(|options| match state {
                                ElementState::Pressed => options.handle_key_down(KeyCode::Enter),
                                ElementState::Released => options.handle_key_up(KeyCode::Enter),
                            })
                        {
                            self.process_control_options_commands(commands)?;
                        }
                    }
                    StartupView::About => {
                        if let Some(about) = self.about_dialog.as_mut() {
                            let actions = match state {
                                ElementState::Pressed => {
                                    about.dialog.handle_key_down(KeyCode::Enter)
                                }
                                ElementState::Released => {
                                    about.dialog.handle_key_up(KeyCode::Enter)
                                }
                            };
                            self.process_about_actions(actions)?;
                        }
                    }
                },
                AppMode::Running | AppMode::Loading => {}
            },
            GamepadActionType::Cancel => match self.mode {
                AppMode::Menu => {
                    self.handle_menu_cancel_action(state)?;
                }
                AppMode::Running | AppMode::Loading => {}
            },
            GamepadActionType::MenuToggle => match self.mode {
                AppMode::Menu => {
                    self.handle_menu_cancel_action(state)?;
                }
                AppMode::Running => {
                    if state == ElementState::Pressed {
                        if self.ingame_menu.is_some() {
                            self.close_ingame_menu();
                        } else {
                            self.open_ingame_menu()?;
                        }
                    }
                }
                AppMode::Loading => {}
            },
        }
        Ok(())
    }

    fn handle_cursor_moved(&mut self, position: PhysicalPosition<f64>) -> Result<(), EngineError> {
        let point = gui_point_from_position(position);
        match self.mode {
            AppMode::Menu => {
                if self.game_over_dialog.is_some() {
                    self.pointer_left();
                    return Ok(());
                }
                match self.startup_view {
                    StartupView::ScenarioBrowser => self.handle_menu_input(|state| {
                        state.set_pointer_position(Some(point));
                        state.menu().handle_pointer_move(point)
                    }),
                    StartupView::NetworkGame | StartupView::PlayerSelection => Ok(()),
                    StartupView::MainMenu => {
                        self.main_menu_state.set_pointer_position(Some(point));
                        let actions = self.main_menu_state.handle_pointer_move(point);
                        self.process_main_menu_actions(actions)
                    }
                    StartupView::NetworkLobby => {
                        if let Some(lobby) = self.network_lobby.as_mut() {
                            let width = self.graphics.surface().width() as f32;
                            let height = self.graphics.surface().height() as f32;
                            lobby.update_layout(width, height);
                            match lobby.pointer_region(point) {
                                LobbyPointerRegion::Menu => self.handle_menu_input(|state| {
                                    state.set_pointer_position(Some(point));
                                    state.menu().handle_pointer_move(point)
                                }),
                                LobbyPointerRegion::Panel => {
                                    lobby.handle_panel_pointer_move(point);
                                    self.menu_state.set_pointer_position(None);
                                    Ok(())
                                }
                            }
                        } else {
                            Ok(())
                        }
                    }
                    StartupView::Options => {
                        let commands = if let Some(options) = self.control_options.as_mut() {
                            options.set_pointer_position(Some(point));
                            Some(options.handle_pointer_move(point))
                        } else {
                            None
                        };
                        if let Some(commands) = commands {
                            self.process_control_options_commands(commands)
                        } else {
                            Ok(())
                        }
                    }
                    StartupView::About => {
                        if let Some(about) = self.about_dialog.as_mut() {
                            about.dialog.set_pointer_position(Some(point));
                            let actions = about.dialog.handle_pointer_move(point);
                            self.process_about_actions(actions)
                        } else {
                            Ok(())
                        }
                    }
                }
            }
            AppMode::Running => {
                self.update_ingame_pointer(point);
                Ok(())
            }
            AppMode::Loading => Ok(()),
        }
    }

    fn update_ingame_pointer(&mut self, point: GuiPoint) {
        if let Some(pointer) = self.graphics.viewport_point_at(point) {
            if let Some(state) = self.mouse_state.as_mut() {
                state.update(pointer);
            }
            self.ingame_pointer = Some(pointer);
        } else {
            if let Some(state) = self.mouse_state.as_mut() {
                state.moved = true;
            }
            self.ingame_pointer = None;
        }
    }

    fn handle_ingame_mouse_button(
        &mut self,
        button_state: ElementState,
    ) -> Result<(), EngineError> {
        match button_state {
            ElementState::Pressed => self.on_ingame_mouse_down(),
            ElementState::Released => self.on_ingame_mouse_up(),
        }
    }

    fn on_ingame_mouse_down(&mut self) -> Result<(), EngineError> {
        let Some(pointer) = self.ingame_pointer else {
            self.mouse_state = None;
            return Ok(());
        };
        self.mouse_state = Some(IngameMouseState::new(pointer));

        if pointer.owner != self.local_owner {
            return Ok(());
        }

        if let Some(crew_id) =
            self.graphics
                .crew_at_point(&self.snapshot, self.local_owner, pointer.screen)
        {
            self.engine.select_crew(self.local_owner, [crew_id])?;
            self.engine
                .set_crew_cursor(self.local_owner, Some(crew_id))?;
            self.focus_id = Some(crew_id);
            self.snapshot = self.engine.snapshot();
            self.refresh_object_menu();
            self.refresh_focus();
        }
        Ok(())
    }

    fn on_ingame_mouse_up(&mut self) -> Result<(), EngineError> {
        let Some(state) = self.mouse_state.take() else {
            return Ok(());
        };
        if state.start.owner != self.local_owner {
            return Ok(());
        }
        if state.moved {
            self.handle_mouse_drag(state)?;
        }
        Ok(())
    }

    fn handle_mouse_drag(&mut self, state: IngameMouseState) -> Result<(), EngineError> {
        if !matches!(self.mode, AppMode::Running) {
            return Ok(());
        }
        let Some(selection) = self.snapshot.crew_selection.get(&self.local_owner) else {
            return Ok(());
        };
        if selection.selected.is_empty() && selection.cursor.is_none() {
            return Ok(());
        }
        let crew_id = selection
            .cursor
            .or_else(|| selection.selected.first().copied());
        let Some(crew_id) = crew_id else {
            return Ok(());
        };
        let Some(crew) = self.snapshot.object(crew_id) else {
            return Ok(());
        };
        if crew.contents.is_empty() {
            return Ok(());
        }

        let delta = state.delta();
        let buttons = drag_direction_buttons(delta);
        if buttons.is_empty() {
            return Ok(());
        }

        for button in &buttons {
            self.dispatch_control_event(ControlEvent::Press(*button))?;
        }
        self.dispatch_control_event(ControlEvent::Command {
            command: ControlCommand::Throw,
            kind: CommandKind::Press,
        })?;
        self.dispatch_control_event(ControlEvent::Command {
            command: ControlCommand::Throw,
            kind: CommandKind::Release,
        })?;
        for button in buttons.into_iter().rev() {
            self.dispatch_control_event(ControlEvent::Release(button))?;
        }

        self.snapshot = self.engine.snapshot();
        self.refresh_object_menu();
        self.refresh_focus();
        Ok(())
    }

    fn handle_mouse_button(&mut self, button_state: ElementState) -> Result<(), EngineError> {
        match self.mode {
            AppMode::Menu => {
                if self.game_over_dialog.is_some() {
                    if button_state == ElementState::Released {
                        self.dismiss_game_over_dialog();
                    }
                    return Ok(());
                }
                match self.startup_view {
                    StartupView::NetworkGame | StartupView::PlayerSelection => Ok(()),
                    StartupView::ScenarioBrowser => {
                        if let Some(point) = self.menu_state.pointer_position() {
                            match button_state {
                                ElementState::Pressed => self.handle_menu_input(|state| {
                                    state.menu().handle_pointer_down(point)
                                })?,
                                ElementState::Released => self.handle_menu_input(|state| {
                                    state.menu().handle_pointer_up(point)
                                })?,
                            }
                        }
                        Ok(())
                    }
                    StartupView::MainMenu => {
                        if let Some(point) = self.main_menu_state.pointer_position() {
                            let actions = match button_state {
                                ElementState::Pressed => {
                                    self.main_menu_state.handle_pointer_down(point)
                                }
                                ElementState::Released => {
                                    self.main_menu_state.handle_pointer_up(point)
                                }
                            };
                            self.process_main_menu_actions(actions)?;
                        }
                        Ok(())
                    }
                    StartupView::NetworkLobby => {
                        if let Some(lobby) = self.network_lobby.as_mut() {
                            let width = self.graphics.surface().width() as f32;
                            let height = self.graphics.surface().height() as f32;
                            lobby.update_layout(width, height);

                            match button_state {
                                ElementState::Pressed => {
                                    if let Some(point) = lobby.pointer_position() {
                                        if matches!(
                                            lobby.pointer_region(point),
                                            LobbyPointerRegion::Panel
                                        ) {
                                            lobby.handle_panel_pointer_down(point);
                                            return Ok(());
                                        }
                                    }
                                    if let Some(point) = self.menu_state.pointer_position() {
                                        self.handle_menu_input(|state| {
                                            state.menu().handle_pointer_down(point)
                                        })?;
                                    }
                                    Ok(())
                                }
                                ElementState::Released => {
                                    if let Some(point) = lobby.pointer_position() {
                                        if matches!(
                                            lobby.pointer_region(point),
                                            LobbyPointerRegion::Panel
                                        ) {
                                            if let Some(action) =
                                                lobby.handle_panel_pointer_up(point)
                                            {
                                                self.process_lobby_action(action)?;
                                            }
                                            return Ok(());
                                        }
                                    }
                                    if let Some(point) = self.menu_state.pointer_position() {
                                        self.handle_menu_input(|state| {
                                            state.menu().handle_pointer_up(point)
                                        })?;
                                    }
                                    Ok(())
                                }
                            }
                        } else {
                            Ok(())
                        }
                    }
                    StartupView::Options => {
                        let commands = if let Some(options) = self.control_options.as_mut() {
                            options.pointer_position().map(|point| match button_state {
                                ElementState::Pressed => options.handle_pointer_down(point),
                                ElementState::Released => options.handle_pointer_up(point),
                            })
                        } else {
                            None
                        };
                        if let Some(commands) = commands {
                            self.process_control_options_commands(commands)?;
                        }
                        Ok(())
                    }
                    StartupView::About => {
                        if let Some(about) = self.about_dialog.as_mut() {
                            if let Some(point) = about.dialog.pointer_position() {
                                let actions = match button_state {
                                    ElementState::Pressed => {
                                        about.dialog.handle_pointer_down(point)
                                    }
                                    ElementState::Released => about.dialog.handle_pointer_up(point),
                                };
                                self.process_about_actions(actions)?;
                            }
                        }
                        Ok(())
                    }
                }
            }
            AppMode::Running => self.handle_ingame_mouse_button(button_state),
            AppMode::Loading => Ok(()),
        }
    }

    fn handle_touch(&mut self, phase: TouchPhase, position: GuiPoint) -> Result<(), EngineError> {
        if self.mode != AppMode::Menu {
            return Ok(());
        }
        if self.game_over_dialog.is_some() {
            if matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                self.dismiss_game_over_dialog();
            }
            return Ok(());
        }
        match self.startup_view {
            StartupView::NetworkGame | StartupView::PlayerSelection => Ok(()),
            StartupView::ScenarioBrowser => match phase {
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
            },
            StartupView::MainMenu => {
                self.main_menu_state.set_pointer_position(Some(position));
                let actions = match phase {
                    TouchPhase::Started => self.main_menu_state.handle_pointer_down(position),
                    TouchPhase::Moved => self.main_menu_state.handle_pointer_move(position),
                    TouchPhase::Ended => {
                        let actions = self.main_menu_state.handle_pointer_up(position);
                        self.pointer_left();
                        actions
                    }
                    TouchPhase::Cancelled => {
                        self.pointer_left();
                        Vec::new()
                    }
                };
                self.process_main_menu_actions(actions)
            }
            StartupView::NetworkLobby => {
                if let Some(lobby) = self.network_lobby.as_mut() {
                    let width = self.graphics.surface().width() as f32;
                    let height = self.graphics.surface().height() as f32;
                    lobby.update_layout(width, height);
                    match phase {
                        TouchPhase::Started => match lobby.pointer_region(position) {
                            LobbyPointerRegion::Menu => self.handle_menu_input(|state| {
                                state.set_pointer_position(Some(position));
                                state.menu().handle_pointer_down(position)
                            }),
                            LobbyPointerRegion::Panel => {
                                lobby.handle_panel_pointer_move(position);
                                lobby.handle_panel_pointer_down(position);
                                self.menu_state.set_pointer_position(None);
                                Ok(())
                            }
                        },
                        TouchPhase::Moved => match lobby.pointer_region(position) {
                            LobbyPointerRegion::Menu => self.handle_menu_input(|state| {
                                state.set_pointer_position(Some(position));
                                state.menu().handle_pointer_move(position)
                            }),
                            LobbyPointerRegion::Panel => {
                                lobby.handle_panel_pointer_move(position);
                                self.menu_state.set_pointer_position(None);
                                Ok(())
                            }
                        },
                        TouchPhase::Ended => match lobby.pointer_region(position) {
                            LobbyPointerRegion::Menu => {
                                let result = self.handle_menu_input(|state| {
                                    state.set_pointer_position(Some(position));
                                    state.menu().handle_pointer_up(position)
                                });
                                self.pointer_left();
                                result
                            }
                            LobbyPointerRegion::Panel => {
                                lobby.handle_panel_pointer_move(position);
                                if let Some(action) = lobby.handle_panel_pointer_up(position) {
                                    self.process_lobby_action(action)?;
                                }
                                self.pointer_left();
                                Ok(())
                            }
                        },
                        TouchPhase::Cancelled => {
                            self.pointer_left();
                            Ok(())
                        }
                    }
                } else {
                    Ok(())
                }
            }
            StartupView::Options => {
                let (commands, clear_pointer) = if let Some(options) = self.control_options.as_mut()
                {
                    options.set_pointer_position(Some(position));
                    match phase {
                        TouchPhase::Started => (options.handle_pointer_down(position), false),
                        TouchPhase::Moved => (options.handle_pointer_move(position), false),
                        TouchPhase::Ended => (options.handle_pointer_up(position), true),
                        TouchPhase::Cancelled => (Vec::new(), true),
                    }
                } else {
                    (
                        Vec::new(),
                        matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled),
                    )
                };
                if clear_pointer {
                    self.pointer_left();
                }
                self.process_control_options_commands(commands)
            }
            StartupView::About => {
                if let Some(about) = self.about_dialog.as_mut() {
                    about.dialog.set_pointer_position(Some(position));
                    let actions = match phase {
                        TouchPhase::Started => about.dialog.handle_pointer_down(position),
                        TouchPhase::Moved => about.dialog.handle_pointer_move(position),
                        TouchPhase::Ended => {
                            let actions = about.dialog.handle_pointer_up(position);
                            self.pointer_left();
                            actions
                        }
                        TouchPhase::Cancelled => {
                            self.pointer_left();
                            Vec::new()
                        }
                    };
                    self.process_about_actions(actions)
                } else {
                    Ok(())
                }
            }
        }
    }

    fn pointer_left(&mut self) {
        match self.mode {
            AppMode::Menu => match self.startup_view {
                StartupView::NetworkGame | StartupView::PlayerSelection => {}
                StartupView::ScenarioBrowser => {
                    self.menu_state.set_pointer_position(None);
                }
                StartupView::MainMenu => {
                    self.main_menu_state.pointer_left();
                }
                StartupView::NetworkLobby => {
                    self.menu_state.set_pointer_position(None);
                    if let Some(lobby) = self.network_lobby.as_mut() {
                        lobby.pointer_left();
                    }
                }
                StartupView::Options => {
                    if let Some(options) = self.control_options.as_mut() {
                        options.set_pointer_position(None);
                    }
                }
                StartupView::About => {
                    if let Some(about) = self.about_dialog.as_mut() {
                        about.dialog.set_pointer_position(None);
                    }
                }
            },
            AppMode::Running => {
                if let Some(state) = self.mouse_state.as_mut() {
                    state.moved = true;
                }
                self.ingame_pointer = None;
            }
            AppMode::Loading => {}
        }
    }

    fn handle_menu_input<F>(&mut self, handler: F) -> Result<(), EngineError>
    where
        F: FnOnce(&mut MenuState) -> Vec<StartupMenuAction>,
    {
        if self.game_over_dialog.is_some() {
            return Ok(());
        }
        if self.mode != AppMode::Menu
            || !matches!(
                self.startup_view,
                StartupView::ScenarioBrowser | StartupView::NetworkLobby
            )
        {
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
        let mut pending: VecDeque<StartupMenuAction> = actions.into();

        while let Some(action) = pending.pop_front() {
            match action {
                StartupMenuAction::SelectionChanged(_) => {
                    self.play_ui_sound("Command");
                }
                StartupMenuAction::StartScenario(summary) => {
                    self.play_ui_sound("Click");
                    if matches!(self.startup_view, StartupView::NetworkLobby) {
                        if let Some(lobby) = self.network_lobby.as_mut() {
                            lobby.select_scenario(&summary.identifier, &summary.title);
                            self.scenario_label = lobby.scenario_label();
                            self.status_text = format!("Selected {}", summary.title);
                        }
                    } else {
                        start_identifier = Some(summary.identifier);
                    }
                }
                StartupMenuAction::OpenEntry(summary) => {
                    if summary.identifier == BACK_ENTRY_IDENTIFIER {
                        self.play_ui_sound("DoorClose");
                        if self.menu_state.stack.len() <= 1 {
                            self.show_main_menu();
                        } else {
                            self.menu_state.leave_folder();
                            updated_label = Some(self.menu_state.label_path());
                            pending.extend(self.menu_state.select_default_entry());
                        }
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
                            self.play_ui_sound("DoorOpen");
                            self.menu_state.enter_folder(&summary.identifier);
                            updated_label = Some(self.menu_state.label_path());
                            pending.extend(self.menu_state.select_default_entry());
                        }
                        Some(ScenarioKind::Scenario) => {
                            self.play_ui_sound("Click");
                            if matches!(self.startup_view, StartupView::NetworkLobby) {
                                if let Some(lobby) = self.network_lobby.as_mut() {
                                    lobby.select_scenario(&summary.identifier, &summary.title);
                                    self.scenario_label = lobby.scenario_label();
                                    self.status_text = format!("Selected {}", summary.title);
                                }
                            } else {
                                start_identifier = Some(summary.identifier);
                            }
                        }
                        Some(ScenarioKind::Editor) => {
                            self.play_ui_sound("Click");
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
                            self.play_ui_sound("DoorOpen");
                            self.menu_state.enter_folder(&summary.identifier);
                            updated_label = Some(self.menu_state.label_path());
                            pending.extend(self.menu_state.select_default_entry());
                        }
                    }
                }
                StartupMenuAction::EditEntry(summary) => {
                    self.play_ui_sound("Click");
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

    fn process_main_menu_actions(
        &mut self,
        actions: Vec<MainMenuAction>,
    ) -> Result<(), EngineError> {
        if self.game_over_dialog.is_some() {
            return Ok(());
        }
        for action in actions {
            match action {
                MainMenuAction::SelectionChanged(_) => {
                    self.play_ui_sound("Command");
                }
                MainMenuAction::Activate(item) => {
                    self.play_ui_sound("Click");
                    self.handle_main_menu_activation(item)?;
                }
            }
        }
        Ok(())
    }

    fn process_control_options_commands(
        &mut self,
        commands: Vec<ControlOptionsCommand>,
    ) -> Result<(), EngineError> {
        for command in commands {
            self.process_control_options_command(command)?;
        }
        Ok(())
    }

    fn process_control_options_command(
        &mut self,
        command: ControlOptionsCommand,
    ) -> Result<(), EngineError> {
        match command {
            ControlOptionsCommand::SelectionChanged(_) => {
                self.status_text.clear();
            }
            ControlOptionsCommand::BeginRebind(binding) => {
                self.status_text = format!(
                    "Press a key for “{}” or Escape to cancel",
                    binding_display_name(binding)
                );
            }
            ControlOptionsCommand::BindingUpdated(binding) => {
                if let Some(options) = self.control_options.as_mut() {
                    options.apply_binding_change(binding, &self.bindings);
                }
                self.persist_control_bindings();
                self.status_text =
                    format!("Updated binding for “{}”", binding_display_name(binding));
            }
            ControlOptionsCommand::ResetAll => {
                self.bindings.reset_all();
                if let Some(options) = self.control_options.as_mut() {
                    options.apply_reset_all(&self.bindings);
                }
                self.persist_control_bindings();
                self.status_text = "Control bindings reset to defaults".to_string();
            }
            ControlOptionsCommand::Close => {
                if let Some(options) = self.control_options.as_mut() {
                    options.set_pointer_position(None);
                    options.cancel_rebind();
                }
                self.show_main_menu();
            }
            ControlOptionsCommand::UnsupportedKey(key) => {
                self.status_text =
                    format!("“{}” cannot be used for controls", format_key_label(key));
            }
        }
        Ok(())
    }

    fn process_about_actions(&mut self, actions: Vec<AboutAction>) -> Result<(), EngineError> {
        for action in actions {
            self.process_about_action(action)?;
        }
        Ok(())
    }

    fn process_about_action(&mut self, action: AboutAction) -> Result<(), EngineError> {
        match action {
            AboutAction::Back => {
                if let Some(dialog) = self.about_dialog.as_mut() {
                    if dialog.dialog.current_page() > 0 {
                        // Let the dialog handle it (already done in handle_key_down/button_click)
                    } else {
                        dialog.dialog.set_pointer_position(None);
                        self.show_main_menu();
                    }
                }
            }
            AboutAction::CheckForUpdates => {
                self.status_text = "Update checking not yet implemented in Rust port".to_string();
            }
            AboutAction::NextPage => {
                // Handled internally by the dialog
            }
        }
        Ok(())
    }

    fn process_lobby_action(&mut self, action: LobbyAction) -> Result<(), EngineError> {
        match action {
            LobbyAction::ToggleReady => {
                if let Some(lobby) = self.network_lobby.as_mut() {
                    let ready = lobby.toggle_local_ready();
                    self.status_text = if ready {
                        "You are ready".to_string()
                    } else {
                        "You are not ready".to_string()
                    };
                }
            }
            LobbyAction::StartGame => {
                if !matches!(self.network_mode, Some(NetworkMode::Host(_))) {
                    self.status_text = "Only the host can start the game".to_string();
                    return Ok(());
                }
                let Some(lobby) = self.network_lobby.as_ref() else {
                    return Ok(());
                };
                let Some(identifier) = lobby.selected_identifier() else {
                    self.status_text = "Select a scenario before starting".to_string();
                    return Ok(());
                };
                let scenario = match self.scenario_catalog.get(identifier).cloned() {
                    Some(scenario) => scenario,
                    None => {
                        self.status_text =
                            format!("Scenario `{}` is not available in the catalog", identifier);
                        return Ok(());
                    }
                };
                self.play_ui_sound("Click");
                self.start_scenario(scenario)?;
            }
        }
        Ok(())
    }

    fn persist_control_bindings(&self) {
        if let Ok(paths) = cached_app_paths() {
            self.bindings.save(paths.as_ref());
        }
    }

    fn handle_main_menu_activation(&mut self, item: MainMenuItem) -> Result<(), EngineError> {
        match item {
            MainMenuItem::LocalGame => {
                self.open_scenario_browser();
            }
            MainMenuItem::NetworkGame => {
                if self.network_mode.is_some() && self.network_lobby.is_some() {
                    self.open_network_lobby();
                } else {
                    // C4StartupMainDlg routes to the network game browser
                    // (C4StartupNetDlg) regardless of host/join state.
                    self.startup_view = StartupView::NetworkGame;
                    self.status_text.clear();
                }
            }
            MainMenuItem::PlayerSelection => {
                self.startup_view = StartupView::PlayerSelection;
                self.status_text.clear();
            }
            MainMenuItem::Options => {
                self.open_options_menu();
            }
            MainMenuItem::About => {
                self.open_about_dialog();
            }
            MainMenuItem::Quit => {
                self.request_exit();
            }
        }
        Ok(())
    }

    fn open_scenario_browser(&mut self) {
        self.startup_view = StartupView::ScenarioBrowser;
        self.menu_state.set_pointer_position(None);
        self.menu_state.refresh_menu_entries();
        let width = self.graphics.surface().width() as f32;
        let height = self.graphics.surface().height() as f32;
        self.menu_state.menu().resize(width, height);
        if let Err(err) = self.handle_menu_input(|menu| menu.select_default_entry()) {
            tracing::error!(error = %err, "failed to select default scenario entry");
        }
        self.scenario_label = self.menu_state.label_path();
        self.status_text.clear();
    }

    fn open_network_lobby(&mut self) {
        self.startup_view = StartupView::NetworkLobby;
        self.menu_state.set_pointer_position(None);
        self.menu_state.refresh_menu_entries();
        let width = self.graphics.surface().width() as f32;
        let height = self.graphics.surface().height() as f32;
        self.menu_state.menu().resize(width, height);
        if let Err(err) = self.handle_menu_input(|menu| menu.select_default_entry()) {
            tracing::error!(error = %err, "failed to select default scenario entry");
        }
        if let Some(lobby) = self.network_lobby.as_mut() {
            lobby.update_layout(width, height);
            self.scenario_label = lobby.scenario_label();
        } else {
            self.scenario_label = "Network lobby unavailable".to_string();
        }
        self.status_text.clear();
    }

    fn open_options_menu(&mut self) {
        let width = self.graphics.surface().width() as f32;
        let height = self.graphics.surface().height() as f32;
        let mut state = self
            .control_options
            .take()
            .unwrap_or_else(|| ControlOptionsState::new(self.assets.font_arc()));
        state.resize(width, height);
        state.refresh_from_bindings(&self.bindings);
        if state.selected_binding().is_none() {
            state.set_selected_binding(ControlBindingId::CursorLeft);
        }
        state.set_pointer_position(None);
        self.control_options = Some(state);
        self.startup_view = StartupView::Options;
        self.status_text.clear();
    }

    fn open_about_dialog(&mut self) {
        let width = self.graphics.surface().width() as f32;
        let height = self.graphics.surface().height() as f32;
        let mut dialog = StartupAboutDialog::new(self.assets.font_arc());
        dialog.resize(width, height);
        dialog.set_pointer_position(None);
        self.about_dialog = Some(AboutDialogState { dialog });
        self.startup_view = StartupView::About;
        self.status_text.clear();
    }

    fn show_main_menu(&mut self) {
        self.startup_view = StartupView::MainMenu;
        self.main_menu_state.pointer_left();
        if let Some(lobby) = self.network_lobby.as_mut() {
            lobby.pointer_left();
        }
        self.refresh_participants_label();
        self.scenario_label = self.menu_state.label_path();
        self.status_text.clear();
        if let Some(options) = self.control_options.as_mut() {
            options.set_pointer_position(None);
            options.cancel_rebind();
        }
    }

    fn refresh_participants_label(&mut self) {
        let label = match cached_app_paths() {
            Ok(paths) => load_participants_label(Some(paths.as_ref())),
            Err(_) => load_participants_label(None),
        };
        self.main_menu_state.update_participants_label(label);
    }

    fn request_exit(&mut self) {
        self.exit_requested = true;
    }

    fn take_exit_request(&mut self) -> bool {
        if self.exit_requested {
            self.exit_requested = false;
            true
        } else {
            false
        }
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
        match self.mode {
            AppMode::Running => {
                if let Some(network) = self.network.as_ref() {
                    let frame = self.engine.frame();
                    let tick = u32::try_from(frame).unwrap_or(u32::MAX);
                    network.finalize_tick(tick);
                }
                self.snapshot = self.engine.tick()?;
                self.handle_menu_requests();
                if self.snapshot.game_over && !self.game_over_handled {
                    self.handle_game_over();
                }
                self.record_current_snapshot();
                self.refresh_object_menu();
                self.refresh_focus();
                self.update_audio();
                self.maybe_emit_sync_check();
            }
            AppMode::Loading => {
                self.poll_boot_loading();
                self.poll_loading()?;
            }
            AppMode::Menu => {}
        }
        Ok(())
    }

    fn handle_menu_requests(&mut self) {
        if !matches!(self.mode, AppMode::Running) {
            return;
        }
        let local_owner = self.local_owner;
        for request in &self.snapshot.menu_requests {
            if request.owner != local_owner {
                continue;
            }
            match &request.kind {
                MenuRequestKind::Activate => {
                    if let Some(mut menu) =
                        ObjectMenuState::new(&mut self.engine, &self.snapshot, request.crew_id)
                    {
                        menu.focus_inventory_mode();
                        self.object_menu = Some(menu);
                    }
                }
                MenuRequestKind::Get { container } => {
                    if let Some(mut menu) =
                        ObjectMenuState::new(&mut self.engine, &self.snapshot, request.crew_id)
                    {
                        if menu.focus_container_mode(&mut self.engine, &self.snapshot, *container) {
                            self.object_menu = Some(menu);
                        }
                    }
                }
                MenuRequestKind::Context { .. } => {
                    if let Some(mut menu) =
                        ObjectMenuState::new(&mut self.engine, &self.snapshot, request.crew_id)
                    {
                        menu.focus_context_mode();
                        self.object_menu = Some(menu);
                    }
                }
            }
        }
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

    fn build_game_over_entries(&self) -> Vec<GameOverEntry> {
        self.snapshot
            .players
            .iter()
            .map(|state| {
                let name = if state.name.trim().is_empty() {
                    format!("Player {}", state.id)
                } else {
                    state.name.clone()
                };
                let status_outcome = match state.status {
                    PlayerStatus::Active => GameOverOutcome::Victory,
                    PlayerStatus::Eliminated | PlayerStatus::Surrendered => GameOverOutcome::Defeat,
                    PlayerStatus::Inactive | PlayerStatus::TeamSelection => {
                        GameOverOutcome::Observer
                    }
                };
                let outcome = if state.surrendered {
                    GameOverOutcome::Defeat
                } else {
                    status_outcome
                };
                let color = state
                    .color
                    .map(|RgbColor { r, g, b }| Color::opaque(r, g, b));
                GameOverEntry {
                    player_id: state.id,
                    name,
                    outcome,
                    wealth: state.wealth,
                    score: state.points,
                    value: state.value,
                    is_local: state.id == self.local_owner,
                    color,
                }
            })
            .collect()
    }

    fn dismiss_game_over_dialog(&mut self) {
        if self.game_over_dialog.take().is_some() {
            self.play_ui_sound("DoorClose");
            self.pointer_left();
        }
    }

    fn handle_game_over(&mut self) {
        let scenario_title = self
            .active_scenario
            .as_ref()
            .map(|scenario| scenario.title.clone())
            .unwrap_or_else(|| "Scenario".to_string());
        let entries = self.build_game_over_entries();
        let dialog = GameOverState::new(scenario_title.clone(), entries);
        let status_message = if dialog.subtitle().is_empty() {
            format!("{scenario_title} complete")
        } else {
            format!("{scenario_title}: {}", dialog.subtitle())
        };
        self.finish_recording();
        self.game_over_handled = true;
        self.mode = AppMode::Menu;
        self.show_main_menu();
        self.status_text = status_message;
        self.active_scenario = None;
        self.ensure_menu_music();
        self.game_over_dialog = Some(dialog);
    }

    fn maybe_emit_sync_check(&mut self) {
        let Some(local_client_id) = self
            .network
            .as_ref()
            .map(|network| network.local_client_id())
        else {
            return;
        };
        if !matches!(self.mode, AppMode::Running) {
            return;
        }
        let Ok(frame_i32) = i32::try_from(self.snapshot.frame) else {
            return;
        };
        if frame_i32 < 0 {
            return;
        }
        if frame_i32 % SYNC_CHECK_RATE as i32 != 0 {
            self.sync_checks
                .prune_before(frame_i32.saturating_sub(SYNC_CHECK_HISTORY));
            return;
        }
        let client_id = i32::try_from(local_client_id).unwrap_or(0);
        let check = self.engine.sync_check(client_id);
        if let Some((local, remote)) = self.sync_checks.record_local(check.clone()) {
            self.evaluate_sync_checks(local, remote);
        }
        let tick = u32::try_from(self.snapshot.frame).unwrap_or(u32::MAX);
        if let Some(network) = self.network.as_ref() {
            network.submit_sync_check(tick, check);
        }
        self.sync_checks
            .prune_before(frame_i32.saturating_sub(SYNC_CHECK_HISTORY));
    }

    fn poll_loading(&mut self) -> Result<(), EngineError> {
        let mut completion: Option<(FrontendScenario, Result<Scenario, String>)> = None;
        if let Some(state) = self.loading_state.as_mut() {
            loop {
                match state.receiver.try_recv() {
                    Ok(ScenarioLoadingEvent::Progress { fraction, message }) => {
                        state.update(fraction, message);
                    }
                    Ok(ScenarioLoadingEvent::Finished(result)) => {
                        state.update(1.0, "Starting scenario".to_string());
                        completion = Some((state.scenario.clone(), result));
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        completion = Some((
                            state.scenario.clone(),
                            Err("Scenario loading interrupted".to_string()),
                        ));
                        break;
                    }
                }
            }
        }

        if let Some((scenario, result)) = completion {
            self.loading_state = None;
            match result {
                Ok(data) => {
                    if let Err(message) = self.activate_loaded_scenario(scenario.clone(), data) {
                        tracing::error!(scenario = %scenario.title, error = %message, "failed to start scenario");
                        self.status_text = message;
                        self.mode = AppMode::Menu;
                        self.ensure_menu_music();
                    }
                }
                Err(message) => {
                    tracing::error!(scenario = %scenario.title, error = %message, "failed to load scenario");
                    self.status_text = message;
                    self.mode = AppMode::Menu;
                    self.ensure_menu_music();
                }
            }
        }

        Ok(())
    }

    fn poll_boot_loading(&mut self) {
        let mut material_library: Option<Option<Arc<MaterialSet>>> = None;
        if let Some(state) = self.boot_loading.as_mut() {
            match state.receiver.try_recv() {
                Ok(BootLoadingEvent::Finished(library)) => {
                    material_library = Some(library);
                }
                Err(TryRecvError::Empty) => {
                    // Still loading, do nothing
                }
                Err(TryRecvError::Disconnected) => {
                    tracing::warn!("boot loading channel disconnected");
                    material_library = Some(None);
                }
            }
        }

        if let Some(library) = material_library {
            self.boot_loading = None;
            self.material_library = library;
            self.apply_material_library();
            // A scenario load can be started before boot finishes (mode is
            // already `Loading`). Boot completion must NOT yank the app back to
            // the menu in that case: the `Menu` update arm does not poll scenario
            // loading, so doing so would strand the in-flight load forever. Stay
            // in `Loading` and let `poll_loading` carry the scenario to `Running`.
            if self.loading_state.is_none() {
                self.mode = AppMode::Menu;
                self.show_main_menu();
                self.ensure_menu_music();
                // `--sandbox`: jump straight into the built-in sandbox once boot
                // completes, so the in-game scene can be launched/captured without
                // navigating the menu. One-shot, so return_to_menu works after.
                if self.auto_start_sandbox {
                    self.auto_start_sandbox = false;
                    if let Err(err) = self.start_sandbox_scenario(FrontendScenario::fallback()) {
                        tracing::warn!(error = ?err, "failed to auto-start sandbox scenario");
                    }
                }
            }
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

        const FRAME_PREFIX: &str = "FRAME ";
        const STATUS_PREFIX: &str = "ENERGY ";

        if let Some(object) = &self.focus_snapshot {
            let default_frame = format!(
                "FRAME {:05} POS {:04}/{:04} VEL {:03}/{:03}",
                self.snapshot.frame,
                object.position.x,
                object.position.y,
                object.velocity.x,
                object.velocity.y
            );
            if overlay_text_needs_update(&self.frame_text, FRAME_PREFIX) {
                self.frame_text = default_frame;
            }

            let default_status = format!(
                "ENERGY {:03} DAMAGE {:03} OWNER {}",
                object.energy.max(0),
                object.damage.max(0),
                object.owner
            );
            if overlay_text_needs_update(&self.status_text, STATUS_PREFIX) {
                self.status_text = default_status;
            }
            self.energy_fraction = (object.energy.max(0).min(100) as f32) / 100.0;
        } else {
            let default_frame = format!("FRAME {:05}", self.snapshot.frame);
            if overlay_text_needs_update(&self.frame_text, FRAME_PREFIX) {
                self.frame_text = default_frame;
            }
            if overlay_text_needs_update(&self.status_text, STATUS_PREFIX) {
                self.status_text.clear();
            }
            self.energy_fraction = 0.0;
        }
    }

    fn render(&mut self, frame: &mut [u8]) -> Result<()> {
        match self.mode {
            AppMode::Menu => {
                let control_options = self.control_options.as_mut();
                let network_lobby = self.network_lobby.as_mut();
                let game_over_dialog = self.game_over_dialog.as_ref();
                let about_dialog = self.about_dialog.as_mut();
                render_startup_frame(
                    &mut self.graphics,
                    self.assets.as_ref(),
                    &mut self.main_menu_state,
                    &mut self.menu_state,
                    control_options,
                    self.startup_view,
                    network_lobby,
                    game_over_dialog,
                    about_dialog,
                    frame,
                );
                Ok(())
            }
            AppMode::Loading => self.render_loading(frame),
            AppMode::Running => self.render_running(frame),
        }
    }

    fn render_loading(&mut self, frame: &mut [u8]) -> Result<()> {
        {
            let surface = self.graphics.surface_mut();
            let width = surface.width() as f32;
            let height = surface.height() as f32;

            // Draw background image (matching C++ LoaderScreen::Draw)
            if let Some(background) = self.assets.menu_background() {
                let rect = lc_gui::Rect::from_origin_size(
                    GuiPoint::new(0.0, 0.0),
                    lc_gui::Size::new(width, height),
                );
                draw_image(surface, &rect, &background);
            } else {
                surface.fill(Color::opaque(16, 28, 52));
            }

            let font = self.assets.font_arc();
            let progress = if let Some(state) = self.loading_state.as_ref() {
                state.progress.clamp(0.0, 1.0)
            } else {
                0.0
            };

            // Layout constants matching C++ (lines 130-135 in C4LoaderScreen.cpp)
            let h_indent = 20.0;
            let v_indent = 20.0;
            let v_margin = 5.0;
            let progress_bar_height = 15.0;

            // Draw "Loading..." text at bottom right (matching C++ line 141)
            let loading_text = "Loading...";
            let loading_metrics = font.measure_text(loading_text, 18.0);
            let loading_x = width - h_indent - loading_metrics.width;
            let loading_y = height - v_indent - loading_metrics.height;
            font.draw_text(
                surface,
                loading_x,
                loading_y,
                loading_text,
                18.0,
                Color::new(221, 221, 221, 221), // 0xdddddddd
            );

            // Draw progress bar frame at bottom (matching C++ line 143)
            let bar_y = height - v_indent - v_margin - progress_bar_height;
            let frame_rect = Rect::new(
                h_indent as i32,
                bar_y as i32,
                (width - h_indent * 2.0) as u32,
                progress_bar_height as u32,
            );
            // Semi-transparent black frame (0x4f000000)
            let frame_fill = Color::new(0, 0, 0, 79); // 0x4f alpha
            Self::fill_rect(surface, frame_rect, frame_fill);

            // Draw progress bar fill (matching C++ line 151)
            let bar_width = (width - h_indent * 2.0 - 2.0).max(0.0);
            let progress_width = (bar_width * progress) as u32;
            if progress_width > 0 {
                let fill_rect = Rect::new(
                    (h_indent + 1.0) as i32,
                    (bar_y + 1.0) as i32,
                    progress_width,
                    (progress_bar_height - 2.0) as u32,
                );
                // Semi-transparent red (0x4fff0000)
                let progress_fill = Color::new(255, 0, 0, 79); // 0x4f alpha, red
                Self::fill_rect(surface, fill_rect, progress_fill);
            }

            // Draw progress percentage centered in bar (matching C++ line 153)
            let progress_text = format!("{}%", (progress * 100.0) as i32);
            let progress_metrics = font.measure_text(&progress_text, 18.0);
            let progress_x = (width - progress_metrics.width) * 0.5;
            let progress_y = bar_y + (progress_bar_height - progress_metrics.height) * 0.5;
            font.draw_text(
                surface,
                progress_x,
                progress_y,
                &progress_text,
                18.0,
                Color::opaque(255, 255, 255), // White
            );

            // Draw copyright/trademark text at bottom left
            let copyright_text = "LegacyClonk is a fan project based on Clonk Rage.";
            let trademark_text = "'Clonk' is a registered trademark of Matthes Bender.";
            let copyright_y = bar_y - v_margin - 40.0;
            font.draw_text(
                surface,
                h_indent,
                copyright_y,
                copyright_text,
                14.0,
                Color::new(200, 200, 200, 255),
            );
            font.draw_text(
                surface,
                h_indent,
                copyright_y + 18.0,
                trademark_text,
                14.0,
                Color::new(200, 200, 200, 255),
            );
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

    fn render_running(&mut self, frame: &mut [u8]) -> Result<()> {
        let viewports = collect_viewport_inputs(&self.snapshot, self.local_owner, self.focus_id);
        if let Some(_focus) = self.focus_snapshot.as_ref() {
            let mut players = collect_player_overlays(&self.snapshot, self.focus_id);
            self.populate_crew_portraits(&mut players);
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

        if let Some(browser) = self.save_browser.as_ref() {
            let font = self.assets.font_arc();
            {
                let surface = self.graphics.surface_mut();
                browser.render(surface, font.as_ref());
            }
        } else if let Some(menu) = self.object_menu.as_ref() {
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
            lines: Vec<MessageLineLayout>,
            has_frame: bool,
            portrait: Option<Color>,
            alignment: HorizontalAlignment,
            vertical_align: VerticalAlignment,
            base_color: Color,
            max_line_width: f32,
        }

        let font = self.assets.font_arc();
        let font_ref = font.as_ref();
        let line_height = 20.0;
        let frame_background = Color::new(12, 20, 36, 192);

        const FONT_SIZE: f32 = 18.0;
        const FRAME_PADDING: f32 = 8.0;
        const PORTRAIT_SIZE: f32 = 42.0;
        const PORTRAIT_GAP: f32 = 8.0;

        let mut prepared: Vec<PreparedMessage> = Vec::new();

        for message in &self.snapshot.hud.messages {
            if let Some(player) = message.player {
                if player != self.local_owner {
                    continue;
                }
            }

            let base_color = Color::new(
                ((message.color >> 16) & 0xff) as u8,
                ((message.color >> 8) & 0xff) as u8,
                (message.color & 0xff) as u8,
                ((message.color >> 24) & 0xff) as u8,
            );

            let mut anchor_x = if (message.flags & FLAG_X_REL) != 0 {
                surface_width * (message.offset.x as f32 / 100.0)
            } else if message.offset.x >= 0 {
                message.offset.x as f32
            } else {
                surface_width * 0.5
            };
            let mut anchor_y = if (message.flags & FLAG_Y_REL) != 0 {
                surface_height * (message.offset.y as f32 / 100.0)
            } else if message.offset.y >= 0 {
                message.offset.y as f32
            } else {
                surface_height * 0.66
            };

            if (message.flags & FLAG_HCENTER) != 0 {
                anchor_x = surface_width * 0.5;
            } else if (message.flags & FLAG_LEFT) != 0 {
                anchor_x = 32.0;
            } else if (message.flags & FLAG_RIGHT) != 0 {
                anchor_x = surface_width - 196.0;
            }

            if (message.flags & FLAG_VCENTER) != 0 {
                anchor_y = surface_height * 0.5;
            } else if (message.flags & FLAG_TOP) != 0 {
                anchor_y = 48.0;
            } else if (message.flags & FLAG_BOTTOM) != 0 {
                anchor_y = surface_height - 160.0;
            }

            let (anchor_x, anchor_y) = match message.kind {
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
                    match self.graphics.world_to_screen(owner, base_position) {
                        Some(coords) => coords,
                        None => continue,
                    }
                }
                MessageKind::Global | MessageKind::GlobalPlayer => (anchor_x, anchor_y),
            };

            let has_decoration = message
                .decoration
                .as_ref()
                .map(|decor| !decor.trim().is_empty())
                .unwrap_or(false);
            let portrait_color = message
                .portrait
                .as_ref()
                .and_then(|spec| Self::parse_portrait_color(spec))
                .or_else(|| {
                    if message.portrait.is_some() {
                        Some(Color::new(base_color.r, base_color.g, base_color.b, 255))
                    } else {
                        None
                    }
                });
            let has_frame = portrait_color.is_some() || has_decoration;

            let default_alignment = if has_frame {
                HorizontalAlignment::Left
            } else {
                HorizontalAlignment::Center
            };
            let alignment = if (message.flags & FLAG_ALIGN_LEFT) != 0 {
                HorizontalAlignment::Left
            } else if (message.flags & FLAG_ALIGN_RIGHT) != 0 {
                HorizontalAlignment::Right
            } else if (message.flags & FLAG_ALIGN_CENTER) != 0 {
                HorizontalAlignment::Center
            } else {
                default_alignment
            };
            let vertical_align = if (message.flags & FLAG_TOP) != 0 {
                VerticalAlignment::Top
            } else if (message.flags & FLAG_BOTTOM) != 0 {
                VerticalAlignment::Bottom
            } else if (message.flags & FLAG_VCENTER) != 0 {
                VerticalAlignment::Center
            } else {
                VerticalAlignment::Baseline
            };

            let mut width_hint = message.width.map(|raw| raw as f32);
            if let Some(value) = width_hint.as_mut() {
                if (message.flags & FLAG_WIDTH_REL) != 0 {
                    *value = surface_width * (*value / 100.0);
                }
            }

            let wrap_width = if (message.flags & FLAG_NO_BREAK) != 0 {
                None
            } else {
                let fallback = || {
                    let max_width = (surface_width - 10.0).min(500.0).max(50.0);
                    if has_frame {
                        if portrait_color.is_some() {
                            Some((surface_width * 0.5).clamp(50.0, max_width))
                        } else {
                            Some((surface_width - 50.0).clamp(50.0, max_width))
                        }
                    } else {
                        Some((surface_width - 50.0).clamp(50.0, max_width))
                    }
                };
                width_hint.or_else(fallback).filter(|value| *value > 0.0)
            };

            let mut units = Vec::new();
            for (idx, line) in message.lines.iter().enumerate() {
                let spans = parse_message_spans(line, base_color);
                for span in spans {
                    for segment in split_span_into_segments(span, font_ref, FONT_SIZE) {
                        if !segment.text.is_empty() {
                            units.push(MessageWordUnit::Segment(segment));
                        }
                    }
                }
                if idx + 1 < message.lines.len() {
                    units.push(MessageWordUnit::ForcedBreak);
                }
            }

            let mut lines = wrap_word_units(units, wrap_width, font_ref, FONT_SIZE);
            if lines.is_empty() {
                lines.push(MessageLineLayout {
                    segments: Vec::new(),
                    width: 0.0,
                });
            }
            let max_line_width = lines.iter().fold(0.0f32, |acc, line| acc.max(line.width));

            prepared.push(PreparedMessage {
                anchor: (anchor_x, anchor_y),
                lines,
                has_frame,
                portrait: portrait_color,
                alignment,
                vertical_align,
                base_color,
                max_line_width,
            });
        }

        if prepared.is_empty() {
            return;
        }

        {
            let surface = self.graphics.surface_mut();
            for message in prepared {
                if message.lines.is_empty() {
                    continue;
                }

                let text_height = (message.lines.len() as f32) * line_height;
                let portrait_space = if message.portrait.is_some() {
                    PORTRAIT_SIZE + PORTRAIT_GAP
                } else {
                    0.0
                };
                let text_block_width = message.max_line_width;

                if message.has_frame {
                    let frame_width =
                        (text_block_width + portrait_space + FRAME_PADDING * 2.0).max(1.0);
                    let frame_height = (text_height + FRAME_PADDING * 2.0).max(1.0);

                    let frame_x = match message.alignment {
                        HorizontalAlignment::Left => message.anchor.0,
                        HorizontalAlignment::Center => message.anchor.0 - frame_width * 0.5,
                        HorizontalAlignment::Right => message.anchor.0 - frame_width,
                    };
                    let frame_y = match message.vertical_align {
                        VerticalAlignment::Top => message.anchor.1,
                        VerticalAlignment::Center => message.anchor.1 - frame_height * 0.5,
                        VerticalAlignment::Bottom => message.anchor.1 - frame_height,
                        VerticalAlignment::Baseline => message.anchor.1,
                    };

                    let rect = Rect::new(
                        frame_x.floor() as i32,
                        frame_y.floor() as i32,
                        frame_width.ceil() as u32,
                        frame_height.ceil() as u32,
                    );

                    Self::fill_rect(surface, rect, frame_background);
                    let border = Color::new(
                        message.base_color.r.saturating_add(24),
                        message.base_color.g.saturating_add(24),
                        message.base_color.b.saturating_add(24),
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

                    let text_base_x = frame_x + FRAME_PADDING + portrait_space;
                    let mut text_y = frame_y + FRAME_PADDING;

                    for line in &message.lines {
                        let line_offset = match message.alignment {
                            HorizontalAlignment::Left => 0.0,
                            HorizontalAlignment::Center => (text_block_width - line.width) * 0.5,
                            HorizontalAlignment::Right => text_block_width - line.width,
                        };
                        let mut cursor_x = text_base_x + line_offset;
                        for segment in &line.segments {
                            font_ref.draw_text(
                                surface,
                                cursor_x,
                                text_y,
                                &segment.text,
                                FONT_SIZE,
                                segment.color,
                            );
                            cursor_x += segment.width;
                        }
                        text_y += line_height;
                    }
                } else {
                    let text_base_x = match message.alignment {
                        HorizontalAlignment::Left => message.anchor.0,
                        HorizontalAlignment::Center => message.anchor.0 - text_block_width * 0.5,
                        HorizontalAlignment::Right => message.anchor.0 - text_block_width,
                    };
                    let mut text_y = match message.vertical_align {
                        VerticalAlignment::Top => message.anchor.1,
                        VerticalAlignment::Center => message.anchor.1 - text_height * 0.5,
                        VerticalAlignment::Bottom => message.anchor.1 - text_height,
                        VerticalAlignment::Baseline => message.anchor.1,
                    };

                    for line in &message.lines {
                        let line_offset = match message.alignment {
                            HorizontalAlignment::Left => 0.0,
                            HorizontalAlignment::Center => (text_block_width - line.width) * 0.5,
                            HorizontalAlignment::Right => text_block_width - line.width,
                        };
                        let mut cursor_x = text_base_x + line_offset;
                        for segment in &line.segments {
                            font_ref.draw_text(
                                surface,
                                cursor_x,
                                text_y,
                                &segment.text,
                                FONT_SIZE,
                                segment.color,
                            );
                            cursor_x += segment.width;
                        }
                        text_y += line_height;
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

    fn start_recording_for(&mut self, scenario: &FrontendScenario) {
        if !self.recording_enabled {
            self.recording = None;
            return;
        }
        if self.recordings_dir.is_none() {
            self.recording = None;
            return;
        }
        let mut recorder = Recorder::new();
        recorder.record(&self.snapshot);
        self.recording = Some(RecordingSession::new(
            recorder,
            scenario.title.clone(),
            scenario.identifier.clone(),
            scenario.path.clone(),
        ));
    }

    fn record_current_snapshot(&mut self) {
        if !self.recording_enabled {
            return;
        }
        if let Some(session) = self.recording.as_mut() {
            session.recorder.record(&self.snapshot);
        }
    }

    fn finish_recording(&mut self) {
        let Some(session) = self.recording.take() else {
            return;
        };
        if !self.recording_enabled {
            return;
        }
        let base_name = session.sanitized_base_name();
        let RecordingSession {
            recorder,
            scenario_title,
            scenario_identifier,
            scenario_path,
            started_at,
        } = session;
        let recording = recorder.into_recording();
        if recording.is_empty() {
            return;
        }
        let Some(dir) = self.recordings_dir.as_ref() else {
            return;
        };
        match self.write_scenario_recording(
            dir,
            base_name,
            scenario_title,
            scenario_identifier,
            scenario_path,
            started_at,
            recording,
        ) {
            Ok(path) => {
                tracing::info!(path = %path.display(), "saved scenario recording");
            }
            Err(err) => {
                tracing::warn!(error = %err, "failed to save scenario recording");
            }
        }
    }

    fn write_scenario_recording(
        &self,
        dir: &Path,
        base_name: String,
        scenario_title: String,
        scenario_identifier: String,
        scenario_path: Option<PathBuf>,
        started_at: SystemTime,
        recording: Recording,
    ) -> io::Result<PathBuf> {
        fs::create_dir_all(dir)?;
        let index = next_recording_index(dir)?;
        let filename = format!("{index:03}-{base_name}.json");
        let path = dir.join(filename);
        let frames = recording.into_frames();
        let frame_count = frames.len() as u64;
        let started_at_unix_millis = started_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let record_file = ScenarioRecordingFile {
            version: RECORD_FILE_VERSION,
            scenario_title,
            scenario_identifier,
            scenario_path: scenario_path.map(|path| path.display().to_string()),
            started_at_unix_millis,
            frame_count,
            frames,
        };
        let mut file = File::create(&path)?;
        serde_json::to_writer_pretty(&mut file, &record_file)?;
        file.flush()?;
        Ok(path)
    }

    fn return_to_menu(&mut self) {
        self.finish_recording();
        self.close_ingame_menu();
        self.object_menu = None;
        self.save_browser = None;
        self.save_browser_return_to_menu = false;
        self.game_over_dialog = None;
        self.engine = Engine::new();
        self.apply_material_library();
        self.input = InputDispatcher::new();
        self.ingame_pointer = None;
        self.mouse_state = None;
        self.sky = None;
        self.snapshot = self.engine.snapshot();
        self.sync_checks.clear();
        self.refresh_object_menu();
        self.focus_id = None;
        self.focus_snapshot = None;
        self.frame_text.clear();
        self.status_text.clear();
        self.energy_fraction = 0.0;
        self.active_scenario = None;
        self.loading_state = None;
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
            self.assets.hud_graphics(),
        );
        self.graphics.surface_mut().fill(Color::opaque(16, 28, 52));
        self.graphics.set_sky(self.sky.clone());

        self.menu_state.set_pointer_position(None);
        self.menu_state.refresh_menu_entries();
        let width_f = width as f32;
        let height_f = height as f32;
        self.menu_state.menu().resize(width_f, height_f);
        self.main_menu_state.resize(width_f, height_f);

        self.mode = AppMode::Menu;
        self.show_main_menu();
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
        self.game_over_dialog = None;
        if scenario.path.is_none() {
            return self.start_sandbox_scenario(scenario);
        }
        self.begin_loading_scenario(scenario)
    }

    fn begin_loading_scenario(&mut self, scenario: FrontendScenario) -> Result<(), EngineError> {
        let path = scenario
            .path
            .clone()
            .expect("scenario path must be present when starting load");
        tracing::info!(
            scenario = %scenario.title,
            path = %path.display(),
            "starting asynchronous scenario load"
        );

        let resolver_paths = cached_app_paths().ok();
        let scenario_title = scenario.title.clone();
        let (sender, receiver) = mpsc::channel();
        let path_for_thread = path.clone();

        thread::spawn(move || {
            let resolver = InstallDefinitionResolver::new(resolver_paths);
            let send_progress = |fraction: f32, message: &str| {
                let _ = sender.send(ScenarioLoadingEvent::Progress {
                    fraction,
                    message: message.to_string(),
                });
            };

            send_progress(0.05, "Reading scenario data");
            let scenario_data = Scenario::load_from_path_with(&path_for_thread, &resolver)
                .map_err(|err| err.to_string());

            match scenario_data {
                Ok(data) => {
                    send_progress(0.7, "Preparing scenario resources");
                    let _ = sender.send(ScenarioLoadingEvent::Finished(Ok(data)));
                }
                Err(err) => {
                    let message = format!("Failed to load {}: {}", scenario_title, err);
                    let _ = sender.send(ScenarioLoadingEvent::Finished(Err(message)));
                }
            }
        });

        if let Some(audio) = self.audio.as_mut() {
            audio.stop_music();
        }
        self.status_text.clear();
        self.loading_state = Some(ScenarioLoadingState::new(scenario, receiver));
        self.mode = AppMode::Loading;
        Ok(())
    }

    fn activate_loaded_scenario(
        &mut self,
        scenario: FrontendScenario,
        scenario_data: Scenario,
    ) -> Result<(), String> {
        self.finish_recording();
        let path = scenario
            .path
            .clone()
            .ok_or_else(|| format!("Scenario `{}` is missing a filesystem path", scenario.title))?;

        tracing::info!(
            scenario = %scenario.title,
            path = %path.display(),
            "applying loaded scenario"
        );

        let mut engine = Engine::new();
        self.apply_material_library_to(&mut engine);

        if let Err(err) = scenario_data.apply(&mut engine) {
            tracing::error!(
                scenario = %scenario.title,
                path = %path.display(),
                error = %err,
                error_debug = ?err,
                "failed to apply scenario"
            );
            return Err(format!("Failed to start {}: {err}", scenario.title));
        }

        if let Some(description) = scenario_data.description() {
            engine.show_scenario_intro(description);
        }

        self.engine = engine;
        self.input = InputDispatcher::new();
        self.ingame_pointer = None;
        self.mouse_state = None;
        if let Some(audio) = self.audio.as_mut() {
            audio.configure_scenario(Some(&path));
            audio.reset_sfx();
            scenario_data.visit_definition_groups(|id, group| {
                audio.register_definition_sounds(id, group);
            });
        }

        if let Err(err) = self.ensure_local_player_registered() {
            tracing::error!(
                scenario = %scenario.title,
                path = %path.display(),
                error = %err,
                "failed to register local player"
            );
            return Err(format!("Failed to start {}: {err}", scenario.title));
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
        self.play_scenario_audio(&path);
        self.status_text.clear();
        self.start_recording_for(&scenario);
        Ok(())
    }

    fn start_sandbox_scenario(&mut self, scenario: FrontendScenario) -> Result<(), EngineError> {
        tracing::info!(
            scenario = %scenario.title,
            "starting sandbox fallback scenario"
        );

        self.finish_recording();
        self.loading_state = None;
        self.engine = Engine::new();
        self.apply_material_library();
        self.input = InputDispatcher::new();
        self.ingame_pointer = None;
        self.mouse_state = None;
        self.sky = None;
        if let Some(audio) = self.audio.as_mut() {
            audio.configure_scenario(None);
            audio.reset_sfx();
        }

        let spawn_definition = match self.audio.as_mut() {
            Some(audio) => configure_sandbox_engine(&mut self.engine, Some(audio))?,
            None => configure_sandbox_engine(&mut self.engine, None)?,
        };

        self.ensure_local_player_registered()?;

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
        let dir = ensure_save_directory()?;
        let path = dir.join(QUICK_SAVE_FILE);
        self.perform_named_save("Quick Save", Some(path))?;
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

        self.load_saved_game_from_path(&path)?;
        Ok(())
    }

    fn apply_loaded_game(&mut self, save: SavedGameFile) -> Result<()> {
        let scenario_info = save.scenario.clone();
        let frontend = scenario_info.to_frontend();

        self.finish_recording();
        self.engine = Engine::new();
        self.apply_material_library();
        self.input = InputDispatcher::new();
        self.ingame_pointer = None;
        self.mouse_state = None;

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

        self.start_recording_for(&frontend);
        self.scenario_catalog
            .insert(frontend.identifier.clone(), frontend.clone());

        self.status_text = format!("Loaded {}", scenario_info.title);
        Ok(())
    }

    fn play_ui_sound(&mut self, name: &str) {
        if let Some(audio) = self.audio.as_mut() {
            audio.play_ui_sound(name);
        }
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
            self.assets.hud_graphics(),
        );
        self.graphics.surface_mut().fill(Color::opaque(12, 24, 40));
        self.graphics.set_sky(self.sky.clone());
        self.frame_text.clear();
        self.status_text.clear();
        self.energy_fraction = 0.0;
        self.sync_checks.clear();
        self.menu_state.set_pointer_position(None);
        self.object_menu = None;
        self.ingame_menu = None;
        self.game_over_handled = false;
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

/// The startup render gamma ramp (default config: identity + black floor).
fn startup_gamma() -> &'static lc_graphics::GammaRamp {
    static STARTUP_GAMMA: std::sync::OnceLock<lc_graphics::GammaRamp> = std::sync::OnceLock::new();
    STARTUP_GAMMA.get_or_init(lc_graphics::GammaRamp::standard)
}

fn render_startup_frame(
    graphics: &mut GraphicsSystem,
    assets: &FrontendAssets,
    main_menu: &mut MainMenuState,
    scenario_menu: &mut MenuState,
    control_options: Option<&mut ControlOptionsState>,
    view: StartupView,
    network_lobby: Option<&mut NetworkLobbyState>,
    game_over: Option<&GameOverState>,
    about_dialog: Option<&mut AboutDialogState>,
    frame: &mut [u8],
) {
    {
        let surface = graphics.surface_mut();
        let background = match view {
            StartupView::ScenarioBrowser | StartupView::NetworkLobby => {
                assets.scenario_browser_background()
            }
            StartupView::Options => assets.options_background(),
            StartupView::About => assets.about_background(),
            _ => assets.menu_background(),
        };
        if let Some(background) = background {
            // C++ stretches the loader fullscreen with GL_LINEAR filtering
            // (C4Facet::DrawFullScreen, C4Facet.cpp:130-140; StdGL.cpp:528-532).
            let rect = lc_gui::Rect::from_origin_size(
                GuiPoint::new(0.0, 0.0),
                lc_gui::Size::new(surface.width() as f32, surface.height() as f32),
            );
            lc_frontend::draw_image_bilinear(surface, &rect, &background, Some(startup_gamma()));
        } else {
            surface.fill(Color::opaque(16, 28, 52));
        }
        match view {
            // Renderers land per-dialog; until wired these views show the
            // bare background.
            StartupView::NetworkGame | StartupView::PlayerSelection => {}
            StartupView::MainMenu => {
                main_menu.render(surface);
                // Logo + version line per C4StartupMainDlg::DrawElement
                // (C4StartupMainDlg.cpp:111-122), in C++ integer math.
                if let Some(logo) = assets.logo() {
                    let width = surface.width() as i32;
                    let height = surface.height() as i32;
                    let logo_w = (0.4 * logo.width() as f32) as i32;
                    let logo_h = (0.4 * logo.height() as f32) as i32;
                    let logo_x = width * 30 / 31 - logo_w;
                    let logo_y = height / 21 - 5;
                    let logo_rect = lc_gui::Rect::new(
                        logo_x as f32,
                        logo_y as f32,
                        logo_w as f32,
                        logo_h as f32,
                    );
                    lc_frontend::draw_image_bilinear(
                        surface,
                        &logo_rect,
                        &logo,
                        Some(startup_gamma()),
                    );

                    // "Version %s" with C4VERSION = "4.9.11.0 [362] " (trailing
                    // space from empty C4VERSIONEXTRA/C4BUILDOPT, C4Version.h:55),
                    // right-aligned at (Wdt*39/40, Hgt/18 + 0.4*logoHgt) in the
                    // GUI TextFont, white, markup on (C4StartupMainDlg.cpp:121).
                    let version_text = "Version 4.9.11.0 [362] ";
                    let version_x = width * 39 / 40;
                    let version_y = height / 18 + logo_h;
                    if let Some(fonts) = assets.clonk_fonts.as_ref() {
                        fonts.text.draw_with_gamma(
                            surface,
                            version_x,
                            version_y,
                            version_text,
                            [255, 255, 255, 255],
                            lc_graphics::clonk_font::TextAlign::Right,
                            true,
                            Some(startup_gamma()),
                        );
                    } else {
                        let font = assets.font_arc();
                        let metrics = font.measure_text(version_text, 14.0);
                        font.draw_text(
                            surface,
                            (version_x as f32) - metrics.width,
                            version_y as f32,
                            version_text,
                            14.0,
                            Color::new(255, 255, 255, 255),
                        );
                    }
                }
            }
            StartupView::ScenarioBrowser => scenario_menu.menu().render(surface),
            StartupView::NetworkLobby => scenario_menu.menu().render(surface),
            StartupView::Options => {
                if let Some(options) = control_options {
                    options.render(surface);
                }
            }
            StartupView::About => {
                if let Some(about) = about_dialog {
                    about.dialog.render(surface);
                }
            }
        }
        if matches!(view, StartupView::NetworkLobby) {
            if let Some(lobby) = network_lobby {
                lobby.render_overlay(surface, assets);
            }
        }
        if let Some(dialog) = game_over {
            let font = assets.font_arc();
            dialog.render(surface, font.as_ref());
        }

        // The C++ blit shader applies the gamma ramp to every fragment
        // (StdGL.cpp:1082-1086, UseShaderGamma default on). The filtered
        // (bilinear) draws already encode through the ramp; this final pass
        // covers the unfiltered ones (planks, text), where the default ramp
        // is identity apart from the black floor — so re-applying it to
        // already-encoded pixels is a no-op.
        startup_gamma().apply_to_surface(surface);
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
            let (label, energy_fraction, is_focus) =
                if let Some(object) = snapshot.object(*object_id) {
                    let label = format!("{} #{}", object.definition_id, object.id.as_u64());
                    let energy_fraction = (object.energy.max(0).min(100) as f32) / 100.0;
                    let is_focus = focus_id == Some(object.id) || cursor == Some(object.id);
                    (label, energy_fraction, is_focus)
                } else {
                    let label = format!("Object #{}", object_id.as_u64());
                    (label, 0.0, false)
                };
            crew.push(CrewOverlay {
                object_id: *object_id,
                label,
                energy_fraction,
                is_focus,
                portrait: None,
            });
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
        let score = detail_map
            .get(&player.owner)
            .map(|state| state.points)
            .unwrap_or(0);
        let owner_color = detail_map
            .get(&player.owner)
            .and_then(|state| state.color.map(|rgb| Color::opaque(rgb.r, rgb.g, rgb.b)))
            .unwrap_or_else(|| default_owner_color(player.owner));
        players.push(PlayerOverlay {
            owner: player.owner,
            name,
            wealth,
            score,
            cursor,
            eliminated: player.eliminated,
            owner_color,
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

/// Script errors raised while executing a control are shown and survived in
/// C++ (ErrorOrWarning → C4AulExecError::show, C4AulExec.cpp:1345-1361); only
/// engine-model errors stay fatal. Returns the status-line message to show.
fn control_script_error_to_status(err: EngineError) -> Result<String, EngineError> {
    match err {
        EngineError::Script { ref source, .. } => Ok(format!("Script error: {err}: {source}")),
        other => Err(other),
    }
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

fn drag_direction_buttons(delta: FloatVector2) -> Vec<ControlButton> {
    let mut buttons = Vec::new();
    if delta.x.abs() >= MIN_THROW_DRAG_DISTANCE {
        if delta.x > 0.0 {
            buttons.push(ControlButton::Right);
        } else {
            buttons.push(ControlButton::Left);
        }
    }
    if delta.y.abs() >= MIN_THROW_DRAG_DISTANCE {
        if delta.y > 0.0 {
            buttons.push(ControlButton::Down);
        } else {
            buttons.push(ControlButton::Up);
        }
    }
    buttons
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
    if let Some(content) = paths.content_dir() {
        bases.push(content.to_path_buf());
    }
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
            let existing_roots: Vec<_> = roots.iter().filter(|root| root.path.exists()).collect();
            if !existing_roots.is_empty() {
                let mut combined_entries: Vec<(resource_scenario::ScenarioEntry, String)> =
                    Vec::new();
                for root in existing_roots {
                    match resource_scenario::discover(&root.path) {
                        Ok(entries) => combined_entries
                            .extend(entries.into_iter().map(|entry| (entry, root.label.clone()))),
                        Err(err) => {
                            tracing::warn!(
                                error = %err,
                                path = %root.path.display(),
                                "failed to discover scenarios from install root"
                            );
                        }
                    }
                }

                if !combined_entries.is_empty() {
                    let mut scenarios = Vec::new();
                    for (entry, label) in combined_entries {
                        scenarios.push(FrontendScenario::from_resource(entry, &label));
                    }
                    if !scenarios.is_empty() {
                        return merge_frontend_scenarios(scenarios);
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

struct ScenarioRoot {
    path: PathBuf,
    label: String,
}

fn scenario_roots(paths: &AppPaths) -> Vec<ScenarioRoot> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    push_root(&mut roots, &mut seen, paths.scenario_dir(), "Scenarios");
    if let Some(content) = paths.content_dir() {
        push_root(&mut roots, &mut seen, content.to_path_buf(), "Scenarios");
    }
    push_root(
        &mut roots,
        &mut seen,
        paths.install_root().join("Scenarios"),
        "Scenarios",
    );
    push_root(
        &mut roots,
        &mut seen,
        paths.install_root().join("scenarios"),
        "Scenarios",
    );
    push_root(
        &mut roots,
        &mut seen,
        paths.planet_dir().to_path_buf(),
        "Scenarios",
    );
    push_root(
        &mut roots,
        &mut seen,
        paths.system_group_path().to_path_buf(),
        "System",
    );
    roots
}

fn push_root(
    roots: &mut Vec<ScenarioRoot>,
    seen: &mut HashSet<String>,
    path: PathBuf,
    label: &str,
) {
    let key = scenario_root_key(&path);
    if !seen.insert(key) {
        return;
    }
    roots.push(ScenarioRoot {
        path,
        label: label.to_string(),
    });
}

fn scenario_root_key(path: &Path) -> String {
    let mut key = String::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => {
                key.push_str(&prefix.as_os_str().to_string_lossy().replace('\\', "/"));
            }
            Component::RootDir => {
                if !key.ends_with('/') {
                    key.push('/');
                }
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !key.ends_with('/') && !key.is_empty() {
                    key.push('/');
                }
                key.push_str("..");
            }
            Component::Normal(part) => {
                if !key.ends_with('/') && !key.is_empty() {
                    key.push('/');
                }
                key.push_str(&part.to_string_lossy());
            }
        }
    }
    if key.is_empty() {
        key.push('.');
    }
    if cfg!(windows) || cfg!(target_os = "macos") {
        key = key.to_ascii_lowercase();
    }
    key
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
    DATA.get_or_init(|| {
        load_menu_music().unwrap_or_else(|err| {
            tracing::warn!(error = %err, "failed to load menu music, no music will play");
            Vec::new()
        })
    })
    .as_slice()
}

fn load_menu_music() -> Result<Vec<u8>> {
    let paths = AppPaths::discover()?;
    let music_group_path = find_music_group(&paths)?;
    let group = Group::open(&music_group_path).with_context(|| {
        format!(
            "failed to open music group at {}",
            music_group_path.display()
        )
    })?;

    // Try Frontend.ogg first (main menu music in C++)
    let music_data = group
        .read_file(Path::new("Frontend.ogg"))
        .or_else(|_| group.read_file(Path::new("frontend.ogg")))
        .context("failed to read Frontend.ogg from music group")?;

    Ok(music_data)
}

fn find_music_group(paths: &AppPaths) -> Result<PathBuf> {
    let mut search_roots = vec![
        paths.install_root().to_path_buf(),
        paths.planet_dir().to_path_buf(),
        paths.user_data_dir().to_path_buf(),
    ];
    if let Some(content) = paths.content_dir() {
        search_roots.push(content.to_path_buf());
    }

    for root in search_roots {
        for name in ["Music.c4g", "music.c4g", "Music.ocg", "music.ocg"] {
            let candidate = root.join(name);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
    }

    Err(anyhow!("Music.c4g not found in standard directories"))
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
        ActionState, CommandDirection, CommandStackSnapshot, Direction, EnvironmentFrame,
        HudPlayerSnapshot, HudSnapshot, ObjectId, ObjectSnapshot, ObjectStatus, PlayerState,
        PlayerStatus, ScriptError, SimulationSnapshot, Vector2, DEFAULT_CATEGORY,
    };
    use parking_lot::ReentrantMutex;
    use std::collections::HashMap;
    use std::env;
    use std::ffi::OsString;
    use std::fs;
    use std::io::BufWriter;
    use std::path::Path;
    use std::sync::OnceLock;
    use std::thread;
    use std::time::Duration;
    use tempfile::tempdir;

    #[test]
    fn overlay_text_helper_respects_custom_text() {
        assert!(overlay_text_needs_update("", "FRAME "));
        assert!(overlay_text_needs_update("FRAME 00005", "FRAME "));
        assert!(!overlay_text_needs_update("Inventory open", "FRAME "));

        assert!(overlay_text_needs_update("", "ENERGY "));
        assert!(overlay_text_needs_update(
            "ENERGY 100 DAMAGE 000 OWNER 1",
            "ENERGY "
        ));
        assert!(!overlay_text_needs_update("Paused", "ENERGY "));
    }

    /// Pump the app until asynchronous boot loading completes and the main menu
    /// is shown. A freshly constructed `GameApp` starts in `AppMode::Loading`
    /// while the boot/material-library thread runs; it transitions to
    /// `AppMode::Menu` only after `update()` polls the boot completion. Panics if
    /// it never settles, so a genuinely stuck boot still fails the test.
    fn wait_for_menu(app: &mut GameApp) {
        for _ in 0..480 {
            if matches!(app.mode, AppMode::Menu) {
                return;
            }
            app.update().expect("tick while waiting for boot to finish");
            thread::sleep(Duration::from_millis(2));
        }
        panic!("app did not reach menu mode in time");
    }

    fn wait_for_running(app: &mut GameApp) {
        for _ in 0..480 {
            if matches!(app.mode, AppMode::Running) {
                return;
            }
            app.update()
                .expect("tick while waiting for scenario to start");
            thread::sleep(Duration::from_millis(2));
        }
        panic!("scenario did not enter running mode in time");
    }

    #[test]
    fn control_script_errors_are_non_fatal_like_cpp() {
        // C++ surfaces control-time script errors and keeps the session
        // alive (ErrorOrWarning → C4AulExecError::show, C4AulExec.cpp:
        // 1345-1361); only the offending call fails, the game keeps running.
        let script_error = EngineError::Script {
            definition: "CLNK".into(),
            function: "Control",
            source: ScriptError::parse("boom", 1, 1),
        };
        let status = control_script_error_to_status(script_error)
            .expect("script errors downgrade to a status message");
        assert!(
            status.contains("CLNK"),
            "status names the definition: {status}"
        );

        let fatal = EngineError::CrewSelection {
            owner: 0,
            detail: "broken".into(),
        };
        control_script_error_to_status(fatal).expect_err("engine-model errors stay fatal");
    }

    fn write_preview_png(path: &Path, pixel: [u8; 4]) {
        let file = File::create(path).expect("create preview image");
        let writer = BufWriter::new(file);
        let mut encoder = Encoder::new(writer, 1, 1);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        let mut writer = encoder.write_header().expect("write PNG header");
        writer
            .write_image_data(&pixel)
            .expect("write PNG pixel data");
    }

    fn make_object(id: u64, definition: &str, position: Vector2) -> ObjectSnapshot {
        ObjectSnapshot {
            id: ObjectId::new(id),
            definition_id: definition.to_string(),
            position,
            velocity: Vector2::new(0, 0),
            rotation: 0,
            energy: 100,
            construction: lc_engine::FULL_CON,
            damage: 0,
            magic_energy: 0,
            magic_capacity: 0,
            action: ActionState::default(),
            direction: Direction::default(),
            command_direction: CommandDirection::default(),
            action_procedure: None,
            effects: Vec::new(),
            vertices: Vec::new(),
            own_vertices: None,
            container: None,
            contents: Vec::new(),
            components: HashMap::new(),
            status: ObjectStatus::Normal,
            owner: 1,
            category: DEFAULT_CATEGORY,
            crew_member: true,
            alive: true,
            base_graphics: None,
            graphics_overlays: Vec::new(),
            draw_transform: None,
            command_queue: Vec::new(),
            command_stack: CommandStackSnapshot::default(),
            local_vars: HashMap::new(),
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
            game_over: false,
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
            rng: lc_engine::LcgRng::seed_from_u64(1),
            surfaces: Vec::new(),
            hud: HudSnapshot {
                players: hud_players,
                messages: Vec::new(),
            },
            controls: Vec::new(),
            network_packets: Vec::new(),
            definition_categories: HashMap::new(),
            transfer_zones: Vec::new(),
            menu_requests: Vec::new(),
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
                wealth: 0,
                score: 0,
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
                wealth: 0,
                score: 0,
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
                wealth: 0,
                score: 0,
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
                root_label: None,
                is_editable: false,
                is_playable: true,
                label: "Test".to_string(),
                fallback_ground: 0,
                sandbox: true,
            },
            focus_id: None,
            user_label: None,
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
            root_label: None,
            preview: None,
            children: Vec::new(),
            folder_index: None,
            icon_index: None,
            difficulty: None,
        };

        let folder = FrontendScenario {
            identifier: "folder_missions".to_string(),
            title: "Missions".to_string(),
            description: Some("Mission pack".to_string()),
            kind: ScenarioKind::Folder,
            is_editable: false,
            is_playable: false,
            path: None,
            root_label: None,
            preview: None,
            children: vec![child],
            folder_index: None,
            icon_index: None,
            difficulty: None,
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
                rotation: 0,
                energy: 80,
                construction: lc_engine::FULL_CON,
                damage: 0,
                magic_energy: 0,
                magic_capacity: 0,
                action: ActionState::default(),
                direction: Direction::default(),
                command_direction: CommandDirection::default(),
                action_procedure: None,
                effects: Vec::new(),
                vertices: Vec::new(),
                own_vertices: None,
                container: None,
                contents: Vec::new(),
                components: HashMap::new(),
                status: ObjectStatus::Normal,
                owner: 1,
                category: DEFAULT_CATEGORY,
                crew_member: true,
                alive: true,
                base_graphics: None,
                graphics_overlays: Vec::new(),
                draw_transform: None,
                command_queue: Vec::new(),
                command_stack: CommandStackSnapshot::default(),
                local_vars: HashMap::new(),
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
            },
            ObjectSnapshot {
                id: teammate,
                definition_id: "Balloon".into(),
                position: Vector2::new(10, 0),
                velocity: Vector2::ZERO,
                rotation: 0,
                energy: 40,
                construction: lc_engine::FULL_CON,
                damage: 0,
                magic_energy: 0,
                magic_capacity: 0,
                action: ActionState::default(),
                direction: Direction::default(),
                command_direction: CommandDirection::default(),
                action_procedure: None,
                effects: Vec::new(),
                vertices: Vec::new(),
                own_vertices: None,
                container: None,
                contents: Vec::new(),
                components: HashMap::new(),
                status: ObjectStatus::Normal,
                owner: 1,
                category: DEFAULT_CATEGORY,
                crew_member: true,
                alive: true,
                base_graphics: None,
                graphics_overlays: Vec::new(),
                draw_transform: None,
                command_queue: Vec::new(),
                command_stack: CommandStackSnapshot::default(),
                local_vars: HashMap::new(),
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
            },
        ];

        let mut snapshot = SimulationSnapshot {
            frame: 0,
            game_over: false,
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
            rng: lc_engine::LcgRng::seed_from_u64(1),
            surfaces: Vec::new(),
            hud: HudSnapshot {
                players: vec![HudPlayerSnapshot {
                    owner: 1,
                    crew: vec![focus, teammate],
                    focus: Some(focus),
                    eliminated: false,
                    wealth: 120,
                    score: 0,
                }],
                messages: Vec::new(),
            },
            controls: Vec::new(),
            network_packets: Vec::new(),
            definition_categories: HashMap::new(),
            transfer_zones: Vec::new(),
            menu_requests: Vec::new(),
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
        assert_eq!(player.owner_color, default_owner_color(1));

        let mut focused = player
            .crew
            .iter()
            .filter(|crew| crew.is_focus)
            .collect::<Vec<_>>();
        assert_eq!(focused.len(), 1, "only cursor object highlighted");
        let focus_entry = focused.pop().expect("focus highlight present");
        assert!(focus_entry.label.contains("Clonk"));
        assert!((focus_entry.energy_fraction - 0.8).abs() < f32::EPSILON);
        assert_eq!(focus_entry.object_id, focus);
        assert!(focus_entry.portrait.is_none());

        let other_entry = player
            .crew
            .iter()
            .find(|crew| crew.label.contains("Balloon"))
            .expect("non-focus crew present");
        assert!(!other_entry.is_focus);
        assert!((other_entry.energy_fraction - 0.4).abs() < f32::EPSILON);
        assert_eq!(other_entry.object_id, teammate);
        assert!(other_entry.portrait.is_none());
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
            root_label: Some("Scenarios".into()),
            preview: None,
            children: Vec::new(),
            folder_index: None,
            icon_index: None,
            difficulty: None,
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
        let root_entries = build_menu_entries(state.current_entries(), true);
        assert_eq!(root_entries.len(), 2);
        assert_eq!(root_entries[0].identifier, BACK_ENTRY_IDENTIFIER);
        assert_eq!(root_entries[1].identifier, "folder_missions");
        assert_eq!(state.label_path(), "Scenarios".to_string());
        state.refresh_menu_entries();
        let root_selection = state.select_default_entry();
        assert!(
            matches!(
                root_selection.as_slice(),
                [StartupMenuAction::SelectionChanged(summary)]
                if summary.identifier == "folder_missions"
            ),
            "expected default selection to target folder_missions"
        );

        state.enter_folder("folder_missions");
        assert_eq!(state.current_entries().len(), 1);
        assert_eq!(state.stack.len(), 2);
        let folder_entries = build_menu_entries(state.current_entries(), true);
        assert_eq!(folder_entries.len(), 2);
        assert_eq!(folder_entries[0].identifier, BACK_ENTRY_IDENTIFIER);
        assert_eq!(folder_entries[1].identifier, "scenario_alpha");
        assert_eq!(state.label_path(), "Scenarios / Missions".to_string());
        let folder_selection = state.select_default_entry();
        assert!(
            matches!(
                folder_selection.as_slice(),
                [StartupMenuAction::SelectionChanged(summary)]
                if summary.identifier == "scenario_alpha"
            ),
            "expected default selection to target scenario_alpha"
        );

        state.leave_folder();
        assert_eq!(state.current_entries().len(), 1);
        assert_eq!(state.stack.len(), 1);
        let root_again = build_menu_entries(state.current_entries(), true);
        assert_eq!(root_again.len(), 2);
        assert_eq!(root_again[0].identifier, BACK_ENTRY_IDENTIFIER);
        assert_eq!(root_again[1].identifier, "folder_missions");
        assert_eq!(state.label_path(), "Scenarios".to_string());
        let root_again_selection = state.select_default_entry();
        assert!(
            root_again_selection.is_empty()
                || matches!(
                    root_again_selection.as_slice(),
                    [StartupMenuAction::SelectionChanged(summary)]
                    if summary.identifier == "folder_missions"
                ),
            "expected default selection to target folder_missions after returning to root"
        );
    }

    #[test]
    fn sandbox_music_is_decodable() {
        let audio = sandbox_music_bytes();
        let decoded = decode_audio(audio).expect("sandbox music decodes");
        assert_eq!(decoded.sample_rate, 44_100);
        assert!(decoded.frames.len() > 2_000);
    }

    #[test]
    fn menu_dump_writes_main_menu_png_at_1280x720() {
        lc_core::logging::init();

        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("menu.png");
        run_menu_dump(
            &path,
            "main",
            None,
            RuntimeConfig {
                player_owner: 1,
                player_name: "Player".to_string(),
                network: None,
                record_enabled: false,
            },
        )
        .expect("dump startup menu frame");

        // PNG IHDR: width/height are big-endian u32 at byte offsets 16/20.
        let png = fs::read(&path).expect("read dumped png");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "not a PNG file");
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        assert_eq!((width, height), (1280, 720));
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
                player_name: "Player".to_string(),
                network: None,
                record_enabled: false,
            },
        )
        .expect("initialise app with audio");

        // Menu music is started by `ensure_menu_music()` when asynchronous boot
        // loading completes and the menu is shown; pump boot to that point first.
        wait_for_menu(&mut app);

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
                    player_name: "Player".to_string(),
                    network: None,
                    record_enabled: false,
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
            let thumbnail_path = resolve_save_directory().join("quicksave.png");
            assert!(
                thumbnail_path.exists(),
                "expected quick save thumbnail to be written"
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
                        player_name: "Player".to_string(),
                        network: None,
                        record_enabled: false,
                    },
                )
                .expect("initialise app");

                let scenario = app
                    .scenario_catalog
                    .get("Alpha.c4s")
                    .cloned()
                    .expect("scenario discovered");
                app.start_scenario(scenario).expect("start disk scenario");
                wait_for_running(&mut app);

                for _ in 0..5 {
                    app.update().expect("advance simulation before save");
                }
                let frame_before_save = app.snapshot.frame;

                app.quick_save().expect("quick save succeeds");
                assert!(
                    quicksave_path.exists(),
                    "expected quick save file to be written"
                );
                assert!(
                    quicksave_path.with_extension("png").exists(),
                    "expected quick save thumbnail to be written"
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
                        player_name: "Player".to_string(),
                        network: None,
                        record_enabled: false,
                    },
                )
                .expect("initialise app after restart");

                // Boot loading is asynchronous; let it settle to the menu before
                // asserting the fresh session is at the menu (not mid-game).
                wait_for_menu(&mut app);

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

        reset_cached_app_paths();
    }

    #[test]
    fn load_frontend_scenarios_discovers_repository_content() {
        let _env_lock = crate::tests::env_lock().lock();
        reset_cached_app_paths();

        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let install_root = manifest_dir
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .expect("repository root");

        let _guard = EnvGuard::set(&[("LC_INSTALL_ROOT", Some(install_root))]);
        let scenarios = load_frontend_scenarios();

        assert!(
            scenarios
                .iter()
                .any(|scenario| scenario.identifier != "rust_sandbox"),
            "expected repository content scenarios to be discoverable"
        );

        reset_cached_app_paths();
    }

    #[test]
    fn load_frontend_scenarios_prefers_user_over_install() {
        reset_cached_app_paths();

        let install_dir = tempdir().unwrap();

        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let install_scenario_dir = install_dir.path().join("Scenarios").join("Alpha.c4s");
        fs::create_dir_all(&install_scenario_dir).unwrap();
        fs::write(
            install_scenario_dir.join("Scenario.json"),
            br#"{"name":"System Alpha"}"#,
        )
        .unwrap();

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();
        let user_scenario_dir = user_dir.join("Scenarios").join("Alpha.c4s");
        fs::create_dir_all(&user_scenario_dir).unwrap();
        fs::write(
            user_scenario_dir.join("Scenario.json"),
            br#"{"name":"User Alpha"}"#,
        )
        .unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);

        let scenarios = load_frontend_scenarios();
        assert_eq!(scenarios.len(), 1, "duplicate scenario should be merged");
        let scenario = &scenarios[0];
        assert_eq!(scenario.identifier, "Alpha.c4s");
        assert_eq!(
            scenario.title, "User Alpha",
            "user scenario should override install variant"
        );
        let path = scenario.path.as_ref().expect("scenario path");
        assert!(
            path.starts_with(&user_dir),
            "expected scenario path to point at user overrides"
        );

        reset_cached_app_paths();
    }

    #[test]
    fn load_frontend_scenarios_fills_missing_preview_from_install() {
        let _env_lock = crate::tests::env_lock().lock();
        reset_cached_app_paths();

        let install_dir = tempdir().unwrap();

        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let install_scenario_dir = install_dir.path().join("Scenarios").join("Alpha.c4s");
        fs::create_dir_all(&install_scenario_dir).unwrap();
        fs::write(
            install_scenario_dir.join("Scenario.json"),
            br#"{"name":"Install Alpha"}"#,
        )
        .unwrap();
        write_preview_png(
            &install_scenario_dir.join("Title.png"),
            [0x10, 0x20, 0x30, 0x40],
        );

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();
        let user_scenario_dir = user_dir.join("Scenarios").join("Alpha.c4s");
        fs::create_dir_all(&user_scenario_dir).unwrap();
        fs::write(
            user_scenario_dir.join("Scenario.json"),
            br#"{"name":"User Alpha"}"#,
        )
        .unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);

        let scenarios = load_frontend_scenarios();
        assert_eq!(scenarios.len(), 1, "duplicate scenario should be merged");
        let scenario = &scenarios[0];
        assert_eq!(scenario.title, "User Alpha");
        let preview = scenario.preview.as_ref().expect("merged preview");
        assert_eq!(preview.width(), 1);
        assert_eq!(preview.height(), 1);
        assert_eq!(preview.pixels(), &[0x10, 0x20, 0x30, 0x40]);

        reset_cached_app_paths();
    }

    #[test]
    fn load_frontend_scenarios_merges_folder_children_across_roots() {
        let _env_lock = crate::tests::env_lock().lock();
        reset_cached_app_paths();

        let install_dir = tempdir().unwrap();

        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let install_folder = install_dir.path().join("Scenarios").join("Worlds.c4f");
        fs::create_dir_all(&install_folder).unwrap();
        fs::write(install_folder.join("Folder.txt"), "Title=Worlds\n").unwrap();
        let install_scenario = install_folder.join("Alpha.c4s");
        fs::create_dir_all(&install_scenario).unwrap();
        fs::write(
            install_scenario.join("Scenario.json"),
            br#"{"name":"Alpha Install"}"#,
        )
        .unwrap();

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();
        let user_folder = user_dir.join("Scenarios").join("Worlds.c4f");
        fs::create_dir_all(&user_folder).unwrap();
        fs::write(user_folder.join("Folder.txt"), "Title=Worlds\n").unwrap();
        let user_scenario = user_folder.join("Beta.c4s");
        fs::create_dir_all(&user_scenario).unwrap();
        fs::write(
            user_scenario.join("Scenario.json"),
            br#"{"name":"Beta User"}"#,
        )
        .unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);

        let scenarios = load_frontend_scenarios();
        assert_eq!(
            scenarios.len(),
            1,
            "duplicate folders should merge instead of duplicating entries"
        );
        let folder = &scenarios[0];
        assert_eq!(folder.identifier, "Worlds.c4f");
        assert!(
            matches!(folder.kind, ScenarioKind::Folder),
            "expected merged entry to remain a folder"
        );
        assert_eq!(
            folder.children.len(),
            2,
            "merged folder should expose children from all roots"
        );
        let identifiers: Vec<_> = folder
            .children
            .iter()
            .map(|child| child.identifier.as_str())
            .collect();
        assert_eq!(
            identifiers,
            vec!["Worlds.c4f/Alpha.c4s", "Worlds.c4f/Beta.c4s"],
            "children should be sorted deterministically"
        );
        let user_entry = folder
            .children
            .iter()
            .find(|child| child.identifier == "Worlds.c4f/Beta.c4s")
            .expect("user scenario present");
        assert_eq!(user_entry.title, "Beta User");
        assert!(
            user_entry
                .path
                .as_ref()
                .map(|path| path.starts_with(&user_dir))
                .unwrap_or(false),
            "user scenario should retain user path"
        );
        let install_entry = folder
            .children
            .iter()
            .find(|child| child.identifier == "Worlds.c4f/Alpha.c4s")
            .expect("install scenario present");
        assert_eq!(install_entry.title, "Alpha Install");
        assert!(
            install_entry
                .path
                .as_ref()
                .map(|path| path.starts_with(&install_dir))
                .unwrap_or(false),
            "install scenario should retain install path"
        );

        reset_cached_app_paths();
    }

    #[test]
    fn load_frontend_scenarios_orders_folders_by_index() {
        let _env_lock = crate::tests::env_lock().lock();
        reset_cached_app_paths();

        let install_dir = tempdir().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let scenarios_dir = install_dir.path().join("Scenarios");
        fs::create_dir_all(&scenarios_dir).unwrap();

        let missions_folder = scenarios_dir.join("Missions.c4f");
        fs::create_dir_all(&missions_folder).unwrap();
        fs::write(
            missions_folder.join("Folder.txt"),
            "Title=Missions\nIndex=1\n",
        )
        .unwrap();

        let arcade_folder = scenarios_dir.join("Arcade.c4f");
        fs::create_dir_all(&arcade_folder).unwrap();
        fs::write(arcade_folder.join("Folder.txt"), "Title=Arcade\nIndex=2\n").unwrap();

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);

        let scenarios = load_frontend_scenarios();
        let identifiers: Vec<_> = scenarios
            .iter()
            .map(|entry| entry.identifier.as_str())
            .collect();
        assert_eq!(
            identifiers,
            vec!["Missions.c4f", "Arcade.c4f"],
            "folders should follow legacy indices"
        );

        reset_cached_app_paths();
    }

    #[test]
    fn load_frontend_scenarios_orders_by_icon_index() {
        let _env_lock = crate::tests::env_lock().lock();
        reset_cached_app_paths();

        let install_dir = tempdir().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let scenarios_dir = install_dir.path().join("Scenarios");
        let missions_folder = scenarios_dir.join("Missions.c4f");
        fs::create_dir_all(&missions_folder).unwrap();
        fs::write(missions_folder.join("Folder.txt"), "Title=Missions\n").unwrap();

        let bravo_dir = missions_folder.join("Bravo.c4s");
        fs::create_dir_all(&bravo_dir).unwrap();
        fs::write(
            bravo_dir.join("Scenario.txt"),
            "[Head]\nTitle=Bravo\nIcon=3\n",
        )
        .unwrap();

        let alpha_dir = missions_folder.join("Alpha.c4s");
        fs::create_dir_all(&alpha_dir).unwrap();
        fs::write(
            alpha_dir.join("Scenario.txt"),
            "[Head]\nTitle=Alpha\nIcon=5\n",
        )
        .unwrap();

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);

        let scenarios = load_frontend_scenarios();
        assert_eq!(scenarios.len(), 1, "expected single folder entry");
        let folder = &scenarios[0];
        assert_eq!(folder.identifier, "Missions.c4f");
        let titles: Vec<_> = folder
            .children
            .iter()
            .map(|child| child.title.as_str())
            .collect();
        assert_eq!(
            titles,
            vec!["Bravo", "Alpha"],
            "icon indices should order scenarios before title fallback"
        );

        reset_cached_app_paths();
    }

    #[test]
    fn load_frontend_scenarios_preserves_legacy_ordering() {
        let _env_lock = crate::tests::env_lock().lock();
        reset_cached_app_paths();

        let install_dir = tempdir().unwrap();
        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let install_folder = install_dir.path().join("Scenarios").join("Worlds.c4f");
        fs::create_dir_all(&install_folder).unwrap();
        fs::write(install_folder.join("Folder.txt"), "Title=Worlds\n").unwrap();
        let install_bravo = install_folder.join("Bravo.c4s");
        fs::create_dir_all(&install_bravo).unwrap();
        fs::write(
            install_bravo.join("Scenario.txt"),
            "[Head]\nTitle=Bravo\nDifficulty=1\n",
        )
        .unwrap();
        let install_charlie = install_folder.join("Charlie.c4s");
        fs::create_dir_all(&install_charlie).unwrap();
        fs::write(
            install_charlie.join("Scenario.txt"),
            "[Head]\nTitle=Charlie\nDifficulty=2\n",
        )
        .unwrap();

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();
        let user_folder = user_dir.join("Scenarios").join("Worlds.c4f");
        fs::create_dir_all(&user_folder).unwrap();
        fs::write(user_folder.join("Folder.txt"), "Title=Worlds\n").unwrap();
        let user_alpha = user_folder.join("Alpha.c4s");
        fs::create_dir_all(&user_alpha).unwrap();
        fs::write(
            user_alpha.join("Scenario.txt"),
            "[Head]\nTitle=Alpha Override\nDifficulty=3\n",
        )
        .unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);

        let scenarios = load_frontend_scenarios();
        assert_eq!(scenarios.len(), 1, "expected merged folder");
        let folder = &scenarios[0];
        assert_eq!(folder.identifier, "Worlds.c4f");
        let identifiers: Vec<_> = folder
            .children
            .iter()
            .map(|child| child.identifier.as_str())
            .collect();
        assert_eq!(
            identifiers,
            vec![
                "Worlds.c4f/Bravo.c4s",
                "Worlds.c4f/Charlie.c4s",
                "Worlds.c4f/Alpha.c4s"
            ],
            "merged children should follow legacy ordering rules"
        );
        assert_eq!(
            folder.children[2].title, "Alpha Override",
            "user override title should be retained"
        );
        assert!(
            folder.children[2]
                .path
                .as_ref()
                .map(|path| path.starts_with(&user_dir))
                .unwrap_or(false),
            "user override should keep user path"
        );

        reset_cached_app_paths();
    }

    #[test]
    fn load_frontend_scenarios_sets_human_readable_location() {
        reset_cached_app_paths();

        let install_dir = tempdir().unwrap();

        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();
        let scenario_dir = user_dir.join("Scenarios").join("Alpha.c4s");
        fs::create_dir_all(&scenario_dir).unwrap();
        fs::write(
            scenario_dir.join("Scenario.json"),
            br#"{"name":"Alpha Mission"}"#,
        )
        .unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);

        let scenarios = load_frontend_scenarios();
        assert_eq!(scenarios.len(), 1, "expected single scenario entry");
        let scenario = &scenarios[0];
        assert_eq!(
            scenario.location_label().as_deref(),
            Some("Scenarios / Alpha.c4s"),
            "location label should mirror catalog path"
        );

        reset_cached_app_paths();
    }

    #[test]
    fn scenario_roots_deduplicates_case_insensitive_variants() {
        reset_cached_app_paths();

        let install_dir = tempdir().unwrap();

        let planet_dir = install_dir.path().join("planet");
        fs::create_dir_all(&planet_dir).unwrap();
        fs::write(planet_dir.join("System.c4g"), b"stub").unwrap();

        let user_dir = install_dir.path().join("user-data");
        fs::create_dir_all(&user_dir).unwrap();
        let install_scenarios = install_dir.path().join("Scenarios");
        fs::create_dir_all(&install_scenarios).unwrap();

        let _guard = EnvGuard::set(&[
            ("LC_INSTALL_ROOT", Some(install_dir.path())),
            ("LC_USER_DATA_DIR", Some(user_dir.as_path())),
        ]);

        let paths = AppPaths::discover().expect("discover app paths");
        let roots = scenario_roots(&paths);

        let expected_key = scenario_root_key(&install_scenarios);
        let duplicate_count = roots
            .iter()
            .map(|root| scenario_root_key(&root.path))
            .filter(|key| key == &expected_key)
            .count();

        assert_eq!(
            duplicate_count, 1,
            "install scenarios path should appear once despite case variants"
        );

        reset_cached_app_paths();
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
                player_name: "Player".to_string(),
                network: None,
                record_enabled: false,
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
        wait_for_running(&mut app);

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
        let _env_lock = crate::tests::env_lock().lock();
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
        let thumbnail = path.with_extension("png");
        if thumbnail.exists() {
            let _ = std::fs::remove_file(&thumbnail);
        }
    }
}
